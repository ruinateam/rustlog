//! Path and query parameters shared by several endpoints.

use super::{extract::TwitchId, problem::ApiProblem};
use crate::{
    domain::logs::{LogsQuery, TimeRange},
    web::logs_response::{JsonResponseType, LogsResponseType},
};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChannelPath {
    /// Id of the channel.
    pub channel_id: TwitchId,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserPath {
    /// Id of the user.
    pub user_id: TwitchId,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChannelUserPath {
    /// Id of the channel.
    pub channel_id: TwitchId,
    /// Id of the user.
    pub user_id: TwitchId,
}

/// A required time range.
#[derive(Deserialize, JsonSchema)]
pub struct Range {
    /// Start of the range, inclusive (RFC 3339).
    pub from: DateTime<Utc>,
    /// End of the range, exclusive (RFC 3339).
    pub to: DateTime<Utc>,
}

impl Range {
    pub fn validate(&self) -> Result<TimeRange, ApiProblem> {
        if self.to <= self.from {
            return Err(ApiProblem::invalid("`to` must be later than `from`"));
        }
        Ok(TimeRange {
            from: self.from,
            to: self.to,
        })
    }
}

/// An optional time range: both ends or neither.
#[derive(Deserialize, JsonSchema)]
pub struct OptionalRange {
    /// Start of the range, inclusive (RFC 3339). Requires `to`.
    pub from: Option<DateTime<Utc>>,
    /// End of the range, exclusive (RFC 3339). Requires `from`.
    pub to: Option<DateTime<Utc>>,
}

impl OptionalRange {
    pub fn validate(&self) -> Result<Option<TimeRange>, ApiProblem> {
        match (self.from, self.to) {
            (None, None) => Ok(None),
            (Some(from), Some(to)) => Range { from, to }.validate().map(Some),
            _ => Err(ApiProblem::invalid("give both `from` and `to`, or neither")),
        }
    }
}

/// How messages are returned.
#[derive(Default, Deserialize, JsonSchema)]
pub struct Format {
    /// Response format; `basic-json` by default.
    #[serde(default)]
    pub format: LogFormat,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LogFormat {
    /// `{ "messages": [BasicMessage] }` as JSON.
    #[default]
    BasicJson,
    /// `{ "messages": [FullMessage] }` as JSON.
    FullJson,
    /// One `BasicMessage` per line.
    Ndjson,
    /// One formatted message per line.
    Text,
    /// One raw IRC message per line.
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

/// Ordering and paging of a list of messages.
#[derive(Default, Deserialize, JsonSchema)]
pub struct Paging {
    /// Newest messages first.
    #[serde(default)]
    pub reverse: bool,
    /// Return at most this many messages.
    #[schemars(range(min = 1))]
    pub limit: Option<u64>,
    /// Skip this many messages first.
    pub offset: Option<u64>,
}

impl Paging {
    pub fn validate(&self) -> Result<LogsQuery, ApiProblem> {
        if self.limit == Some(0) {
            return Err(ApiProblem::invalid("`limit` must be at least 1"));
        }
        Ok(LogsQuery {
            reverse: self.reverse,
            limit: self.limit,
            offset: self.offset,
        })
    }
}
