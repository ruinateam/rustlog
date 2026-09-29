//! Message counts and the logins a user had.

use super::{
    extract::{Path, Query, TwitchId},
    params::{ChannelPath, ChannelUserPath, OptionalRange, UserPath},
    problem::ApiProblem,
};
use crate::{app::App, storage::stats, web::cache_control::Cached};
use axum::{Json, extract::State};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChannelStats {
    pub message_count: u64,
    /// The users with the most messages, most first.
    pub top_chatters: Vec<UserStats>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserStats {
    pub user_id: TwitchId,
    /// Left out when Twitch does not know the user or cannot be asked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    pub message_count: u64,
}

pub async fn channel_stats(
    State(app): State<App>,
    Path(path): Path<ChannelPath>,
    Query(range): Query<OptionalRange>,
) -> Result<Cached<Json<ChannelStats>>, ApiProblem> {
    let range = range.validate()?;
    let channel_id = path.channel_id.as_str();
    app.check_opted_out(channel_id, None)?;

    let (message_count, rows) = stats::get_channel_stats(&app.db, channel_id, range).await?;
    let user_ids = rows.iter().map(|row| row.user_id.clone()).collect();
    let mut logins = known_logins(&app, user_ids).await;

    let top_chatters = rows
        .into_iter()
        .map(|row| UserStats {
            login: logins.remove(&row.user_id),
            user_id: TwitchId::new_unchecked(row.user_id),
            message_count: row.cnt,
        })
        .collect();

    Ok(Cached::no_cache(Json(ChannelStats {
        message_count,
        top_chatters,
    })))
}

pub async fn user_stats(
    State(app): State<App>,
    Path(path): Path<ChannelUserPath>,
    Query(range): Query<OptionalRange>,
) -> Result<Cached<Json<UserStats>>, ApiProblem> {
    let range = range.validate()?;
    let (channel_id, user_id) = (path.channel_id.as_str(), path.user_id.as_str());
    app.check_opted_out(channel_id, Some(user_id))?;

    let login = known_logins(&app, vec![user_id.to_owned()])
        .await
        .into_values()
        .next();
    let count =
        stats::get_user_stats(&app.db, channel_id, user_id.to_owned(), login, range).await?;

    Ok(Cached::no_cache(Json(UserStats {
        user_id: path.user_id,
        login: count.user_login,
        message_count: count.message_count,
    })))
}

/// The logins of the users, as far as Twitch can tell right now: counts
/// are still worth answering without them.
async fn known_logins(app: &App, user_ids: Vec<String>) -> HashMap<String, String> {
    app.twitch
        .get_users(user_ids, Vec::new(), false)
        .await
        .unwrap_or_default()
}

#[derive(Serialize, JsonSchema)]
pub struct NameHistory {
    pub names: Vec<PreviousName>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PreviousName {
    pub login: String,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

pub async fn name_history(
    State(app): State<App>,
    Path(path): Path<UserPath>,
) -> Result<Cached<Json<NameHistory>>, ApiProblem> {
    let user_id = path.user_id.as_str();
    app.check_user_opted_out(user_id)?;

    let names = stats::get_user_name_history(&app.db, user_id)
        .await?
        .into_iter()
        .map(|entry| PreviousName {
            login: entry.user_login,
            first_seen_at: entry.first_seen,
            last_seen_at: entry.last_seen,
        })
        .collect();

    Ok(Cached::no_cache(Json(NameHistory { names })))
}
