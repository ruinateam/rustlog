//! Users and channels: login lookups, the logged channels and chat badges.

use super::{
    extract::{Path, Query, TwitchId},
    params::ChannelPath,
    problem::ApiProblem,
};
use crate::{app::App, web::cache_control::Cached};
use axum::{Json, extract::State};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Twitch answers at most this many users per lookup.
const MAX_USERS_PER_LOOKUP: usize = 100;

#[derive(Serialize, JsonSchema)]
pub struct User {
    pub id: TwitchId,
    pub login: String,
}

#[derive(Serialize, JsonSchema)]
pub struct Users {
    pub users: Vec<User>,
}

#[derive(Serialize, JsonSchema)]
pub struct Channels {
    pub channels: Vec<User>,
}

/// Users to look up; repeat a parameter for several.
#[derive(Deserialize, JsonSchema)]
pub struct UsersQuery {
    /// Twitch login.
    #[serde(default)]
    pub login: Vec<String>,
    /// Numeric Twitch id.
    #[serde(default)]
    pub id: Vec<TwitchId>,
}

pub async fn users(
    State(app): State<App>,
    Query(query): Query<UsersQuery>,
) -> Result<Json<Users>, ApiProblem> {
    let logins: Vec<String> = query
        .login
        .iter()
        .map(|login| login.trim().to_lowercase())
        .filter(|login| !login.is_empty())
        .collect();
    let ids: Vec<String> = query.id.iter().map(|id| id.as_str().to_owned()).collect();

    let count = logins.len() + ids.len();
    if count == 0 {
        return Err(ApiProblem::invalid("give at least one `login` or `id`"));
    }
    if count > MAX_USERS_PER_LOOKUP {
        return Err(ApiProblem::invalid(format!(
            "give at most {MAX_USERS_PER_LOOKUP} logins and ids together"
        )));
    }

    let found = app.twitch.get_users(ids, logins, false).await?;
    Ok(Json(Users {
        users: sorted_users(found),
    }))
}

pub async fn channels(State(app): State<App>) -> Result<Cached<Json<Channels>>, ApiProblem> {
    // Channels that opted out stay joined, to hear an opt-in, but are hidden.
    let ids = app
        .state
        .channel_ids()
        .into_iter()
        .filter(|id| !app.state.is_channel_opted_out(id))
        .collect();
    let found = app.twitch.get_users(ids, Vec::new(), false).await?;
    Ok(Cached::no_cache(Json(Channels {
        channels: sorted_users(found),
    })))
}

/// Users ordered by login, from a map of ids to logins.
fn sorted_users(logins_by_id: HashMap<String, String>) -> Vec<User> {
    let mut users: Vec<User> = logins_by_id
        .into_iter()
        .map(|(id, login)| User {
            id: TwitchId::new_unchecked(id),
            login,
        })
        .collect();
    users.sort_by(|a, b| a.login.cmp(&b.login));
    users
}

#[derive(Serialize, JsonSchema)]
pub struct ChatBadges {
    pub badges: Vec<ChatBadge>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChatBadge {
    /// Badge set, such as `subscriber`.
    pub set_id: String,
    /// Version within the set, such as `12`; together with the set it is
    /// the value of the IRC `badges` tag.
    pub version: String,
    pub title: String,
    pub description: String,
    #[serde(rename = "imageUrl1x")]
    pub image_url_1x: String,
    #[serde(rename = "imageUrl2x")]
    pub image_url_2x: String,
}

pub async fn badges(
    State(app): State<App>,
    Path(path): Path<ChannelPath>,
) -> Result<Cached<Json<ChatBadges>>, ApiProblem> {
    let channel_id = path.channel_id.as_str();
    // Only logged channels, so that arbitrary ids cannot drive Helix requests.
    if !app.state.is_channel_enabled(channel_id) {
        return Err(ApiProblem::not_found("the channel is not logged"));
    }

    let (global, channel) = app.twitch.get_chat_badges(channel_id).await?;
    let badges = global
        .into_iter()
        .chain(channel)
        .flat_map(|set| {
            let set_id = set.set_id.to_string();
            set.versions.into_iter().map(move |badge| ChatBadge {
                set_id: set_id.clone(),
                version: badge.id.to_string(),
                title: badge.title,
                description: badge.description,
                image_url_1x: badge.image_url_1x,
                image_url_2x: badge.image_url_2x,
            })
        })
        .collect();

    Ok(Cached::public(3600, Json(ChatBadges { badges })))
}
