//! Snapshot tests of the HTTP API against a real ClickHouse.
//!
//! They pin the observable behaviour of every route (status, relevant headers
//! and body) so that refactoring cannot silently change the legacy API.
//!
//! The tests are ignored by default because they need ClickHouse: run them
//! with `just test-integration`, or set `RUSTLOG_TEST_CLICKHOUSE_URL` (plus
//! `RUSTLOG_TEST_CLICKHOUSE_USER` / `RUSTLOG_TEST_CLICKHOUSE_PASSWORD` if
//! needed) and run `cargo nextest run --run-ignored only`. The ClickHouse
//! server must run in UTC, as the Docker image does.

use super::service;
use crate::{
    app::{cache::UsersCache, App},
    config::Config,
    db::{setup_db, writer::FlushBuffer},
    state::OperationalState,
};
use axum::{
    body::{to_bytes, Body},
    http::{Method, Request},
};
use serde_json::{json, Map, Value};
use std::{env, sync::Arc};
use tokio::sync::{broadcast, mpsc, watch, RwLock};
use tower::ServiceExt;
use twitch_api::HelixClient;

const ADMIN_KEY: &str = "testkey";
const JSON: &str = "application/json";

/// Headers worth pinning; the rest (date, vary, ...) are noise.
const SNAPSHOT_HEADERS: &[&str] = &[
    "content-type",
    "location",
    "cache-control",
    "x-rustlog-capabilities",
    "content-disposition",
];

const SEED: &str = "
INSERT INTO message_structured
    (channel_id, channel_login, timestamp, id, message_type, user_id, user_login, display_name, color, badges, text, message_flags)
VALUES
    ('11111', 'testchan', toDateTime64('2026-03-01 07:00:00.000', 3, 'UTC'), '00000000-0000-0000-0000-000000000001', 1, '22222', 'alice', 'Alice', 16711680, ['subscriber/12'], 'hello world', 1),
    ('11111', 'testchan', toDateTime64('2026-03-01 08:30:00.500', 3, 'UTC'), '00000000-0000-0000-0000-000000000002', 1, '33333', 'bob', 'Bob', NULL, [], 'hi alice', 0),
    ('11111', 'testchan', toDateTime64('2026-03-01 09:00:00.000', 3, 'UTC'), '00000000-0000-0000-0000-000000000003', 1, '22222', 'alice', 'Alice', 16711680, ['subscriber/12'], 'hello again', 1),
    ('11111', 'testchan', toDateTime64('2026-03-02 06:15:00.000', 3, 'UTC'), '00000000-0000-0000-0000-000000000004', 1, '33333', 'bob', 'Bob', NULL, [], 'good morning', 0),
    ('11111', 'testchan', toDateTime64('2026-03-15 17:00:00.000', 3, 'UTC'), '00000000-0000-0000-0000-000000000005', 1, '22222', 'alice', 'Alice', 16711680, ['subscriber/12'], 'mid month', 1),
    ('11111', 'testchan', toDateTime64('2026-02-10 05:00:00.000', 3, 'UTC'), '00000000-0000-0000-0000-000000000006', 1, '22222', 'alice', 'Alice', 16711680, [], 'last month', 0)
";

/// A fresh database with seeded messages and the full HTTP service on top.
struct TestServer {
    service: tower_http::normalize_path::NormalizePath<axum::Router>,
    root: clickhouse::Client,
    db_name: String,
    // Keep the channel ends alive for the lifetime of the service.
    _bot_rx: mpsc::Receiver<crate::bot::BotMessage>,
    _shutdown_tx: watch::Sender<()>,
}

impl TestServer {
    async fn start() -> Self {
        let url = env::var("RUSTLOG_TEST_CLICKHOUSE_URL")
            .expect("RUSTLOG_TEST_CLICKHOUSE_URL must point to a ClickHouse server");
        let mut root = clickhouse::Client::default()
            .with_url(&url)
            .with_compression(clickhouse::Compression::None);
        if let Ok(user) = env::var("RUSTLOG_TEST_CLICKHOUSE_USER") {
            root = root.with_user(user);
        }
        if let Ok(password) = env::var("RUSTLOG_TEST_CLICKHOUSE_PASSWORD") {
            root = root.with_password(password);
        }

        let db_name = format!("rustlog_test_{}", uuid::Uuid::new_v4().simple());
        root.query(&format!("CREATE DATABASE {db_name}"))
            .execute()
            .await
            .expect("Could not create the test database");
        let db = root.clone().with_database(&db_name);

        let config: Config = serde_json::from_value(json!({
            "clickhouseUrl": url,
            "clickhouseDb": db_name,
            "channels": ["11111"],
            "clientID": "fake",
            "clientSecret": "fake",
            "admins": [],
            "adminAPIKey": ADMIN_KEY,
        }))
        .unwrap();

        setup_db(&db, &db_name, &config).await.unwrap();
        db.query(SEED).execute().await.unwrap();

        let db = Arc::new(db);
        let app = App {
            helix_client: HelixClient::default(),
            // Without a token every Twitch lookup fails fast, so the tests
            // never reach the network.
            token: Arc::new(RwLock::new(None)),
            users: UsersCache::default(),
            optout_codes: Arc::default(),
            state: OperationalState::load(db.clone()).await.unwrap(),
            db,
            config: Arc::new(config),
            flush_buffer: FlushBuffer::default(),
            firehose_tx: broadcast::channel(16).0,
        };

        let (bot_tx, bot_rx) = mpsc::channel(1);
        let (shutdown_tx, shutdown_rx) = watch::channel(());

        Self {
            service: service(app, bot_tx, shutdown_rx),
            root,
            db_name,
            _bot_rx: bot_rx,
            _shutdown_tx: shutdown_tx,
        }
    }

    async fn stop(self) {
        self.root
            .query(&format!("DROP DATABASE {}", self.db_name))
            .execute()
            .await
            .unwrap();
    }

    /// Sends a request and renders the response for a snapshot.
    async fn render(&self, case: &Case) -> String {
        let mut request = Request::builder()
            .method(case.method.clone())
            .uri(&case.path);
        for (name, value) in case.headers {
            request = request.header(*name, *value);
        }
        let body = case.body.map_or_else(Body::empty, Body::from);

        let response = self
            .service
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();

        let mut rendered = format!("{} {}\n", case.method, case.path);
        rendered.push_str(&format!("status: {}\n", response.status()));
        for name in SNAPSHOT_HEADERS {
            if let Some(value) = response.headers().get(*name) {
                rendered.push_str(&format!("{name}: {}\n", value.to_str().unwrap()));
            }
        }
        rendered.push_str("---\n");

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        if case.body_in_snapshot {
            rendered.push_str(&normalize_body(&String::from_utf8_lossy(&body)));
        } else {
            rendered.push_str("<body omitted>");
        }
        rendered
    }
}

struct Case {
    name: &'static str,
    method: Method,
    path: String,
    headers: &'static [(&'static str, &'static str)],
    body: Option<&'static str>,
    body_in_snapshot: bool,
}

impl Case {
    fn get(name: &'static str, path: impl Into<String>) -> Self {
        Self {
            name,
            method: Method::GET,
            path: path.into(),
            headers: &[],
            body: None,
            body_in_snapshot: true,
        }
    }

    fn method(mut self, method: Method) -> Self {
        self.method = method;
        self
    }

    fn headers(mut self, headers: &'static [(&'static str, &'static str)]) -> Self {
        self.headers = headers;
        self
    }

    fn json_body(mut self, body: &'static str) -> Self {
        self.body = Some(body);
        self
    }

    fn without_body(mut self) -> Self {
        self.body_in_snapshot = false;
        self
    }
}

const ADMIN: &[(&str, &str)] = &[("x-api-key", ADMIN_KEY)];
const ADMIN_JSON: &[(&str, &str)] = &[("x-api-key", ADMIN_KEY), ("content-type", JSON)];
const WRONG_ADMIN_JSON: &[(&str, &str)] = &[("x-api-key", "wrong"), ("content-type", JSON)];
const CONTENT_JSON: &[(&str, &str)] = &[("content-type", JSON)];

async fn run_cases(cases: Vec<Case>) {
    let server = TestServer::start().await;
    for case in &cases {
        let rendered = server.render(case).await;
        insta::assert_snapshot!(case.name, rendered);
    }
    server.stop().await;
}

// TODO: `GET /channels` panics without a Twitch token (`unwrap` in
// `handlers::get_channels`); cover it once that is fixed.
#[tokio::test]
#[ignore = "needs ClickHouse, run with `just test-integration`"]
async fn legacy_logs() {
    run_cases(vec![
        Case::get("capabilities", "/capabilities"),
        Case::get("list_all", "/list"),
        Case::get("list_channel", "/list?channelid=11111"),
        Case::get("list_channel_user", "/list?channelid=11111&userid=22222"),
        Case::get("channel_latest_redirect", "/channelid/11111"),
        Case::get("channel_day", "/channelid/11111/2026/3/1"),
        Case::get("channel_day_json", "/channelid/11111/2026/3/1?json"),
        Case::get(
            "channel_day_json_basic",
            "/channelid/11111/2026/3/1?jsonBasic",
        ),
        Case::get("channel_day_raw", "/channelid/11111/2026/3/1?raw"),
        Case::get("channel_day_ndjson", "/channelid/11111/2026/3/1?ndjson"),
        Case::get(
            "channel_day_reverse_limit",
            "/channelid/11111/2026/3/1?reverse&limit=1",
        ),
        Case::get(
            "channel_day_offset",
            "/channelid/11111/2026/3/1?limit=1&offset=1&json",
        ),
        Case::get(
            "channel_range",
            "/channelid/11111?from=2026-03-01T00:00:00Z&to=2026-03-02T12:00:00Z",
        ),
        Case::get(
            "channel_range_json",
            "/channelid/11111?from=2026-03-01T00:00:00Z&to=2026-03-02T12:00:00Z&json",
        ),
        Case::get(
            "channel_range_invalid",
            "/channelid/11111?from=garbage&to=2026-03-02T12:00:00Z",
        ),
        Case::get("channel_invalid_month", "/channelid/11111/2026/13/1"),
        Case::get("channel_unknown", "/channelid/99999/2026/3/1"),
        Case::get("channel_by_login", "/channel/testchan/2026/3/1"),
        Case::get("user_latest_redirect", "/channelid/11111/userid/22222"),
        Case::get("user_month", "/channelid/11111/userid/22222/2026/3"),
        Case::get(
            "user_month_json",
            "/channelid/11111/userid/22222/2026/3?json",
        ),
        Case::get("user_month_raw", "/channelid/11111/userid/22222/2026/3?raw"),
        Case::get("user_by_login", "/channel/testchan/user/alice/2026/3"),
        Case::get(
            "user_search",
            "/channelid/11111/userid/22222/search?q=hello",
        ),
        Case::get(
            "user_search_json",
            "/channelid/11111/userid/22222/search?q=hello&json",
        ),
        Case::get("user_stats", "/channelid/11111/userid/22222/stats"),
        Case::get("channel_stats", "/channelid/11111/stats"),
        Case::get("channel_random", "/channelid/11111/random").without_body(),
        Case::get("user_random", "/channelid/11111/userid/22222/random").without_body(),
        Case::get("namehistory", "/namehistory/22222"),
    ])
    .await;
}

#[tokio::test]
#[ignore = "needs ClickHouse, run with `just test-integration`"]
async fn legacy_tiers() {
    run_cases(vec![
        Case::get("tiers_day", "/channelid/11111/tiers/2026/3/1"),
        Case::get("tiers_month", "/channelid/11111/tiers/2026/3"),
        Case::get("tiers_year", "/channelid/11111/tiers/2026"),
        Case::get(
            "tiers_month_exclude_bots",
            "/channelid/11111/tiers/2026/3?exclude_bots=nightbot",
        ),
    ])
    .await;
}

#[tokio::test]
#[ignore = "needs ClickHouse, run with `just test-integration`"]
async fn legacy_admin() {
    run_cases(vec![
        Case::get("admin_channels_no_key", "/admin/channels")
            .method(Method::POST)
            .headers(CONTENT_JSON)
            .json_body(r#"{"channels":["11111"]}"#),
        Case::get("admin_channels_wrong_key", "/admin/channels")
            .method(Method::POST)
            .headers(WRONG_ADMIN_JSON)
            .json_body(r#"{"channels":["11111"]}"#),
        Case::get("admin_channels_invalid_body", "/admin/channels")
            .method(Method::POST)
            .headers(ADMIN_JSON)
            .json_body(r#"{"bad":true}"#),
        Case::get("admin_channels_delete_no_key", "/admin/channels")
            .method(Method::DELETE)
            .headers(CONTENT_JSON)
            .json_body(r#"{"channels":["11111"]}"#),
        // The auth middleware runs before method matching: 403, not 405.
        Case::get("admin_channels_get_no_key", "/admin/channels"),
        Case::get("admin_channels_get", "/admin/channels").headers(ADMIN),
        Case::get("admin_channels_trailing_slash", "/admin/channels/"),
        Case::get("admin_badges_no_key", "/admin/badges/11111"),
        Case::get("admin_badges", "/admin/badges/11111").headers(ADMIN),
        Case::get("admin_badges_post_no_key", "/admin/badges/11111").method(Method::POST),
        Case::get("admin_badges_post", "/admin/badges/11111")
            .method(Method::POST)
            .headers(ADMIN),
        Case::get("admin_firehose_no_key", "/admin/firehose"),
        Case::get("admin_firehose_no_upgrade", "/admin/firehose").headers(ADMIN),
        Case::get("admin_firehose_patch_no_key", "/admin/firehose").method(Method::PATCH),
        Case::get("admin_unknown", "/admin/unknown"),
        Case::get("admin_unknown_with_key", "/admin/unknown").headers(ADMIN),
        Case::get("root_badges", "/badges/11111"),
    ])
    .await;
}

#[tokio::test]
#[ignore = "needs ClickHouse, run with `just test-integration`"]
async fn legacy_misc() {
    run_cases(vec![
        Case::get("optout", "/optout")
            .method(Method::POST)
            .without_body(),
        Case::get("channels_put", "/channels").method(Method::PUT),
        Case::get("metrics", "/metrics").without_body(),
        Case::get("docs", "/docs").without_body(),
        Case::get("openapi", "/openapi.json").without_body(),
        Case::get("frontend_root", "/"),
        Case::get("frontend_index", "/index.html"),
        Case::get("frontend_spa_path", "/some/spa/path"),
        Case::get("frontend_missing_asset", "/assets/app.js"),
        Case::get("frontend_missing_file", "/missing.png"),
    ])
    .await;
}

const V2_RANGE: &str = "from=2026-03-01T00:00:00Z&to=2026-04-01T00:00:00Z";

#[tokio::test]
#[ignore = "needs ClickHouse, run with `just test-integration`"]
async fn v2() {
    run_cases(vec![
        Case::get("v2_root", "/api/v2"),
        Case::get("v2_root_trailing_slash", "/api/v2/"),
        Case::get("v2_unknown", "/api/v2/nonexistent"),
        Case::get("v2_availability", "/api/v2/channels/11111/availability"),
        Case::get(
            "v2_availability_user",
            "/api/v2/channels/11111/availability?userId=22222",
        ),
        Case::get(
            "v2_logs",
            format!("/api/v2/channels/11111/users/22222/logs?{V2_RANGE}"),
        ),
        Case::get(
            "v2_logs_full_json",
            format!("/api/v2/channels/11111/users/22222/logs?{V2_RANGE}&format=full-json"),
        ),
        Case::get(
            "v2_logs_ndjson",
            format!("/api/v2/channels/11111/users/22222/logs?{V2_RANGE}&format=ndjson"),
        ),
        Case::get(
            "v2_logs_text",
            format!("/api/v2/channels/11111/users/22222/logs?{V2_RANGE}&format=text"),
        ),
        Case::get(
            "v2_logs_raw",
            format!("/api/v2/channels/11111/users/22222/logs?{V2_RANGE}&format=raw"),
        ),
        Case::get(
            "v2_logs_reverse_limit",
            format!("/api/v2/channels/11111/users/22222/logs?{V2_RANGE}&reverse=true&limit=1"),
        ),
        Case::get(
            "v2_logs_invalid_format",
            format!("/api/v2/channels/11111/users/22222/logs?{V2_RANGE}&format=bogus"),
        ),
        Case::get(
            "v2_logs_inverted_range",
            "/api/v2/channels/11111/users/22222/logs?from=2026-04-01T00:00:00Z&to=2026-03-01T00:00:00Z",
        ),
        Case::get(
            "v2_logs_missing_range",
            "/api/v2/channels/11111/users/22222/logs",
        ),
        Case::get("v2_resolve", "/api/v2/users/resolve?login=testchan"),
        Case::get("v2_docs", "/api/v2/docs").without_body(),
        Case::get("v2_openapi", "/api/v2/openapi.json").without_body(),
    ])
    .await;
}

/// Makes a response body deterministic: message tags come from hash maps, so
/// JSON object keys and IRC tags are sorted.
fn normalize_body(body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        return serde_json::to_string_pretty(&canonical(value)).unwrap();
    }

    body.split('\n')
        .map(|line| match serde_json::from_str::<Value>(line) {
            Ok(value) => serde_json::to_string(&canonical(value)).unwrap(),
            Err(_) => sort_irc_tags(line),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn canonical(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(sort_irc_tags(&text)),
        Value::Array(items) => Value::Array(items.into_iter().map(canonical).collect()),
        Value::Object(object) => {
            let mut entries: Vec<_> = object.into_iter().collect();
            entries.sort_by(|(a, _), (b, _)| a.cmp(b));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, canonical(value)))
                    .collect::<Map<_, _>>(),
            )
        }
        other => other,
    }
}

/// Sorts the tags of a raw IRC line (`@a=1;b=2 :prefix COMMAND ...`).
fn sort_irc_tags(line: &str) -> String {
    let Some(rest) = line.strip_prefix('@') else {
        return line.to_owned();
    };
    let Some((tags, message)) = rest.split_once(' ') else {
        return line.to_owned();
    };
    let mut tags: Vec<_> = tags.split(';').collect();
    tags.sort_unstable();
    format!("@{} {message}", tags.join(";"))
}
