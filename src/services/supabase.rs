//! Tier table snapshots written to Supabase when `enableTierSnapshots` is on.

use crate::{
    config::Config,
    domain::tiers::{RankedTiers, TierEntry, TierMode, TierPeriod},
};
use serde::Serialize;
use serde_json::Value;
use tracing::warn;

#[derive(Clone)]
pub struct SupabaseConfig {
    pub url: String,
    pub service_key: String,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub struct TierSnapshotPayload {
    pub p_channel: String,
    pub p_scope: String,
    pub p_period_key: String,
    pub p_mode: String,
    pub p_total_users: i32,
    pub p_total_messages: i32,
    pub p_total_unique_messages: i32,
    pub p_entries: Value,
}

pub async fn write_tier_snapshot(
    cfg: SupabaseConfig,
    payload: TierSnapshotPayload,
) -> anyhow::Result<()> {
    let url = format!(
        "{}/rest/v1/rpc/upsert_tier_snapshot",
        cfg.url.trim_end_matches('/')
    );

    let client = reqwest::Client::new();
    let res = client
        .post(&url)
        .header("apikey", &cfg.service_key)
        .header("Authorization", format!("Bearer {}", cfg.service_key))
        .json(&payload)
        .send()
        .await?;

    if !res.status().is_success() {
        let body = res.text().await.unwrap_or_default();
        anyhow::bail!("Supabase RPC failed: {}", body);
    }

    Ok(())
}

/// Writes a snapshot of a tier table in the background, if snapshots are
/// enabled and configured. Failures are only logged.
pub fn spawn_tier_snapshot(
    config: &Config,
    channel: &str,
    period: TierPeriod,
    mode: TierMode,
    tiers: &RankedTiers,
) {
    if !config.enable_tier_snapshots {
        return;
    }

    let Some(url) = config.supabase_url.clone() else {
        return;
    };
    let Some(service_key) = config.supabase_service_key.clone() else {
        return;
    };

    let p_total_users = match i32::try_from(tiers.total_users) {
        Ok(v) => v,
        Err(_) => {
            warn!(
                total_users = tiers.total_users,
                "Skipping tier snapshot: too many users"
            );
            return;
        }
    };
    let p_total_messages = match i32::try_from(tiers.total_messages) {
        Ok(v) => v,
        Err(_) => {
            warn!(
                total_messages = tiers.total_messages,
                "Skipping tier snapshot: too many messages"
            );
            return;
        }
    };
    let p_total_unique_messages = match i32::try_from(tiers.total_unique_messages) {
        Ok(v) => v,
        Err(_) => {
            warn!(
                total_unique_messages = tiers.total_unique_messages,
                "Skipping tier snapshot: too many unique messages"
            );
            return;
        }
    };

    let entries: Vec<SnapshotEntry> = tiers.entries.iter().map(SnapshotEntry::from).collect();
    let entries_json = match serde_json::to_value(entries) {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "Skipping tier snapshot: could not serialize the entries");
            return;
        }
    };

    let payload = TierSnapshotPayload {
        p_channel: channel.to_string(),
        p_scope: period.scope().to_string(),
        p_period_key: period.key(),
        p_mode: mode.as_str().to_string(),
        p_total_users,
        p_total_messages,
        p_total_unique_messages,
        p_entries: entries_json,
    };

    let cfg = SupabaseConfig { url, service_key };
    tokio::spawn(async move {
        if let Err(e) = write_tier_snapshot(cfg, payload).await {
            warn!(
                error = format!("{e:#}"),
                "Could not write the tier snapshot to Supabase"
            );
        }
    });
}

/// A tier entry as stored in snapshots. This is a contract with the Supabase
/// schema, so it must keep its JSON shape.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotEntry<'a> {
    user_id: &'a str,
    user_login: Option<&'a str>,
    messages: u64,
    unique_messages: u64,
    windows_1m: u64,
    windows_5m: u64,
    windows_15m: u64,
    windows_30m: u64,
    windows_60m: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    rank_1m: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tier_1m: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rank_5m: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tier_5m: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rank_15m: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tier_15m: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rank_30m: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tier_30m: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rank_60m: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tier_60m: Option<&'a str>,
    tier_score: u32,
}

impl<'a> From<&'a TierEntry> for SnapshotEntry<'a> {
    fn from(entry: &'a TierEntry) -> Self {
        Self {
            user_id: &entry.user_id,
            user_login: entry.user_login.as_deref(),
            messages: entry.messages,
            unique_messages: entry.unique_messages,
            windows_1m: entry.windows_1m,
            windows_5m: entry.windows_5m,
            windows_15m: entry.windows_15m,
            windows_30m: entry.windows_30m,
            windows_60m: entry.windows_60m,
            rank_1m: entry.rank_1m,
            tier_1m: entry.tier_1m.as_deref(),
            rank_5m: entry.rank_5m,
            tier_5m: entry.tier_5m.as_deref(),
            rank_15m: entry.rank_15m,
            tier_15m: entry.tier_15m.as_deref(),
            rank_30m: entry.rank_30m,
            tier_30m: entry.tier_30m.as_deref(),
            rank_60m: entry.rank_60m,
            tier_60m: entry.tier_60m.as_deref(),
            tier_score: entry.tier_score,
        }
    }
}
