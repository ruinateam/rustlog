//! Per-user activity windows behind tier tables, counted in Moscow time.

use super::ACTIVE_USER_OPT_OUT_PREDICATE;
use crate::domain::{logs::TimeRange, tiers::UserWindows};
use crate::storage::Result;
use chrono::NaiveDate;
use clickhouse::{Client, Row};
use serde::Deserialize;

/// A calendar day or month in Moscow time.
#[derive(Debug, Clone, Copy)]
pub enum CalendarPeriod {
    Day(NaiveDate),
    Month { year: i32, month: u32 },
}

impl CalendarPeriod {
    /// SQL condition on the Moscow date of `timestamp`, bound to
    /// [`Self::sql_date`].
    fn sql_condition(self) -> &'static str {
        match self {
            CalendarPeriod::Day(_) => {
                "toDate(toTimeZone(toDateTime(timestamp), 'Europe/Moscow')) = toDate(?)"
            }
            CalendarPeriod::Month { .. } => {
                "toStartOfMonth(toTimeZone(toDateTime(timestamp), 'Europe/Moscow')) = toDate(?)"
            }
        }
    }

    /// The day, or the first day of the month, as `YYYY-MM-DD`.
    fn sql_date(self) -> String {
        match self {
            CalendarPeriod::Day(date) => date.format("%Y-%m-%d").to_string(),
            CalendarPeriod::Month { year, month } => format!("{year:04}-{month:02}-01"),
        }
    }
}

/// Which messages of the period count, relative to stream intervals.
#[derive(Debug, Clone, Copy)]
pub enum StreamFilter<'a> {
    All,
    DuringStreams(&'a [TimeRange]),
    OutsideStreams(&'a [TimeRange]),
}

/// Counts every user's messages and active 1, 5, 15, 30 and 60 minute
/// windows in a period. Users without any counted message are left out.
pub async fn user_windows(
    db: &Client,
    channel_id: &str,
    period: CalendarPeriod,
    filter: StreamFilter<'_>,
) -> Result<Vec<UserWindows>> {
    let rows = match filter {
        StreamFilter::All => all_windows(db, channel_id, period).await?,
        // Without streams nothing happened during one and everything outside.
        StreamFilter::DuringStreams([]) => Vec::new(),
        StreamFilter::OutsideStreams([]) => all_windows(db, channel_id, period).await?,
        StreamFilter::DuringStreams(streams) => {
            windows_relative_to_streams(db, channel_id, period, streams, true).await?
        }
        StreamFilter::OutsideStreams(streams) => {
            windows_relative_to_streams(db, channel_id, period, streams, false).await?
        }
    };

    Ok(rows.into_iter().map(UserWindows::from).collect())
}

#[derive(Deserialize, Row)]
struct UserWindowsRow {
    user_id: String,
    messages: u64,
    unique_messages: u64,
    windows_1m: u64,
    windows_5m: u64,
    windows_15m: u64,
    windows_30m: u64,
    windows_60m: u64,
}

impl From<UserWindowsRow> for UserWindows {
    fn from(row: UserWindowsRow) -> Self {
        Self {
            user_id: row.user_id,
            messages: row.messages,
            unique_messages: row.unique_messages,
            windows_1m: row.windows_1m,
            windows_5m: row.windows_5m,
            windows_15m: row.windows_15m,
            windows_30m: row.windows_30m,
            windows_60m: row.windows_60m,
        }
    }
}

async fn all_windows(
    db: &Client,
    channel_id: &str,
    period: CalendarPeriod,
) -> Result<Vec<UserWindowsRow>> {
    let period_condition = period.sql_condition();
    let query = format!(
        "
        SELECT
            user_id,
            count() AS messages,
            uniqExact(text) AS unique_messages,
            countDistinct(toStartOfInterval(toTimeZone(toDateTime(timestamp), 'Europe/Moscow'), INTERVAL 1 MINUTE))  AS windows_1m,
            countDistinct(toStartOfInterval(toTimeZone(toDateTime(timestamp), 'Europe/Moscow'), INTERVAL 5 MINUTE))  AS windows_5m,
            countDistinct(toStartOfInterval(toTimeZone(toDateTime(timestamp), 'Europe/Moscow'), INTERVAL 15 MINUTE)) AS windows_15m,
            countDistinct(toStartOfInterval(toTimeZone(toDateTime(timestamp), 'Europe/Moscow'), INTERVAL 30 MINUTE)) AS windows_30m,
            countDistinct(toStartOfInterval(toTimeZone(toDateTime(timestamp), 'Europe/Moscow'), INTERVAL 60 MINUTE)) AS windows_60m
         FROM message_structured
         WHERE channel_id = ?
           AND user_id != ''
           AND {ACTIVE_USER_OPT_OUT_PREDICATE}
           AND {period_condition}
        GROUP BY user_id
        HAVING windows_1m > 0 OR windows_5m > 0 OR windows_15m > 0 OR windows_30m > 0 OR windows_60m > 0
        ORDER BY windows_1m DESC
        "
    );

    Ok(db
        .query(&query)
        .bind(channel_id)
        .bind(period.sql_date())
        .fetch_all()
        .await?)
}

/// Counts only messages sent during (`during = true`) or outside the given
/// stream intervals, which must not be empty.
async fn windows_relative_to_streams(
    db: &Client,
    channel_id: &str,
    period: CalendarPeriod,
    streams: &[TimeRange],
    during: bool,
) -> Result<Vec<UserWindowsRow>> {
    let streams_sql = streams
        .iter()
        .map(|stream| {
            format!(
                "(toDateTime64({}, 3, 'Europe/Moscow'), toDateTime64({}, 3, 'Europe/Moscow'))",
                stream.from.timestamp_millis() as f64 / 1000.0,
                stream.to.timestamp_millis() as f64 / 1000.0
            )
        })
        .collect::<Vec<_>>()
        .join(", ");

    let counted = if during {
        "arrayExists(r -> ts >= r.1 AND ts < r.2, ranges)"
    } else {
        "NOT arrayExists(r -> ts >= r.1 AND ts < r.2, ranges)"
    };

    let period_condition = period.sql_condition();

    let query = format!(
        "
        WITH [{streams_sql}] AS ranges
        SELECT
            user_id,
            countIf({counted}) AS messages,
            uniqExactIf(text, {counted}) AS unique_messages,
            countDistinctIf(toStartOfInterval(ts, INTERVAL 1 MINUTE), {counted}) AS windows_1m,
            countDistinctIf(toStartOfInterval(ts, INTERVAL 5 MINUTE), {counted}) AS windows_5m,
            countDistinctIf(toStartOfInterval(ts, INTERVAL 15 MINUTE), {counted}) AS windows_15m,
            countDistinctIf(toStartOfInterval(ts, INTERVAL 30 MINUTE), {counted}) AS windows_30m,
            countDistinctIf(toStartOfInterval(ts, INTERVAL 60 MINUTE), {counted}) AS windows_60m
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
               AND {period_condition}
        )
        GROUP BY user_id
        HAVING windows_1m > 0 OR windows_5m > 0 OR windows_15m > 0 OR windows_30m > 0 OR windows_60m > 0
        "
    );

    Ok(db
        .query(&query)
        .bind(channel_id)
        .bind(period.sql_date())
        .fetch_all()
        .await?)
}
