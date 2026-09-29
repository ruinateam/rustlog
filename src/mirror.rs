use crate::{
    state::OperationalState,
    storage::message::{MESSAGES_STRUCTURED_TABLE, MessageFlags, MessageType, StructuredMessage},
};
use anyhow::Context;
use chrono::{DateTime, Utc};
use clickhouse::{Client, Row};
use dashmap::DashSet;
use futures::{StreamExt, stream};
use rand::RngExt;
use reqwest::{Client as HttpClient, StatusCode};
use serde::Deserialize;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MIN_VALID_TS_MS: i64 = 1_577_836_800_000; // 2020-01-01T00:00:00Z in ms
const FETCH_RETRIES: usize = 8;
const CHANNEL_BUF_SIZE: usize = 100_000;
const DEFAULT_RPS: f64 = 2.0;
const MAX_RETRY_AFTER: Duration = Duration::from_secs(120);
const MIN_RETRY_AFTER: Duration = Duration::from_secs(1);

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

/// Shared rate-limited HTTP client pool for mirror / fill-missing.
#[derive(Clone)]
pub struct MirrorHttp {
    inner: Arc<MirrorHttpInner>,
}

struct MirrorHttpInner {
    slots: Vec<MirrorSlot>,
    rr: AtomicUsize,
}

struct MirrorSlot {
    client: HttpClient,
    limiter: SlotLimiter,
    label: String,
}

struct SlotLimiter {
    min_interval: Duration,
    next_free: Mutex<Instant>,
}

impl SlotLimiter {
    fn new(rps: f64) -> Self {
        let rps = if rps.is_finite() && rps > 0.0 {
            rps
        } else {
            DEFAULT_RPS
        };
        Self {
            min_interval: Duration::from_secs_f64(1.0 / rps),
            next_free: Mutex::new(Instant::now()),
        }
    }

    async fn acquire(&self) {
        let wait = {
            let mut next = self.next_free.lock().await;
            let now = Instant::now();
            if *next <= now {
                *next = now + self.min_interval;
                Duration::ZERO
            } else {
                let wait = *next - now;
                *next += self.min_interval;
                wait
            }
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }

    async fn penalize(&self, delay: Duration) {
        let mut next = self.next_free.lock().await;
        let candidate = Instant::now() + delay;
        if candidate > *next {
            *next = candidate;
        }
    }
}

impl MirrorHttp {
    pub fn new(
        proxies: &[String],
        rps: f64,
        user_agent: &'static str,
        pool_max_idle_per_host: usize,
    ) -> anyhow::Result<Self> {
        let rps = if rps.is_finite() && rps > 0.0 {
            rps
        } else {
            DEFAULT_RPS
        };
        let proxies: Vec<String> = proxies
            .iter()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();

        let mut slots = Vec::new();
        if proxies.is_empty() {
            slots.push(build_slot(
                None,
                "direct",
                rps,
                user_agent,
                pool_max_idle_per_host,
            )?);
        } else {
            for proxy in &proxies {
                let label = proxy_label(proxy);
                slots.push(build_slot(
                    Some(proxy.as_str()),
                    &label,
                    rps,
                    user_agent,
                    pool_max_idle_per_host,
                )?);
            }
        }

        info!(
            slots = slots.len(),
            rps_per_slot = rps,
            total_rps = rps * slots.len() as f64,
            "created HTTP pool"
        );

        Ok(Self {
            inner: Arc::new(MirrorHttpInner {
                slots,
                rr: AtomicUsize::new(0),
            }),
        })
    }

    fn next_slot_index(&self) -> usize {
        let n = self.inner.slots.len().max(1);
        self.inner.rr.fetch_add(1, Ordering::Relaxed) % n
    }

    pub async fn get_bytes(&self, url: &str, timeout: Duration) -> anyhow::Result<Vec<u8>> {
        let mut last_err = None;

        for attempt in 0..FETCH_RETRIES {
            let slot_idx = self.next_slot_index();
            let slot = &self.inner.slots[slot_idx];
            slot.limiter.acquire().await;

            match slot.client.get(url).timeout(timeout).send().await {
                Ok(response) => {
                    let status = response.status();
                    let retry_after = parse_retry_after(response.headers());

                    if status == StatusCode::TOO_MANY_REQUESTS
                        || status == StatusCode::SERVICE_UNAVAILABLE
                        || status.as_u16() == 408
                    {
                        let delay = rate_limit_delay(attempt, retry_after);
                        warn!(
                            slot = %slot.label,
                            status = status.as_u16(),
                            attempt = attempt + 1,
                            max_attempts = FETCH_RETRIES,
                            retry_in_ms = delay.as_millis() as u64,
                            "rate limited, backing off"
                        );
                        slot.limiter.penalize(delay).await;
                        tokio::time::sleep(delay).await;
                        last_err = Some(anyhow::anyhow!(
                            "http {} from {} (attempt {}/{})",
                            status.as_u16(),
                            slot.label,
                            attempt + 1,
                            FETCH_RETRIES
                        ));
                        continue;
                    }

                    let body = response.bytes().await?;
                    if !status.is_success() {
                        last_err = Some(anyhow::anyhow!(
                            "http error slot={} attempt={}/{} status={}: body preview: {:.200}",
                            slot.label,
                            attempt + 1,
                            FETCH_RETRIES,
                            status,
                            String::from_utf8_lossy(&body)
                        ));
                        if status.is_server_error() && attempt + 1 < FETCH_RETRIES {
                            let delay = normal_backoff(attempt);
                            slot.limiter.penalize(delay).await;
                            tokio::time::sleep(delay).await;
                            continue;
                        }
                        break;
                    }

                    return Ok(body.to_vec());
                }
                Err(e) => {
                    last_err = Some(anyhow::anyhow!(
                        "request error slot={} attempt={}/{}: {}",
                        slot.label,
                        attempt + 1,
                        FETCH_RETRIES,
                        e
                    ));
                    if attempt + 1 < FETCH_RETRIES {
                        let delay = normal_backoff(attempt);
                        slot.limiter.penalize(delay).await;
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("mirror get_bytes exhausted retries")))
    }

    pub async fn get_json<T: for<'de> Deserialize<'de>>(
        &self,
        url: &str,
        timeout: Duration,
    ) -> anyhow::Result<T> {
        let body = self.get_bytes(url, timeout).await?;
        serde_json::from_slice(&body).with_context(|| {
            format!(
                "json decode failed ({} bytes), preview: {:.200}",
                body.len(),
                String::from_utf8_lossy(&body)
            )
        })
    }
}

fn build_slot(
    proxy: Option<&str>,
    label: &str,
    rps: f64,
    user_agent: &'static str,
    pool_max_idle_per_host: usize,
) -> anyhow::Result<MirrorSlot> {
    let mut builder = HttpClient::builder()
        .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        .pool_max_idle_per_host(pool_max_idle_per_host.max(1))
        .user_agent(user_agent);
    if let Some(proxy) = proxy {
        builder = builder.proxy(reqwest::Proxy::all(proxy)?);
    }
    Ok(MirrorSlot {
        client: builder.build()?,
        limiter: SlotLimiter::new(rps),
        label: label.to_owned(),
    })
}

fn proxy_label(proxy: &str) -> String {
    // Avoid dumping credentials into logs.
    if let Ok(url) = reqwest::Url::parse(proxy) {
        let host = url.host_str().unwrap_or("proxy");
        let port = url
            .port_or_known_default()
            .map(|p| format!(":{p}"))
            .unwrap_or_default();
        return format!("proxy://{host}{port}");
    }
    "proxy".to_owned()
}

fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    None
}

fn rate_limit_delay(attempt: usize, retry_after: Option<Duration>) -> Duration {
    let base = retry_after.unwrap_or_else(|| {
        Duration::from_secs(2u64.saturating_pow(attempt.min(5) as u32)).max(MIN_RETRY_AFTER)
    });
    let capped = base.min(MAX_RETRY_AFTER).max(MIN_RETRY_AFTER);
    let jitter_ms = rand::rng().random_range(0..500);
    capped + Duration::from_millis(jitter_ms)
}

fn normal_backoff(attempt: usize) -> Duration {
    let base = Duration::from_secs(1u64 << attempt.min(4));
    let jitter_ms = rand::rng().random_range(0..250);
    base + Duration::from_millis(jitter_ms)
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

/// Options of [`run`].
pub struct MirrorOptions {
    /// Base URL of the remote rustlog or justlog instance.
    pub base_url: String,
    /// Read the logs from this local cache directory instead of over HTTP.
    pub local_cache: Option<String>,
    /// Login of the channel to mirror.
    pub channel: String,
    /// When given, only days in this year.
    pub year: Option<u32>,
    /// When given, only days in this month (1-12).
    pub month: Option<u32>,
    /// When given, only this day of the month.
    pub day: Option<u32>,
    /// Rows per ClickHouse insert.
    pub batch: usize,
    /// Days fetched in parallel.
    pub http_concurrency: usize,
    /// HTTP(S) proxies to spread the requests over.
    pub proxies: Vec<String>,
    /// Requests per second per proxy; a direct connection counts as one.
    pub rps: f64,
}

/// Mirror remote logs for a specific channel.
pub async fn run(db: Client, options: MirrorOptions) -> anyhow::Result<()> {
    let MirrorOptions {
        base_url,
        local_cache,
        channel,
        year,
        month,
        day,
        batch,
        http_concurrency,
        proxies,
        rps,
    } = options;
    let base_url = base_url.trim_end_matches('/').to_string();
    let options = RunDaysOptions::new(batch, http_concurrency);
    let http = MirrorHttp::new(
        &proxies,
        rps,
        "rustlog-mirror/0.2",
        options.http_concurrency,
    )?;

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
            year.is_none_or(|yy| yy == *y)
                && month.is_none_or(|mm| mm == *m)
                && day.is_none_or(|dd| dd == *d)
        })
        .collect();

    tasks.sort();
    info!(days = tasks.len(), "found days to mirror");

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
pub async fn run_days(
    db: Client,
    http: &MirrorHttp,
    base_url: &str,
    local_cache: Option<&str>,
    channel: &str,
    days: Vec<(u32, u32, u32)>,
    options: RunDaysOptions,
) -> anyhow::Result<()> {
    if days.is_empty() {
        return Ok(());
    }

    let state = OperationalState::load(Arc::new(db.clone())).await?;

    let (global_start_ms, global_end_ms) = compute_time_range(&days);
    info!(
        from_ms = global_start_ms,
        to_ms = global_end_ms,
        "mirroring time range"
    );
    let channel_id = resolve_channel_id(&db, channel).await?;
    info!(channel_id = ?channel_id, "resolved channel id");

    let seen_ids: Arc<DashSet<Uuid>> = Arc::new(DashSet::new());
    if let Some(ref cid) = channel_id {
        let existing = fetch_existing_ids(&db, cid, global_start_ms, global_end_ms).await?;
        info!(ids = existing.len(), "loaded existing message ids");
        for id in existing {
            seen_ids.insert(id);
        }
    }

    let (tx, mut rx) = mpsc::channel::<StructuredMessage<'static>>(CHANNEL_BUF_SIZE);

    let db_writer = db.clone();
    let writer_state = state.clone();
    let insert_max_rows = options.insert_max_rows;
    let writer_handle = tokio::spawn(async move {
        debug!("writer started");
        let mut insert = match db_writer
            .insert::<StructuredMessage<'static>>(MESSAGES_STRUCTURED_TABLE)
            .await
            .context("open insert")
        {
            Ok(i) => i,
            Err(e) => {
                error!(error = %e, "could not open an insert");
                return Err(e);
            }
        };
        debug!("opened insert");

        let mut written: u64 = 0;
        let mut first_msg = true;

        while let Some(msg) = rx.recv().await {
            if !writer_state.permits_historical_message(&msg.channel_id, &msg.user_id) {
                continue;
            }

            if first_msg {
                debug!("writer received its first message");
                first_msg = false;
            }
            if let Err(e) = insert.write(&msg).await {
                error!(rows = written, error = %e, "could not write a row");
                return Err(e.into());
            }
            written += 1;

            if written.is_multiple_of(insert_max_rows) {
                info!(rows = written, "flushing insert");
                if let Err(e) = insert.end().await {
                    error!(rows = written, error = %e, "could not finish an insert");
                    return Err(e.into());
                }
                debug!("flushed insert, opening the next one");
                insert = match db_writer
                    .insert::<StructuredMessage<'static>>(MESSAGES_STRUCTURED_TABLE)
                    .await
                    .context("reopen insert")
                {
                    Ok(i) => i,
                    Err(e) => {
                        error!(rows = written, error = %e, "could not open the next insert");
                        return Err(e);
                    }
                };
            }
        }

        info!(rows = written, "flushing the final insert");
        if let Err(e) = insert.end().await {
            error!(rows = written, error = %e, "could not finish the final insert");
            return Err(e.into());
        }
        info!(rows = written, "writer finished");
        anyhow::Result::<(), anyhow::Error>::Ok(())
    });

    let total = days.len();
    let mut failed: Vec<(u32, u32, u32)> = Vec::new();
    let seen_ids_retry = Arc::clone(&seen_ids);

    {
        let tx_stream = tx.clone();
        let seen_ids_stream = Arc::clone(&seen_ids);
        let state_stream = state.clone();
        let http = http.clone();
        let mut stream = stream::iter(days.into_iter().enumerate())
            .map(move |(idx, (y, m, d))| {
                let http = http.clone();
                let tx = tx_stream.clone();
                let seen_ids = Arc::clone(&seen_ids_stream);
                let local_cache = local_cache.map(|s| s.to_string());
                let channel = channel.to_owned();
                let base_url = base_url.to_owned();
                let state = state_stream.clone();
                async move {
                    let started = Instant::now();
                    let res = process_day(
                        &http,
                        &tx,
                        &seen_ids,
                        &state,
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
                Ok(stats) => info!(
                    day = idx + 1,
                    days = total,
                    date = format!("{y:04}-{m:02}-{d:02}"),
                    added = stats.added,
                    skipped_duplicates = stats.skipped_dup,
                    skipped_without_id = stats.skipped_no_id,
                    skipped_opted_out = stats.skipped_optout,
                    took_ms = elapsed.as_millis() as u64,
                    "mirrored day"
                ),
                Err(err) => {
                    warn!(
                        day = idx + 1,
                        days = total,
                        date = format!("{y:04}-{m:02}-{d:02}"),
                        error = format!("{err:#}"),
                        "could not mirror day"
                    );
                    failed.push((y, m, d));
                }
            }
        }
    }

    if !failed.is_empty() {
        info!(days = failed.len(), "retrying failed days");
        {
            let tx_retry = tx.clone();
            let state_retry = state.clone();
            let http = http.clone();
            let mut retry_stream = stream::iter(failed.into_iter().enumerate())
                .map(move |(idx, (y, m, d))| {
                    let http = http.clone();
                    let tx = tx_retry.clone();
                    let seen_ids = Arc::clone(&seen_ids_retry);
                    let local_cache = local_cache.map(|s| s.to_string());
                    let channel = channel.to_owned();
                    let base_url = base_url.to_owned();
                    let state = state_retry.clone();
                    async move {
                        let started = Instant::now();
                        let res = process_day(
                            &http,
                            &tx,
                            &seen_ids,
                            &state,
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
                    Ok(stats) => info!(
                        day = idx + 1,
                        days = total,
                        date = format!("{y:04}-{m:02}-{d:02}"),
                        added = stats.added,
                        skipped_duplicates = stats.skipped_dup,
                        skipped_without_id = stats.skipped_no_id,
                        skipped_opted_out = stats.skipped_optout,
                        took_ms = elapsed.as_millis() as u64,
                        "mirrored day on retry"
                    ),
                    Err(err) => warn!(
                        day = idx + 1,
                        days = total,
                        date = format!("{y:04}-{m:02}-{d:02}"),
                        error = format!("{err:#}"),
                        "could not mirror day on retry"
                    ),
                }
            }
        }
    }

    drop(tx);
    writer_handle
        .await
        .context("writer task join")?
        .context("writer task error")?;

    Ok(())
}

struct DayStats {
    added: u64,
    skipped_dup: u64,
    skipped_no_id: u64,
    skipped_optout: u64,
}

#[allow(clippy::too_many_arguments)]
async fn process_day(
    http: &MirrorHttp,
    tx: &mpsc::Sender<StructuredMessage<'static>>,
    seen_ids: &DashSet<Uuid>,
    state: &OperationalState,
    local_cache: Option<&str>,
    channel: &str,
    base_url: &str,
    y: u32,
    m: u32,
    d: u32,
) -> anyhow::Result<DayStats> {
    let msgs_result = if let Some(cache_root) = local_cache {
        fetch_daily_local(cache_root, channel, y, m, d)
    } else {
        let url = format!(
            "{}/channel/{}/{:04}/{:02}/{:02}?jsonBasic=1",
            base_url, channel, y, m, d
        );
        fetch_daily_remote(http, &url).await
    };

    match msgs_result {
        Ok(msgs) => {
            let mut stats = DayStats {
                added: 0,
                skipped_dup: 0,
                skipped_no_id: 0,
                skipped_optout: 0,
            };

            for msg in msgs {
                match map_message(channel, msg) {
                    MapResult::Ok(mapped, id) => {
                        if !state.permits_historical_message(&mapped.channel_id, &mapped.user_id) {
                            stats.skipped_optout += 1;
                            continue;
                        }
                        if !seen_ids.insert(id) {
                            stats.skipped_dup += 1;
                            continue;
                        }
                        tx.send(*mapped).await.context("send to writer channel")?;
                        stats.added += 1;
                    }
                    MapResult::SkipNoId => stats.skipped_no_id += 1,
                    MapResult::SkipInvalid => {}
                }
            }

            Ok(stats)
        }
        Err(err) => Err(err),
    }
}

async fn fetch_available_remote(
    http: &MirrorHttp,
    base_url: &str,
    channel: &str,
) -> anyhow::Result<Vec<AvailableLogEntry>> {
    let url = format!("{}/list?channel={}", base_url, channel);
    let resp: AvailableLogsResp = http
        .get_json(&url, Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        .await?;
    Ok(resp.available_logs.unwrap_or_default())
}

fn fetch_available_local(
    cache_root: &str,
    channel: &str,
) -> anyhow::Result<Vec<AvailableLogEntry>> {
    let mut out = Vec::new();
    let channel_dir = std::path::Path::new(cache_root).join(channel).join("daily");
    if !channel_dir.is_dir() {
        return Ok(out);
    }
    for year_entry in std::fs::read_dir(&channel_dir)? {
        let year_entry = year_entry?;
        if !year_entry.file_type()?.is_dir() {
            continue;
        }
        let year_name = year_entry.file_name().to_string_lossy().into_owned();
        for month_entry in std::fs::read_dir(year_entry.path())? {
            let month_entry = month_entry?;
            if !month_entry.file_type()?.is_dir() {
                continue;
            }
            let month_name = month_entry.file_name().to_string_lossy().into_owned();
            for day_file in std::fs::read_dir(month_entry.path())? {
                let day_file = day_file?;
                if !day_file.file_type()?.is_file() {
                    continue;
                }
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

async fn fetch_daily_remote(http: &MirrorHttp, url: &str) -> anyhow::Result<Vec<RemoteMessage>> {
    let resp: DailyLogResp = http
        .get_json(url, Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        .await?;
    Ok(resp.messages.unwrap_or_default())
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

enum MapResult {
    Ok(Box<StructuredMessage<'static>>, Uuid),
    SkipNoId,
    SkipInvalid,
}

fn parse_message_id(msg: &RemoteMessage) -> Option<Uuid> {
    msg.id
        .as_deref()
        .or_else(|| msg.tags.get("id").map(String::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| Uuid::parse_str(s).ok())
}

fn map_message(channel_login: &str, msg: RemoteMessage) -> MapResult {
    let Some(parsed_id) = parse_message_id(&msg) else {
        return MapResult::SkipNoId;
    };

    let Some(ts_str) = msg.timestamp else {
        return MapResult::SkipInvalid;
    };
    let Some(ts_ms) = DateTime::parse_from_rfc3339(&ts_str)
        .ok()
        .map(|dt| dt.with_timezone(&Utc).timestamp_millis().max(0))
    else {
        return MapResult::SkipInvalid;
    };

    if ts_ms < MIN_VALID_TS_MS {
        return MapResult::SkipInvalid;
    }

    let ts = ts_ms as u64;

    let Some(channel_id) = msg.tags.get("room-id").cloned() else {
        return MapResult::SkipInvalid;
    };
    let Some(user_id) = msg.tags.get("user-id").cloned() else {
        return MapResult::SkipInvalid;
    };
    if user_id.is_empty() {
        return MapResult::SkipInvalid;
    }

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
        id: parsed_id,
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

    MapResult::Ok(Box::new(structured), parsed_id)
}

#[cfg(test)]
mod tests {
    use super::{MapResult, RemoteMessage, map_message, parse_message_id};
    use std::collections::HashMap;
    use uuid::Uuid;

    fn base_msg() -> RemoteMessage {
        let mut tags = HashMap::new();
        tags.insert("room-id".into(), "123".into());
        tags.insert("user-id".into(), "456".into());
        tags.insert("display-name".into(), "Alice".into());
        RemoteMessage {
            text: Some("hi".into()),
            display_name: Some("Alice".into()),
            timestamp: Some("2024-01-02T03:04:05.678Z".into()),
            id: Some("00000000-0000-4000-8000-000000000001".into()),
            tags,
        }
    }

    #[test]
    fn requires_message_id() {
        let mut msg = base_msg();
        msg.id = None;
        assert!(matches!(map_message("chan", msg), MapResult::SkipNoId));
    }

    #[test]
    fn accepts_id_from_tags() {
        let mut msg = base_msg();
        msg.id = None;
        msg.tags
            .insert("id".into(), "00000000-0000-4000-8000-000000000002".into());
        let id = parse_message_id(&msg).unwrap();
        assert_eq!(
            id,
            Uuid::parse_str("00000000-0000-4000-8000-000000000002").unwrap()
        );
        assert!(matches!(map_message("chan", msg), MapResult::Ok(_, _)));
    }

    #[test]
    fn requires_user_id() {
        let mut msg = base_msg();
        msg.tags.remove("user-id");
        assert!(matches!(map_message("chan", msg), MapResult::SkipInvalid));
    }
}
