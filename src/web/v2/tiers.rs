//! Chat tier tables and the stream history they are split by.

use super::{
    extract::{Path, Query, TwitchId},
    params::ChannelPath,
    problem::ApiProblem,
};
use crate::{
    app::App,
    domain::tiers::{self, DEFAULT_EXCLUDED_BOTS, TIMEZONE, TierEntry, TierPeriod},
    services::{self, tiers::TierQuery},
    web::cache_control::Cached,
};
use axum::{Json, extract::State};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize};
use std::{borrow::Cow, fmt};

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TiersPath {
    /// Id of the channel.
    pub channel_id: TwitchId,
    /// Calendar day, month or year in Europe/Moscow time.
    pub period: Period,
}

/// A calendar period as written in a path: `2026-03-01`, `2026-03` or `2026`.
#[derive(Debug, Clone, Copy)]
pub struct Period(TierPeriod);

impl Period {
    fn parse(text: &str) -> Option<TierPeriod> {
        match text.split('-').collect::<Vec<_>>().as_slice() {
            [year] if year.len() == 4 => Some(TierPeriod::Year {
                year: year.parse().ok()?,
            }),
            [year, month] if year.len() == 4 && month.len() == 2 => {
                let (year, month) = (year.parse().ok()?, month.parse().ok()?);
                NaiveDate::from_ymd_opt(year, month, 1)?;
                Some(TierPeriod::Month { year, month })
            }
            [_, _, _] if text.len() == 10 => NaiveDate::parse_from_str(text, "%Y-%m-%d")
                .ok()
                .map(TierPeriod::Day),
            _ => None,
        }
    }
}

impl fmt::Display for Period {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            TierPeriod::Day(date) => write!(f, "{}", date.format("%Y-%m-%d")),
            TierPeriod::Month { year, month } => write!(f, "{year:04}-{month:02}"),
            TierPeriod::Year { year } => write!(f, "{year:04}"),
        }
    }
}

impl<'de> Deserialize<'de> for Period {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map(Self).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "`{text}` is not a day (`YYYY-MM-DD`), month (`YYYY-MM`) or year (`YYYY`)"
            ))
        })
    }
}

impl JsonSchema for Period {
    fn schema_name() -> Cow<'static, str> {
        "TierPeriod".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": "^[0-9]{4}(-[0-9]{2}(-[0-9]{2})?)?$",
            "examples": ["2026-03-01", "2026-03", "2026"],
        })
    }

    fn inline_schema() -> bool {
        true
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TierMode {
    /// All messages.
    #[default]
    All,
    /// Messages sent while the stream was live, as SullyGnome knows it.
    Online,
    /// Messages sent while the stream was offline.
    Offline,
}

impl From<TierMode> for tiers::TierMode {
    fn from(mode: TierMode) -> Self {
        match mode {
            TierMode::All => Self::All,
            TierMode::Online => Self::Online,
            TierMode::Offline => Self::Offline,
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TiersQuery {
    /// Which messages count.
    #[serde(default)]
    pub mode: TierMode,
    /// Bot logins to leave out, repeated for several. Replaces the default
    /// list of common bots; give it once empty to leave out none.
    #[serde(default = "default_excluded_bots")]
    #[schemars(example = DEFAULT_EXCLUDED_BOTS)]
    pub exclude_bots: Vec<String>,
}

fn default_excluded_bots() -> Vec<String> {
    DEFAULT_EXCLUDED_BOTS
        .iter()
        .map(|bot| (*bot).to_owned())
        .collect()
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TierTable {
    /// The requested period.
    pub period: String,
    /// Time zone of the period.
    pub timezone: &'static str,
    pub mode: TierMode,
    pub total_users: u64,
    pub total_messages: u64,
    pub total_unique_messages: u64,
    /// Up to 500 users, the most active first.
    pub entries: Vec<TierTableEntry>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TierTableEntry {
    pub user_id: TwitchId,
    /// Left out when Twitch does not know the user or cannot be asked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    pub messages: u64,
    pub unique_messages: u64,
    /// Sum of the tier points over the window sizes.
    pub tier_score: u32,
    pub windows: Windows,
}

/// Activity per window size.
#[derive(Serialize, JsonSchema)]
pub struct Windows {
    #[serde(rename = "1m")]
    pub one_minute: WindowActivity,
    #[serde(rename = "5m")]
    pub five_minutes: WindowActivity,
    #[serde(rename = "15m")]
    pub fifteen_minutes: WindowActivity,
    #[serde(rename = "30m")]
    pub thirty_minutes: WindowActivity,
    #[serde(rename = "60m")]
    pub sixty_minutes: WindowActivity,
}

#[derive(Serialize, JsonSchema)]
pub struct WindowActivity {
    /// Windows of this size with at least one message.
    pub active: u64,
    /// Rank among the users, from 1; left out when not ranked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<u32>,
    /// Tier of the rank; left out when not ranked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
}

impl From<TierEntry> for TierTableEntry {
    fn from(entry: TierEntry) -> Self {
        let window = |active, rank, tier| WindowActivity { active, rank, tier };
        Self {
            user_id: TwitchId::new_unchecked(entry.user_id),
            login: entry.user_login,
            messages: entry.messages,
            unique_messages: entry.unique_messages,
            tier_score: entry.tier_score,
            windows: Windows {
                one_minute: window(entry.windows_1m, entry.rank_1m, entry.tier_1m),
                five_minutes: window(entry.windows_5m, entry.rank_5m, entry.tier_5m),
                fifteen_minutes: window(entry.windows_15m, entry.rank_15m, entry.tier_15m),
                thirty_minutes: window(entry.windows_30m, entry.rank_30m, entry.tier_30m),
                sixty_minutes: window(entry.windows_60m, entry.rank_60m, entry.tier_60m),
            },
        }
    }
}

pub async fn tier_table(
    State(app): State<App>,
    Path(path): Path<TiersPath>,
    Query(query): Query<TiersQuery>,
) -> Result<Cached<Json<TierTable>>, ApiProblem> {
    let channel_id = path.channel_id.as_str();
    app.check_opted_out(channel_id, None)?;
    let channel = channel_name(&app, channel_id).await;
    let excluded_bots: Vec<String> = query
        .exclude_bots
        .into_iter()
        .filter(|bot| !bot.trim().is_empty())
        .collect();

    let period = path.period.0;
    let mode = query.mode.into();
    let tiers = app
        .tiers
        .compute(TierQuery {
            channel_id,
            channel: &channel,
            period,
            mode,
            excluded_bots: &excluded_bots,
        })
        .await?;
    services::supabase::spawn_tier_snapshot(&app.config, &channel, period, mode, &tiers);

    Ok(Cached::no_cache(Json(TierTable {
        period: path.period.to_string(),
        timezone: TIMEZONE,
        mode: query.mode,
        total_users: tiers.total_users,
        total_messages: tiers.total_messages,
        total_unique_messages: tiers.total_unique_messages,
        entries: tiers
            .entries
            .into_iter()
            .map(TierTableEntry::from)
            .collect(),
    })))
}

/// The channel's login, by which SullyGnome and the tier snapshots know it,
/// or its id when Twitch cannot tell.
async fn channel_name(app: &App, channel_id: &str) -> String {
    app.twitch
        .get_users(vec![channel_id.to_owned()], Vec::new(), false)
        .await
        .ok()
        .and_then(|logins| logins.into_values().next())
        .unwrap_or_else(|| channel_id.to_owned())
}

#[derive(Deserialize, JsonSchema)]
pub struct StreamsQuery {
    /// Year of the streams.
    pub year: i32,
}

#[derive(Serialize, JsonSchema)]
pub struct Streams {
    /// The streams of the year, as SullyGnome knows them.
    pub streams: Vec<Stream>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Stream {
    /// SullyGnome's id of the stream.
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_minutes: Option<u32>,
    /// The games played, as SullyGnome lists them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub games: Option<String>,
}

impl From<services::sully::Stream> for Stream {
    fn from(stream: services::sully::Stream) -> Self {
        let started_at = stream
            .start_iso
            .as_deref()
            .and_then(|start| DateTime::parse_from_rfc3339(start).ok())
            .map(|start| start.to_utc());
        let ended_at = started_at
            .zip(stream.length_minutes)
            .map(|(start, minutes)| start + Duration::minutes(minutes.into()));
        Self {
            id: stream.stream_id,
            started_at,
            ended_at,
            duration_minutes: stream.length_minutes,
            games: stream.gamesplayed,
        }
    }
}

pub async fn streams(
    State(app): State<App>,
    Path(path): Path<ChannelPath>,
    Query(query): Query<StreamsQuery>,
) -> Result<Cached<Json<Streams>>, ApiProblem> {
    let channel_id = path.channel_id.as_str();
    app.check_opted_out(channel_id, None)?;
    let login = app
        .twitch
        .get_users(vec![channel_id.to_owned()], Vec::new(), false)
        .await?
        .into_values()
        .next()
        .ok_or_else(|| ApiProblem::not_found("Twitch does not know the channel"))?;

    // A live answer is cached for less time than one from the file cache.
    let (list, cache_seconds) = match app.sully.fetch(&login, query.year).await {
        Ok((total, streams)) => {
            let list = services::sully::StreamList {
                channel: login,
                year: query.year,
                total,
                streams,
            };
            app.sully.write_cache(&list);
            (list, 600)
        }
        Err(_) => (
            app.sully
                .read_cache(&login, query.year)
                .filter(|list| !list.streams.is_empty())
                .ok_or_else(|| ApiProblem::not_found("SullyGnome has no streams of the channel"))?,
            3600,
        ),
    };

    Ok(Cached::public(
        cache_seconds,
        Json(Streams {
            streams: list.streams.into_iter().map(Stream::from).collect(),
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::Period;

    #[test]
    fn periods_round_trip() {
        for text in ["2026-03-01", "2026-03", "2026"] {
            let period = Period(Period::parse(text).unwrap());
            assert_eq!(period.to_string(), text);
        }
    }

    #[test]
    fn rejects_invalid_periods() {
        for text in [
            "",
            "26",
            "2026-3",
            "2026-13",
            "2026-02-30",
            "2026-03-01-02",
            "2026-3-01",
            "abcd",
        ] {
            assert!(Period::parse(text).is_none(), "{text}");
        }
    }
}
