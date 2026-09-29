use super::responders::logs::{JsonResponseType, LogsResponseType};
use crate::{
    domain::{
        self,
        logs::{LogDate, LogsQuery, TimeRange},
        stats::{NameHistoryEntry, UserMessageCount},
    },
    services::sully::{Stream, StreamList},
};
use chrono::{DateTime, Utc};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{
    de::{Error as DeError, SeqAccess, Visitor},
    Deserialize, Deserializer, Serialize,
};
use std::fmt::Display;
use strum::Display;

pub const DEFAULT_EXCLUDED_BOTS: &[&str] = &[
    "twirapp",
    "streamelements",
    "nightbot",
    "moobot",
    "mejkizbot",
    "supibot",
    "potatbotat",
    "fossabot",
];

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

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChatBadgesResponse {
    pub badges: Vec<ChatBadge>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChatBadge {
    pub set_id: String,
    pub version: String,
    pub image_url_1x: String,
    pub image_url_2x: String,
    pub title: String,
    pub description: String,
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

#[derive(Deserialize, JsonSchema, Clone, Copy, Debug)]
pub struct LogRangeParams {
    /// RFC 3339 start date
    pub from: Option<DateTime<Utc>>,
    /// RFC 3339 end date
    pub to: Option<DateTime<Utc>>,
}

impl LogRangeParams {
    pub fn range(&self) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        self.from.zip(self.to)
    }

    pub fn time_range(&self) -> Option<TimeRange> {
        self.range().map(|(from, to)| TimeRange { from, to })
    }
}

#[derive(Deserialize, Debug, JsonSchema, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct LogsParams {
    /// Return the full JSON response shape.
    #[serde(default, deserialize_with = "deserialize_bool_param")]
    pub json: bool,
    /// Return compact JSON with basic message fields.
    #[serde(default, deserialize_with = "deserialize_bool_param")]
    pub json_basic: bool,
    /// Return raw IRC lines.
    #[serde(default, deserialize_with = "deserialize_bool_param")]
    pub raw: bool,
    /// Reverse log order.
    #[serde(default, deserialize_with = "deserialize_bool_param")]
    pub reverse: bool,
    /// Return newline-delimited JSON.
    #[serde(default, deserialize_with = "deserialize_bool_param")]
    pub ndjson: bool,
    /// Maximum number of messages to return.
    #[schemars(range(min = 1), example = 100)]
    pub limit: Option<u64>,
    /// Number of messages to skip before returning results.
    #[schemars(range(min = 0), example = 0)]
    pub offset: Option<u64>,
}

impl LogsParams {
    pub fn query(&self) -> LogsQuery {
        LogsQuery {
            reverse: self.reverse,
            limit: self.limit,
            offset: self.offset,
        }
    }

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
    let Some(value) = Option::<&str>::deserialize(deserializer)? else {
        return Ok(false);
    };

    match value.to_ascii_lowercase().as_str() {
        "" | "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Ok(true),
    }
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

impl From<LogDate> for AvailableLogDate {
    fn from(date: LogDate) -> Self {
        Self {
            year: date.year.to_string(),
            month: date.month.to_string(),
            day: date.day.map(|day| day.to_string()),
        }
    }
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
    pub channel: Option<ChannelParam>,
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

impl From<UserMessageCount> for UserLogsStats {
    fn from(count: UserMessageCount) -> Self {
        Self {
            user_id: count.user_id,
            user_login: count.user_login,
            message_count: count.message_count,
        }
    }
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

impl From<NameHistoryEntry> for PreviousName {
    fn from(entry: NameHistoryEntry) -> Self {
        Self {
            user_login: entry.user_login,
            last_timestamp: entry.last_seen,
            first_timestamp: entry.first_seen,
        }
    }
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

impl From<domain::tiers::TierEntry> for TierEntry {
    fn from(entry: domain::tiers::TierEntry) -> Self {
        Self {
            user_id: entry.user_id,
            user_login: entry.user_login,
            messages: entry.messages,
            unique_messages: entry.unique_messages,
            windows_1m: entry.windows_1m,
            windows_5m: entry.windows_5m,
            windows_15m: entry.windows_15m,
            windows_30m: entry.windows_30m,
            windows_60m: entry.windows_60m,
            rank_1m: entry.rank_1m,
            tier_1m: entry.tier_1m,
            rank_5m: entry.rank_5m,
            tier_5m: entry.tier_5m,
            rank_15m: entry.rank_15m,
            tier_15m: entry.tier_15m,
            rank_30m: entry.rank_30m,
            tier_30m: entry.tier_30m,
            rank_60m: entry.rank_60m,
            tier_60m: entry.tier_60m,
            tier_score: entry.tier_score,
        }
    }
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

impl From<StreamList> for SullyStreamsResponse {
    fn from(list: StreamList) -> Self {
        Self {
            channel: list.channel,
            year: list.year,
            total: list.total,
            streams: list
                .streams
                .into_iter()
                .map(SullyStreamEntry::from)
                .collect(),
        }
    }
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

impl From<Stream> for SullyStreamEntry {
    fn from(stream: Stream) -> Self {
        Self {
            stream_id: stream.stream_id,
            start_iso: stream.start_iso,
            start_human: stream.start_human,
            end_human: stream.end_human,
            length_minutes: stream.length_minutes,
            gamesplayed: stream.gamesplayed,
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum TierMode {
    All,
    Online,
    Offline,
}

impl From<TierMode> for domain::tiers::TierMode {
    fn from(mode: TierMode) -> Self {
        match mode {
            TierMode::All => Self::All,
            TierMode::Online => Self::Online,
            TierMode::Offline => Self::Offline,
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Default)]
pub struct TierModeQuery {
    /// Which messages are included in tier calculations.
    #[schemars(
        description = "Tier calculation mode: all messages, messages sent while the stream was online, or messages sent while the stream was offline."
    )]
    #[serde(default = "default_tier_mode")]
    pub mode: TierMode,
    /// Bot logins excluded from tier tables.
    #[schemars(schema_with = "exclude_bots_schema")]
    #[serde(
        default = "default_exclude_bots",
        deserialize_with = "deserialize_exclude_bots"
    )]
    pub exclude_bots: Vec<String>,
}

impl Default for TierMode {
    fn default() -> Self {
        Self::All
    }
}

fn default_tier_mode() -> TierMode {
    TierMode::All
}

pub fn default_exclude_bots() -> Vec<String> {
    DEFAULT_EXCLUDED_BOTS
        .iter()
        .map(|bot| (*bot).to_owned())
        .collect()
}

fn exclude_bots_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "array",
        "description": "Bot logins excluded from tier tables. The default excludes common Twitch bots.",
        "items": {
            "type": "string",
            "enum": DEFAULT_EXCLUDED_BOTS
        },
        "uniqueItems": true
    })
}

fn deserialize_exclude_bots<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    struct ExcludeBotsVisitor;

    impl<'de> Visitor<'de> for ExcludeBotsVisitor {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a comma-separated string or a list of bot logins")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: DeError,
        {
            Ok(split_bot_list(value))
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: DeError,
        {
            Ok(split_bot_list(&value))
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut bots = Vec::new();

            while let Some(value) = seq.next_element::<String>()? {
                bots.extend(split_bot_list(&value));
            }

            Ok(bots)
        }
    }

    deserializer.deserialize_any(ExcludeBotsVisitor)
}

fn split_bot_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|bot| bot.trim().to_lowercase())
        .filter(|bot| !bot.is_empty())
        .collect()
}
