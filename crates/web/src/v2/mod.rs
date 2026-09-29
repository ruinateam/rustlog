//! The v2 API under `/api/v2`: canonical Twitch ids, explicit ranges and
//! `application/problem+json` errors.

mod docs;
mod dto;
mod handlers;
mod problem;

use self::docs::V2Spec;
use crate::openapi::enrich_openapi;
use aide::{
    axum::{
        ApiRouter,
        routing::{get, get_with},
    },
    openapi::{Info, OpenApi, Server},
};
use axum::{Extension, Router, http::StatusCode, routing::any};
use rustlog_app::App;
use std::sync::Arc;

/// The v2 routes, to be nested under `/api/v2`, with their OpenAPI document.
pub struct V2Api {
    pub router: Router<App>,
    pub spec: Arc<OpenApi>,
}

pub fn api() -> V2Api {
    let mut spec = openapi();
    let router = routes()
        .route("/docs", get(docs::scalar_page))
        .route("/openapi.json", get(docs::serve_openapi))
        // Unlike a nested service, a nested router neither matches its bare
        // prefix nor keeps its own default fallback: without these, `/api/v2`
        // would hit the legacy routes and unknown paths the frontend fallback.
        .route("/", any(not_found))
        .fallback(not_found)
        .finish_api(&mut spec);
    enrich_openapi(&mut spec);
    let spec = Arc::new(spec);

    V2Api {
        router: router.layer(Extension(V2Spec(spec.clone()))),
        spec,
    }
}

fn openapi() -> OpenApi {
    OpenApi {
        info: Info {
            title: "ChatTiers Rustlog API v2".to_owned(),
            summary: Some("Versioned API for canonical Twitch ids and explicit query semantics.".to_owned()),
            description: Some(
                "v2 is additive and does not change legacy routes. Errors use `application/problem+json`; log ranges require both `from` and `to` in RFC 3339 format."
                    .to_owned(),
            ),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            ..Info::default()
        },
        servers: vec![Server {
            url: "/api/v2".to_owned(),
            description: Some("Current rustlog instance, version 2 API".to_owned()),
            ..Server::default()
        }],
        ..OpenApi::default()
    }
}

fn routes() -> ApiRouter<App> {
    ApiRouter::new()
        .api_route(
            "/users/resolve",
            get_with(handlers::resolve_user, |op| {
                op.id("resolveUser")
                    .tag("Users")
                    .summary("Resolve a Twitch login")
                    .description("Resolve one Twitch login to the canonical numeric user id.")
            }),
        )
        .api_route(
            "/channels/{channel_id}/availability",
            get_with(handlers::availability, |op| {
                op.id("getChannelAvailability")
                    .tag("Logs")
                    .summary("List available log periods")
                    .description("List day buckets for a channel or month buckets for one canonical user id.")
            }),
        )
        .api_route(
            "/channels/{channel_id}/users/{user_id}/logs",
            get_with(handlers::user_logs, |op| {
                op.id("getUserLogs")
                    .tag("Logs")
                    .summary("Get user logs for an explicit range")
                    .description("Both RFC 3339 range endpoints are required. `from` is inclusive and `to` is exclusive.")
            }),
        )
}

async fn not_found() -> StatusCode {
    StatusCode::NOT_FOUND
}
