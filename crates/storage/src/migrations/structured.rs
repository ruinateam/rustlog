use super::migratable::Migratable;
use crate::message::{MESSAGES_STRUCTURED_TABLE, StructuredMessage, UnstructuredMessage};
use anyhow::{Context, bail};
use std::{
    env,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;
use tracing::{error, info};

// Keep structure migrations bounded so they do not overwhelm ClickHouse or the host.
const INSERT_BATCH_SIZE: u64 = 200_000;

pub struct StructuredMigration<'a> {
    pub db_name: &'a str,
}

impl<'a> Migratable<'a> for StructuredMigration<'a> {
    async fn run(&self, db: &'a clickhouse::Client) -> anyhow::Result<()> {
        db.query(
            "
CREATE TABLE message_structured
(
    `channel_id` LowCardinality(String) CODEC(ZSTD(8)),
    `channel_login` LowCardinality(String) CODEC(ZSTD(8)),
    `timestamp` DateTime64(3) CODEC(T64, ZSTD(5)),
    `id` UUID CODEC(ZSTD(1)),
    `message_type` UInt8 CODEC(ZSTD(8)),
    `user_id` String CODEC(ZSTD(8)),
    `user_login` String CODEC(ZSTD(8)),
    `display_name` String CODEC(ZSTD(8)),
    `color` Nullable(UInt32) CODEC(ZSTD(8)),
    `user_type` LowCardinality(String) CODEC(ZSTD(8)),
    `badges` Array(LowCardinality(String)) CODEC(ZSTD(8)),
    `badge_info` String CODEC(ZSTD(8)),
    `client_nonce` String CODEC(ZSTD(1)),
    `emotes` String CODEC(ZSTD(8)),
    `automod_flags` String CODEC(ZSTD(8)),
    `text` String CODEC(ZSTD(8)),
    `message_flags` UInt16 CODEC(ZSTD(8)),
    `extra_tags` Map(LowCardinality(String), String) CODEC(ZSTD(8)),
    PROJECTION channel_log_dates
    (
        SELECT
            channel_id,
            toDateTime(toStartOfDay(timestamp)) AS date
        GROUP BY
            channel_id,
            date
    )
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(timestamp)
ORDER BY (channel_id, user_id, timestamp)
        ",
        )
        .execute()
        .await?;

        let partitions = db
            .query("SELECT DISTINCT partition FROM system.parts WHERE database = ? AND table = 'message' ORDER BY partition ASC")
            .bind(self.db_name)
            .fetch_all::<String>()
            .await
            .context("could not fetch partition list")?;

        if partitions.len() > 1
            && env::var("RUSTLOG_ACKNOWLEDGE_STRUCTURE_MIGRATION").as_deref() != Ok("1")
        {
            bail!(
                "The current version of rustlog needs to perform a migration to a new database structure. This process can take from a few minutes to several hours depending on the database size. \
                The database will also increase in size up to a factor of 1.5x in the process, but after it's done it will become smaller. \
                Set the environment variable RUSTLOG_ACKNOWLEDGE_STRUCTURE_MIGRATION=1 to confirm and run the migration, or downgrade to an older version if you don't want to run it right now."
            );
        }

        info!(
            partitions = partitions.len(),
            "migrating messages to the structured table"
        );

        let i = Arc::new(AtomicU64::new(1));
        let semaphore = Arc::new(Semaphore::new(2));

        let started_at = Instant::now();

        let mut tasks = Vec::new();

        for partition in partitions {
            let _permit = semaphore.clone().acquire_owned().await?;

            let db = db.clone();
            let i = i.clone();
            let task = tokio::spawn(async move {
                let result = migrate_partition(partition, &db, i).await;
                drop(_permit);
                result
            });

            tasks.push(task);
        }

        for task in tasks {
            task.await.unwrap()?;
        }

        info!(
            messages = i.load(Ordering::SeqCst),
            took_secs = started_at.elapsed().as_secs(),
            "migrated messages to the structured table"
        );

        info!("dropping the old message table");
        if let Err(err) = db.query("DROP TABLE message").execute().await {
            error!(
                error = %err,
                "could not drop the old message table; drop it manually with `DROP TABLE message` to free the space"
            );
        }

        Ok(())
    }
}

async fn migrate_partition(
    partition: String,
    db: &clickhouse::Client,
    i: Arc<AtomicU64>,
) -> anyhow::Result<()> {
    info!(%partition, "migrating partition");

    let mut inserter = db
        .inserter::<StructuredMessage<'static>>(MESSAGES_STRUCTURED_TABLE)
        .with_timeouts(
            Some(Duration::from_secs(60)),
            Some(Duration::from_secs(300)),
        )
        .with_max_rows(INSERT_BATCH_SIZE)
        .with_period(Some(Duration::from_secs(15)));

    let mut cursor = db
        .query("SELECT * FROM message WHERE toYYYYMM(timestamp) = ?")
        .bind(&partition)
        .fetch::<UnstructuredMessage>()
        .with_context(|| format!("could not fetch messages for partition {partition}"))?;

    while let Some(unstructured_msg) = cursor.next().await? {
        match StructuredMessage::from_unstructured(&unstructured_msg) {
            Ok(msg) => {
                inserter.write(&msg).await.with_context(|| {
                    format!("could not write message for partition {partition}")
                })?;

                let stats = inserter
                    .commit()
                    .await
                    .with_context(|| format!("could not commit batch for partition {partition}"))?;
                if stats.rows > 0 {
                    info!(%partition, messages = stats.rows, "inserted messages from partition");
                }

                i.fetch_add(1, Ordering::Relaxed);
                let value = i.load(Ordering::Relaxed);
                if value.is_multiple_of(1_000_000) {
                    info!(messages = value, "migration progress");
                }
            }
            Err(err) => {
                error!(raw = %unstructured_msg.raw, error = format!("{err:#}"), "could not parse a stored IRC message");
            }
        }
    }

    inserter
        .end()
        .await
        .with_context(|| format!("could not finalize migration for partition {partition}"))?;
    info!(%partition, "migrated partition");

    Ok(())
}
