//! The logs.zonian.dev API, which knows the days that instances have logs
//! of and which instances log a channel.

use crate::mirror::HttpPool;
use chrono::NaiveDate;
use serde::Deserialize;
use std::collections::BTreeSet;

pub struct ChannelOverview {
    /// Days of the requested year that some instance has logs of.
    pub days: BTreeSet<NaiveDate>,
    /// Base URLs of the instances that log the channel.
    pub instances: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiResponse {
    logged_data: LoggedData,
    channel_logs: ChannelLogs,
}

#[derive(Deserialize)]
struct LoggedData {
    list: Vec<LoggedDay>,
}

#[derive(Deserialize)]
struct LoggedDay {
    year: String,
    month: String,
    day: String,
}

#[derive(Deserialize)]
struct ChannelLogs {
    instances: Vec<String>,
}

impl LoggedDay {
    fn date(&self) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(
            self.year.parse().ok()?,
            self.month.parse().ok()?,
            self.day.parse().ok()?,
        )
    }
}

pub async fn channel_overview(
    http: &HttpPool,
    api_base: &str,
    channel: &str,
    year: i32,
) -> anyhow::Result<ChannelOverview> {
    let url = format!("{}/api/{channel}", api_base.trim_end_matches('/'));
    let response: ApiResponse = http.get_json(&url).await?;

    Ok(ChannelOverview {
        days: response
            .logged_data
            .list
            .iter()
            .filter_map(LoggedDay::date)
            .filter(|date| chrono::Datelike::year(date) == year)
            .collect(),
        instances: response
            .channel_logs
            .instances
            .into_iter()
            .map(|instance| instance.trim_end_matches('/').to_owned())
            .collect(),
    })
}
