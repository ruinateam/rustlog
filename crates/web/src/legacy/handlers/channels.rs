//! Logged channels and their chat badges.

use crate::{
    cache_control::{no_cache, public_cache},
    legacy::{
        dto::{Channel, ChannelsList, ChatBadge, ChatBadgesResponse},
        error::{Error, Result},
    },
};
use aide::axum::IntoApiResponse;
use axum::{
    Json,
    extract::{Path, State},
};
use rustlog_app::App;

pub async fn get_channels(app: State<App>) -> Result<impl IntoApiResponse> {
    let channel_ids = app.state.channel_ids();

    let channels = app
        .twitch
        .get_users(Vec::from_iter(channel_ids), vec![], false)
        .await?;

    let json = Json(ChannelsList {
        channels: channels
            .into_iter()
            .map(|(user_id, name)| Channel { name, user_id })
            .collect(),
    });
    Ok((no_cache(), json))
}

pub async fn get_chat_badges(
    Path(channel_id): Path<String>,
    app: State<App>,
) -> Result<impl IntoApiResponse> {
    // Only logged channels, so that arbitrary ids cannot drive Helix requests.
    if !app.state.is_channel_enabled(&channel_id) {
        return Err(Error::NotFound);
    }

    let (global, channel) = app.twitch.get_chat_badges(&channel_id).await?;
    let badges = global
        .into_iter()
        .chain(channel)
        .flat_map(|set| {
            let set_id = set.set_id.to_string();
            set.versions.into_iter().map(move |badge| ChatBadge {
                set_id: set_id.clone(),
                version: badge.id.to_string(),
                image_url_1x: badge.image_url_1x,
                image_url_2x: badge.image_url_2x,
                title: badge.title,
                description: badge.description,
            })
        })
        .collect();

    Ok((public_cache(3600), Json(ChatBadgesResponse { badges })))
}
