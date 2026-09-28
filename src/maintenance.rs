use anyhow::{anyhow, Context};
use chrono::{Datelike, NaiveDate};
use clickhouse::{Client, Row};
use futures::{stream, StreamExt};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    time::Duration,
};
use tracing::{info, warn};

use crate::mirror::MirrorHttp;

const NIL_UUID: &str = "00000000-0000-0000-0000-000000000000";
const DEFAULT_EXCLUDED_INSTANCES: &[&str] = &["https://logs.twitchmetrics.xyz"];
const HTTP_TIMEOUT_SECS: u64 = 120;
const REPAIR_DAY_TIMEOUT_SECS: u64 = 90;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ZonianApiResponse {
    logged_data: ZonianLoggedData,
    channel_logs: ZonianChannelLogs,
}

#[derive(Deserialize)]
struct ZonianLoggedData {
    list: Vec<ZonianLogDay>,
}

#[derive(Deserialize)]
struct ZonianLogDay {
    year: String,
    month: String,
    day: String,
}

#[derive(Deserialize)]
struct ZonianChannelLogs {
    instances: Vec<String>,
}

#[derive(Row, Deserialize)]
struct LocalDayRow {
    channel_login: String,
    day: String,
    rows: u64,
    unique_ids: u64,
}

#[derive(Clone, Copy)]
struct LocalDayStats {
    rows: u64,
    unique_ids: u64,
}

#[derive(Deserialize)]
struct RemoteDayResponse<'a> {
    #[serde(default, borrow)]
    messages: Vec<RemoteDayMessage<'a>>,
}

#[derive(Deserialize)]
struct RemoteDayMessage<'a> {
    #[serde(default, borrow)]
    id: Option<&'a str>,
}

struct RepairCandidate {
    channel: String,
    date: NaiveDate,
    local_unique_ids: u64,
    remote_unique_ids: u64,
    base_url: String,
}

#[derive(Row, Deserialize)]
struct DuplicateSummaryRow {
    channel_login: String,
    rows: u64,
    unique_ids: u64,
    duplicate_rows: u64,
}

#[derive(Row, Deserialize)]
struct DuplicateSampleRow {
    channel_login: String,
    message_id: String,
    copies: u64,
    first_ts: String,
    last_ts: String,
}

pub struct FillMissingOptions {
    pub channels: Vec<String>,
    pub year: u32,
    pub api_base: String,
    pub batch: usize,
    pub http_concurrency: usize,
    pub proxies: Vec<String>,
    pub rps: f64,
    pub exclude_instances: Vec<String>,
    pub dry_run: bool,
    pub repair_existing: bool,
    pub deep: bool,
}

pub struct CleanupDuplicateIdsOptions {
    pub channels: Vec<String>,
    pub year: Option<u32>,
    pub execute: bool,
    pub sample_limit: usize,
    pub wait_timeout: u64,
}

pub async fn fill_missing(db: Client, options: FillMissingOptions) -> anyhow::Result<()> {
    let http_concurrency = options.http_concurrency.max(1);
    let mirror_options = crate::mirror::RunDaysOptions::new(options.batch, http_concurrency);
    let http = MirrorHttp::new(
        &options.proxies,
        options.rps,
        "rustlog-fill-missing/0.2",
        http_concurrency,
    )?;
    let excluded_instances: HashSet<String> = DEFAULT_EXCLUDED_INSTANCES
        .iter()
        .map(|instance| instance.to_string())
        .chain(
            options
                .exclude_instances
                .iter()
                .map(|instance| instance.trim_end_matches('/').to_string()),
        )
        .collect();

    let mut expected_by_channel = BTreeMap::new();
    let mut instances_by_channel = BTreeMap::new();

    for channel in &options.channels {
        let api = fetch_zonian_api(&http, &options.api_base, channel).await?;
        let expected_days = api
            .logged_data
            .list
            .into_iter()
            .filter_map(|entry| {
                let year: u32 = entry.year.parse().ok()?;
                if year != options.year {
                    return None;
                }
                let month: u32 = entry.month.parse().ok()?;
                let day: u32 = entry.day.parse().ok()?;
                NaiveDate::from_ymd_opt(year as i32, month, day)
            })
            .collect::<BTreeSet<_>>();
        let instances = api
            .channel_logs
            .instances
            .into_iter()
            .map(|instance| instance.trim_end_matches('/').to_string())
            .filter(|instance| !excluded_instances.contains(instance))
            .collect::<Vec<_>>();

        info!(
            "{}: expected_days={} instances={:?}",
            channel,
            expected_days.len(),
            instances
        );
        expected_by_channel.insert(channel.clone(), expected_days);
        instances_by_channel.insert(channel.clone(), instances);
    }

    let local_days_snapshot = read_local_days(&db, &options.channels, options.year).await?;
    let mut local_days = local_days_snapshot.clone();
    let mut missing_by_channel = BTreeMap::new();
    for channel in &options.channels {
        let expected = expected_by_channel
            .get(channel)
            .ok_or_else(|| anyhow!("Missing expected days for {channel}"))?;
        let local = local_days.remove(channel).unwrap_or_default();
        let missing = expected
            .difference(&local.keys().copied().collect::<BTreeSet<_>>())
            .copied()
            .collect::<Vec<_>>();
        info!(
            "{}: local_days={} missing={}",
            channel,
            local.len(),
            missing.len()
        );
        if !missing.is_empty() {
            info!(
                "{} missing: {}",
                channel,
                missing
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }
        missing_by_channel.insert(channel.clone(), missing);
    }

    if options.dry_run {
        if options.repair_existing {
            let repair_candidates = find_repair_candidates(
                &http,
                &options,
                &expected_by_channel,
                &instances_by_channel,
                &local_days_snapshot,
            )
            .await?;
            log_repair_candidates(&repair_candidates);
        }
        return Ok(());
    }

    let mut failures = Vec::new();
    for channel in &options.channels {
        let missing = missing_by_channel
            .get(channel)
            .ok_or_else(|| anyhow!("Missing work list for {channel}"))?;
        let instances = instances_by_channel
            .get(channel)
            .ok_or_else(|| anyhow!("Missing instances for {channel}"))?;

        if missing.is_empty() {
            continue;
        }

        let mut remaining: Vec<NaiveDate> = missing.iter().copied().collect();

        for base_url in instances {
            if remaining.is_empty() {
                break;
            }

            let days: Vec<(u32, u32, u32)> = remaining
                .iter()
                .map(|d| (d.year() as u32, d.month(), d.day()))
                .collect();

            info!(
                "try {} batch {} days base={}",
                channel,
                days.len(),
                base_url
            );

            let res = crate::mirror::run_days(
                db.clone(),
                &http,
                base_url,
                None,
                channel,
                days,
                mirror_options,
            )
            .await;

            if let Err(err) = res {
                warn!(
                    "mirror batch failed channel={} base={}: {err}",
                    channel, base_url
                );
            }

            // Check which days got filled after this batch.
            let current = read_local_days(&db, &[channel.clone()], options.year).await?;
            let filled_days: BTreeSet<NaiveDate> = current
                .get(channel)
                .map(|days| days.keys().copied().collect())
                .unwrap_or_default();

            let filled_count = remaining.len();
            remaining.retain(|d| !filled_days.contains(d));
            let newly_filled = filled_count - remaining.len();

            info!(
                "ok {} filled={}/{} remaining={} base={}",
                channel,
                newly_filled,
                filled_count,
                remaining.len(),
                base_url
            );
        }

        for date in &remaining {
            warn!("still missing {} {}", channel, date);
            failures.push((channel.clone(), *date));
        }
    }

    if options.repair_existing {
        let current_days = read_local_days(&db, &options.channels, options.year).await?;
        let repair_candidates = find_repair_candidates(
            &http,
            &options,
            &expected_by_channel,
            &instances_by_channel,
            &current_days,
        )
        .await?;
        log_repair_candidates(&repair_candidates);
        run_repair_candidates(&db, &http, &repair_candidates, mirror_options).await?;
    }

    let final_days = read_local_days(&db, &options.channels, options.year).await?;
    for channel in &options.channels {
        let expected = expected_by_channel
            .get(channel)
            .ok_or_else(|| anyhow!("Missing expected days for {channel}"))?;
        let local = final_days.get(channel).cloned().unwrap_or_default();
        let missing = expected
            .difference(&local.keys().copied().collect::<BTreeSet<_>>())
            .copied()
            .collect::<Vec<_>>();
        let rows = local.values().map(|stats| stats.rows).sum::<u64>();
        info!(
            "summary {}: days={}/{} rows={} missing={}",
            channel,
            local.len(),
            expected.len(),
            rows,
            missing.len()
        );
        if !missing.is_empty() {
            info!(
                "summary missing {}: {}",
                channel,
                missing
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }
    }

    if !failures.is_empty() {
        return Err(anyhow!("{} days are still missing", failures.len()));
    }
    Ok(())
}

pub async fn cleanup_duplicate_ids(
    db: Client,
    options: CleanupDuplicateIdsOptions,
) -> anyhow::Result<()> {
    let _wait_timeout = options.wait_timeout;
    let summary = duplicate_summary(&db, &options).await?;
    if summary.is_empty() {
        info!("No duplicate message ids found in selected scope");
        return Ok(());
    }

    info!("Duplicate message ids:");
    for row in &summary {
        info!(
            "{} rows={} unique_ids={} duplicate_rows={}",
            row.channel_login, row.rows, row.unique_ids, row.duplicate_rows
        );
    }

    let samples = duplicate_samples(&db, &options).await?;
    for sample in samples {
        info!(
            "sample channel={} id={} copies={} first={} last={}",
            sample.channel_login, sample.message_id, sample.copies, sample.first_ts, sample.last_ts
        );
    }

    if !options.execute {
        info!("Dry-run only. Re-run with --execute to rewrite duplicate ids.");
        return Ok(());
    }

    execute_duplicate_cleanup(&db, &options).await?;
    let remaining = duplicate_summary(&db, &options).await?;
    if remaining.is_empty() {
        info!("Duplicate cleanup finished");
        Ok(())
    } else {
        Err(anyhow!(
            "Duplicate cleanup finished with remaining duplicates"
        ))
    }
}

async fn fetch_zonian_api(
    http: &MirrorHttp,
    api_base: &str,
    channel: &str,
) -> anyhow::Result<ZonianApiResponse> {
    let url = format!("{}/api/{}", api_base.trim_end_matches('/'), channel);
    http.get_json(&url, Duration::from_secs(HTTP_TIMEOUT_SECS))
        .await
}

async fn find_repair_candidates(
    http: &MirrorHttp,
    options: &FillMissingOptions,
    expected_by_channel: &BTreeMap<String, BTreeSet<NaiveDate>>,
    instances_by_channel: &BTreeMap<String, Vec<String>>,
    local_days: &BTreeMap<String, BTreeMap<NaiveDate, LocalDayStats>>,
) -> anyhow::Result<Vec<RepairCandidate>> {
    let mut jobs = Vec::new();

    for channel in &options.channels {
        let expected = expected_by_channel
            .get(channel)
            .ok_or_else(|| anyhow!("Missing expected days for {channel}"))?;
        let instances = instances_by_channel
            .get(channel)
            .ok_or_else(|| anyhow!("Missing instances for {channel}"))?;
        let local = local_days.get(channel);

        for date in expected {
            if let Some(stats) = local.and_then(|days| days.get(date)).copied() {
                jobs.push((channel.clone(), *date, stats, instances.clone()));
            }
        }
    }

    if jobs.is_empty() {
        return Ok(Vec::new());
    }

    let requested_concurrency = options.http_concurrency.max(1);
    let max_mirrors_per_day = jobs
        .iter()
        .map(|(_, _, _, instances)| {
            if options.deep {
                instances.len().max(1)
            } else {
                1
            }
        })
        .max()
        .unwrap_or(1);
    let day_concurrency = if options.deep {
        (requested_concurrency / max_mirrors_per_day).max(1)
    } else {
        requested_concurrency
    };

    info!(
        "repair-existing: checking {} existing days with day_concurrency={} request_concurrency={} deep={}",
        jobs.len(),
        day_concurrency,
        requested_concurrency,
        options.deep
    );

    let mut candidates = Vec::new();
    let mut stream = stream::iter(jobs)
        .map(|(channel, date, local_stats, instances)| {
            let http = http.clone();
            let deep = options.deep;
            async move {
                find_repair_candidate_for_day(&http, channel, date, local_stats, instances, deep)
                    .await
            }
        })
        .buffer_unordered(day_concurrency);

    while let Some(candidate) = stream.next().await {
        if let Some(candidate) = candidate? {
            candidates.push(candidate);
        }
    }

    candidates.sort_by(|a, b| a.channel.cmp(&b.channel).then_with(|| a.date.cmp(&b.date)));
    Ok(candidates)
}

async fn find_repair_candidate_for_day(
    http: &MirrorHttp,
    channel: String,
    date: NaiveDate,
    local_stats: LocalDayStats,
    instances: Vec<String>,
    deep: bool,
) -> anyhow::Result<Option<RepairCandidate>> {
    let mut failed_checks = 0_u32;
    let mut first_error = None;
    let best = if deep {
        let mut best: Option<(String, u64)> = None;
        let mirror_concurrency = instances.len().max(1);
        let mut stream = stream::iter(instances)
            .map(|base_url| {
                let http = http.clone();
                let channel = channel.clone();
                async move {
                    let result =
                        fetch_remote_day_unique_ids(&http, &base_url, &channel, date).await;
                    (base_url, result)
                }
            })
            .buffer_unordered(mirror_concurrency);

        while let Some((base_url, result)) = stream.next().await {
            match result {
                Ok(remote_unique_ids) => {
                    if best
                        .as_ref()
                        .map_or(true, |(_, best_count)| remote_unique_ids > *best_count)
                    {
                        best = Some((base_url, remote_unique_ids));
                    }
                }
                Err(err) => {
                    failed_checks += 1;
                    if first_error.is_none() {
                        first_error = Some(format!("{base_url}: {err}"));
                    }
                }
            }
        }

        best
    } else {
        let mut best: Option<(String, u64)> = None;
        for base_url in instances {
            match fetch_remote_day_unique_ids(http, &base_url, &channel, date).await {
                Ok(remote_unique_ids) => {
                    best = Some((base_url, remote_unique_ids));
                    break;
                }
                Err(err) => {
                    failed_checks += 1;
                    if first_error.is_none() {
                        first_error = Some(format!("{base_url}: {err}"));
                    }
                }
            }
        }

        best
    };

    let Some((base_url, remote_unique_ids)) = best else {
        warn!(
            "repair-existing: no readable mirrors channel={} date={} failed_checks={} first_error={}",
            channel,
            date,
            failed_checks,
            first_error.unwrap_or_else(|| "none".to_owned())
        );
        return Ok(None);
    };

    if remote_unique_ids > local_stats.unique_ids {
        Ok(Some(RepairCandidate {
            channel,
            date,
            local_unique_ids: local_stats.unique_ids,
            remote_unique_ids,
            base_url,
        }))
    } else {
        Ok(None)
    }
}

async fn fetch_remote_day_unique_ids(
    http: &MirrorHttp,
    base_url: &str,
    channel: &str,
    date: NaiveDate,
) -> anyhow::Result<u64> {
    let url = format!(
        "{}/channel/{}/{:04}/{:02}/{:02}?jsonBasic=1",
        base_url.trim_end_matches('/'),
        channel,
        date.year(),
        date.month(),
        date.day()
    );
    let body = http
        .get_bytes(&url, Duration::from_secs(REPAIR_DAY_TIMEOUT_SECS))
        .await?;
    let resp: RemoteDayResponse = serde_json::from_slice(&body)
        .with_context(|| format!("decode remote day ({} bytes)", body.len()))?;
    let mut ids = HashSet::new();
    let mut no_id_count = 0_u64;

    for message in resp.messages {
        match message.id {
            Some(id) if !id.is_empty() => {
                ids.insert(id);
            }
            _ => no_id_count += 1,
        }
    }

    if ids.is_empty() {
        Ok(no_id_count)
    } else {
        Ok(ids.len() as u64)
    }
}

fn log_repair_candidates(candidates: &[RepairCandidate]) {
    if candidates.is_empty() {
        info!("repair-existing: no incomplete days found");
        return;
    }

    info!("repair-existing: {} candidate days", candidates.len());
    for candidate in candidates {
        info!(
            "repair-existing candidate channel={} date={} local_unique_ids={} remote_unique_ids={} base={}",
            candidate.channel,
            candidate.date,
            candidate.local_unique_ids,
            candidate.remote_unique_ids,
            candidate.base_url
        );
    }
}

async fn run_repair_candidates(
    db: &Client,
    http: &MirrorHttp,
    candidates: &[RepairCandidate],
    mirror_options: crate::mirror::RunDaysOptions,
) -> anyhow::Result<()> {
    let mut grouped: BTreeMap<(String, String), Vec<NaiveDate>> = BTreeMap::new();
    for candidate in candidates {
        grouped
            .entry((candidate.channel.clone(), candidate.base_url.clone()))
            .or_default()
            .push(candidate.date);
    }

    let mut failures = Vec::new();
    for ((channel, base_url), dates) in grouped {
        let days = dates
            .iter()
            .map(|date| (date.year() as u32, date.month(), date.day()))
            .collect::<Vec<_>>();
        info!(
            "repair-existing: mirror channel={} days={} base={}",
            channel,
            days.len(),
            base_url
        );

        if let Err(err) = crate::mirror::run_days(
            db.clone(),
            http,
            &base_url,
            None,
            &channel,
            days,
            mirror_options,
        )
        .await
        {
            warn!(
                "repair-existing mirror failed channel={} base={}: {err}",
                channel, base_url
            );
            failures.push((channel, base_url));
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(anyhow!(
            "repair-existing failed for {} channel/mirror groups",
            failures.len()
        ))
    }
}

async fn read_local_days(
    db: &Client,
    channels: &[String],
    year: u32,
) -> anyhow::Result<BTreeMap<String, BTreeMap<NaiveDate, LocalDayStats>>> {
    let channels_sql = quote_string_list(channels);
    let query = format!(
        "
        SELECT
            channel_login,
            toString(toDate(toTimeZone(timestamp, 'UTC'))) AS day,
            count() AS rows,
            uniqExact(id) AS unique_ids
        FROM message_structured
        WHERE channel_login IN ({channels_sql})
          AND toYear(toTimeZone(timestamp, 'UTC')) = {year}
        GROUP BY channel_login, day
        ORDER BY channel_login, day
        "
    );
    let rows = db.query(&query).fetch_all::<LocalDayRow>().await?;
    let mut out = BTreeMap::new();
    for row in rows {
        let day = NaiveDate::parse_from_str(&row.day, "%Y-%m-%d")
            .with_context(|| format!("Invalid ClickHouse date {}", row.day))?;
        out.entry(row.channel_login)
            .or_insert_with(BTreeMap::new)
            .insert(
                day,
                LocalDayStats {
                    rows: row.rows,
                    unique_ids: row.unique_ids,
                },
            );
    }
    Ok(out)
}

async fn duplicate_summary(
    db: &Client,
    options: &CleanupDuplicateIdsOptions,
) -> anyhow::Result<Vec<DuplicateSummaryRow>> {
    let filter = duplicate_scope_filter(options);
    let query = format!(
        "
        SELECT
            channel_login,
            count() AS rows,
            uniqExact(id) AS unique_ids,
            rows - unique_ids AS duplicate_rows
        FROM message_structured
        WHERE {filter}
        GROUP BY channel_login
        HAVING duplicate_rows > 0
        ORDER BY duplicate_rows DESC
        "
    );
    Ok(db.query(&query).fetch_all().await?)
}

async fn duplicate_samples(
    db: &Client,
    options: &CleanupDuplicateIdsOptions,
) -> anyhow::Result<Vec<DuplicateSampleRow>> {
    let filter = duplicate_scope_filter(options);
    let query = format!(
        "
        SELECT
            channel_login,
            toString(id) AS message_id,
            count() AS copies,
            toString(min(timestamp)) AS first_ts,
            toString(max(timestamp)) AS last_ts
        FROM message_structured
        WHERE {filter}
        GROUP BY channel_login, id
        HAVING copies > 1
        ORDER BY copies DESC, channel_login, id
        LIMIT {}
        ",
        options.sample_limit
    );
    Ok(db.query(&query).fetch_all().await?)
}

async fn execute_duplicate_cleanup(
    db: &Client,
    options: &CleanupDuplicateIdsOptions,
) -> anyhow::Result<()> {
    let filter = duplicate_scope_filter(options);

    // Check scope row count to estimate impact.
    let scope_rows: u64 = db
        .query(&format!(
            "SELECT count() FROM message_structured WHERE {filter}"
        ))
        .fetch_one()
        .await?;
    info!("Rows in cleanup scope: {}", scope_rows);

    let new_table = format!(
        "message_structured_dedup_{}_{}",
        std::process::id(),
        chrono::Utc::now().timestamp()
    );
    let old_table = format!(
        "message_structured_old_{}_{}",
        std::process::id(),
        chrono::Utc::now().timestamp()
    );

    info!("Creating new table {}", new_table);
    db.query(&format!(
        "
        CREATE TABLE {new_table} AS message_structured
        ENGINE = MergeTree
        PARTITION BY toYYYYMM(timestamp)
        ORDER BY (channel_id, user_id, timestamp)
        "
    ))
    .execute()
    .await?;

    let cleanup_result = async {
        // Step 1: Materialize duplicate IDs into a small temp table (avoids re-evaluating the heavy GROUP BY).
        let dup_table = format!("{}_dup_ids", new_table);
        info!("Creating temp duplicate-id table {}", dup_table);
        db.query(&format!(
            "
            CREATE TABLE {dup_table} (channel_login String, id UUID)
            ENGINE = Memory
            "
        ))
        .execute()
        .await?;

        info!("Finding duplicate IDs...");
        db.query(&format!(
            "
            INSERT INTO {dup_table}
            SELECT channel_login, id
            FROM message_structured
            WHERE {filter}
            GROUP BY channel_login, id
            HAVING count() > 1
            SETTINGS max_bytes_before_external_group_by = 1000000000
            "
        ))
        .execute()
        .await?;

        let dup_count: u64 = db
            .query(&format!("SELECT count() FROM {dup_table}"))
            .fetch_one()
            .await?;
        info!("Found {} duplicate IDs", dup_count);

        // Step 2: Insert month-by-month to stay within memory limits.
        let partitions: Vec<String> = db
            .query(&format!(
                "
                SELECT DISTINCT partition
                FROM system.parts
                WHERE database = currentDatabase()
                  AND table = 'message_structured'
                  AND active = 1
                ORDER BY partition
                "
            ))
            .fetch_all()
            .await?;

        let mut non_dup: u64 = 0;
        for partition in &partitions {
            info!("Inserting non-dup partition {}", partition);
            db.query(&format!(
                "
                INSERT INTO {new_table}
                SELECT * FROM message_structured
                WHERE toYYYYMM(timestamp) = {partition}
                  AND {filter}
                  AND (channel_login, id) NOT IN (SELECT channel_login, id FROM {dup_table})
                "
            ))
            .execute()
            .await?;
            let n: u64 = db
                .query(&format!("SELECT count() FROM {new_table}"))
                .fetch_one()
                .await?;
            let batch = n - non_dup;
            non_dup = n;
            info!("  partition {}: +{} non-dup rows", partition, batch);
        }
        info!("Non-duplicated rows inserted: {}", non_dup);

        // Insert one row per duplicate month-by-month.
        for partition in &partitions {
            info!("Inserting dup partition {}", partition);
            db.query(&format!(
                "
                INSERT INTO {new_table}
                SELECT * FROM message_structured
                WHERE toYYYYMM(timestamp) = {partition}
                  AND {filter}
                  AND (channel_login, id) IN (SELECT channel_login, id FROM {dup_table})
                ORDER BY timestamp
                LIMIT 1 BY id
                "
            ))
            .execute()
            .await?;
        }

        let total: u64 = db
            .query(&format!("SELECT count() FROM {new_table}"))
            .fetch_one()
            .await?;
        let dup_rows = total - non_dup;
        info!(
            "Total rows in new table: {} (kept {} duplicates)",
            total, dup_rows
        );

        info!("Dropping temp table {}", dup_table);
        let _ = db
            .query(&format!("DROP TABLE IF EXISTS {dup_table}"))
            .execute()
            .await;

        // Atomic swap.
        info!("Renaming tables atomically");
        db.query(&format!(
            "
            RENAME TABLE message_structured TO {old_table},
                         {new_table} TO message_structured
            "
        ))
        .execute()
        .await?;

        info!("Dropping old table {}", old_table);
        db.query(&format!("DROP TABLE IF EXISTS {old_table}"))
            .execute()
            .await?;

        anyhow::Ok(())
    }
    .await;

    // If anything failed before the swap, drop the temporary new table.
    if cleanup_result.is_err() {
        let _ = db
            .query(&format!("DROP TABLE IF EXISTS {new_table}"))
            .execute()
            .await;
    }
    cleanup_result
}

fn duplicate_scope_filter(options: &CleanupDuplicateIdsOptions) -> String {
    let mut filters = vec![format!("id != toUUID('{NIL_UUID}')")];
    if !options.channels.is_empty() {
        filters.push(format!(
            "channel_login IN ({})",
            quote_string_list(&options.channels)
        ));
    }
    if let Some(year) = options.year {
        filters.push(format!("toYear(toTimeZone(timestamp, 'UTC')) = {year}"));
    }
    filters.join(" AND ")
}

fn quote_string_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("'{}'", value.replace('\'', "\\'")))
        .collect::<Vec<_>>()
        .join(",")
}
