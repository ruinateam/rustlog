//! Per-user activity windows behind tier tables.

use super::ACTIVE_USER_OPT_OUT_PREDICATE;
use crate::{domain::tiers::UserWindows, Result};
use clickhouse::{Client, Row};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Clone, Deserialize, Row)]
pub struct WindowsAggRow {
    pub user_id: String,
    pub messages: u64,
    pub uniq_messages: u64,
    pub w1: u64,
    pub w5: u64,
    pub w15: u64,
    pub w30: u64,
    pub w60: u64,
}

#[derive(Clone)]
pub struct WindowsAgg {
    pub messages: u64,
    pub uniq_messages: u64,
    pub w1: u64,
    pub w5: u64,
    pub w15: u64,
    pub w30: u64,
    pub w60: u64,
}

impl From<WindowsAggRow> for UserWindows {
    fn from(row: WindowsAggRow) -> Self {
        Self {
            user_id: row.user_id,
            messages: row.messages,
            uniq_messages: row.uniq_messages,
            w1: row.w1,
            w5: row.w5,
            w15: row.w15,
            w30: row.w30,
            w60: row.w60,
        }
    }
}

impl WindowsAgg {
    pub fn into_user_windows(self, user_id: String) -> UserWindows {
        UserWindows {
            user_id,
            messages: self.messages,
            uniq_messages: self.uniq_messages,
            w1: self.w1,
            w5: self.w5,
            w15: self.w15,
            w30: self.w30,
            w60: self.w60,
        }
    }
}

pub async fn get_day_windows(
    db: &Client,
    channel_id: &str,
    yyyymmdd: i32,
) -> Result<Vec<WindowsAggRow>> {
    // Aggregate fixed windows in MSK (Europe/Moscow) for a specific day.
    let rows = db
        .query(&format!(
            "
            SELECT
                user_id,
                count() AS messages,
                uniqExact(text) AS uniq_messages,
                countDistinct(toStartOfInterval(toTimeZone(toDateTime(timestamp), 'Europe/Moscow'), INTERVAL 1 MINUTE))  AS w1,
                countDistinct(toStartOfInterval(toTimeZone(toDateTime(timestamp), 'Europe/Moscow'), INTERVAL 5 MINUTE))  AS w5,
                countDistinct(toStartOfInterval(toTimeZone(toDateTime(timestamp), 'Europe/Moscow'), INTERVAL 15 MINUTE)) AS w15,
                countDistinct(toStartOfInterval(toTimeZone(toDateTime(timestamp), 'Europe/Moscow'), INTERVAL 30 MINUTE)) AS w30,
                countDistinct(toStartOfInterval(toTimeZone(toDateTime(timestamp), 'Europe/Moscow'), INTERVAL 60 MINUTE)) AS w60
             FROM message_structured
             WHERE channel_id = ?
               AND user_id != ''
               AND {ACTIVE_USER_OPT_OUT_PREDICATE}
               AND toYYYYMMDD(toTimeZone(toDateTime(timestamp), 'Europe/Moscow')) = ?
            GROUP BY user_id
            HAVING w1 > 0 OR w5 > 0 OR w15 > 0 OR w30 > 0 OR w60 > 0
            ORDER BY w1 DESC
            ",
        ))
        .bind(channel_id)
        .bind(yyyymmdd)
        .fetch_all::<WindowsAggRow>()
        .await?;

    Ok(rows)
}

pub async fn get_day_windows_with_ranges(
    db: &Client,
    channel_id: &str,
    yyyymmdd: i32,
    ranges: &[(i64, i64)],
    mode_online: bool,
) -> Result<HashMap<String, WindowsAgg>> {
    if ranges.is_empty() {
        if mode_online {
            return Ok(HashMap::new());
        }

        return Ok(get_day_windows(db, channel_id, yyyymmdd)
            .await?
            .into_iter()
            .map(|row| {
                (
                    row.user_id,
                    WindowsAgg {
                        messages: row.messages,
                        uniq_messages: row.uniq_messages,
                        w1: row.w1,
                        w5: row.w5,
                        w15: row.w15,
                        w30: row.w30,
                        w60: row.w60,
                    },
                )
            })
            .collect());
    }

    let ranges_sql = ranges
        .iter()
        .map(|(s, e)| {
            format!(
                "(toDateTime64({}, 3, 'Europe/Moscow'), toDateTime64({}, 3, 'Europe/Moscow'))",
                *s as f64 / 1000.0,
                *e as f64 / 1000.0
            )
        })
        .collect::<Vec<_>>()
        .join(", ");

    let condition = if mode_online {
        "arrayExists(r -> ts >= r.1 AND ts < r.2, ranges)"
    } else {
        "NOT arrayExists(r -> ts >= r.1 AND ts < r.2, ranges)"
    };

    let query = format!(
        "
        WITH [{ranges_sql}] AS ranges
        SELECT
            user_id,
            countIf({cond}) AS messages,
            uniqExactIf(text, {cond}) AS uniq_messages,
            countDistinctIf(toStartOfInterval(ts, INTERVAL 1 MINUTE), {cond}) AS w1,
            countDistinctIf(toStartOfInterval(ts, INTERVAL 5 MINUTE), {cond}) AS w5,
            countDistinctIf(toStartOfInterval(ts, INTERVAL 15 MINUTE), {cond}) AS w15,
            countDistinctIf(toStartOfInterval(ts, INTERVAL 30 MINUTE), {cond}) AS w30,
            countDistinctIf(toStartOfInterval(ts, INTERVAL 60 MINUTE), {cond}) AS w60
        FROM
        (
            SELECT
                user_id,
                toTimeZone(timestamp, 'Europe/Moscow') AS ts,
                text
             FROM message_structured
             WHERE channel_id = ?
               AND user_id != ''
               AND {ACTIVE_USER_OPT_OUT_PREDICATE}
               AND toYYYYMMDD(toTimeZone(toDateTime(timestamp), 'Europe/Moscow')) = ?
        )
        GROUP BY user_id
        HAVING w1 > 0 OR w5 > 0 OR w15 > 0 OR w30 > 0 OR w60 > 0
        ",
        ranges_sql = ranges_sql,
        cond = condition
    );

    let rows: Vec<WindowsAggRow> = db
        .query(&query)
        .bind(channel_id)
        .bind(yyyymmdd)
        .fetch_all()
        .await?;

    let mut map: HashMap<String, WindowsAgg> = HashMap::new();
    for row in rows {
        map.insert(
            row.user_id.clone(),
            WindowsAgg {
                messages: row.messages,
                uniq_messages: row.uniq_messages,
                w1: row.w1,
                w5: row.w5,
                w30: row.w30,
                w15: row.w15,
                w60: row.w60,
            },
        );
    }

    Ok(map)
}

pub async fn get_month_windows(
    db: &Client,
    channel_id: &str,
    yyyymm: i32,
) -> Result<Vec<WindowsAggRow>> {
    // Aggregate fixed windows in MSK (Europe/Moscow) for a specific month.
    let rows = db
        .query(&format!(
            "
            SELECT
                user_id,
                count() AS messages,
                uniqExact(text) AS uniq_messages,
                countDistinct(toStartOfInterval(toTimeZone(toDateTime(timestamp), 'Europe/Moscow'), INTERVAL 1 MINUTE))  AS w1,
                countDistinct(toStartOfInterval(toTimeZone(toDateTime(timestamp), 'Europe/Moscow'), INTERVAL 5 MINUTE))  AS w5,
                countDistinct(toStartOfInterval(toTimeZone(toDateTime(timestamp), 'Europe/Moscow'), INTERVAL 15 MINUTE)) AS w15,
                countDistinct(toStartOfInterval(toTimeZone(toDateTime(timestamp), 'Europe/Moscow'), INTERVAL 30 MINUTE)) AS w30,
                countDistinct(toStartOfInterval(toTimeZone(toDateTime(timestamp), 'Europe/Moscow'), INTERVAL 60 MINUTE)) AS w60
             FROM message_structured
             WHERE channel_id = ?
               AND user_id != ''
               AND {ACTIVE_USER_OPT_OUT_PREDICATE}
               AND toYYYYMM(toTimeZone(toDateTime(timestamp), 'Europe/Moscow')) = ?
            GROUP BY user_id
            HAVING w1 > 0 OR w5 > 0 OR w15 > 0 OR w30 > 0 OR w60 > 0
            ORDER BY w1 DESC
            ",
        ))
        .bind(channel_id)
        .bind(yyyymm)
        .fetch_all::<WindowsAggRow>()
        .await?;

    Ok(rows)
}

pub async fn get_month_windows_with_ranges(
    db: &Client,
    channel_id: &str,
    yyyymm: i32,
    ranges: &[(i64, i64)],
    mode_online: bool,
) -> Result<HashMap<String, WindowsAgg>> {
    if ranges.is_empty() {
        if mode_online {
            return Ok(HashMap::new());
        }

        return Ok(get_month_windows(db, channel_id, yyyymm)
            .await?
            .into_iter()
            .map(|row| {
                (
                    row.user_id,
                    WindowsAgg {
                        messages: row.messages,
                        uniq_messages: row.uniq_messages,
                        w1: row.w1,
                        w5: row.w5,
                        w15: row.w15,
                        w30: row.w30,
                        w60: row.w60,
                    },
                )
            })
            .collect());
    }

    let ranges_sql = ranges
        .iter()
        .map(|(s, e)| {
            format!(
                "(toDateTime64({}, 3, 'Europe/Moscow'), toDateTime64({}, 3, 'Europe/Moscow'))",
                *s as f64 / 1000.0,
                *e as f64 / 1000.0
            )
        })
        .collect::<Vec<_>>()
        .join(", ");

    let condition = if mode_online {
        "arrayExists(r -> ts >= r.1 AND ts < r.2, ranges)"
    } else {
        "NOT arrayExists(r -> ts >= r.1 AND ts < r.2, ranges)"
    };

    let query = format!(
        "
        WITH [{ranges_sql}] AS ranges
        SELECT
            user_id,
            countIf({cond}) AS messages,
            uniqExactIf(text, {cond}) AS uniq_messages,
            countDistinctIf(toStartOfInterval(ts, INTERVAL 1 MINUTE), {cond}) AS w1,
            countDistinctIf(toStartOfInterval(ts, INTERVAL 5 MINUTE), {cond}) AS w5,
            countDistinctIf(toStartOfInterval(ts, INTERVAL 15 MINUTE), {cond}) AS w15,
            countDistinctIf(toStartOfInterval(ts, INTERVAL 30 MINUTE), {cond}) AS w30,
            countDistinctIf(toStartOfInterval(ts, INTERVAL 60 MINUTE), {cond}) AS w60
        FROM
        (
            SELECT
                user_id,
                toTimeZone(timestamp, 'Europe/Moscow') AS ts,
                text
             FROM message_structured
             WHERE channel_id = ?
               AND user_id != ''
               AND {ACTIVE_USER_OPT_OUT_PREDICATE}
               AND toYYYYMM(toTimeZone(toDateTime(timestamp), 'Europe/Moscow')) = ?
        )
        GROUP BY user_id
        HAVING w1 > 0 OR w5 > 0 OR w15 > 0 OR w30 > 0 OR w60 > 0
        ",
        ranges_sql = ranges_sql,
        cond = condition
    );

    let rows: Vec<WindowsAggRow> = db
        .query(&query)
        .bind(channel_id)
        .bind(yyyymm)
        .fetch_all()
        .await?;

    let mut map = HashMap::new();
    for row in rows {
        map.insert(
            row.user_id.clone(),
            WindowsAgg {
                messages: row.messages,
                uniq_messages: row.uniq_messages,
                w1: row.w1,
                w5: row.w5,
                w15: row.w15,
                w30: row.w30,
                w60: row.w60,
            },
        );
    }

    Ok(map)
}
