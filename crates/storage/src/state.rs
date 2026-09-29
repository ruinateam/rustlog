use crate::Result;
use clickhouse::{Client, Row};
use dashmap::DashSet;
use serde::Deserialize;
use std::{collections::BTreeSet, sync::Arc};
use tokio::sync::Mutex;

const CHANNEL_MEMBERSHIP_TABLE: &str = "channel_membership_state";
const OPT_OUT_TABLE: &str = "opt_out_state";
const LEGACY_REVISION: u64 = 1;

#[derive(Clone)]
pub struct OperationalState {
    db: Arc<Client>,
    channels: Arc<DashSet<String>>,
    opted_out_users: Arc<DashSet<String>>,
    opted_out_channels: Arc<DashSet<String>>,
    legacy_opt_out: Arc<DashSet<String>>,
    transition_lock: Arc<Mutex<()>>,
}

#[derive(Deserialize, Row)]
struct IdRow {
    subject_id: String,
}

impl OperationalState {
    pub async fn load(db: Arc<Client>) -> Result<Self> {
        let channels = db
            .query(
                "
                SELECT channel_id
                FROM
                (
                    SELECT channel_id, argMax(enabled, revision) AS enabled
                    FROM channel_membership_state
                    GROUP BY channel_id
                )
                WHERE enabled = 1
                ",
            )
            .fetch_all::<ChannelIdRow>()
            .await?
            .into_iter()
            .map(|row| row.channel_id)
            .collect();

        let opted_out_users = load_opted_out(&db, "user").await?;
        let opted_out_channels = load_opted_out(&db, "channel").await?;
        let legacy_opt_out = load_opted_out(&db, "legacy").await?;

        Ok(Self {
            db,
            channels: Arc::new(channels),
            opted_out_users: Arc::new(opted_out_users),
            opted_out_channels: Arc::new(opted_out_channels),
            legacy_opt_out: Arc::new(legacy_opt_out),
            transition_lock: Arc::default(),
        })
    }

    pub fn channel_ids(&self) -> BTreeSet<String> {
        self.channels
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    pub fn is_channel_enabled(&self, channel_id: &str) -> bool {
        self.channels.contains(channel_id)
    }

    pub fn is_channel_opted_out(&self, channel_id: &str) -> bool {
        self.opted_out_channels.contains(channel_id) || self.legacy_opt_out.contains(channel_id)
    }

    pub fn is_user_opted_out(&self, user_id: &str) -> bool {
        self.opted_out_users.contains(user_id) || self.legacy_opt_out.contains(user_id)
    }

    pub fn is_loggable(&self, channel_id: &str, user_id: &str) -> bool {
        self.is_channel_enabled(channel_id)
            && !self.is_channel_opted_out(channel_id)
            && !self.is_user_opted_out(user_id)
    }

    pub fn permits_historical_message(&self, channel_id: &str, user_id: &str) -> bool {
        !self.is_channel_opted_out(channel_id) && !self.is_user_opted_out(user_id)
    }

    pub async fn enable_channel(&self, channel_id: &str) -> Result<()> {
        self.set_channel_enabled(channel_id, true).await
    }

    pub async fn disable_channel(&self, channel_id: &str) -> Result<()> {
        self.set_channel_enabled(channel_id, false).await
    }

    pub async fn optout_user(&self, user_id: &str) -> Result<()> {
        let _transition = self.transition_lock.lock().await;
        let revision = next_revision(&self.db, OPT_OUT_TABLE).await?;

        self.db
            .query(
                "INSERT INTO opt_out_state (scope, subject_id, opted_out, revision, changed_at) VALUES ('user', ?, 1, ?, now64(3, 'UTC'))",
            )
            .bind(user_id)
            .bind(revision)
            .execute()
            .await?;

        self.opted_out_users.insert(user_id.to_owned());
        Ok(())
    }

    async fn set_channel_enabled(&self, channel_id: &str, enabled: bool) -> Result<()> {
        let _transition = self.transition_lock.lock().await;
        let revision = next_revision(&self.db, CHANNEL_MEMBERSHIP_TABLE).await?;

        self.db
            .query(
                "INSERT INTO channel_membership_state (channel_id, enabled, revision, changed_at) VALUES (?, ?, ?, now64(3, 'UTC'))",
            )
            .bind(channel_id)
            .bind(u8::from(enabled))
            .bind(revision)
            .execute()
            .await?;

        if enabled {
            self.channels.insert(channel_id.to_owned());
        } else {
            self.channels.remove(channel_id);
        }
        Ok(())
    }
}

#[derive(Deserialize, Row)]
struct ChannelIdRow {
    channel_id: String,
}

async fn load_opted_out(db: &Client, scope: &str) -> Result<DashSet<String>> {
    let rows = db
        .query(
            "
            SELECT subject_id
            FROM
            (
                SELECT subject_id, argMax(opted_out, revision) AS opted_out
                FROM opt_out_state
                WHERE scope = ?
                GROUP BY subject_id
            )
            WHERE opted_out = 1
            ",
        )
        .bind(scope)
        .fetch_all::<IdRow>()
        .await?;

    Ok(rows.into_iter().map(|row| row.subject_id).collect())
}

async fn next_revision(db: &Client, table: &str) -> Result<u64> {
    Ok(db
        .query(&format!("SELECT ifNull(max(revision), 0) + 1 FROM {table}"))
        .fetch_one()
        .await?)
}

/// The channels and opt-outs that older versions kept in the config file.
#[derive(Debug, Default)]
pub struct LegacyConfig {
    /// Ids of the logged channels.
    pub channels: Vec<String>,
    /// Ids of the users and channels that opted out.
    pub opted_out: Vec<String>,
}

pub async fn migrate_legacy_config(db: &Client, legacy: &LegacyConfig) -> anyhow::Result<()> {
    db.query(
        "
        CREATE TABLE IF NOT EXISTS channel_membership_state
        (
            channel_id String CODEC(ZSTD(8)),
            enabled UInt8,
            revision UInt64,
            changed_at DateTime64(3, 'UTC')
        )
        ENGINE = ReplacingMergeTree(revision)
        ORDER BY channel_id
        ",
    )
    .execute()
    .await?;

    db.query(
        "
        CREATE TABLE IF NOT EXISTS opt_out_state
        (
            scope Enum8('user' = 1, 'channel' = 2, 'legacy' = 3),
            subject_id String CODEC(ZSTD(8)),
            opted_out UInt8,
            revision UInt64,
            changed_at DateTime64(3, 'UTC')
        )
        ENGINE = ReplacingMergeTree(revision)
        ORDER BY (scope, subject_id)
        ",
    )
    .execute()
    .await?;

    for channel_id in &legacy.channels {
        db.query(
            "INSERT INTO channel_membership_state (channel_id, enabled, revision, changed_at) VALUES (?, 1, ?, now64(3, 'UTC'))",
        )
        .bind(channel_id)
        .bind(LEGACY_REVISION)
        .execute()
        .await?;
    }

    for subject_id in legacy.opted_out.iter().collect::<BTreeSet<_>>() {
        db.query(
            "INSERT INTO opt_out_state (scope, subject_id, opted_out, revision, changed_at) VALUES ('legacy', ?, 1, ?, now64(3, 'UTC'))",
        )
        .bind(subject_id)
        .bind(LEGACY_REVISION)
        .execute()
        .await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::OperationalState;
    use dashmap::DashSet;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn test_state(channels: &[&str], users: &[&str], channel_optouts: &[&str]) -> OperationalState {
        OperationalState {
            db: Arc::new(clickhouse::Client::default()),
            channels: Arc::new(channels.iter().map(|id| (*id).to_owned()).collect()),
            opted_out_users: Arc::new(users.iter().map(|id| (*id).to_owned()).collect()),
            opted_out_channels: Arc::new(
                channel_optouts.iter().map(|id| (*id).to_owned()).collect(),
            ),
            legacy_opt_out: Arc::new(DashSet::new()),
            transition_lock: Arc::new(Mutex::new(())),
        }
    }

    #[test]
    fn logging_requires_active_channel_and_no_opt_out() {
        let state = test_state(&["channel"], &["user"], &[]);
        assert!(!state.is_loggable("channel", "user"));
        assert!(!state.permits_historical_message("channel", "user"));
        assert!(!state.is_loggable("missing", "other"));
        assert!(state.permits_historical_message("missing", "other"));

        let state = test_state(&["channel"], &[], &["channel"]);
        assert!(!state.is_loggable("channel", "other"));
        assert!(!state.permits_historical_message("channel", "other"));

        let state = test_state(&["channel"], &[], &[]);
        assert!(state.is_loggable("channel", "other"));
    }
}
