use super::migratable::Migratable;
use anyhow::Context;
use tracing::{info, warn};

pub struct UsernameHistoryMigration;

impl<'a> Migratable<'a> for UsernameHistoryMigration {
    async fn run(&self, db: &'a clickhouse::Client) -> anyhow::Result<()> {
        let partitions = db
            .query("SELECT DISTINCT toYYYYMM(timestamp) as partition FROM message_structured ORDER BY partition ASC")
            .fetch_all::<u32>()
            .await
            .context("Could not fetch partition list")?;

        db.query(
            "
            CREATE TABLE username_history
            (
                user_id String CODEC(ZSTD(8)),
                user_login String CODEC(ZSTD(8)),
                first_timestamp SimpleAggregateFunction(min, DateTime64(3)) CODEC(ZSTD(5)),
                last_timestamp SimpleAggregateFunction(max, DateTime64(3)) CODEC(ZSTD(5))
            )
            ENGINE = AggregatingMergeTree
            ORDER BY (user_id, user_login)
        ",
        )
        .execute()
        .await?;

        info!(partitions = partitions.len(), "Filling username history");

        for partition in partitions {
            info!(%partition, "Filling username history for partition");
            db.query(
                "
                INSERT INTO username_history
                SELECT
                    user_id,
                    user_login,
                    minSimpleState(timestamp) AS first_timestamp,
                    maxSimpleState(timestamp) AS last_timestamp
                FROM message_structured
                WHERE toYYYYMM(timestamp) = ?
                GROUP BY user_id, user_login
            ",
            )
            .bind(partition)
            .execute()
            .await
            .context("Could not fill username history")?;
        }

        db.query(
            "
            CREATE MATERIALIZED VIEW username_history_mv
            TO username_history
            AS SELECT
                user_id,
                user_login,
                minSimpleState(timestamp) AS first_timestamp,
                maxSimpleState(timestamp) AS last_timestamp
            FROM message_structured
            GROUP BY user_id, user_login
        ",
        )
        .execute()
        .await?;

        if let Err(err) = db.query("OPTIMIZE TABLE username_history").execute().await {
            warn!(error = %err, "Could not optimize the username history table");
        }

        info!("Filled username history");

        Ok(())
    }
}
