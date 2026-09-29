//! The days and months for which logs exist.

use super::ACTIVE_USER_OPT_OUT_PREDICATE;
use crate::{domain::logs::LogDate, Result};
use clickhouse::{Client, Row};
use serde::Deserialize;

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
