//! The days of logs stored locally.

use crate::tools::sql;
use anyhow::Context;
use chrono::NaiveDate;
use clickhouse::{Client, Row};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy)]
pub struct LocalDay {
    pub rows: u64,
    pub unique_ids: u64,
}

/// By channel login, then by UTC day.
pub type LocalDays = BTreeMap<String, BTreeMap<NaiveDate, LocalDay>>;

#[derive(Row, Deserialize)]
struct LocalDayRow {
    channel_login: String,
    day: String,
    rows: u64,
    unique_ids: u64,
}

pub async fn read(db: &Client, channels: &[String], year: i32) -> anyhow::Result<LocalDays> {
    let rows = db
        .query(&format!(
            "
            SELECT
                channel_login,
                toString(toDate(toTimeZone(timestamp, 'UTC'))) AS day,
                count() AS rows,
                uniqExact(id) AS unique_ids
            FROM message_structured
            WHERE channel_login IN ({channels})
              AND toYear(toTimeZone(timestamp, 'UTC')) = ?
            GROUP BY channel_login, day
            ORDER BY channel_login, day
            ",
            channels = sql::string_list(channels),
        ))
        .bind(year)
        .fetch_all::<LocalDayRow>()
        .await?;

    let mut days = LocalDays::new();
    for row in rows {
        let date = NaiveDate::parse_from_str(&row.day, "%Y-%m-%d")
            .with_context(|| format!("invalid ClickHouse date {}", row.day))?;
        days.entry(row.channel_login).or_default().insert(
            date,
            LocalDay {
                rows: row.rows,
                unique_ids: row.unique_ids,
            },
        );
    }
    Ok(days)
}
