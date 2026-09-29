//! Shared fixture: a seeded ClickHouse database with the full HTTP service on
//! top, and a request builder that renders responses for snapshots.

mod normalize;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request},
};
use rustlog_app::{
    App, BotMessage,
    config::Config,
    services::{sully::SullyGnome, tiers::Tiers},
};
use rustlog_storage::{setup_db, state::OperationalState, writer::FlushBuffer};
use rustlog_twitch::Twitch;
use serde_json::json;
use std::{env, fs, sync::Arc};
use tempfile::TempDir;
use tokio::sync::{broadcast, mpsc, watch};
use tower::ServiceExt;
use tower_http::normalize_path::NormalizePath;

pub const ADMIN_KEY: &str = "testkey";

/// Headers worth pinning; the rest (date, vary, ...) are noise.
const SNAPSHOT_HEADERS: &[&str] = &[
    "content-type",
    "location",
    "cache-control",
    "x-rustlog-capabilities",
    "content-disposition",
];

/// Channel `11111` (`testchan`) with messages from `alice` (`22222`) and
/// `bob` (`33333`) in February and March 2026.
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

/// SullyGnome is never reachable in tests (nothing listens on the discard
/// port), so stream lists always come from the cache.
const UNREACHABLE_SULLYGNOME: &str = "http://127.0.0.1:9";

/// Cached SullyGnome streams of channel `11111` in 2026: one stream on
/// 2026-03-01 from 06:30 to 08:30 UTC, which covers alice's 07:00 message.
const SULLY_CACHE: (&str, &str) = (
    "11111-2026.json",
    r#"{"channel":"11111","year":2026,"total":1,"streams":[{"streamId":"1","startIso":"2026-03-01T09:30:00+03:00","lengthMinutes":120}]}"#,
);

/// A fresh database with seeded messages and the full HTTP service on top.
struct TestServer {
    service: NormalizePath<Router>,
    root: clickhouse::Client,
    db_name: String,
    _sully_cache: TempDir,
    // Keep the channel ends alive for the lifetime of the service.
    _bot_rx: mpsc::Receiver<BotMessage>,
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
            .expect("could not create the test database");
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

        setup_db(&db, &db_name, &config.legacy_state())
            .await
            .unwrap();
        db.query(SEED).execute().await.unwrap();

        let sully_cache = TempDir::new().unwrap();
        let (file_name, contents) = SULLY_CACHE;
        fs::write(sully_cache.path().join(file_name), contents).unwrap();

        let db = Arc::new(db);
        // Without a token every Twitch lookup fails fast, so the tests never
        // reach the network.
        let twitch = Twitch::new();
        let sully = SullyGnome::new(UNREACHABLE_SULLYGNOME, sully_cache.path()).unwrap();
        let app = App {
            twitch: twitch.clone(),
            sully: sully.clone(),
            tiers: Tiers::new(db.clone(), sully, twitch),
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
            service: rustlog_web::service(app, bot_tx, shutdown_rx),
            root,
            db_name,
            _sully_cache: sully_cache,
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
}

/// A request against a fresh [`TestServer`], rendered for a snapshot.
pub struct TestRequest {
    method: Method,
    path: String,
    headers: Vec<(&'static str, &'static str)>,
    body: Option<&'static str>,
    body_in_snapshot: bool,
}

pub fn get(path: impl Into<String>) -> TestRequest {
    request(Method::GET, path)
}

pub fn request(method: Method, path: impl Into<String>) -> TestRequest {
    TestRequest {
        method,
        path: path.into(),
        headers: Vec::new(),
        body: None,
        body_in_snapshot: true,
    }
}

impl TestRequest {
    pub fn header(mut self, name: &'static str, value: &'static str) -> Self {
        self.headers.push((name, value));
        self
    }

    pub fn admin_key(self) -> Self {
        self.header("x-api-key", ADMIN_KEY)
    }

    pub fn json(mut self, body: &'static str) -> Self {
        self.body = Some(body);
        self.header("content-type", "application/json")
    }

    /// Leaves the body out of the snapshot, for random or huge responses.
    pub fn without_body(mut self) -> Self {
        self.body_in_snapshot = false;
        self
    }

    /// Sends the request to a fresh server and renders the status, the
    /// relevant headers and the normalized body.
    pub async fn send(self) -> String {
        let server = TestServer::start().await;

        let mut request = Request::builder().method(&self.method).uri(&self.path);
        for (name, value) in &self.headers {
            request = request.header(*name, *value);
        }
        let body = self.body.map_or_else(Body::empty, Body::from);

        let response = server
            .service
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();

        let mut rendered = format!("{} {}\n", self.method, self.path);
        rendered.push_str(&format!("status: {}\n", response.status()));
        for name in SNAPSHOT_HEADERS {
            if let Some(value) = response.headers().get(*name) {
                rendered.push_str(&format!("{name}: {}\n", value.to_str().unwrap()));
            }
        }
        rendered.push_str("---\n");

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        if self.body_in_snapshot {
            rendered.push_str(&normalize::body(&String::from_utf8_lossy(&body)));
        } else {
            rendered.push_str("<body omitted>");
        }

        server.stop().await;
        rendered
    }
}
