//! Reading, searching and deleting stored chat messages.

use super::{
    ACTIVE_USER_OPT_OUT_PREDICATE,
    message::StructuredMessage,
    stream::{FlushBufferResponse, LogsStream},
    writer::FlushBuffer,
};
use crate::domain::logs::LogsQuery;
use crate::storage::{Error, Result};
use chrono::{DateTime, Duration, Utc};
use clickhouse::{Client, query::RowCursor};
use rand::{rng, seq::IteratorRandom};
use tracing::debug;

const CHANNEL_MULTI_QUERY_SIZE_DAYS: i64 = 14;

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

    let mut query = format!(
        "SELECT ?fields FROM message_structured WHERE channel_id = ? AND {ACTIVE_USER_OPT_OUT_PREDICATE} AND timestamp >= ? AND timestamp < ? ORDER BY timestamp {suffix}"
    );

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

        debug!(
            queries = streams.len(),
            "Reading channel logs in several queries"
        );

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
    let mut query = format!(
        "SELECT * FROM message_structured WHERE channel_id = ? AND user_id = ? AND {ACTIVE_USER_OPT_OUT_PREDICATE} AND timestamp >= ? AND timestamp < ? ORDER BY timestamp {suffix}"
    );
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

    let mut query = format!(
        "SELECT * FROM message_structured WHERE channel_id = ? AND user_id = ? AND {ACTIVE_USER_OPT_OUT_PREDICATE} AND positionCaseInsensitive(text, ?) != 0 ORDER BY timestamp {suffix}"
    );
    apply_limit_offset(&mut query, &buffer_response);

    let cursor = db
        .query(&query)
        .bind(channel_id)
        .bind(user_id)
        .bind(search)
        .fetch()?;

    LogsStream::new_cursor(cursor, buffer_response).await
}

fn apply_limit_offset(query: &mut String, buffer_response: &FlushBufferResponse) {
    if let Some(limit) = buffer_response.normalized_limit() {
        *query = format!("{query} LIMIT {limit}");
    }
    if let Some(offset) = buffer_response.normalized_offset() {
        *query = format!("{query} OFFSET {offset}");
    }
}
