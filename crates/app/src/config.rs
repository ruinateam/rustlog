use crate::logging::{LogFormat, LoggingConfig};
use anyhow::Context;
use dashmap::DashMap;
use rustlog_storage::state::LegacyConfig;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::{collections::HashSet, sync::RwLock};
use std::{env, fs};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub clickhouse_url: String,
    pub clickhouse_db: String,
    pub clickhouse_username: Option<String>,
    pub clickhouse_password: Option<String>,
    #[serde(default = "clickhouse_flush_interval")]
    pub clickhouse_flush_interval: u64,
    #[serde(default = "default_listen_address")]
    pub listen_address: String,
    pub channels: RwLock<HashSet<String>>,
    #[serde(rename = "clientID")]
    pub client_id: String,
    pub client_secret: String,
    pub admins: Vec<String>,
    #[serde(default)]
    pub opt_out: DashMap<String, bool>,
    #[serde(rename = "adminAPIKey")]
    pub admin_api_key: Option<String>,
    #[serde(default)]
    pub supabase_url: Option<String>,
    #[serde(default)]
    pub supabase_service_key: Option<String>,
    #[serde(default)]
    pub enable_tier_snapshots: bool,
    #[serde(default)]
    pub logging: LoggingConfig,
}

impl Config {
    pub fn load(config_path: &Path) -> anyhow::Result<Self> {
        let contents = fs::read_to_string(config_path)
            .with_context(|| format!("failed to load config from {}", config_path.display()))?;
        let mut config: Self =
            serde_json::from_str(&contents).context("config deserialization error")?;

        if let Ok(value) = env::var("RUSTLOG_CLICKHOUSE_URL") {
            config.clickhouse_url = value;
        }
        if let Ok(value) = env::var("RUSTLOG_CLICKHOUSE_DB") {
            config.clickhouse_db = value;
        }
        if let Ok(value) = env::var("RUSTLOG_CLICKHOUSE_USERNAME") {
            config.clickhouse_username = Some(value);
        }
        if let Ok(value) = env::var("RUSTLOG_CLICKHOUSE_PASSWORD") {
            config.clickhouse_password = Some(value);
        }
        if let Ok(value) = env::var("RUSTLOG_LISTEN_ADDRESS") {
            config.listen_address = value;
        }
        if let Ok(value) = env::var("RUSTLOG_LOG_FORMAT") {
            config.logging.format = match value.as_str() {
                "text" => LogFormat::Text,
                "json" => LogFormat::Json,
                _ => anyhow::bail!("RUSTLOG_LOG_FORMAT must be `text` or `json`, got `{value}`"),
            };
        }

        Ok(config)
    }
}

impl Config {
    /// The channels and opt-outs that older versions kept in this file, for
    /// the migration that moved them to ClickHouse.
    pub fn legacy_state(&self) -> LegacyConfig {
        LegacyConfig {
            channels: self.channels.read().unwrap().iter().cloned().collect(),
            // Only the presence of a key counted, not its value.
            opted_out: self
                .opt_out
                .iter()
                .map(|entry| entry.key().clone())
                .collect(),
        }
    }
}

fn default_listen_address() -> String {
    String::from("0.0.0.0:8025")
}

fn clickhouse_flush_interval() -> u64 {
    10
}
