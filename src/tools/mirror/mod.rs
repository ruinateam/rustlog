//! `mirror`: import a channel's logs from the JSON API of another rustlog or
//! justlog instance, or from a local cache of its responses.

mod http;
mod message;
mod source;
mod writer;

pub use self::{http::HttpPool, source::LogSource};

use self::{message::Skipped, writer::Writer};
use crate::{state::OperationalState, storage::message::MESSAGES_STRUCTURED_TABLE};
use anyhow::Context;
use chrono::{DateTime, Datelike, Days, NaiveDate, NaiveTime, Utc};
use clickhouse::{Client, Row};
use dashmap::DashSet;
use futures::{StreamExt, stream};
use serde::Deserialize;
use std::{path::PathBuf, sync::Arc, time::Instant};
use tokio::sync::mpsc;
use tracing::{info, warn};
use uuid::Uuid;

/// Options shared by the commands that import from remote instances.
#[derive(Debug, Clone, clap::Args)]
pub struct TransferOptions {
    /// Rows per ClickHouse insert
    #[arg(long, default_value_t = 25_000)]
    pub batch: usize,
    /// Days fetched or checked in parallel
    #[arg(long, default_value_t = 4)]
    pub http_concurrency: usize,
    /// HTTP(S) proxy for the requests. Repeatable for a proxy pool.
    #[arg(long = "proxy", value_name = "PROXY")]
    pub proxies: Vec<String>,
    /// Max requests per second per proxy (a direct connection counts as one)
    #[arg(long, default_value_t = 2.0)]
    pub rps: f64,
}

impl TransferOptions {
    pub fn http_pool(&self, user_agent: &'static str) -> anyhow::Result<HttpPool> {
        HttpPool::new(&self.proxies, self.rps, user_agent, self.concurrency())
    }

    pub fn import_settings(&self) -> ImportSettings {
        ImportSettings {
            batch_rows: (self.batch as u64).max(1),
            concurrency: self.concurrency(),
        }
    }

    fn concurrency(&self) -> usize {
        self.http_concurrency.max(1)
    }
}

#[derive(Debug, Clone, clap::Args)]
pub struct MirrorOptions {
    /// Base URL of the remote instance
    #[arg(long, default_value = "https://logs.zonian.dev")]
    pub base_url: String,
    /// Read the logs from this cache of API responses instead of over HTTP
    #[arg(long)]
    pub local_cache: Option<PathBuf>,
    /// Login of the channel to mirror
    #[arg(long)]
    pub channel: String,
    /// Only days in this year
    #[arg(long)]
    pub year: Option<u32>,
    /// Only days in this month (1-12)
    #[arg(long)]
    pub month: Option<u32>,
    /// Only this day of the month (1-31)
    #[arg(long)]
    pub day: Option<u32>,
    #[command(flatten)]
    pub transfer: TransferOptions,
}

impl MirrorOptions {
    fn wants(&self, date: NaiveDate) -> bool {
        self.year.is_none_or(|year| date.year() == year as i32)
            && self.month.is_none_or(|month| date.month() == month)
            && self.day.is_none_or(|day| date.day() == day)
    }
}

/// How [`import_days`] works through the days.
#[derive(Debug, Clone, Copy)]
pub struct ImportSettings {
    /// Rows per ClickHouse insert.
    pub batch_rows: u64,
    /// Days fetched in parallel.
    pub concurrency: usize,
}

pub async fn run(db: Client, options: MirrorOptions) -> anyhow::Result<()> {
    let source = match &options.local_cache {
        Some(cache) => LogSource::LocalCache(cache.clone()),
        None => LogSource::remote(
            options.transfer.http_pool("rustlog-mirror/0.2")?,
            &options.base_url,
        ),
    };

    let mut days: Vec<NaiveDate> = source
        .available_days(&options.channel)
        .await?
        .into_iter()
        .filter(|date| options.wants(*date))
        .collect();
    days.sort_unstable();
    info!(days = days.len(), "found days to mirror");

    import_days(
        &db,
        &source,
        &options.channel,
        &days,
        options.transfer.import_settings(),
    )
    .await
}

/// Imports the channel's logs of `days` from `source`, skipping messages
/// that are stored already and messages of opted out users. Days that fail
/// are retried once, then logged and skipped.
pub async fn import_days(
    db: &Client,
    source: &LogSource,
    channel: &str,
    days: &[NaiveDate],
    settings: ImportSettings,
) -> anyhow::Result<()> {
    let (Some(first_day), Some(last_day)) = (days.iter().min(), days.iter().max()) else {
        return Ok(());
    };

    let state = OperationalState::load(Arc::new(db.clone())).await?;
    let stored_ids = match channel_id(db, channel).await? {
        Some(channel_id) => {
            let from = start_of(*first_day);
            let to = start_of(*last_day + Days::new(1));
            stored_message_ids(db, &channel_id, from, to).await?
        }
        None => DashSet::new(),
    };
    info!(
        from = %first_day,
        to = %last_day,
        stored_ids = stored_ids.len(),
        "loaded the stored message ids"
    );

    let writer = Writer::spawn(db.clone(), settings.batch_rows);
    let importer = DayImporter {
        source,
        channel,
        state: &state,
        seen_ids: &stored_ids,
        writer: &writer.sender,
    };

    let failed = importer
        .import_all(days, settings.concurrency, Pass::First)
        .await;
    if !failed.is_empty() {
        info!(days = failed.len(), "retrying failed days");
        importer
            .import_all(&failed, settings.concurrency, Pass::Retry)
            .await;
    }

    writer.finish().await?;
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pass {
    First,
    Retry,
}

/// Imports days of one channel from one source.
struct DayImporter<'a> {
    source: &'a LogSource,
    channel: &'a str,
    state: &'a OperationalState,
    /// Ids stored before or imported during this run.
    seen_ids: &'a DashSet<Uuid>,
    writer: &'a mpsc::Sender<crate::storage::message::StructuredMessage<'static>>,
}

#[derive(Default)]
struct DayStats {
    added: u64,
    skipped_duplicates: u64,
    skipped_without_id: u64,
    skipped_opted_out: u64,
}

impl DayImporter<'_> {
    /// Imports `days`, `concurrency` at a time; returns the days that failed.
    async fn import_all(
        &self,
        days: &[NaiveDate],
        concurrency: usize,
        pass: Pass,
    ) -> Vec<NaiveDate> {
        let mut imports = stream::iter(days)
            .map(|&date| async move {
                let started = Instant::now();
                (date, self.import(date).await, started)
            })
            .buffer_unordered(concurrency);

        let mut failed = Vec::new();
        let mut done = 0;
        while let Some((date, result, started)) = imports.next().await {
            done += 1;
            match result {
                Ok(stats) => info!(
                    done,
                    days = days.len(),
                    %date,
                    added = stats.added,
                    skipped_duplicates = stats.skipped_duplicates,
                    skipped_without_id = stats.skipped_without_id,
                    skipped_opted_out = stats.skipped_opted_out,
                    took_ms = started.elapsed().as_millis() as u64,
                    retry = pass == Pass::Retry,
                    "mirrored day"
                ),
                Err(error) => {
                    warn!(
                        done,
                        days = days.len(),
                        %date,
                        error = format!("{error:#}"),
                        retry = pass == Pass::Retry,
                        "could not mirror day"
                    );
                    failed.push(date);
                }
            }
        }
        failed
    }

    async fn import(&self, date: NaiveDate) -> anyhow::Result<DayStats> {
        let messages = self.source.messages(self.channel, date).await?;

        let mut stats = DayStats::default();
        for message in messages {
            let message = match message.into_structured(self.channel) {
                Ok(message) => message,
                Err(Skipped::MissingId) => {
                    stats.skipped_without_id += 1;
                    continue;
                }
                Err(Skipped::Invalid) => continue,
            };
            if !self
                .state
                .permits_historical_message(&message.channel_id, &message.user_id)
            {
                stats.skipped_opted_out += 1;
                continue;
            }
            if !self.seen_ids.insert(message.id) {
                stats.skipped_duplicates += 1;
                continue;
            }

            self.writer
                .send(message)
                .await
                .context("the writer stopped")?;
            stats.added += 1;
        }
        Ok(stats)
    }
}

fn start_of(date: NaiveDate) -> DateTime<Utc> {
    date.and_time(NaiveTime::MIN).and_utc()
}

/// The id of the channel with this login, if any of its messages are stored.
async fn channel_id(db: &Client, channel_login: &str) -> anyhow::Result<Option<String>> {
    let channel_id = db
        .query(&format!(
            "SELECT channel_id FROM {MESSAGES_STRUCTURED_TABLE} WHERE channel_login = ? LIMIT 1"
        ))
        .bind(channel_login)
        .fetch_optional::<String>()
        .await?;
    Ok(channel_id)
}

async fn stored_message_ids(
    db: &Client,
    channel_id: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<DashSet<Uuid>> {
    #[derive(Row, Deserialize)]
    struct StoredId {
        #[serde(with = "clickhouse::serde::uuid")]
        id: Uuid,
    }

    // A bare number compared with a DateTime64 counts seconds, not
    // milliseconds.
    let mut cursor = db
        .query(&format!(
            "
            SELECT id FROM {MESSAGES_STRUCTURED_TABLE}
            WHERE channel_id = ?
              AND timestamp >= fromUnixTimestamp64Milli(toInt64(?), 'UTC')
              AND timestamp < fromUnixTimestamp64Milli(toInt64(?), 'UTC')
            "
        ))
        .bind(channel_id)
        .bind(from.timestamp_millis())
        .bind(to.timestamp_millis())
        .fetch::<StoredId>()?;

    let ids = DashSet::new();
    while let Some(row) = cursor.next().await? {
        ids.insert(row.id);
    }
    Ok(ids)
}
