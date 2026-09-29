//! `migrate`: import the logs of a justlog instance.

mod archive;

use self::archive::{JustlogArchive, MonthOfLogs};
use anyhow::Context;
use chrono::{NaiveDate, NaiveTime};
use clickhouse::{Client, inserter::Inserter};
use rustlog_storage::{
    irc::tags::{extract_raw_timestamp, extract_user_id},
    message::{MESSAGES_STRUCTURED_TABLE, StructuredMessage, UnstructuredMessage},
    state::OperationalState,
};
use std::{
    io::BufRead,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tmi::Command;
use tokio::{sync::Semaphore, task::JoinSet};
use tracing::{error, info, warn};

const INSERT_BATCH_ROWS: u64 = 10_000_000;
const INSERT_PERIOD: Duration = Duration::from_secs(15);
const SEND_TIMEOUT: Duration = Duration::from_secs(30);
const END_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone, clap::Args)]
pub struct MigrateOptions {
    /// The justlog logs folder
    #[arg(short, long)]
    pub source_dir: PathBuf,
    /// Id of a channel to migrate. Repeatable; all channels when not given.
    #[arg(short = 'c', long = "channel-id", value_name = "CHANNEL_ID")]
    pub channel_ids: Vec<String>,
    /// Months migrated in parallel
    #[arg(short, long, default_value_t = 1)]
    pub jobs: usize,
}

pub async fn run(db: Client, options: MigrateOptions) -> anyhow::Result<()> {
    let started = Instant::now();
    let archive = Arc::new(JustlogArchive::open(&options.source_dir)?);
    let state = OperationalState::load(Arc::new(db.clone())).await?;

    let channel_ids: Vec<String> = archive
        .channel_ids()?
        .into_iter()
        .filter(|id| options.channel_ids.is_empty() || options.channel_ids.contains(id))
        .collect();
    info!(channels = ?channel_ids, "scanning the logs to migrate");

    let mut months = Vec::new();
    let mut total_bytes = 0;
    for channel_id in &channel_ids {
        let logs = archive.channel_logs(channel_id)?;
        total_bytes += logs.bytes;
        months.extend(
            logs.months
                .into_iter()
                .map(|month| (channel_id.clone(), month)),
        );
    }
    info!(
        channels = channel_ids.len(),
        months = months.len(),
        size_mib = total_bytes / 1024 / 1024,
        "migrating logs; progress estimates are wrong for gzip compressed logs"
    );

    let progress = Arc::new(Progress::new(total_bytes));
    let job_slots = Arc::new(Semaphore::new(options.jobs.max(1)));
    let mut jobs = JoinSet::new();
    for (channel_id, month) in months {
        let job_slot = job_slots.clone().acquire_owned().await?;
        let importer = MonthImporter {
            db: db.clone(),
            state: state.clone(),
            archive: archive.clone(),
            progress: progress.clone(),
        };
        jobs.spawn(async move {
            let result = importer.import(&channel_id, month).await;
            drop(job_slot);
            result
        });
    }
    while let Some(result) = jobs.join_next().await {
        result.context("a migration job panicked")??;
    }

    let elapsed = started.elapsed();
    info!(took_secs = elapsed.as_secs(), "migration finished");
    if let Some(mib_per_sec) = progress.read_mib().checked_div(elapsed.as_secs()) {
        info!(mib_per_sec, "average migration speed");
    }
    Ok(())
}

/// Imports a month of a channel's logs.
struct MonthImporter {
    db: Client,
    state: OperationalState,
    archive: Arc<JustlogArchive>,
    progress: Arc<Progress>,
}

impl MonthImporter {
    async fn import(&self, channel_id: &str, month: MonthOfLogs) -> anyhow::Result<()> {
        info!(
            channel_id,
            year = month.year,
            month = month.month,
            "migrating month"
        );
        let mut inserter = self
            .db
            .inserter::<StructuredMessage<'static>>(MESSAGES_STRUCTURED_TABLE)
            .with_timeouts(Some(SEND_TIMEOUT), Some(END_TIMEOUT))
            .with_max_rows(INSERT_BATCH_ROWS)
            .with_period(Some(INSERT_PERIOD));

        for date in month.days {
            let read_bytes = self
                .import_day(channel_id, date, &mut inserter)
                .await
                .with_context(|| format!("could not migrate channel {channel_id} date {date}"))?;
            self.progress.add(read_bytes);
        }

        let inserted = inserter.end().await.context("could not flush messages")?;
        if inserted.rows > 0 {
            info!(
                rows = inserted.rows,
                transactions = inserted.transactions,
                "inserted messages"
            );
        }
        Ok(())
    }

    /// Returns the number of bytes read.
    async fn import_day(
        &self,
        channel_id: &str,
        date: NaiveDate,
        inserter: &mut Inserter<StructuredMessage<'static>>,
    ) -> anyhow::Result<u64> {
        let reader = self.archive.read_day(channel_id, date)?;
        let start_of_day_ms = date.and_time(NaiveTime::MIN).and_utc().timestamp_millis() as u64;
        let mut read_bytes = 0;

        for (number, line) in reader.lines().enumerate() {
            let line = line.with_context(|| format!("could not read line {number}"))?;
            read_bytes += line.len() as u64 + 1;
            self.import_line(channel_id, &line, start_of_day_ms, inserter)
                .await
                .with_context(|| format!("could not write line {number}"))?;
        }

        let committed = inserter.commit().await?;
        if committed.rows > 0 {
            info!(
                rows = committed.rows,
                transactions = committed.transactions,
                "inserted messages"
            );
        }
        Ok(read_bytes)
    }

    /// Lines that are not IRC messages are logged and skipped.
    async fn import_line(
        &self,
        channel_id: &str,
        line: &str,
        start_of_day_ms: u64,
        inserter: &mut Inserter<StructuredMessage<'static>>,
    ) -> anyhow::Result<()> {
        let Some(irc_message) = tmi::IrcMessageRef::parse(line) else {
            warn!(raw = %line, "skipping unparsable line");
            return Ok(());
        };

        let user_id = extract_user_id(&irc_message).unwrap_or_else(|| {
            if irc_message.command() == Command::Privmsg {
                warn!(
                    raw = irc_message.raw(),
                    "skipping PRIVMSG without a user id"
                );
            }
            ""
        });
        let unstructured = UnstructuredMessage {
            channel_id,
            user_id,
            timestamp: extract_raw_timestamp(&irc_message).unwrap_or(start_of_day_ms),
            raw: irc_message.raw(),
        };

        match StructuredMessage::from_unstructured(&unstructured) {
            Ok(message) => {
                if self
                    .state
                    .permits_historical_message(&message.channel_id, &message.user_id)
                {
                    inserter.write(&message).await?;
                }
            }
            Err(error) => {
                error!(
                    raw = %unstructured.raw,
                    error = format!("{error:#}"),
                    "could not parse an IRC message"
                );
            }
        }
        Ok(())
    }
}

/// Logs the share of the log bytes read, in whole percent steps.
struct Progress {
    total_bytes: u64,
    read_bytes: AtomicU64,
    reported_percent: AtomicU64,
}

impl Progress {
    fn new(total_bytes: u64) -> Self {
        Self {
            total_bytes,
            read_bytes: AtomicU64::new(0),
            reported_percent: AtomicU64::new(0),
        }
    }

    fn add(&self, bytes: u64) {
        let read_bytes = self.read_bytes.fetch_add(bytes, Ordering::Relaxed) + bytes;
        let percent = (read_bytes * 100)
            .checked_div(self.total_bytes)
            .unwrap_or(100);
        if self.reported_percent.fetch_max(percent, Ordering::Relaxed) < percent {
            info!(
                processed_mib = read_bytes / 1024 / 1024,
                total_mib = self.total_bytes / 1024 / 1024,
                percent,
                "migration progress estimate"
            );
        }
    }

    fn read_mib(&self) -> u64 {
        self.read_bytes.load(Ordering::Relaxed) / 1024 / 1024
    }
}
