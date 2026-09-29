mod reader;

use self::reader::{COMPRESSED_CHANNEL_FILE, LogsReader, UNCOMPRESSED_CHANNEL_FILE};
use crate::{
    irc::tags::{extract_raw_timestamp, extract_user_id},
    migrator::reader::ChannelLogDateMap,
    state::OperationalState,
    storage::message::{MESSAGES_STRUCTURED_TABLE, StructuredMessage, UnstructuredMessage},
};
use anyhow::{Context, anyhow};
use chrono::{DateTime, Datelike, TimeZone, Utc};
use clickhouse::inserter::Inserter;
use flate2::bufread::GzDecoder;
use indexmap::IndexMap;
use std::{
    convert::TryInto,
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tmi::Command;
use tokio::sync::Semaphore;
use tracing::{debug, error, info, warn};

const INSERT_BATCH_SIZE: u64 = 10_000_000;

#[derive(Clone)]
pub struct Migrator {
    db: clickhouse::Client,
    state: OperationalState,
    source_logs_path: String,
    channel_ids: Arc<Vec<String>>,
}

impl Migrator {
    pub async fn new(
        db: clickhouse::Client,
        source_logs_path: String,
        channel_ids: Vec<String>,
    ) -> anyhow::Result<Migrator> {
        Ok(Self {
            state: OperationalState::load(Arc::new(db.clone())).await?,
            db,
            source_logs_path,
            channel_ids: Arc::new(channel_ids),
        })
    }

    pub async fn run(self, parallel_count: usize) -> anyhow::Result<()> {
        let source_logs = LogsReader::new(&self.source_logs_path)?;

        let started_at = Instant::now();
        let channels = source_logs.get_stored_channels().await?;

        let semaphore = Arc::new(Semaphore::new(parallel_count));
        let mut handles = Vec::with_capacity(parallel_count);

        let filtered_channels: Vec<_> = channels
            .into_iter()
            .filter(|channel| self.channel_ids.is_empty() || self.channel_ids.contains(channel))
            .collect();

        info!(channels = ?filtered_channels, "migrating channels");

        let mut channel_logs: IndexMap<String, ChannelLogDateMap> = IndexMap::new();

        info!("scanning the logs to migrate");

        let mut total_bytes = 0;

        for channel_id in filtered_channels {
            let (available_logs, channel_bytes) =
                source_logs.get_available_channel_logs(&channel_id)?;
            total_bytes += channel_bytes;
            channel_logs.insert(channel_id, available_logs);
        }

        let channel_count = channel_logs.len();
        let total_mb = total_bytes / 1024 / 1024;

        info!(
            channels = channel_count,
            size_mib = total_mb,
            "migrating logs; progress estimates are wrong for gzip compressed logs"
        );

        let total_read_bytes = Arc::new(AtomicU64::new(0));
        let migrated_percentage = Arc::new(AtomicU64::new(0));

        for (i, (channel_id, available_logs)) in (1..).zip(channel_logs) {
            info!(%channel_id, channel = i, channels = channel_count, "reading channel");

            for (year, months) in available_logs {
                for (month, days) in months {
                    debug!("waiting for a free job slot");
                    let permit = semaphore.clone().acquire_owned().await.unwrap();
                    let migrator = self.clone();
                    let channel_id = channel_id.clone();
                    let root_path = source_logs.root_path.clone();
                    let total_read_bytes = total_read_bytes.clone();
                    let migrated_percentage = migrated_percentage.clone();

                    let handle = tokio::spawn(async move {
                        let mut inserter = migrator
                            .db
                            .inserter::<StructuredMessage<'static>>(MESSAGES_STRUCTURED_TABLE)
                            .with_timeouts(
                                Some(Duration::from_secs(30)),
                                Some(Duration::from_secs(180)),
                            )
                            .with_max_rows(INSERT_BATCH_SIZE)
                            .with_period(Some(Duration::from_secs(15)));

                        info!(%channel_id, year, month, "migrating month");

                        for day in days {
                            let date = Utc
                                .with_ymd_and_hms(year.try_into().unwrap(), month, day, 0, 0, 0)
                                .unwrap();
                            let day_bytes = migrator
                                .migrate_day(&root_path, &channel_id, date, &mut inserter)
                                .await
                                .with_context(|| {
                                    format!("Could not migrate channel {channel_id} date {date}")
                                })?;

                            total_read_bytes.fetch_add(day_bytes as u64, Ordering::SeqCst);
                            let processed_bytes = total_read_bytes.load(Ordering::SeqCst);

                            let old_percentage = migrated_percentage.load(Ordering::SeqCst);
                            let new_percentage =
                                (processed_bytes as f64 / total_bytes as f64 * 100.0) as u64;

                            if new_percentage - old_percentage >= 1 {
                                let processed_mb = processed_bytes / 1024 / 1024;
                                info!(
                                    processed_mib = processed_mb,
                                    total_mib = total_mb,
                                    percent = new_percentage,
                                    "migration progress estimate"
                                );
                                migrated_percentage.store(new_percentage, Ordering::SeqCst);
                            }
                        }

                        debug!("flushing inserts");
                        let stats = inserter.end().await.context("could not flush messages")?;
                        if stats.rows > 0 {
                            info!(
                                rows = stats.rows,
                                transactions = stats.transactions,
                                "inserted messages"
                            );
                        }

                        drop(permit);
                        Result::<_, anyhow::Error>::Ok(())
                    });
                    handles.push(handle);
                }
            }
        }

        for handle in handles {
            handle.await.unwrap()?;
        }

        let elapsed = started_at.elapsed();
        info!(took_secs = elapsed.as_secs(), "migration finished");

        if let Some(throughput) =
            (total_read_bytes.load(Ordering::SeqCst) / 1024 / 1024).checked_div(elapsed.as_secs())
        {
            info!(mib_per_sec = throughput, "average migration speed");
        }

        Ok(())
    }

    // Returns the number of read bytes
    async fn migrate_day(
        &self,
        root_path: &Path,
        channel_id: &str,
        date: DateTime<Utc>,
        inserter: &mut Inserter<StructuredMessage<'static>>,
    ) -> anyhow::Result<usize> {
        let day_path = get_day_path(root_path, channel_id, date);

        let compressed_file_path = day_path.join(COMPRESSED_CHANNEL_FILE);
        let uncompressed_file_path = day_path.join(UNCOMPRESSED_CHANNEL_FILE);

        if compressed_file_path.exists() {
            debug!(path = %compressed_file_path.display(), "reading compressed log");
            let file_reader = BufReader::new(File::open(&compressed_file_path)?);
            let gz = BufReader::new(GzDecoder::new(file_reader));

            self.migrate_reader(gz, date, channel_id, inserter).await
        } else if uncompressed_file_path.exists() {
            debug!(path = %uncompressed_file_path.display(), "reading uncompressed log");
            let file_reader = BufReader::new(File::open(&uncompressed_file_path)?);

            self.migrate_reader(file_reader, date, channel_id, inserter)
                .await
        } else {
            Err(anyhow!("file does not exist"))
        }
    }

    async fn migrate_reader<R: BufRead>(
        &self,
        reader: R,
        datetime: DateTime<Utc>,
        channel_id: &str,
        inserter: &mut Inserter<StructuredMessage<'static>>,
    ) -> anyhow::Result<usize> {
        let mut read_bytes = 0;

        for (i, line) in reader.lines().enumerate() {
            let line = line.with_context(|| format!("could not read line {i} from input"))?;
            read_bytes += line.len() + 1; // Add 1 byte for newline symbol
            write_line(&self.state, channel_id, line, inserter, datetime)
                .await
                .with_context(|| format!("could not write line {i} to inserter"))?;
        }

        let stats = inserter.commit().await?;
        if stats.rows > 0 {
            info!(
                rows = stats.rows,
                transactions = stats.transactions,
                "inserted messages"
            );
        }

        Ok(read_bytes)
    }
}

async fn write_line(
    state: &OperationalState,
    channel_id: &str,
    raw: String,
    inserter: &mut Inserter<StructuredMessage<'static>>,
    datetime: DateTime<Utc>,
) -> anyhow::Result<()> {
    match tmi::IrcMessageRef::parse(&raw) {
        Some(irc_message) => {
            let timestamp = extract_raw_timestamp(&irc_message)
                .unwrap_or_else(|| datetime.timestamp_millis() as u64);
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
                timestamp,
                raw: irc_message.raw(),
            };
            match StructuredMessage::from_unstructured(&unstructured) {
                Ok(msg) => {
                    if state.permits_historical_message(&msg.channel_id, &msg.user_id) {
                        inserter.write(&msg).await?;
                    }
                }
                Err(err) => {
                    error!(raw = %unstructured.raw, error = format!("{err:#}"), "could not parse an IRC message");
                }
            }
        }
        None => {
            warn!(raw = %raw, "skipping unparsable line");
        }
    }

    Ok(())
}

fn get_day_path(root_path: &Path, channel_id: &str, date: DateTime<Utc>) -> PathBuf {
    root_path
        .join(channel_id)
        .join(date.year().to_string())
        .join(date.month().to_string())
        .join(date.day().to_string())
}
