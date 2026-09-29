//! Request and response types of the v2 API.

use crate::logs_response::{JsonResponseType, LogsResponseType};
use chrono::{DateTime, Utc};
use rustlog_domain::logs::LogDate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResolveUserQuery {
    pub login: String,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedUser {
    pub id: String,
    pub login: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AvailabilityQuery {
    pub user_id: Option<String>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AvailableLogs {
    pub available_logs: Vec<AvailableLogDate>,
}

#[derive(Serialize, JsonSchema)]
pub struct AvailableLogDate {
    pub year: String,
    pub month: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub day: Option<String>,
}

impl From<LogDate> for AvailableLogDate {
    fn from(date: LogDate) -> Self {
        Self {
            year: date.year.to_string(),
            month: date.month.to_string(),
            day: date.day.map(|day| day.to_string()),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "kebab-case")]
pub enum LogFormat {
    #[default]
    BasicJson,
    FullJson,
    Ndjson,
    Text,
    Raw,
}

impl From<LogFormat> for LogsResponseType {
    fn from(format: LogFormat) -> Self {
        match format {
            LogFormat::BasicJson => Self::Json(JsonResponseType::Basic),
            LogFormat::FullJson => Self::Json(JsonResponseType::Full),
            LogFormat::Ndjson => Self::NdJson,
            LogFormat::Text => Self::Text,
            LogFormat::Raw => Self::Raw,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserLogsQuery {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    #[serde(default)]
    pub format: LogFormat,
    #[serde(default)]
    pub reverse: bool,
    #[schemars(range(min = 1))]
    pub limit: Option<u64>,
    #[schemars(range(min = 0))]
    pub offset: Option<u64>,
}
