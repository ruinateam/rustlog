//! Stream history from [SullyGnome](https://sullygnome.com), used to tell
//! online from offline chat in tier tables.
//!
//! Fetched stream lists are cached as `{channel}-{year}.json` files, by
//! default in `cache/sullygnome`, as a fallback for when SullyGnome is
//! unreachable.

use crate::{error::Error, Result};
use reqwest::{
    header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT},
    Client as HttpClient,
};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, time::Duration};
use tracing::{error, warn};

/// A channel's streams in one year. Also, the format of the cache files.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamList {
    pub channel: String,
    pub year: i32,
    pub total: u32,
    pub streams: Vec<Stream>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stream {
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

/// SullyGnome client with a file cache of fetched stream lists.
#[derive(Clone)]
pub struct SullyGnome {
    http: HttpClient,
    base_url: String,
    cache_dir: PathBuf,
}

impl SullyGnome {
    pub const DEFAULT_URL: &str = "https://sullygnome.com";
    pub const DEFAULT_CACHE_DIR: &str = "cache/sullygnome";

    pub fn new(base_url: impl Into<String>, cache_dir: impl Into<PathBuf>) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static("rustlog/1.0 (https://localhost)"),
        );
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        let http = HttpClient::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| Error::Internal)?;

        Ok(Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            cache_dir: cache_dir.into(),
        })
    }

    /// Fetches a channel's streams in `year`, returning SullyGnome's total
    /// count and the streams. Failures are logged.
    pub async fn fetch(&self, channel: &str, year: i32) -> Result<(u32, Vec<Stream>)> {
        if !is_valid_channel(channel) {
            return Err(Error::NotFound);
        }

        let internal_id = self.fetch_internal_id(channel).await.inspect_err(|e| {
            error!(
                "sully fetch internal_id failed channel={} err={:?}",
                channel, e
            );
        })?;

        self.fetch_streams(&internal_id, year)
            .await
            .inspect_err(|e| {
                error!(
                    "sully fetch streams failed channel={} year={} err={:?}",
                    channel, year, e
                );
            })
    }

    /// Fetches a channel's streams in `year` and refreshes the cache, falling
    /// back to the cache when SullyGnome is unreachable.
    pub async fn load(&self, channel: &str, year: i32) -> Option<StreamList> {
        if let Ok((_, streams)) = self.fetch(channel, year).await {
            let list = StreamList {
                channel: channel.to_owned(),
                year,
                total: streams.len() as u32,
                streams,
            };
            let _ = self.write_cache(&list);
            return Some(list);
        }

        if let Some(cached) = self.read_cache(channel, year) {
            warn!(
                "sully fetch falling back to cache channel={} year={} streams={}",
                channel,
                year,
                cached.streams.len()
            );
            Some(cached)
        } else {
            warn!(
                "sully fetch failed and no cache available channel={} year={}",
                channel, year
            );
            None
        }
    }

    pub fn read_cache(&self, channel: &str, year: i32) -> Option<StreamList> {
        let path = self.cache_path(channel, year)?;
        let data = fs::read_to_string(path).ok()?;
        parse_body(channel, year, &data).ok()
    }

    pub fn write_cache(&self, list: &StreamList) -> std::io::Result<()> {
        let Some(path) = self.cache_path(&list.channel, list.year) else {
            return Ok(());
        };
        fs::create_dir_all(&self.cache_dir)?;
        let data = serde_json::to_string_pretty(list)?;
        fs::write(path, data)
    }

    /// `None` for channel names that are not a Twitch login or id, which
    /// keeps request input from escaping the cache directory.
    fn cache_path(&self, channel: &str, year: i32) -> Option<PathBuf> {
        is_valid_channel(channel).then(|| self.cache_dir.join(format!("{channel}-{year}.json")))
    }

    async fn fetch_internal_id(&self, login: &str) -> Result<String> {
        let url = format!("{}/api/standardsearch/{login}", self.base_url);
        let resp = self.http.get(&url).send().await.map_err(|e| {
            error!("sully id http error login={} err={:?}", login, e);
            Error::Internal
        })?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            error!(
                "sully id non-success status={} login={} body_snippet={}",
                status,
                login,
                body.chars().take(500).collect::<String>()
            );
            return Err(Error::NotFound);
        }
        let items: Vec<SearchItem> = serde_json::from_str(&body).map_err(|e| {
            error!(
                "sully id parse error login={} err={:?} body_snippet={}",
                login,
                e,
                body.chars().take(500).collect::<String>()
            );
            Error::Internal
        })?;
        let id = items
            .into_iter()
            .find(|it| it.itemtype == 1)
            .map(|it| it.value.into_string())
            .ok_or(Error::NotFound)?;
        Ok(id)
    }

    async fn fetch_streams(&self, internal_id: &str, year: i32) -> Result<(u32, Vec<Stream>)> {
        let mut offset: u32 = 0;
        let limit: u32 = 2000;
        let mut streams = Vec::new();

        let total = loop {
            let url = format!(
                "{}/api/tables/channeltables/streams/{year}/{internal_id}/%20/1/1/desc/{offset}/{limit}",
                self.base_url
            );
            let resp_raw = self.http.get(&url).send().await.map_err(|e| {
                error!("sully streams http error url={} err={:?}", url, e);
                Error::Internal
            })?;
            let status = resp_raw.status();
            let body = resp_raw.text().await.unwrap_or_default();
            if !status.is_success() {
                error!(
                    "sully streams non-success status={} url={} body_snippet={}",
                    status,
                    url,
                    body.chars().take(500).collect::<String>()
                );
                return Err(Error::NotFound);
            }
            let parsed = parse_body("", year, &body).map_err(|e| {
                error!(
                    "sully streams parse error url={} err={:?} body_snippet={}",
                    url,
                    e,
                    body.chars().take(500).collect::<String>()
                );
                Error::Internal
            })?;
            let total = parsed.total;
            let count_added = parsed.streams.len() as u32;
            streams.extend(parsed.streams);
            offset += limit;
            if offset >= total || count_added == 0 {
                break total;
            }
        };

        Ok((total, streams))
    }
}

/// Twitch logins and ids only contain ASCII letters, digits and underscores.
fn is_valid_channel(channel: &str) -> bool {
    !channel.is_empty()
        && channel
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SullyId {
    Str(String),
    Num(i64),
}

impl SullyId {
    fn into_string(self) -> String {
        match self {
            SullyId::Str(s) => s,
            SullyId::Num(n) => n.to_string(),
        }
    }
}

#[derive(Deserialize)]
struct SearchItem {
    value: SullyId,
    itemtype: i32,
}

#[derive(Deserialize)]
struct RawStreams {
    #[serde(rename = "recordsTotal")]
    records_total: u32,
    data: Vec<RawStream>,
}

#[derive(Deserialize)]
struct RawStream {
    #[serde(rename = "streamId")]
    stream_id: String,
    #[serde(rename = "startDateTime")]
    start_iso: Option<String>,
    #[serde(rename = "starttime")]
    start_human: Option<String>,
    #[serde(rename = "endtime")]
    end_human: Option<String>,
    #[serde(rename = "length")]
    length: Option<f64>,
    gamesplayed: Option<String>,
}

/// Parses a SullyGnome API page or a cache file.
fn parse_body(channel: &str, year: i32, body: &str) -> Result<StreamList> {
    if let Ok(raw) = serde_json::from_str::<RawStreams>(body) {
        let streams = raw
            .data
            .into_iter()
            .map(|row| Stream {
                stream_id: row.stream_id,
                start_iso: row.start_iso,
                start_human: row.start_human,
                end_human: row.end_human,
                length_minutes: row.length.map(|v| v.round() as u32),
                gamesplayed: row.gamesplayed,
            })
            .collect();
        return Ok(StreamList {
            channel: channel.to_string(),
            year,
            total: raw.records_total,
            streams,
        });
    }
    if let Ok(list) = serde_json::from_str::<StreamList>(body) {
        return Ok(list);
    }
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(body) {
        let records_total = val
            .get("recordsTotal")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let mut streams = Vec::new();
        if let Some(arr) = val.get("data").and_then(|v| v.as_array()) {
            for row in arr {
                let length_minutes = row
                    .get("length")
                    .and_then(|v| v.as_f64())
                    .map(|v| v.round() as u32);
                streams.push(Stream {
                    stream_id: row
                        .get("streamId")
                        .and_then(|v| v.as_i64())
                        .map(|v| v.to_string())
                        .or_else(|| {
                            row.get("streamId")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                        })
                        .unwrap_or_default(),
                    start_iso: row
                        .get("startDateTime")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    start_human: row
                        .get("starttime")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    end_human: row
                        .get("endtime")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    length_minutes,
                    gamesplayed: row
                        .get("gamesplayed")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                });
            }
        }
        return Ok(StreamList {
            channel: channel.to_string(),
            year,
            total: records_total,
            streams,
        });
    }
    Err(Error::NotFound)
}
