//! Handlers of the legacy routes, grouped by topic, and the path parsing
//! they share.

pub mod channels;
pub mod logs;
pub mod optout;
pub mod stats;
pub mod tiers;

use super::{
    dto::{ChannelIdType, UserIdType, UserLogPathParams},
    error::{Error, Result},
};
use chrono::NaiveDate;
use rustlog_app::App;

/// Resolves a channel given by login or id to its id.
async fn resolve_channel(app: &App, id_type: ChannelIdType, channel: &str) -> Result<String> {
    match id_type {
        ChannelIdType::Name => Ok(app.twitch.get_user_id_by_name(channel).await?),
        ChannelIdType::Id => Ok(channel.to_owned()),
    }
}

/// Resolves the channel and the user of a path to their ids.
async fn resolve_user_params(params: &UserLogPathParams, app: &App) -> Result<(String, String)> {
    let channel_id = resolve_channel(app, params.channel_id_type, &params.channel).await?;
    let user_id = match params.user_id_type {
        UserIdType::Name => app.twitch.get_user_id_by_name(&params.user).await?,
        UserIdType::Id => params.user.clone(),
    };
    Ok((channel_id, user_id))
}

fn normalize_month(month_raw: i32) -> Result<u32> {
    // Use strict 1-based months to avoid duplicated “13th” month (0..11 + 1..12).
    // Valid range: 1 (Jan) ..= 12 (Dec).
    if (1..=12).contains(&month_raw) {
        Ok(month_raw as u32)
    } else {
        Err(Error::InvalidParam(format!(
            "Invalid month: {month_raw} (expected 1-12)"
        )))
    }
}

fn parse_date(year: i32, month: u32, day_raw: i32) -> Result<NaiveDate> {
    NaiveDate::from_ymd_opt(year, month, day_raw as u32)
        .ok_or_else(|| Error::InvalidParam(format!("Invalid date: {year}-{month:02}-{day_raw:02}")))
}
