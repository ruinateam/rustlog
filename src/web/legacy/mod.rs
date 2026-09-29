//! The legacy API at the root path, compatible with justlog clients.
//!
//! Its routes, bodies, error texts and OpenAPI document are frozen: fixes
//! and new features go to [`super::v2`].

pub mod admin;
pub mod docs;
mod dto;
mod error;
mod handlers;

use self::handlers::{channels, logs, optout, stats, tiers};
use crate::app::App;
use aide::{
    axum::{
        ApiRouter,
        routing::{get, get_with, post},
    },
    openapi::{Info, OpenApi, Server},
};
use axum::{Json, extract::Request, middleware::Next, response::Response};

const CAPABILITIES: &[&str] = &["arbitrary-range-query", "search", "stats", "namehistory"];

/// The document that [`router`] fills in with its routes.
pub fn openapi() -> OpenApi {
    OpenApi {
        info: Info {
            title: "ChatTiers Rustlog API".to_owned(),
            summary: Some(
                "Query chat logs, search history, inspect stats, and manage live logging."
                    .to_owned(),
            ),
            description: Some(
                "Use `channel` / `user` when you want login-based routes and `channelid` / `userid` when you already have Twitch ids.\n\nFor log endpoints, append `?json`, `?jsonBasic`, `?raw`, or `?ndjson` to switch the response format."
                    .to_owned(),
            ),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            ..Info::default()
        },
        servers: vec![Server {
            url: "/".to_owned(),
            description: Some("Current rustlog instance".to_owned()),
            ..Server::default()
        }],
        ..OpenApi::default()
    }
}

/// The legacy routes, including `/admin`. The docs routes need the
/// [`docs::LegacySpec`] extension, and the admin routes the
/// [`admin::AdminApiKey`] one.
pub fn router() -> ApiRouter<App> {
    ApiRouter::new()
        .nest("/admin", admin::router())
        .api_route(
            "/channels",
            get_with(channels::get_channels, |op| {
                op.summary("List logged channels").description(
                    "Return the channel logins and Twitch ids currently configured for live logging.",
                )
            }),
        )
        .api_route(
            "/list",
            get_with(logs::list_available_logs, |op| {
                op.summary("List available log buckets").description(
                    "Show which years, months, or days exist. Without query parameters, returns buckets for all configured channels. With `channel` or `channelid`, returns buckets for that channel, optionally narrowed to a specific user.",
                )
            }),
        )
        .api_route(
            "/badges/{channel_id}",
            get_with(channels::get_chat_badges, |op| {
                op.summary("Get Twitch chat badge metadata").description(
                    "Return global and channel-specific Twitch badge images for rendering stored chat messages. Only available for logged channels.",
                )
            }),
        )
        // Paths with static parts should go first so they aren't overridden by the dynamic date paths later
        .api_route(
            "/namehistory/{user_id}",
            get_with(stats::get_user_name_history, |op| {
                op.summary("Get historical logins for a user id").description(
                    "Return previous usernames seen for the provided Twitch user id together with their first and last timestamps.",
                )
            }),
        )
        .api_route(
            "/{channel_id_type}/{channel}/{user_id_type}/{user}/search",
            get_with(logs::search_user_logs, |op| {
                op.summary("Search a user's messages in a channel")
                    .description("Run a text search over one user's messages inside one channel.")
            }),
        )
        .api_route(
            "/{channel_id_type}/{channel}/{user_id_type}/{user}/stats",
            get_with(stats::get_user_stats, |op| {
                op.summary("Get per-user message stats").description(
                    "Return message totals and related aggregates for one user in one channel.",
                )
            }),
        )
        .api_route(
            "/{channel_id_type}/{channel}/stats",
            get_with(stats::get_channel_stats, |op| {
                op.summary("Get channel-wide stats")
                    .description("Return message totals and top chatters for the selected channel.")
            }),
        )
        .api_route(
            "/{channel_id_type}/{channel}/tiers/{year}/{month}/{day}",
            get_with(tiers::get_channel_tiers_day, |op| {
                op.summary("Get daily chat tiers").description(
                    "Return up to 500 users for one Europe/Moscow calendar day with deterministic windowed ranks and tier calculations.",
                )
            }),
        )
        .api_route(
            "/{channel_id_type}/{channel}/tiers/{year}/{month}",
            get_with(tiers::get_channel_tiers_month, |op| {
                op.summary("Get monthly chat tiers").description(
                    "Return up to 500 users for one Europe/Moscow calendar month with deterministic aggregated tier metrics.",
                )
            }),
        )
        .api_route(
            "/{channel_id_type}/{channel}/tiers/{year}",
            get_with(tiers::get_channel_tiers_year, |op| {
                op.summary("Get yearly chat tiers").description(
                    "Return up to 500 users for one Europe/Moscow calendar year with deterministic aggregated tier metrics.",
                )
            }),
        )
        .api_route(
            "/sully/{channel}/{year}",
            get_with(tiers::get_sully_streams, |op| {
                op.summary("Fetch SullyGnome stream windows").description(
                    "Return stream start and end windows from SullyGnome for one channel and year.",
                )
            }),
        )
        .api_route(
            "/{channel_id_type}/{channel}/random",
            get_with(logs::random_channel_line, |op| {
                op.summary("Get a random channel message")
                    .description("Return one random message sampled from the selected channel.")
            }),
        )
        .api_route(
            "/{channel_id_type}/{channel}/{user_id_type}/{user}/random",
            get_with(logs::random_user_line, |op| {
                op.summary("Get a random user message").description(
                    "Return one random message sampled from the selected user's history in the channel.",
                )
            }),
        )
        .api_route(
            "/{channel_id_type}/{channel}",
            get_with(logs::get_channel_logs, |op| {
                op.summary("Get channel logs").description(
                    "Fetch channel logs. If `from` and `to` are omitted, the endpoint redirects to the latest available day.",
                )
            }),
        )
        .api_route(
            "/{channel_id_type}/{channel}/{user_id_type}/{user}",
            get_with(logs::get_user_logs, |op| {
                op.summary("Get logs for one user in one channel").description(
                    "Fetch one user's logs inside a channel. If `from` and `to` are omitted, the endpoint redirects to the latest available month.",
                )
            }),
        )
        .api_route(
            "/{channel_id_type}/{channel}/{year}/{month}/{day}",
            get_with(logs::get_channel_logs_by_date, |op| {
                op.summary("Get channel logs for one day")
                    .description("Fetch channel logs for the exact UTC day provided in the path.")
            }),
        )
        .api_route(
            "/{channel_id_type}/{channel}/{user_id_type}/{user}/{year}/{month}",
            get_with(logs::get_user_logs_by_date, |op| {
                op.summary("Get one user's logs for one month").description(
                    "Fetch one user's logs inside a channel for the exact UTC month provided in the path.",
                )
            }),
        )
        .api_route("/optout", post(optout::optout))
        .api_route("/capabilities", get(capabilities))
        .route("/docs", get(docs::scalar_page))
        .route("/openapi.json", get(docs::serve_openapi))
}

async fn capabilities() -> Json<Vec<&'static str>> {
    Json(CAPABILITIES.to_vec())
}

/// Adds the `x-rustlog-capabilities` header that justlog clients look for.
pub async fn capabilities_header(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        "x-rustlog-capabilities",
        CAPABILITIES.join(",").try_into().unwrap(),
    );
    response
}
