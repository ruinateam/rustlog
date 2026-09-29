//! Message counts and username history.

use super::ACTIVE_USER_OPT_OUT_PREDICATE;
use crate::domain::{
    logs::TimeRange,
    stats::{NameHistoryEntry, UserMessageCount},
};
use crate::storage::Result;
use chrono::DateTime;
use clickhouse::{Client, Row};
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Deserialize, Row)]
pub struct StatsRow {
    pub cnt: u64,
    pub user_id: String,
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
                        .expect("invalid DateTime"),
                    first_seen: DateTime::from_timestamp_millis(row.first_timestamp)
                        .expect("invalid DateTime"),
                })
            } else {
                None
            }
        })
        .collect();

    Ok(names)
}
