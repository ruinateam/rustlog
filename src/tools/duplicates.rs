//! `cleanup-duplicate-ids`: report and remove rows that repeat a message
//! id, which imports from several sources can leave behind.

use super::sql;
use anyhow::{Context, anyhow, bail};
use clickhouse::{Client, Row};
use serde::Deserialize;
use std::time::Duration;
use tracing::info;

/// Messages without an id have the nil id; they are never duplicates.
const NIL_UUID: &str = "00000000-0000-0000-0000-000000000000";

#[derive(Debug, Clone, clap::Args)]
pub struct CleanupDuplicateIdsOptions {
    /// Only this channel login. Repeatable.
    #[arg(long = "channel", value_name = "CHANNEL")]
    pub channels: Vec<String>,
    /// Only this UTC year
    #[arg(long)]
    pub year: Option<u32>,
    /// Delete the duplicates; without this flag only report them
    #[arg(long)]
    pub execute: bool,
    /// Number of duplicate ids to show as examples
    #[arg(long, default_value_t = 30)]
    pub sample_limit: usize,
    /// Seconds to wait for the ClickHouse mutation that deletes the duplicates
    #[arg(long, default_value_t = 600)]
    pub wait_timeout: u64,
}

#[derive(Row, Deserialize)]
struct DuplicateSummaryRow {
    channel_login: String,
    rows: u64,
    unique_ids: u64,
    duplicate_rows: u64,
}

#[derive(Row, Deserialize)]
struct DuplicateSampleRow {
    channel_login: String,
    message_id: String,
    copies: u64,
    first_timestamp: String,
    last_timestamp: String,
}

pub async fn run(db: Client, options: CleanupDuplicateIdsOptions) -> anyhow::Result<()> {
    let duplicates = summary(&db, &options).await?;
    if duplicates.is_empty() {
        info!("no duplicate message ids in scope");
        return Ok(());
    }

    for row in &duplicates {
        info!(
            channel = %row.channel_login,
            rows = row.rows,
            unique_ids = row.unique_ids,
            duplicate_rows = row.duplicate_rows,
            "duplicate message ids"
        );
    }

    let samples = samples(&db, &options).await?;
    for sample in samples {
        info!(
            channel = %sample.channel_login,
            message_id = %sample.message_id,
            copies = sample.copies,
            first = %sample.first_timestamp,
            last = %sample.last_timestamp,
            "duplicate message id sample"
        );
    }

    if !options.execute {
        info!("dry run: re-run with --execute to remove the duplicates");
        return Ok(());
    }

    delete_duplicates(&db, &options).await?;
    let remaining = summary(&db, &options).await?;
    if remaining.is_empty() {
        info!("duplicate cleanup finished");
        Ok(())
    } else {
        Err(anyhow!("duplicates remain after the cleanup"))
    }
}

async fn summary(
    db: &Client,
    options: &CleanupDuplicateIdsOptions,
) -> anyhow::Result<Vec<DuplicateSummaryRow>> {
    let filter = scope_filter(options);
    let query = format!(
        "
        SELECT
            channel_login,
            count() AS rows,
            uniqExact(id) AS unique_ids,
            toUInt64(rows - unique_ids) AS duplicate_rows
        FROM message_structured
        WHERE {filter}
        GROUP BY channel_login
        HAVING duplicate_rows > 0
        ORDER BY duplicate_rows DESC
        "
    );
    Ok(db.query(&query).fetch_all().await?)
}

async fn samples(
    db: &Client,
    options: &CleanupDuplicateIdsOptions,
) -> anyhow::Result<Vec<DuplicateSampleRow>> {
    let filter = scope_filter(options);
    let query = format!(
        "
        SELECT
            channel_login,
            toString(id) AS message_id,
            count() AS copies,
            toString(min(timestamp)) AS first_timestamp,
            toString(max(timestamp)) AS last_timestamp
        FROM message_structured
        WHERE {filter}
        GROUP BY channel_login, id
        HAVING copies > 1
        ORDER BY copies DESC, channel_login, id
        LIMIT {}
        ",
        options.sample_limit
    );
    Ok(db.query(&query).fetch_all().await?)
}

/// Keeps the earliest copy of every duplicated id and deletes the others.
///
/// Only the duplicated rows are touched: the kept copies are set aside, all
/// copies are deleted with a mutation, and the kept copies are inserted
/// back. Should the mutation not finish in time, the kept copies stay in
/// their table so that they can be inserted back by hand.
async fn delete_duplicates(
    db: &Client,
    options: &CleanupDuplicateIdsOptions,
) -> anyhow::Result<()> {
    let filter = scope_filter(options);
    let run_id = format!("{}_{}", std::process::id(), chrono::Utc::now().timestamp());
    let duplicate_ids_table = format!("message_structured_duplicate_ids_{run_id}");
    let kept_copies_table = format!("message_structured_kept_copies_{run_id}");

    info!(table = %duplicate_ids_table, "collecting duplicate message ids");
    db.query(&format!(
        "
        CREATE TABLE {duplicate_ids_table} (channel_login String, id UUID)
        ENGINE = MergeTree
        ORDER BY (channel_login, id)
        "
    ))
    .execute()
    .await?;
    db.query(&format!(
        "
        INSERT INTO {duplicate_ids_table}
        SELECT channel_login, id
        FROM message_structured
        WHERE {filter}
        GROUP BY channel_login, id
        HAVING count() > 1
        SETTINGS max_bytes_before_external_group_by = 1000000000
        "
    ))
    .execute()
    .await?;
    let duplicate_ids: u64 = db
        .query(&format!("SELECT count() FROM {duplicate_ids_table}"))
        .fetch_one()
        .await?;
    info!(ids = duplicate_ids, "collected duplicate message ids");

    let duplicates = format!(
        "{filter} AND (channel_login, id) IN (SELECT channel_login, id FROM {duplicate_ids_table})"
    );

    info!(table = %kept_copies_table, "setting the earliest copies aside");
    db.query(&format!(
        "CREATE TABLE {kept_copies_table} AS message_structured"
    ))
    .execute()
    .await?;
    db.query(&format!(
        "
        INSERT INTO {kept_copies_table}
        SELECT *
        FROM message_structured
        WHERE {duplicates}
        ORDER BY timestamp
        LIMIT 1 BY channel_login, id
        "
    ))
    .execute()
    .await?;
    let kept_copies: u64 = db
        .query(&format!("SELECT count() FROM {kept_copies_table}"))
        .fetch_one()
        .await?;
    if kept_copies != duplicate_ids {
        bail!(
            "set {kept_copies} copies aside for {duplicate_ids} duplicate ids; nothing was deleted"
        );
    }

    info!("deleting all copies of the duplicate ids");
    db.query(&format!(
        "ALTER TABLE message_structured DELETE WHERE {duplicates}"
    ))
    .execute()
    .await?;
    wait_for_mutations(db, Duration::from_secs(options.wait_timeout))
        .await
        .with_context(|| {
            format!(
                "the kept copies are in {kept_copies_table}: once the deletion is done, insert them into message_structured"
            )
        })?;

    info!(rows = kept_copies, "inserting the kept copies back");
    db.query(&format!(
        "INSERT INTO message_structured SELECT * FROM {kept_copies_table}"
    ))
    .execute()
    .await?;

    for table in [&duplicate_ids_table, &kept_copies_table] {
        db.query(&format!("DROP TABLE {table}")).execute().await?;
    }
    Ok(())
}

/// Waits until the mutations of `message_structured` are done.
async fn wait_for_mutations(db: &Client, timeout: Duration) -> anyhow::Result<()> {
    #[derive(Row, Deserialize)]
    struct PendingMutations {
        count: u64,
        fail_reason: String,
    }

    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let pending: PendingMutations = db
            .query(
                "
                SELECT count() AS count, max(latest_fail_reason) AS fail_reason
                FROM system.mutations
                WHERE database = currentDatabase()
                  AND table = 'message_structured'
                  AND is_done = 0
                ",
            )
            .fetch_one()
            .await?;
        if pending.count == 0 {
            return Ok(());
        }
        if !pending.fail_reason.is_empty() {
            bail!("the deletion failed: {}", pending.fail_reason);
        }
        if tokio::time::Instant::now() >= deadline {
            bail!(
                "the deletion did not finish within {} seconds",
                timeout.as_secs()
            );
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn scope_filter(options: &CleanupDuplicateIdsOptions) -> String {
    let mut filters = vec![format!("id != toUUID('{NIL_UUID}')")];
    if !options.channels.is_empty() {
        filters.push(format!(
            "channel_login IN ({})",
            sql::string_list(&options.channels)
        ));
    }
    if let Some(year) = options.year {
        filters.push(format!("toYear(toTimeZone(timestamp, 'UTC')) = {year}"));
    }
    filters.join(" AND ")
}
