//! The log folder of a justlog instance: raw IRC lines in
//! `{channel id}/{year}/{month}/{day}/channel.txt`, optionally gzipped.

use anyhow::{Context, bail};
use chrono::{Datelike, NaiveDate};
use flate2::bufread::GzDecoder;
use std::{
    fs::{self, File},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};
use tracing::warn;

const COMPRESSED_FILE: &str = "channel.txt.gz";
const PLAIN_FILE: &str = "channel.txt";

pub struct JustlogArchive {
    root: PathBuf,
}

/// The days of one month that a channel has logs of.
pub struct MonthOfLogs {
    pub year: i32,
    pub month: u32,
    pub days: Vec<NaiveDate>,
}

/// What a channel has logs of.
pub struct ChannelLogs {
    pub months: Vec<MonthOfLogs>,
    /// Size of the log files; compressed ones count compressed.
    pub bytes: u64,
}

impl JustlogArchive {
    pub fn open(root: &Path) -> anyhow::Result<Self> {
        if !root.is_dir() {
            bail!("{} is not a directory", root.display());
        }
        Ok(Self {
            root: root.to_owned(),
        })
    }

    /// The ids of the channels with a log folder.
    pub fn channel_ids(&self) -> anyhow::Result<Vec<String>> {
        let mut channel_ids = Vec::new();
        for entry in list(&self.root)? {
            if entry.file_type()?.is_dir() {
                match entry.file_name().into_string() {
                    Ok(channel_id) => channel_ids.push(channel_id),
                    Err(name) => warn!(?name, "skipping a folder that is not a channel id"),
                }
            }
        }
        Ok(channel_ids)
    }

    pub fn channel_logs(&self, channel_id: &str) -> anyhow::Result<ChannelLogs> {
        let mut months = Vec::new();
        let mut bytes = 0;

        for year_entry in list(&self.root.join(channel_id))? {
            let name = year_entry.file_name();
            let Some(year) = name.to_str().and_then(|name| name.parse().ok()) else {
                warn!(channel_id, ?name, "skipping a folder that is not a year");
                continue;
            };
            if !year_entry.file_type()?.is_dir() {
                continue;
            }

            for month in 1..=12 {
                let mut days = Vec::new();
                for day in 1..=31 {
                    let Some(date) = NaiveDate::from_ymd_opt(year, month, day) else {
                        continue;
                    };
                    if let Some(file) = self.day_file(channel_id, date) {
                        bytes += fs::metadata(&file.path)?.len();
                        days.push(date);
                    }
                }
                if !days.is_empty() {
                    months.push(MonthOfLogs { year, month, days });
                }
            }
        }

        months.sort_by_key(|month| (month.year, month.month));
        Ok(ChannelLogs { months, bytes })
    }

    /// The raw IRC lines of a day, decompressed if needed.
    pub fn read_day(
        &self,
        channel_id: &str,
        date: NaiveDate,
    ) -> anyhow::Result<Box<dyn BufRead + Send>> {
        let file = self
            .day_file(channel_id, date)
            .context("the log file disappeared")?;
        let reader = BufReader::new(
            File::open(&file.path)
                .with_context(|| format!("could not open {}", file.path.display()))?,
        );
        Ok(if file.compressed {
            Box::new(BufReader::new(GzDecoder::new(reader)))
        } else {
            Box::new(reader)
        })
    }

    /// The compressed file when both exist.
    fn day_file(&self, channel_id: &str, date: NaiveDate) -> Option<DayFile> {
        let day_dir = self
            .root
            .join(channel_id)
            .join(date.year().to_string())
            .join(date.month().to_string())
            .join(date.day().to_string());
        [(COMPRESSED_FILE, true), (PLAIN_FILE, false)]
            .into_iter()
            .map(|(name, compressed)| DayFile {
                path: day_dir.join(name),
                compressed,
            })
            .find(|file| file.path.is_file())
    }
}

struct DayFile {
    path: PathBuf,
    compressed: bool,
}

fn list(dir: &Path) -> anyhow::Result<Vec<fs::DirEntry>> {
    fs::read_dir(dir)
        .with_context(|| format!("could not list {}", dir.display()))?
        .collect::<Result<_, _>>()
        .with_context(|| format!("could not list {}", dir.display()))
}
