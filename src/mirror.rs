use crate::db::schema::{MessageFlags, MessageType, StructuredMessage, MESSAGES_STRUCTURED_TABLE};
use anyhow::Context;
use chrono::{DateTime, Utc};
use clickhouse::{Client, Row};
use dashmap::DashSet;
use futures::{stream, StreamExt};
use reqwest::Client as HttpClient;
use serde::Deserialize;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use uuid::Uuid;

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MIN_VALID_TS_MS: i64 = 1_577_836_800_000; // 2020-01-01T00:00:00Z in ms
const FETCH_RETRIES: usize = 3;
const CHANNEL_BUF_SIZE: usize = 100_000;

#[derive(Clone, Copy, Debug)]
pub struct RunDaysOptions {
    pub http_concurrency: usize,
    pub insert_max_rows: u64,
}

impl RunDaysOptions {
    pub fn new(batch: usize, http_concurrency: usize) -> Self {
        Self {
            http_concurrency: http_concurrency.max(1),
            insert_max_rows: (batch as u64).max(1),
        }
    }
}

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

/// Mirror remote logs for a specific channel.
/// Builds the list of days from the remote API and delegates to `run_days`.
pub async fn run(
    db: Client,
    base_url: String,
    local_cache: Option<String>,
    channel: String,
    year: Option<u32>,
    month: Option<u32>,
    day: Option<u32>,
    batch: usize,
    http_concurrency: usize,
    proxy: Option<String>,
) -> anyhow::Result<()> {
    let base_url = base_url.trim_end_matches('/').to_string();
    let options = RunDaysOptions::new(batch, http_concurrency);
    let mut http_builder = HttpClient::builder()
        .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        .pool_max_idle_per_host(options.http_concurrency)
        .user_agent("rustlog-mirror/0.1");
    if let Some(proxy) = proxy {
        http_builder = http_builder.proxy(reqwest::Proxy::all(proxy)?);
    }
    let http = http_builder.build()?;

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

    if tasks.is_empty() {
        return Ok(());
    }

    run_days(
        db,
        &http,
        &base_url,
        local_cache.as_deref(),
        &channel,
        tasks,
        options,
    )
    .await
}

/// Batch mirror: downloads all given days concurrently and inserts through a single writer.
/// Used directly by `fill-missing` to avoid per-day round-trips.
pub async fn run_days(
    db: Client,
    http: &HttpClient,
    base_url: &str,
    local_cache: Option<&str>,
    channel: &str,
    days: Vec<(u32, u32, u32)>,
    options: RunDaysOptions,
) -> anyhow::Result<()> {
    if days.is_empty() {
        return Ok(());
    }

    // Pre-load existing message IDs for the entire date range using channel_id
    // (first column in the primary key — fast index scan instead of full partition scan).
    let (global_start_ms, global_end_ms) = compute_time_range(&days);
    info!("Date range: {} to {}", global_start_ms, global_end_ms);
    let channel_id = resolve_channel_id(&db, channel).await?;
    info!("Resolved channel_id: {:?}", channel_id);

    let seen_ids: Arc<DashSet<Uuid>> = Arc::new(DashSet::new());
    if let Some(ref cid) = channel_id {
        let existing = fetch_existing_ids(&db, cid, global_start_ms, global_end_ms).await?;
        info!("Pre-loaded {} existing message IDs", existing.len());
        for id in existing {
            seen_ids.insert(id);
        }
    }

    // Single writer task. Uses db.insert() directly and reopens every N rows
    // so that each Insert::end() is small and fast (avoids ClickHouse timeout on huge batches).
    let (tx, mut rx) = mpsc::channel::<StructuredMessage<'static>>(CHANNEL_BUF_SIZE);

    let db_writer = db.clone();
    let insert_max_rows = options.insert_max_rows;
    let writer_handle = tokio::spawn(async move {
        info!("Writer task started");
        let mut insert = match db_writer
            .insert(MESSAGES_STRUCTURED_TABLE)
            .context("open insert")
        {
            Ok(i) => i,
            Err(e) => {
                error!("Writer failed to open insert: {}", e);
                return Err(e);
            }
        };
        info!("Insert opened");

        let mut written: u64 = 0;
        let mut first_msg = true;

        while let Some(msg) = rx.recv().await {
            if first_msg {
                info!("Writer received first message");
                first_msg = false;
            }
            // Safety: insert.write serializes immediately; borrow does not outlive the call.
            let msg: StructuredMessage<'static> = unsafe { std::mem::transmute(msg) };
            if let Err(e) = insert.write(&msg).await {
                error!("insert.write failed after {} rows: {}", written, e);
                return Err(e.into());
            }
            written += 1;

            if written % insert_max_rows == 0 {
                info!("Flushing insert after {} rows...", written);
                if let Err(e) = insert.end().await {
                    error!("insert.end failed after {} rows: {}", written, e);
                    return Err(e.into());
                }
                info!("Insert flushed, reopening...");
                insert = match db_writer
                    .insert(MESSAGES_STRUCTURED_TABLE)
                    .context("reopen insert")
                {
                    Ok(i) => i,
                    Err(e) => {
                        error!("Failed to reopen insert after {} rows: {}", written, e);
                        return Err(e);
                    }
                };
            }
        }

        info!(
            "Channel closed, flushing final insert. Total written={}",
            written
        );
        if let Err(e) = insert.end().await {
            error!("final insert.end failed after {} rows: {}", written, e);
            return Err(e.into());
        }
        info!(
            "Writer finished successfully. Total rows written: {}",
            written
        );
        anyhow::Result::<(), anyhow::Error>::Ok(())
    });

    let total = days.len();
    let mut failed: Vec<(u32, u32, u32)> = Vec::new();
    let seen_ids_retry = Arc::clone(&seen_ids);

    {
        let tx_stream = tx.clone();
        let seen_ids_stream = Arc::clone(&seen_ids);
        let mut stream = stream::iter(days.into_iter().enumerate())
            .map(move |(idx, (y, m, d))| {
                let http = http.clone();
                let tx = tx_stream.clone();
                let seen_ids = Arc::clone(&seen_ids_stream);
                let local_cache = local_cache.map(|s| s.to_string());
                let channel = channel.to_owned();
                let base_url = base_url.to_owned();
                async move {
                    let started = Instant::now();
                    let res = process_day(
                        &http,
                        &tx,
                        &seen_ids,
                        local_cache.as_deref(),
                        &channel,
                        &base_url,
                        y,
                        m,
                        d,
                    )
                    .await;
                    (idx, res, started.elapsed(), y, m, d)
                }
            })
            .buffer_unordered(options.http_concurrency);

        while let Some((idx, res, elapsed, y, m, d)) = stream.next().await {
            match res {
                Ok((added, skipped)) => info!(
                    "[{:>3}/{:>3}] {:04}-{:02}-{:02} added={} skipped={} in {:?}",
                    idx + 1,
                    total,
                    y,
                    m,
                    d,
                    added,
                    skipped,
                    elapsed
                ),
                Err(err) => {
                    warn!(
                        "[{:>3}/{:>3}] {:04}-{:02}-{:02} failed: {}",
                        idx + 1,
                        total,
                        y,
                        m,
                        d,
                        err
                    );
                    failed.push((y, m, d));
                }
            }
        }
    }

    // Retry failed days once with a fresh channel (same writer).
    if !failed.is_empty() {
        info!("Retrying {} failed days...", failed.len());
        {
            let tx_retry = tx.clone();
            let mut retry_stream = stream::iter(failed.into_iter().enumerate())
                .map(move |(idx, (y, m, d))| {
                    let http = http.clone();
                    let tx = tx_retry.clone();
                    let seen_ids = Arc::clone(&seen_ids_retry);
                    let local_cache = local_cache.map(|s| s.to_string());
                    let channel = channel.to_owned();
                    let base_url = base_url.to_owned();
                    async move {
                        let started = Instant::now();
                        let res = process_day(
                            &http,
                            &tx,
                            &seen_ids,
                            local_cache.as_deref(),
                            &channel,
                            &base_url,
                            y,
                            m,
                            d,
                        )
                        .await;
                        (idx, res, started.elapsed(), y, m, d)
                    }
                })
                .buffer_unordered(options.http_concurrency);

            while let Some((idx, res, elapsed, y, m, d)) = retry_stream.next().await {
                match res {
                    Ok((added, skipped)) => info!(
                        "[retry {:>3}/{:>3}] {:04}-{:02}-{:02} added={} skipped={} in {:?}",
                        idx + 1,
                        total,
                        y,
                        m,
                        d,
                        added,
                        skipped,
                        elapsed
                    ),
                    Err(err) => warn!(
                        "[retry {:>3}/{:>3}] {:04}-{:02}-{:02} failed: {}",
                        idx + 1,
                        total,
                        y,
                        m,
                        d,
                        err
                    ),
                }
            }
        }
    }

    // Close channel so the writer can finish.
    drop(tx);
    writer_handle
        .await
        .context("writer task join")?
        .context("writer task error")?;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn process_day(
    http: &HttpClient,
    tx: &mpsc::Sender<StructuredMessage<'static>>,
    seen_ids: &DashSet<Uuid>,
    local_cache: Option<&str>,
    channel: &str,
    base_url: &str,
    y: u32,
    m: u32,
    d: u32,
) -> anyhow::Result<(u64, u64)> {
    let msgs_result = if let Some(cache_root) = local_cache {
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
            let mut added: u64 = 0;
            let mut skipped: u64 = 0;

            for msg in msgs {
                if let Some((mapped, parsed_id)) = map_message(channel, msg) {
                    if let Some(id) = parsed_id {
                        // DashSet::insert returns true when the value did not exist.
                        if !seen_ids.insert(id) {
                            skipped += 1;
                            continue;
                        }
                    }
                    tx.send(mapped).await.context("send to writer channel")?;
                    added += 1;
                }
            }

            Ok((added, skipped))
        }
        Err(err) => Err(err),
    }
}

async fn fetch_available_remote(
    http: &HttpClient,
    base_url: &str,
    channel: &str,
) -> anyhow::Result<Vec<AvailableLogEntry>> {
    let url = format!("{}/list?channel={}", base_url, channel);
    let resp: AvailableLogsResp = http.get(url).send().await?.json().await?;
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
    let mut last_err = None;
    for attempt in 0..FETCH_RETRIES {
        match http
            .get(url)
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .send()
            .await
        {
            Ok(response) => {
                let status = response.status();
                let body = response.bytes().await?;
                if !status.is_success() {
                    last_err = Some(anyhow::anyhow!(
                        "http error (attempt {}/{} status={}): body preview: {:.200}",
                        attempt + 1,
                        FETCH_RETRIES,
                        status,
                        String::from_utf8_lossy(&body)
                    ));
                } else {
                    match serde_json::from_slice::<DailyLogResp>(&body) {
                        Ok(resp) => return Ok(resp.messages.unwrap_or_default()),
                        Err(e) => {
                            last_err = Some(anyhow::anyhow!(
                                "decode error (attempt {}/{} status={}): {} | body preview: {:.200}",
                                attempt + 1,
                                FETCH_RETRIES,
                                status,
                                e,
                                String::from_utf8_lossy(&body)
                            ));
                        }
                    }
                }
            }
            Err(e) => {
                last_err = Some(anyhow::anyhow!(
                    "request error (attempt {}/{}): {}",
                    attempt + 1,
                    FETCH_RETRIES,
                    e
                ));
            }
        }
        if attempt + 1 < FETCH_RETRIES {
            let backoff = Duration::from_secs(1 << attempt); // 1s, 2s, 4s
            tokio::time::sleep(backoff).await;
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("fetch_daily_remote exhausted retries")))
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

async fn resolve_channel_id(db: &Client, channel_login: &str) -> anyhow::Result<Option<String>> {
    let esc = channel_login.replace('\'', "\\'");
    let sql = format!(
        "SELECT DISTINCT channel_id FROM {} WHERE channel_login = '{}' LIMIT 1",
        MESSAGES_STRUCTURED_TABLE, esc
    );

    #[derive(Row, Deserialize)]
    struct RowChannelId {
        channel_id: String,
    }

    let mut cursor = db.query(&sql).fetch::<RowChannelId>()?;
    if let Some(row) = cursor.next().await? {
        Ok(Some(row.channel_id))
    } else {
        Ok(None)
    }
}

async fn fetch_existing_ids(
    db: &Client,
    channel_id: &str,
    start_ms: u64,
    end_ms: u64,
) -> anyhow::Result<Vec<Uuid>> {
    let esc = channel_id.replace('\'', "\\'");
    let sql = format!(
        "SELECT id FROM {} \
         WHERE channel_id = '{}' \
           AND timestamp >= {} \
           AND timestamp < {}",
        MESSAGES_STRUCTURED_TABLE, esc, start_ms, end_ms
    );

    #[derive(Row, Deserialize)]
    struct ExistingId {
        #[serde(with = "clickhouse::serde::uuid")]
        id: Uuid,
    }

    let mut ids = Vec::new();
    let mut cursor = db.query(&sql).fetch::<ExistingId>()?;
    while let Some(row) = cursor.next().await? {
        ids.push(row.id);
    }
    Ok(ids)
}

fn compute_time_range(tasks: &[(u32, u32, u32)]) -> (u64, u64) {
    let (min_y, min_m, min_d) = tasks.first().unwrap();
    let (max_y, max_m, max_d) = tasks.last().unwrap();

    let start_ms = chrono::NaiveDate::from_ymd_opt(*min_y as i32, *min_m, *min_d)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|dt| dt.and_utc().timestamp_millis() as u64)
        .unwrap_or(0);

    let end_ms = chrono::NaiveDate::from_ymd_opt(*max_y as i32, *max_m, *max_d)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|dt| dt.and_utc().timestamp_millis() as u64)
        .unwrap_or(0)
        .saturating_add(24 * 60 * 60 * 1000);

    (start_ms, end_ms)
}

fn map_message(
    channel_login: &str,
    msg: RemoteMessage,
) -> Option<(StructuredMessage<'static>, Option<Uuid>)> {
    let ts_str = msg.timestamp?;
    let ts_ms = DateTime::parse_from_rfc3339(&ts_str)
        .ok()?
        .with_timezone(&Utc)
        .timestamp_millis()
        .max(0);

    if ts_ms < MIN_VALID_TS_MS {
        return None;
    }

    let ts = ts_ms as u64;

    let channel_id = msg.tags.get("room-id")?.to_owned();
    let user_id = msg.tags.get("user-id")?.to_owned();
    let parsed_id = msg.id.as_deref().and_then(|s| Uuid::parse_str(s).ok());
    let user_login = msg
        .display_name
        .as_deref()
        .unwrap_or_default()
        .to_lowercase();

    let color = msg
        .tags
        .get("color")
        .map(|c: &String| c.strip_prefix('#').unwrap_or(c.as_str()))
        .and_then(|c| u32::from_str_radix(c, 16).ok());

    let badges: Vec<Cow<'static, str>> = msg
        .tags
        .get("badges")
        .map(|b: &String| {
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
        id: parsed_id.unwrap_or_else(Uuid::new_v4),
        message_type: MessageType::PrivMsg,
        user_id: user_id.into(),
        user_login: user_login.into(),
        display_name: display_name.into(),
        color,
        user_type: msg
            .tags
            .get("user-type")
            .cloned()
            .unwrap_or_default()
            .into(),
        badges,
        badge_info: msg
            .tags
            .get("badge-info")
            .cloned()
            .unwrap_or_default()
            .into(),
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

    Some((structured, parsed_id))
}
