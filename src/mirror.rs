use crate::db::schema::{MessageFlags, MessageType, StructuredMessage, MESSAGES_STRUCTURED_TABLE};
use anyhow::Context;
use chrono::{DateTime, Utc};
use clickhouse::Client;
use futures::{stream, StreamExt};
use reqwest::Client as HttpClient;
use serde::Deserialize;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use tracing::{info, warn};
use uuid::Uuid;

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MIN_VALID_TS_MS: i64 = 1_577_836_800_000; // 2020-01-01T00:00:00Z in ms

#[derive(Deserialize)]
struct AvailableLogsResp {
    #[serde(rename = "availableLogs")]
    available_logs: Option<Vec<AvailableLogEntry>>,
}

#[derive(Deserialize)]
struct AvailableLogEntry {
    year: String,
    month: String,
    #[serde(default)]
    day: Option<String>,
}

#[derive(Deserialize)]
struct DailyLogResp {
    messages: Option<Vec<RemoteMessage>>,
}

#[derive(Deserialize)]
struct RemoteMessage {
    text: Option<String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    timestamp: Option<String>,
    id: Option<String>,
    #[serde(default)]
    tags: HashMap<String, String>,
}

pub async fn run(
    db: Client,
    base_url: String,
    local_cache: Option<String>,
    channel: String,
    year: Option<u32>,
    month: Option<u32>,
    day: Option<u32>,
    batch: usize,
) -> anyhow::Result<()> {
    let base_url = base_url.trim_end_matches('/').to_string();
    let http = HttpClient::builder()
        .timeout(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        .tcp_nodelay(true)
        .pool_max_idle_per_host(8)
        .user_agent("rustlog-mirror/0.1")
        .build()?;

    let avail = if let Some(ref cache_root) = local_cache {
        fetch_available_local(cache_root, &channel)?
    } else {
        fetch_available_remote(&http, &base_url, &channel).await?
    };

    let mut tasks: Vec<(u32, u32, u32)> = avail
        .into_iter()
        .filter_map(|entry| {
            let y: u32 = entry.year.parse().ok()?;
            let m: u32 = entry.month.parse().ok()?;
            let d: u32 = entry.day.as_deref()?.parse().ok()?;
            Some((y, m, d))
        })
        .filter(|(y, m, d)| {
            year.map_or(true, |yy| yy == *y)
                && month.map_or(true, |mm| mm == *m)
                && day.map_or(true, |dd| dd == *d)
        })
        .collect();

    tasks.sort();
    info!("Found {} daily logs to mirror", tasks.len());

    let total = tasks.len();
    let concurrency = 8usize;

    let mut stream = stream::iter(tasks.into_iter().enumerate())
        .map(|(idx, (y, m, d))| {
            let http = http.clone();
            let db = db.clone();
            let base_url = base_url.clone();
            let channel = channel.clone();
            let local_cache = local_cache.clone();
            async move {
                let started = Instant::now();
                let res = process_day(
                    &http,
                    &db,
                    &base_url,
                    local_cache.as_deref(),
                    &channel,
                    y,
                    m,
                    d,
                    batch,
                )
                .await;
                (idx, res, started.elapsed(), y, m, d)
            }
        })
        .buffer_unordered(concurrency);

    while let Some((idx, res, elapsed, y, m, d)) = stream.next().await {
        match res {
            Ok((added, skipped, existed)) => info!(
                "[{:>3}/{:>3}] {:04}-{:02}-{:02} added={} skipped={} existing={} in {:?}",
                idx + 1,
                total,
                y,
                m,
                d,
                added,
                skipped,
                existed,
                elapsed
            ),
            Err(err) => warn!("[{:>3}/{:>3}] {:04}-{:02}-{:02} failed: {}", idx + 1, total, y, m, d, err),
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn process_day(
    http: &HttpClient,
    db: &Client,
    base_url: &str,
    local_cache: Option<&str>,
    channel: &str,
    y: u32,
    m: u32,
    d: u32,
    batch: usize,
) -> anyhow::Result<(u64, u64, usize)> {
    let msgs_result = if let Some(ref cache_root) = local_cache {
        fetch_daily_local(cache_root, channel, y, m, d)
    } else {
        let url = format!(
            "{}/channel/{}/{:04}/{:02}/{:02}?jsonBasic=1",
            base_url, channel, y, m, d
        );
        fetch_daily_remote(http, &url).await.map_err(|e| e.into())
    };

    match msgs_result {
        Ok(msgs) => {
            let start_ms = chrono::NaiveDate::from_ymd_opt(y as i32, m, d)
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|dt| dt.and_utc().timestamp_millis() as u64)
                .unwrap_or(0);
            let end_ms = start_ms.saturating_add(24 * 60 * 60 * 1000);
            let mut seen = fetch_existing_keys(db, channel, start_ms, end_ms).await?;
            let existed = seen.len();
            let mut buffer: Vec<StructuredMessage<'static>> = Vec::with_capacity(batch);
            let mut added: u64 = 0;
            let mut processed: u64 = 0;
            for msg in msgs {
                processed += 1;
                if let Some((mapped, key)) = map_message(channel, msg) {
                    if !seen.insert(key) {
                        continue;
                    }
                    buffer.push(mapped);
                    added += 1;
                    if buffer.len() >= batch {
                        insert_batch(db, &mut buffer).await?;
                    }
                }
            }
            if !buffer.is_empty() {
                insert_batch(db, &mut buffer).await?;
            }
            let skipped = processed.saturating_sub(added);
            Ok((added, skipped, existed))
        }
        Err(err) => Err(err),
    }
}

async fn fetch_available(
    http: &HttpClient,
    base_url: &str,
    channel: &str,
) -> anyhow::Result<Vec<AvailableLogEntry>> {
    let url = format!("{}/list?channel={}", base_url, channel);
    let resp: AvailableLogsResp = http.get(url).send().await?.json().await?;
    Ok(resp.available_logs.unwrap_or_default())
}

async fn fetch_available_remote(
    http: &HttpClient,
    base_url: &str,
    channel: &str,
) -> anyhow::Result<Vec<AvailableLogEntry>> {
    let url = format!("{}/list?channel={}", base_url, channel);
    let resp: AvailableLogsResp = http
        .get(url)
        .header("Accept-Encoding", "br, gzip, deflate")
        .send()
        .await?
        .json()
        .await?;
    Ok(resp.available_logs.unwrap_or_default())
}

fn fetch_available_local(
    cache_root: &str,
    channel: &str,
) -> anyhow::Result<Vec<AvailableLogEntry>> {
    let mut out = Vec::new();
    let daily_root = std::path::Path::new(cache_root).join(channel).join("daily");
    if !daily_root.exists() {
        return Ok(out);
    }
    for year_dir in std::fs::read_dir(&daily_root)? {
        let year_dir = year_dir?;
        let year_name = year_dir.file_name().to_string_lossy().into_owned();
        let year_path = year_dir.path();
        if !year_path.is_dir() {
            continue;
        }
        for month_dir in std::fs::read_dir(&year_path)? {
            let month_dir = month_dir?;
            let month_name = month_dir.file_name().to_string_lossy().into_owned();
            let month_path = month_dir.path();
            if !month_path.is_dir() {
                continue;
            }
            for day_file in std::fs::read_dir(&month_path)? {
                let day_file = day_file?;
                let fname = day_file.file_name().to_string_lossy().into_owned();
                if !fname.to_lowercase().ends_with(".json") {
                    continue;
                }
                let day_num = fname.trim_end_matches(".json").to_string();
                out.push(AvailableLogEntry {
                    year: year_name.clone(),
                    month: month_name.clone(),
                    day: Some(day_num),
                });
            }
        }
    }
    Ok(out)
}

async fn fetch_daily_remote(http: &HttpClient, url: &str) -> anyhow::Result<Vec<RemoteMessage>> {
    let resp: DailyLogResp = http
        .get(url)
        .header("Accept-Encoding", "br, gzip, deflate")
        .send()
        .await?
        .json()
        .await?;
    Ok(resp.messages.unwrap_or_default())
}

async fn fetch_existing_keys(
    db: &Client,
    channel_login: &str,
    start_ms: u64,
    end_ms: u64,
) -> anyhow::Result<HashSet<DedupKey>> {
    let mut set = HashSet::new();
    let esc_channel = channel_login.replace('\'', "\\'");
    let sql = format!(
        "SELECT user_id, timestamp, text FROM {} \
         WHERE channel_login='{}' AND timestamp >= {} AND timestamp < {}",
        MESSAGES_STRUCTURED_TABLE, esc_channel, start_ms, end_ms
    );
    let mut cursor = db.query(sql).fetch::<(String, u64, String)>();
    while let Some(row) = cursor.next().await.transpose()? {
        set.insert(row);
    }
    Ok(set)
}

fn fetch_daily_local(
    cache_root: &str,
    channel: &str,
    year: u32,
    month: u32,
    day: u32,
) -> anyhow::Result<Vec<RemoteMessage>> {
    let path = std::path::Path::new(cache_root)
        .join(channel)
        .join("daily")
        .join(format!("{year:04}"))
        .join(format!("{month:02}"))
        .join(format!("{day:02}.json"));
    let data = std::fs::read_to_string(path)?;
    let resp: DailyLogResp = serde_json::from_str(&data)?;
    Ok(resp.messages.unwrap_or_default())
}

type DedupKey = (String, u64, String);

fn map_message(
    channel_login: &str,
    msg: RemoteMessage,
) -> Option<(StructuredMessage<'static>, DedupKey)> {
    let ts_str = msg.timestamp?;
    let ts_ms = DateTime::parse_from_rfc3339(&ts_str)
        .ok()?
        .with_timezone(&Utc)
        .timestamp_millis()
        .max(0);

    if ts_ms < MIN_VALID_TS_MS {
        // Skip obviously broken timestamps (epoch/1970 junk).
        return None;
    }

    let ts = ts_ms as u64;

    let channel_id = msg.tags.get("room-id")?.to_owned();
    let user_id = msg.tags.get("user-id")?.to_owned();
    let user_login = msg
        .display_name
        .as_deref()
        .unwrap_or_default()
        .to_lowercase();

    let color = msg
        .tags
        .get("color")
        .and_then(|c| c.strip_prefix('#').or(Some(c.as_str())))
        .and_then(|c| u32::from_str_radix(c, 16).ok());

    let badges: Vec<Cow<'static, str>> = msg
        .tags
        .get("badges")
        .map(|b| {
            b.split(',')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string().into())
                .collect()
        })
        .unwrap_or_default();

    let display_name = msg
        .tags
        .get("display-name")
        .cloned()
        .or(msg.display_name)
        .unwrap_or_default();

    let mut extra_tags: Vec<(Cow<'static, str>, Cow<'static, str>)> = Vec::new();
    for (k, v) in msg.tags.iter() {
        if matches!(
            k.as_str(),
            "user-id"
                | "display-name"
                | "color"
                | "user-type"
                | "badges"
                | "badge-info"
                | "client-nonce"
                | "emotes"
                | "flags"
                | "room-id"
                | "id"
                | "tmi-sent-ts"
        ) {
            continue;
        }
        extra_tags.push((k.to_string().into(), v.to_string().into()));
    }

    let structured = StructuredMessage {
        channel_id: channel_id.into(),
        channel_login: channel_login.to_owned().into(),
        timestamp: ts,
        id: msg
            .id
            .as_deref()
            .and_then(|s| Uuid::parse_str(s).ok())
            .unwrap_or_else(Uuid::new_v4),
        message_type: MessageType::PrivMsg,
        user_id: user_id.into(),
        user_login: user_login.into(),
        display_name: display_name.into(),
        color,
        user_type: msg.tags.get("user-type").cloned().unwrap_or_default().into(),
        badges,
        badge_info: msg.tags.get("badge-info").cloned().unwrap_or_default().into(),
        client_nonce: msg
            .tags
            .get("client-nonce")
            .cloned()
            .unwrap_or_default()
            .into(),
        emotes: msg.tags.get("emotes").cloned().unwrap_or_default().into(),
        automod_flags: msg.tags.get("flags").cloned().unwrap_or_default().into(),
        text: msg.text.clone().unwrap_or_default().into(),
        message_flags: MessageFlags::empty(),
        extra_tags,
    };

    let key: DedupKey = (user_id, ts, msg.text.unwrap_or_default());
    Some((structured, key))
}

async fn insert_batch(
    db: &Client,
    buffer: &mut Vec<StructuredMessage<'static>>,
) -> anyhow::Result<()> {
    if buffer.is_empty() {
        return Ok(());
    }
    let write_count = buffer.len();
    let mut inserter = db
        .insert(MESSAGES_STRUCTURED_TABLE)
        .context("open inserter")?;
    for row in buffer.drain(..) {
        inserter.write(&row).await.context("write row")?;
    }
    inserter.end().await.context("flush inserter")?;
    info!("Flushed batch of {}", write_count);
    Ok(())
}
