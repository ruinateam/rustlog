mod admin;
mod firehose;
mod frontend;
mod handlers;
mod responders;
pub mod schema;
mod trace_layer;
mod v2;

#[cfg(test)]
mod tests;

use self::handlers::no_cache_header;
use crate::{
    app::App,
    bot::BotMessage,
    web::admin::{admin_auth, AdminApiKey},
    ShutdownRx,
};
use aide::{
    axum::{
        routing::{get, get_with, post, post_with},
        ApiRouter, IntoApiResponse,
    },
    openapi::{Info, OpenApi, Operation, ParameterSchemaOrContent, ReferenceOr, Server},
    scalar::Scalar,
};
use axum::{
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::any,
    Extension, Json, Router, ServiceExt,
};
use axum_prometheus::PrometheusMetricLayerBuilder;
use prometheus::TextEncoder;
use serde_json::json;
use std::{
    net::{AddrParseError, SocketAddr},
    str::FromStr,
    sync::{Arc, OnceLock},
};
use tokio::{net::TcpListener, sync::mpsc::Sender};
use tower_http::{
    compression::CompressionLayer, cors::CorsLayer, normalize_path::NormalizePath,
    trace::TraceLayer, CompressionLevel,
};
use tracing::{debug, info};

const CAPABILITIES: &[&str] = &["arbitrary-range-query", "search", "stats", "namehistory"];
const SCALAR_SPEC_URL: &str = "/openapi.json";
const SCALAR_PAGE_TITLE: &str = "ChatTiers Rustlog API";
const SCALAR_HEAD_INJECT: &str = r#"
<link rel="preconnect" href="https://fonts.googleapis.com" />
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin />
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&family=JetBrains+Mono:wght@400;500;600&display=swap" rel="stylesheet" />
<style>
  :root {
    --scalar-font: 'Inter', sans-serif;
    --scalar-font-code: 'JetBrains Mono', monospace;
  }
</style>
"#;

/// The HTTP routes together with the OpenAPI documents that describe them.
pub struct Api {
    /// Needs the [`App`] state and the extensions added in [`run`].
    pub router: Router<App>,
    pub legacy_openapi: Arc<OpenApi>,
    pub v2_openapi: Arc<OpenApi>,
}

pub async fn run(app: App, mut shutdown_rx: ShutdownRx, bot_tx: Sender<BotMessage>) {
    metrics_prometheus::install();

    let listen_address =
        parse_listen_addr(&app.config.listen_address).expect("Invalid listen address");

    let app = service(app, bot_tx, shutdown_rx.clone());

    info!("Listening on {listen_address}");

    let listener = TcpListener::bind(&listen_address)
        .await
        .expect("Could not create TCP listener");

    axum::serve(listener, ServiceExt::<Request>::into_make_service(app))
        .with_graceful_shutdown(async move {
            shutdown_rx.changed().await.ok();
            debug!("Shutting down web task");
        })
        .await
        .unwrap();
}

/// Builds the complete HTTP service that [`run`] serves.
pub fn service(
    app: App,
    bot_tx: Sender<BotMessage>,
    shutdown_rx: ShutdownRx,
) -> NormalizePath<Router> {
    let admin_api_key = AdminApiKey(app.config.admin_api_key.as_deref().map(Arc::from));

    let router = api()
        .router
        .layer(Extension(bot_tx))
        .layer(Extension(shutdown_rx))
        .layer(Extension(admin_api_key))
        .with_state(app)
        .layer(CorsLayer::permissive())
        .layer(CompressionLayer::new().quality(CompressionLevel::Fastest));
    NormalizePath::trim_trailing_slash(router)
}

/// Builds the routes and generates their OpenAPI documents.
pub fn api() -> Api {
    aide::generate::on_error(|error| {
        panic!("Could not generate docs: {error}");
    });
    aide::generate::infer_responses(true);
    aide::generate::extract_schemas(true);

    let mut api = OpenApi {
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
    };

    let admin_routes = ApiRouter::new()
        .api_route(
            "/channels",
            post_with(admin::add_channels, |mut op| {
                admin::admin_auth_doc(&mut op);
                op.summary("Join channels for live logging")
                    .tag("Admin")
                    .description("Join the specified channels")
            })
            .delete_with(admin::remove_channels, |mut op| {
                admin::admin_auth_doc(&mut op);
                op.summary("Leave channels and stop live logging")
                    .tag("Admin")
                    .description("Leave the specified channels")
            }),
        )
        .api_route(
            "/firehose",
            get_with(firehose::firehose, |mut op| {
                admin::admin_auth_doc(&mut op);
                op.summary("Stream live accepted chat events")
                    .tag("Admin")
                    .description("Open a WebSocket feed after authenticating with `X-Api-Key`. Messages are live, at-least-once deliveries accepted by the writer queue; reconnect and replay through the HTTP logs API after a `1013` close.")
            }),
        )
        .route_layer(middleware::from_fn(admin_auth));

    let mut v2_api = OpenApi {
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
    };
    let v2_router = v2::router()
        .route("/docs", get(v2::scalar_page))
        .route("/openapi.json", get(v2::serve_openapi))
        // Unlike a nested service, a nested router neither matches its bare
        // prefix nor keeps its own default fallback: without these, `/api/v2`
        // would hit the legacy routes and unknown paths the frontend fallback.
        .route("/", any(not_found))
        .fallback(not_found)
        .finish_api(&mut v2_api);
    enrich_openapi(&mut v2_api);
    let v2_openapi = Arc::new(v2_api);
    let v2_router = v2_router.layer(Extension(v2::V2OpenApi(v2_openapi.clone())));

    let router = ApiRouter::new()
        .merge(Router::new().nest("/api/v2", v2_router))
        .nest("/admin", admin_routes)
        .api_route(
            "/channels",
            get_with(handlers::get_channels, |op| {
                op.summary("List logged channels").description(
                    "Return the channel logins and Twitch ids currently configured for live logging.",
                )
            }),
        )
        .api_route(
            "/list",
            get_with(handlers::list_available_logs, |op| {
                op.summary("List available log buckets").description(
                    "Show which years, months, or days exist. Without query parameters, returns buckets for all configured channels. With `channel` or `channelid`, returns buckets for that channel, optionally narrowed to a specific user.",
                )
            }),
        )
        .api_route(
            "/badges/{channel_id}",
            get_with(handlers::get_chat_badges, |op| {
                op.summary("Get Twitch chat badge metadata").description(
                    "Return global and channel-specific Twitch badge images for rendering stored chat messages. Only available for logged channels.",
                )
            }),
        )
        // Paths with static parts should go first so they aren't overridden by the dynamic date paths later
        .api_route(
            "/namehistory/{user_id}",
            get_with(handlers::get_user_name_history, |op| {
                op.summary("Get historical logins for a user id").description(
                    "Return previous usernames seen for the provided Twitch user id together with their first and last timestamps.",
                )
            }),
        )
        .api_route(
            "/{channel_id_type}/{channel}/{user_id_type}/{user}/search",
            get_with(handlers::search_user_logs, |op| {
                op.summary("Search a user's messages in a channel")
                    .description("Run a text search over one user's messages inside one channel.")
            }),
        )
        .api_route(
            "/{channel_id_type}/{channel}/{user_id_type}/{user}/stats",
            get_with(handlers::get_user_stats, |op| {
                op.summary("Get per-user message stats").description(
                    "Return message totals and related aggregates for one user in one channel.",
                )
            }),
        )
        .api_route(
            "/{channel_id_type}/{channel}/stats",
            get_with(handlers::get_channel_stats, |op| {
                op.summary("Get channel-wide stats")
                    .description("Return message totals and top chatters for the selected channel.")
            }),
        )
        .api_route(
            "/{channel_id_type}/{channel}/tiers/{year}/{month}/{day}",
            get_with(handlers::get_channel_tiers_day, |op| {
                op.summary("Get daily chat tiers").description(
                    "Return up to 500 users for one Europe/Moscow calendar day with deterministic windowed ranks and tier calculations.",
                )
            }),
        )
        .api_route(
            "/{channel_id_type}/{channel}/tiers/{year}/{month}",
            get_with(handlers::get_channel_tiers_month, |op| {
                op.summary("Get monthly chat tiers").description(
                    "Return up to 500 users for one Europe/Moscow calendar month with deterministic aggregated tier metrics.",
                )
            }),
        )
        .api_route(
            "/{channel_id_type}/{channel}/tiers/{year}",
            get_with(handlers::get_channel_tiers_year, |op| {
                op.summary("Get yearly chat tiers").description(
                    "Return up to 500 users for one Europe/Moscow calendar year with deterministic aggregated tier metrics.",
                )
            }),
        )
        .api_route(
            "/sully/{channel}/{year}",
            get_with(handlers::get_sully_streams, |op| {
                op.summary("Fetch SullyGnome stream windows").description(
                    "Return stream start and end windows from SullyGnome for one channel and year.",
                )
            }),
        )
        .api_route(
            "/{channel_id_type}/{channel}/random",
            get_with(handlers::random_channel_line, |op| {
                op.summary("Get a random channel message")
                    .description("Return one random message sampled from the selected channel.")
            }),
        )
        .api_route(
            "/{channel_id_type}/{channel}/{user_id_type}/{user}/random",
            get_with(handlers::random_user_line, |op| {
                op.summary("Get a random user message").description(
                    "Return one random message sampled from the selected user's history in the channel.",
                )
            }),
        )
        .api_route(
            "/{channel_id_type}/{channel}",
            get_with(handlers::get_channel_logs, |op| {
                op.summary("Get channel logs").description(
                    "Fetch channel logs. If `from` and `to` are omitted, the endpoint redirects to the latest available day.",
                )
            }),
        )
        .api_route(
            "/{channel_id_type}/{channel}/{user_id_type}/{user}",
            get_with(handlers::get_user_logs, |op| {
                op.summary("Get logs for one user in one channel").description(
                    "Fetch one user's logs inside a channel. If `from` and `to` are omitted, the endpoint redirects to the latest available month.",
                )
            }),
        )
        .api_route(
            "/{channel_id_type}/{channel}/{year}/{month}/{day}",
            get_with(handlers::get_channel_logs_by_date, |op| {
                op.summary("Get channel logs for one day")
                    .description("Fetch channel logs for the exact UTC day provided in the path.")
            }),
        )
        .api_route(
            "/{channel_id_type}/{channel}/{user_id_type}/{user}/{year}/{month}",
            get_with(handlers::get_user_logs_by_date, |op| {
                op.summary("Get one user's logs for one month").description(
                    "Fetch one user's logs inside a channel for the exact UTC month provided in the path.",
                )
            }),
        )
        .api_route("/optout", post(handlers::optout))
        .api_route("/capabilities", get(capabilities))
        .route("/docs", get(scalar_page))
        .route("/openapi.json", get(serve_openapi))
        .route("/assets/{*asset}", get(frontend::static_asset))
        .fallback(frontend::static_asset)
        .layer(middleware::from_fn(capabilities_header_middleware))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(trace_layer::make_span_with)
                .on_response(trace_layer::on_response),
        )
        .layer(
            PrometheusMetricLayerBuilder::new()
                .with_prefix("rustlog")
                .build(),
        )
        .route("/metrics", get(metrics))
        .finish_api(&mut api);

    enrich_openapi(&mut api);
    let legacy_openapi = Arc::new(api);

    Api {
        router: router.layer(Extension(legacy_openapi.clone())),
        legacy_openapi,
        v2_openapi,
    }
}

pub fn parse_listen_addr(addr: &str) -> Result<SocketAddr, AddrParseError> {
    if addr.starts_with(':') {
        SocketAddr::from_str(&format!("0.0.0.0{addr}"))
    } else {
        SocketAddr::from_str(addr)
    }
}

async fn not_found() -> StatusCode {
    StatusCode::NOT_FOUND
}

async fn capabilities() -> Json<Vec<&'static str>> {
    Json(CAPABILITIES.to_vec())
}

async fn capabilities_header_middleware(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        "x-rustlog-capabilities",
        CAPABILITIES.join(",").try_into().unwrap(),
    );
    response
}

async fn metrics() -> impl IntoApiResponse {
    let metric_families = prometheus::gather();

    let encoder = TextEncoder::new();
    let metrics = encoder.encode_to_string(&metric_families).unwrap();
    (no_cache_header(), metrics)
}

fn enrich_openapi(api: &mut OpenApi) {
    let Some(paths) = &mut api.paths else {
        return;
    };

    for path_item in paths.paths.values_mut() {
        let ReferenceOr::Item(path_item) = path_item else {
            continue;
        };

        if let Some(operation) = &mut path_item.get {
            enrich_operation(operation);
        }
        if let Some(operation) = &mut path_item.post {
            enrich_operation(operation);
        }
        if let Some(operation) = &mut path_item.delete {
            enrich_operation(operation);
        }
        if let Some(operation) = &mut path_item.put {
            enrich_operation(operation);
        }
        if let Some(operation) = &mut path_item.patch {
            enrich_operation(operation);
        }
    }
}

fn enrich_operation(operation: &mut Operation) {
    for parameter in &mut operation.parameters {
        let Some(parameter) = parameter.as_item_mut() else {
            continue;
        };

        {
            let data = parameter.parameter_data_mut();
            if data.description.is_none() {
                data.description = parameter_description(&data.name).map(str::to_owned);
            }
            if let ParameterSchemaOrContent::Schema(schema) = &mut data.format {
                enrich_parameter_schema(&data.name, schema);
            }
            if data.name == "exclude_bots" {
                data.explode = Some(false);
            }
        }
    }
}

fn parameter_description(name: &str) -> Option<&'static str> {
    Some(match name {
        "channel_id_type" => {
            "Use `channel` for a Twitch login or `channelid` for a Twitch user id."
        }
        "user_id_type" => "Use `user` for a Twitch login or `userid` for a Twitch user id.",
        "channel" => "Twitch channel login or id, depending on the selected channel id type.",
        "channelid" => {
            "Twitch channel user id. Use this instead of `channel` when you already know the id."
        }
        "user" => "Twitch user login or id, depending on the selected user id type.",
        "userid" => "Twitch user id. Use this instead of `user` when you already know the id.",
        "year" => "UTC year.",
        "month" => "UTC month number from 1 to 12.",
        "day" => "UTC day of month.",
        "from" => "RFC 3339 inclusive start timestamp.",
        "to" => "RFC 3339 exclusive end timestamp.",
        "q" => "Search text.",
        "json" => "Return full JSON messages.",
        "jsonBasic" => "Return compact JSON messages.",
        "raw" => "Return raw IRC lines.",
        "reverse" => "Return newest messages first.",
        "ndjson" => "Return newline-delimited JSON.",
        "limit" => "Maximum number of messages to return.",
        "offset" => "Number of messages to skip.",
        "mode" => "Tier mode: all messages, online stream windows, or offline windows.",
        "exclude_bots" => "Bot logins excluded from tier tables.",
        "X-Api-Key" => "Configured admin API key.",
        _ => return None,
    })
}

fn enrich_parameter_schema(name: &str, schema: &mut aide::openapi::SchemaObject) {
    match name {
        "channel_id_type" => set_schema(
            schema,
            json!({
                "type": "string",
                "enum": ["channel", "channelid"]
            }),
        ),
        "user_id_type" => set_schema(
            schema,
            json!({
                "type": "string",
                "enum": ["user", "userid"]
            }),
        ),
        "mode" => set_schema(
            schema,
            json!({
                "type": "string",
                "enum": ["all", "online", "offline"]
            }),
        ),
        "exclude_bots" => set_schema(
            schema,
            json!({
                "type": "array",
                "items": {
                    "type": "string",
                    "enum": schema::DEFAULT_EXCLUDED_BOTS
                },
                "uniqueItems": true
            }),
        ),
        "json" | "jsonBasic" | "raw" | "reverse" | "ndjson" => {
            let object = schema.json_schema.ensure_object();
            object.insert("type".to_owned(), json!("boolean"));
            object.remove("default");
            object.remove("examples");
        }
        "month" => {
            set_schema(
                schema,
                json!({
                    "type": "integer",
                    "enum": [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
                    "minimum": 1,
                    "maximum": 12
                }),
            );
        }
        "limit" => {
            let object = schema.json_schema.ensure_object();
            object.insert("minimum".to_owned(), json!(1));
        }
        "offset" => {
            let object = schema.json_schema.ensure_object();
            object.insert("minimum".to_owned(), json!(0));
        }
        _ => clear_examples_and_defaults(schema),
    }
}

fn clear_examples_and_defaults(schema: &mut aide::openapi::SchemaObject) {
    let object = schema.json_schema.ensure_object();
    object.remove("example");
    object.remove("examples");
    object.remove("default");
}

fn set_schema(schema: &mut aide::openapi::SchemaObject, value: serde_json::Value) {
    schema.json_schema = value
        .try_into()
        .expect("OpenAPI parameter schema must be a JSON object");
}

async fn serve_openapi(Extension(api): Extension<Arc<OpenApi>>) -> impl IntoApiResponse {
    Json(api.as_ref()).into_response()
}

async fn scalar_page() -> Html<&'static str> {
    static HTML: OnceLock<&'static str> = OnceLock::new();

    Html(HTML.get_or_init(build_scalar_html))
}

fn build_scalar_html() -> &'static str {
    let html = Scalar::new(SCALAR_SPEC_URL)
        .with_title(SCALAR_PAGE_TITLE)
        .html()
        .replace(
            "<style>",
            &format!("{SCALAR_HEAD_INJECT}\n<style>"),
        )
        .replace(
            "theme: 'purple',",
            r#"theme: 'default',
                    layout: 'modern',
                    withDefaultFonts: false,
                    operationTitleSource: 'summary',
                    defaultHttpClient: { targetKey: 'shell', clientKey: 'curl' },
                    searchHotKey: 'k',
                    hideDarkModeToggle: false,
                    metaData: {
                      title: 'ChatTiers Rustlog API',
                      description: 'Browse endpoints, inspect request and response shapes, and test the current rustlog instance.'
                    },"#,
        );

    Box::leak(html.into_boxed_str())
}
