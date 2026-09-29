//! Where mirrored logs come from: the JSON API of a remote rustlog or
//! justlog instance, or a local cache of its responses.

use super::{http::HttpPool, message::RemoteMessage};
use anyhow::Context;
use chrono::{Datelike, NaiveDate};
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub enum LogSource {
    Remote {
        http: HttpPool,
        base_url: String,
    },
    /// Responses saved as `{channel}/daily/{year}/{month}/{day}.json`.
    LocalCache(PathBuf),
}

/// `/list` of a remote instance.
#[derive(Deserialize)]
struct AvailableLogs {
    #[serde(rename = "availableLogs")]
    available_logs: Option<Vec<AvailableLogDate>>,
}

#[derive(Deserialize)]
struct AvailableLogDate {
    year: String,
    month: String,
    /// Absent for months, which user logs are listed by.
    day: Option<String>,
}

impl AvailableLogDate {
    fn day(&self) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(
            self.year.parse().ok()?,
            self.month.parse().ok()?,
            self.day.as_deref()?.parse().ok()?,
        )
    }
}

/// A day of logs, as `/channel/{channel}/{year}/{month}/{day}?jsonBasic`
/// returns it.
#[derive(Deserialize)]
struct DayOfLogs {
    messages: Option<Vec<RemoteMessage>>,
}

impl LogSource {
    pub fn remote(http: HttpPool, base_url: &str) -> Self {
        Self::Remote {
            http,
            base_url: base_url.trim_end_matches('/').to_owned(),
        }
    }

    /// The days of the channel that have logs, in no particular order.
    pub async fn available_days(&self, channel: &str) -> anyhow::Result<Vec<NaiveDate>> {
        match self {
            Self::Remote { http, base_url } => {
                let list: AvailableLogs = http
                    .get_json(&format!("{base_url}/list?channel={channel}"))
                    .await?;
                let dates = list.available_logs.unwrap_or_default();
                Ok(dates.iter().filter_map(AvailableLogDate::day).collect())
            }
            Self::LocalCache(root) => cached_days(&root.join(channel).join("daily")),
        }
    }

    pub async fn messages(
        &self,
        channel: &str,
        date: NaiveDate,
    ) -> anyhow::Result<Vec<RemoteMessage>> {
        let (year, month, day) = (date.year(), date.month(), date.day());
        let day_of_logs: DayOfLogs = match self {
            Self::Remote { http, base_url } => {
                let url = format!(
                    "{base_url}/channel/{channel}/{year:04}/{month:02}/{day:02}?jsonBasic=1"
                );
                http.get_json(&url).await?
            }
            Self::LocalCache(root) => {
                let path = root
                    .join(channel)
                    .join(format!("daily/{year:04}/{month:02}/{day:02}.json"));
                let json = fs::read_to_string(&path)
                    .with_context(|| format!("could not read {}", path.display()))?;
                serde_json::from_str(&json)
                    .with_context(|| format!("could not decode {}", path.display()))?
            }
        };
        Ok(day_of_logs.messages.unwrap_or_default())
    }
}

/// The days in `{year}/{month}/{day}.json` below `daily_dir`.
fn cached_days(daily_dir: &Path) -> anyhow::Result<Vec<NaiveDate>> {
    let mut days = Vec::new();
    if !daily_dir.is_dir() {
        return Ok(days);
    }

    for (year, year_dir) in numbered_entries(daily_dir, true)? {
        for (month, month_dir) in numbered_entries(&year_dir, true)? {
            for (day, _) in numbered_entries(&month_dir, false)? {
                if let Some(date) = NaiveDate::from_ymd_opt(year as i32, month, day) {
                    days.push(date);
                }
            }
        }
    }
    Ok(days)
}

/// Directories, or `.json` files, named by a number.
fn numbered_entries(dir: &Path, directories: bool) -> anyhow::Result<Vec<(u32, PathBuf)>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("could not list {}", dir.display()))? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let name = entry.file_name().to_string_lossy().to_lowercase();
        let number = if directories && file_type.is_dir() {
            name.parse().ok()
        } else if !directories && file_type.is_file() {
            name.strip_suffix(".json")
                .and_then(|stem| stem.parse().ok())
        } else {
            None
        };
        if let Some(number) = number {
            entries.push((number, entry.path()));
        }
    }
    Ok(entries)
}
