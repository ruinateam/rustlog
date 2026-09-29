mod migrations;
pub mod schema;
pub mod stream;
pub mod writer;
use std::collections::HashSet;

pub use migrations::run as setup_db;
use serde::Deserialize;
use stream::{FlushBufferResponse, LogsStream};
use writer::FlushBuffer;

use crate::{
    domain::{
        logs::{LogDate, LogsQuery, TimeRange},
        stats::{NameHistoryEntry, UserMessageCount},
        tiers::UserWindows,
    },
    error::Error,
    Result,
};
use chrono::{DateTime, Duration, Utc};
use clickhouse::{query::RowCursor, Client, Row};
use rand::{rng, seq::IteratorRandom};
use schema::StructuredMessage;
use std::collections::HashMap;
use tracing::debug;

const CHANNEL_MULTI_QUERY_SIZE_DAYS: i64 = 14;

// Deletion mutations are asynchronous, so every read excludes opt-outs until
// ClickHouse has physically removed their historical rows.
const ACTIVE_USER_OPT_OUT_PREDICATE: &str = "user_id NOT IN (SELECT subject_id FROM (SELECT subject_id, argMax(opted_out, revision) AS opted_out FROM opt_out_state WHERE scope IN ('user', 'legacy') GROUP BY scope, subject_id) WHERE opted_out = 1)";

pub async fn read_channel(
    db: &Client,
    channel_id: &str,
    logs_query: LogsQuery,
    flush_buffer: &FlushBuffer,
    (from, to): (DateTime<Utc>, DateTime<Utc>),
) -> Result<LogsStream> {
    let buffer_response =
        FlushBufferResponse::new(flush_buffer, channel_id, None, logs_query, (from, to)).await;

    let suffix = if logs_query.reverse { "DESC" } else { "ASC" };

    let mut query = format!("SELECT ?fields FROM message_structured WHERE channel_id = ? AND {ACTIVE_USER_OPT_OUT_PREDICATE} AND timestamp >= ? AND timestamp < ? ORDER BY timestamp {suffix}");

    if to - from > Duration::days(CHANNEL_MULTI_QUERY_SIZE_DAYS) {
        let count = db
            .query(&format!("SELECT count() FROM (SELECT timestamp FROM message_structured WHERE channel_id = ? AND {ACTIVE_USER_OPT_OUT_PREDICATE} AND timestamp >= ? AND timestamp < ? LIMIT 1)"))
            .bind(channel_id)
            .bind(from.timestamp_millis() as f64 / 1000.0)
            .bind(to.timestamp_millis() as f64 / 1000.0)
            .fetch_one::<i32>().await?;
        if count == 0 {
            return Err(Error::NotFound);
        }

        let mut streams = Vec::with_capacity(1);

        let interval = Duration::days(CHANNEL_MULTI_QUERY_SIZE_DAYS);

        let mut current_from = from;
        let mut current_to = current_from + interval;

        loop {
            let cursor = next_cursor(db, &query, channel_id, current_from, current_to)?;
            streams.push(cursor);

            current_from += interval;
            current_to += interval;

            if current_to > to {
                let cursor = next_cursor(db, &query, channel_id, current_from, to)?;
                streams.push(cursor);
                break;
            }
        }

        if logs_query.reverse {
            streams.reverse();
        }

        debug!("Using {} queries for multi-query stream", streams.len());

        LogsStream::new_multi_query(streams, buffer_response)
    } else {
        apply_limit_offset(&mut query, &buffer_response);

        let cursor = db
            .query(&query)
            .bind(channel_id)
            .bind(from.timestamp_millis() as f64 / 1000.0)
            .bind(to.timestamp_millis() as f64 / 1000.0)
            .fetch()?;
        LogsStream::new_cursor(cursor, buffer_response).await
    }
}

fn next_cursor(
    db: &Client,
    query: &str,
    channel_id: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<RowCursor<StructuredMessage<'static>>> {
    let cursor = db
        .query(query)
        .bind(channel_id)
        .bind(from.timestamp_millis() as f64 / 1000.0)
        .bind(to.timestamp_millis() as f64 / 1000.0)
        .fetch()?;
    Ok(cursor)
}

pub async fn read_user(
    db: &Client,
    channel_id: &str,
    user_id: &str,
    logs_query: LogsQuery,
    flush_buffer: &FlushBuffer,
    (from, to): (DateTime<Utc>, DateTime<Utc>),
) -> Result<LogsStream> {
    let buffer_response = FlushBufferResponse::new(
        flush_buffer,
        channel_id,
        Some(user_id),
        logs_query,
        (from, to),
    )
    .await;

    let suffix = if logs_query.reverse { "DESC" } else { "ASC" };
    let mut query = format!("SELECT * FROM message_structured WHERE channel_id = ? AND user_id = ? AND {ACTIVE_USER_OPT_OUT_PREDICATE} AND timestamp >= ? AND timestamp < ? ORDER BY timestamp {suffix}");
    apply_limit_offset(&mut query, &buffer_response);

    let cursor = db
        .query(&query)
        .bind(channel_id)
        .bind(user_id)
        .bind(from.timestamp_millis() as f64 / 1000.0)
        .bind(to.timestamp_millis() as f64 / 1000.0)
        .fetch()?;
    LogsStream::new_cursor(cursor, buffer_response).await
}

#[derive(Row, Deserialize)]
struct AvailableDay {
    year: u16,
    month: u8,
    day: u8,
}

#[derive(Row, Deserialize)]
struct AvailableMonth {
    year: u16,
    month: u8,
}

// Ignore obviously broken timestamps (e.g., unix epoch -> 1970) when listing available logs.
const MIN_VALID_TIMESTAMP: &str = "2020-01-01 00:00:00";

pub async fn read_available_channel_logs(db: &Client, channel_id: &str) -> Result<Vec<LogDate>> {
    // Query year/month/day directly in UTC to avoid timezone skew when converting to chrono.
    let query = format!(
        "SELECT
                toYear(toTimeZone(timestamp, 'UTC'))        AS year,
                toMonth(toTimeZone(timestamp, 'UTC'))       AS month,
                toDayOfMonth(toTimeZone(timestamp, 'UTC'))  AS day
            FROM message_structured
            WHERE channel_id = ?
              AND {ACTIVE_USER_OPT_OUT_PREDICATE}
              AND timestamp >= toDateTime('{MIN_VALID_TIMESTAMP}')
            GROUP BY year, month, day
            ORDER BY year DESC, month DESC, day DESC"
    );

    let rows: Vec<AvailableDay> = db
        .query(query.as_str())
        .bind(channel_id)
        .fetch_all()
        .await?;

    let dates = rows
        .into_iter()
        .map(|row| LogDate {
            year: row.year,
            month: row.month,
            day: Some(row.day),
        })
        .collect();

    Ok(dates)
}

pub async fn read_available_user_logs(
    db: &Client,
    channel_id: &str,
    user_id: &str,
) -> Result<Vec<LogDate>> {
    // Query year/month directly in UTC to avoid local timezone skew.
    let query = format!(
        "SELECT
                toYear(toTimeZone(timestamp, 'UTC'))  AS year,
                toMonth(toTimeZone(timestamp, 'UTC')) AS month
            FROM message_structured
            WHERE channel_id = ? AND user_id = ?
              AND {ACTIVE_USER_OPT_OUT_PREDICATE}
              AND timestamp >= toDateTime('{MIN_VALID_TIMESTAMP}')
            GROUP BY year, month
            ORDER BY year DESC, month DESC"
    );

    let rows: Vec<AvailableMonth> = db
        .query(query.as_str())
        .bind(channel_id)
        .bind(user_id)
        .fetch_all()
        .await?;

    let dates = rows
        .into_iter()
        .map(|row| LogDate {
            year: row.year,
            month: row.month,
            day: None,
        })
        .collect();

    Ok(dates)
}

pub async fn read_random_user_line(
    db: &Client,
    channel_id: &str,
    user_id: &str,
) -> Result<StructuredMessage<'static>> {
    let total_count = db
        .query(&format!("SELECT count(*) FROM message_structured WHERE channel_id = ? AND user_id = ? AND {ACTIVE_USER_OPT_OUT_PREDICATE}"))
        .bind(channel_id)
        .bind(user_id)
        .fetch_one::<u64>()
        .await?;

    if total_count == 0 {
        return Err(Error::NotFound);
    }

    let offset = {
        let mut rng = rng();
        (0..total_count).choose(&mut rng).ok_or(Error::NotFound)
    }?;

    let mut cursor = db
        .query(&format!(
            "WITH
            (SELECT timestamp FROM message_structured WHERE channel_id = ? AND user_id = ? AND {ACTIVE_USER_OPT_OUT_PREDICATE} LIMIT 1 OFFSET ?)
            AS random_timestamp
            SELECT * FROM message_structured WHERE channel_id = ? AND user_id = ? AND {ACTIVE_USER_OPT_OUT_PREDICATE} AND timestamp = random_timestamp",
        ))
        .bind(channel_id)
        .bind(user_id)
        .bind(offset)
        .bind(channel_id)
        .bind(user_id)
        .fetch::<StructuredMessage<'static>>()?;

    let msg = cursor
        .next()
        .await?
        .map(StructuredMessage::into_owned)
        .ok_or(Error::NotFound)?;

    Ok(msg)
}

pub async fn read_random_channel_line(
    db: &Client,
    channel_id: &str,
) -> Result<StructuredMessage<'static>> {
    let total_count = db
        .query(&format!("SELECT count(*) FROM message_structured WHERE channel_id = ? AND {ACTIVE_USER_OPT_OUT_PREDICATE}"))
        .bind(channel_id)
        .fetch_one::<u64>()
        .await?;

    if total_count == 0 {
        return Err(Error::NotFound);
    }

    let offset = {
        let mut rng = rng();
        (0..total_count).choose(&mut rng).ok_or(Error::NotFound)
    }?;

    let mut cursor = db
        .query(&format!(
            "WITH
            (SELECT timestamp FROM message_structured WHERE channel_id = ? AND {ACTIVE_USER_OPT_OUT_PREDICATE} LIMIT 1 OFFSET ?)
            AS random_timestamp
            SELECT * FROM message_structured WHERE channel_id = ? AND {ACTIVE_USER_OPT_OUT_PREDICATE} AND timestamp = random_timestamp",
        ))
        .bind(channel_id)
        .bind(offset)
        .bind(channel_id)
        .fetch::<StructuredMessage<'static>>()?;

    let msg = cursor
        .next()
        .await?
        .map(StructuredMessage::into_owned)
        .ok_or(Error::NotFound)?;

    Ok(msg)
}

pub async fn delete_user_logs(db: &Client, user_id: &str) -> Result<()> {
    db.query("ALTER TABLE message_structured DELETE WHERE user_id = ?")
        .bind(user_id)
        .execute()
        .await?;
    db.query("ALTER TABLE username_history DELETE WHERE user_id = ?")
        .bind(user_id)
        .execute()
        .await?;
    Ok(())
}

pub async fn search_user_logs(
    db: &Client,
    channel_id: &str,
    user_id: &str,
    search: &str,
    logs_query: LogsQuery,
) -> Result<LogsStream> {
    let buffer_response = FlushBufferResponse::empty(logs_query);

    let suffix = if logs_query.reverse { "DESC" } else { "ASC" };

    let mut query = format!("SELECT * FROM message_structured WHERE channel_id = ? AND user_id = ? AND {ACTIVE_USER_OPT_OUT_PREDICATE} AND positionCaseInsensitive(text, ?) != 0 ORDER BY timestamp {suffix}");
    apply_limit_offset(&mut query, &buffer_response);

    let cursor = db
        .query(&query)
        .bind(channel_id)
        .bind(user_id)
        .bind(search)
        .fetch()?;

    LogsStream::new_cursor(cursor, buffer_response).await
}

#[derive(Deserialize, Row)]
pub struct StatsRow {
    pub cnt: u64,
    pub user_id: String,
}

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

pub async fn get_channel_stats(
    db: &Client,
    channel_id: &str,
    range: Option<TimeRange>,
) -> Result<(u64, Vec<StatsRow>)> {
    let mut query = format!(
        "SELECT count(*) FROM message_structured WHERE channel_id = ? AND {ACTIVE_USER_OPT_OUT_PREDICATE}"
    );

    if range.is_some() {
        query.push_str(" AND timestamp >= ? AND timestamp < ?");
    }

    let mut query = db.query(&query).bind(channel_id);

    if let Some(TimeRange { from, to }) = range {
        query = query
            .bind(from.timestamp_millis() as f64 / 1000.0)
            .bind(to.timestamp_millis() as f64 / 1000.0);
    }

    let total_count = query.fetch_one().await?;

    let mut query = format!(
        "SELECT count(*) as cnt, user_id FROM message_structured WHERE channel_id = ? AND user_id != '' AND {ACTIVE_USER_OPT_OUT_PREDICATE}"
    );

    if range.is_some() {
        query.push_str(" AND timestamp >= ? AND timestamp < ?");
    }

    query.push_str(" GROUP BY user_id ORDER BY cnt DESC LIMIT 5 SETTINGS use_query_cache = 1, query_cache_ttl = 300");

    let mut query = db.query(&query).bind(channel_id);

    if let Some(TimeRange { from, to }) = range {
        query = query
            .bind(from.timestamp_millis() as f64 / 1000.0)
            .bind(to.timestamp_millis() as f64 / 1000.0);
    }

    let stats_rows = query.fetch_all::<StatsRow>().await?;

    Ok((total_count, stats_rows))
}

pub async fn get_user_stats(
    db: &Client,
    channel_id: &str,
    user_id: String,
    user_login: Option<String>,
    range: Option<TimeRange>,
) -> Result<UserMessageCount> {
    let mut query = format!(
        "SELECT count(*) FROM message_structured WHERE channel_id = ? AND user_id = ? AND {ACTIVE_USER_OPT_OUT_PREDICATE}"
    );

    if range.is_some() {
        query.push_str(" AND timestamp >= ? AND timestamp < ?");
    }

    let mut query = db.query(&query).bind(channel_id).bind(&user_id);

    if let Some(TimeRange { from, to }) = range {
        query = query
            .bind(from.timestamp_millis() as f64 / 1000.0)
            .bind(to.timestamp_millis() as f64 / 1000.0);
    }

    let count = query.fetch_one().await?;

    Ok(UserMessageCount {
        message_count: count,
        user_login,
        user_id,
    })
}

pub async fn get_user_name_history(db: &Client, user_id: &str) -> Result<Vec<NameHistoryEntry>> {
    #[derive(Deserialize, Row)]
    struct SingleNameHistory {
        user_login: String,
        last_timestamp: i64,
        first_timestamp: i64,
    }

    let query = "
        SELECT trim(LEADING ':' FROM user_login) as user_login,
        max(last_timestamp) AS last_timestamp,
        min(first_timestamp) AS first_timestamp
        FROM username_history
        WHERE user_id = ?
        GROUP BY user_login";

    let name_history_rows: Vec<SingleNameHistory> =
        db.query(query).bind(user_id).fetch_all().await?;

    let mut seen_logins = HashSet::new();

    let names = name_history_rows
        .into_iter()
        .filter_map(|row| {
            if seen_logins.insert(row.user_login.clone()) {
                Some(NameHistoryEntry {
                    user_login: row.user_login,
                    last_seen: DateTime::from_timestamp_millis(row.last_timestamp)
                        .expect("Invalid DateTime"),
                    first_seen: DateTime::from_timestamp_millis(row.first_timestamp)
                        .expect("Invalid DateTime"),
                })
            } else {
                None
            }
        })
        .collect();

    Ok(names)
}

fn apply_limit_offset(query: &mut String, buffer_response: &FlushBufferResponse) {
    if let Some(limit) = buffer_response.normalized_limit() {
        *query = format!("{query} LIMIT {limit}");
    }
    if let Some(offset) = buffer_response.normalized_offset() {
        *query = format!("{query} OFFSET {offset}");
    }
}
