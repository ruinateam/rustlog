use super::responders::logs::{JsonResponseType, LogsResponseType};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt::Display;
use strum::Display;

#[derive(Serialize, JsonSchema)]
pub struct ChannelsList {
    pub channels: Vec<Channel>,
}

#[derive(Serialize, JsonSchema)]
pub struct Channel {
    pub name: String,
    #[serde(rename = "userID")]
    pub user_id: String,
}

#[derive(Debug, Deserialize, JsonSchema, Display)]
pub enum ChannelIdType {
    #[serde(rename = "channel")]
    #[strum(serialize = "channel")]
    Name,
    #[serde(rename = "channelid")]
    #[strum(serialize = "channelid")]
    Id,
}

#[derive(Debug, Deserialize, JsonSchema, Display)]
pub enum UserIdType {
    #[serde(rename = "user")]
    #[strum(serialize = "user")]
    Name,
    #[serde(rename = "userid")]
    #[strum(serialize = "userid")]
    Id,
}

#[derive(Deserialize, JsonSchema)]
pub struct UserLogsDatePath {
    pub year: String,
    pub month: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct ChannelLogsByDatePath {
    #[serde(flatten)]
    pub channel_info: LogsPathChannel,
    #[serde(flatten)]
    pub date: LogsPathDate,
}

#[derive(Deserialize, JsonSchema)]
pub struct LogsPathDate {
    pub year: String,
    pub month: String,
    pub day: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct LogsPathChannel {
    pub channel_id_type: ChannelIdType,
    pub channel: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct ChannelMonthPath {
    #[serde(flatten)]
    pub channel_info: LogsPathChannel,
    pub year: String,
    pub month: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct ChannelDayPath {
    #[serde(flatten)]
    pub channel_info: LogsPathChannel,
    pub year: String,
    pub month: String,
    pub day: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct ChannelYearPath {
    #[serde(flatten)]
    pub channel_info: LogsPathChannel,
    pub year: String,
}

#[derive(Deserialize, Debug, JsonSchema, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct LogsParams {
    #[serde(default, deserialize_with = "deserialize_bool_param")]
    pub json: bool,
    #[serde(default, deserialize_with = "deserialize_bool_param")]
    pub json_basic: bool,
    #[serde(default, deserialize_with = "deserialize_bool_param")]
    pub raw: bool,
    #[serde(default, deserialize_with = "deserialize_bool_param")]
    pub reverse: bool,
    #[serde(default, deserialize_with = "deserialize_bool_param")]
    pub ndjson: bool,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
}

impl LogsParams {
    pub fn response_type(&self) -> LogsResponseType {
        if self.raw {
            LogsResponseType::Raw
        } else if self.json_basic {
            LogsResponseType::Json(JsonResponseType::Basic)
        } else if self.json {
            LogsResponseType::Json(JsonResponseType::Full)
        } else if self.ndjson {
            LogsResponseType::NdJson
        } else {
            LogsResponseType::Text
        }
    }
}

fn deserialize_bool_param<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<&str>::deserialize(deserializer)?.is_some())
}

#[derive(Deserialize, Debug, JsonSchema)]
pub struct SearchParams {
    pub q: String,
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

impl Display for AvailableLogDate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.year, self.month)?;

        if let Some(day) = &self.day {
            write!(f, "/{day}")?;
        }

        Ok(())
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct AvailableLogsParams {
    #[serde(flatten)]
    pub channel: ChannelParam,
    #[serde(flatten)]
    pub user: Option<UserParam>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum UserParam {
    User(String),
    UserId(String),
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ChannelParam {
    Channel(String),
    ChannelId(String),
}

#[derive(Deserialize, JsonSchema)]
pub struct UserLogPathParams {
    pub channel_id_type: ChannelIdType,
    pub channel: String,
    pub user_id_type: UserIdType,
    pub user: String,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChannelLogsStats {
    pub message_count: u64,
    pub top_chatters: Vec<UserLogsStats>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserLogsStats {
    pub user_id: String,
    pub user_login: Option<String>,
    pub message_count: u64,
}

#[derive(Deserialize, JsonSchema)]
pub struct UserNameHistoryParam {
    pub user_id: String,
}

#[derive(Serialize, JsonSchema)]
pub struct PreviousName {
    pub user_login: String,
    pub last_timestamp: DateTime<Utc>,
    pub first_timestamp: DateTime<Utc>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TierEntry {
    pub user_id: String,
    pub user_login: Option<String>,
    pub messages: u64,
    pub unique_messages: u64,
    pub windows_1m: u64,
    pub windows_5m: u64,
    pub windows_15m: u64,
    pub windows_30m: u64,
    pub windows_60m: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank_1m: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier_1m: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank_5m: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier_5m: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank_15m: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier_15m: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank_30m: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier_30m: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank_60m: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier_60m: Option<String>,
    pub tier_score: u32,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TierResponse {
    pub year: i32,
    pub month: u32,
    pub timezone: &'static str,
    pub total_users: u64,
    pub total_messages: u64,
    pub total_unique_messages: u64,
    pub entries: Vec<TierEntry>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TierDayResponse {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub timezone: &'static str,
    pub total_users: u64,
    pub total_messages: u64,
    pub total_unique_messages: u64,
    pub entries: Vec<TierEntry>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TierYearResponse {
    pub year: i32,
    pub timezone: &'static str,
    pub total_users: u64,
    pub total_messages: u64,
    pub total_unique_messages: u64,
    pub entries: Vec<TierEntry>,
}

// SullyGnome stream listing
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SullyStreamsResponse {
    pub channel: String,
    pub year: i32,
    pub total: u32,
    pub streams: Vec<SullyStreamEntry>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SullyStreamEntry {
    pub stream_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_iso: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_human: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_human: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length_minutes: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gamesplayed: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum TierMode {
    All,
    Online,
    Offline,
}

#[derive(Serialize, Deserialize, JsonSchema, Default)]
pub struct TierModeQuery {
    #[serde(default)]
    pub mode: Option<TierMode>,
    /// Comma-separated list of user ids or logins to exclude from tiers (e.g. bot accounts).
    #[schemars(description = "Comma-separated user ids or logins to exclude from tiers (e.g. bot accounts, case-insensitive).", example = "\"moobot,nightbot\"")]
    #[serde(default)]
    pub exclude_bots: Option<String>,
}
