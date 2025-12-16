use serde::Serialize;
use serde_json::Value;

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
    let url = format!("{}/rest/v1/rpc/upsert_tier_snapshot", cfg.url.trim_end_matches('/'));

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
