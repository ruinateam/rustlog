//! Shared fixtures: a fresh ClickHouse database, a fake remote instance that
//! serves canned JSON, and rendering of the stored messages for snapshots.

use axum::{Router, http::StatusCode, http::Uri, response::IntoResponse};
use clickhouse::Row;
use rustlog::{config::Config, storage::setup_db};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    env,
    fmt::Write,
    sync::{Arc, Mutex},
};
use tokio::net::TcpListener;

/// A fresh database with the rustlog schema, dropped by [`TestDb::stop`].
pub struct TestDb {
    pub db: clickhouse::Client,
    root: clickhouse::Client,
    name: String,
}

impl TestDb {
    pub async fn start() -> Self {
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

        let name = format!("rustlog_test_{}", uuid::Uuid::new_v4().simple());
        root.query(&format!("CREATE DATABASE {name}"))
            .execute()
            .await
            .expect("could not create the test database");
        let db = root.clone().with_database(&name);

        let config: Config = serde_json::from_value(json!({
            "clickhouseUrl": url,
            "clickhouseDb": name,
            "channels": [],
            "clientID": "fake",
            "clientSecret": "fake",
            "admins": [],
        }))
        .unwrap();
        setup_db(&db, &name, &config).await.unwrap();

        Self { db, root, name }
    }

    pub async fn execute(&self, sql: &str) {
        self.db.query(sql).execute().await.unwrap();
    }

    /// Every stored message, one line each, ordered by time and id.
    pub async fn messages(&self) -> String {
        let rows = self
            .db
            .query(
                "
                SELECT
                    toString(timestamp) AS timestamp,
                    toString(id) AS id,
                    channel_id,
                    channel_login,
                    user_id,
                    user_login,
                    display_name,
                    color,
                    badges,
                    text
                FROM message_structured
                ORDER BY timestamp, id, user_id
                ",
            )
            .fetch_all::<MessageRow>()
            .await
            .unwrap();

        let mut rendered = String::new();
        for row in rows {
            writeln!(
                rendered,
                "{} {} #{}({}) {}({}) {:?} color={:?} badges={:?}: {}",
                row.timestamp,
                row.id,
                row.channel_login,
                row.channel_id,
                row.user_login,
                row.user_id,
                row.display_name,
                row.color,
                row.badges,
                row.text,
            )
            .unwrap();
        }
        rendered
    }

    pub async fn tables(&self) -> Vec<String> {
        self.db
            .query(
                "SELECT name FROM system.tables WHERE database = currentDatabase() ORDER BY name",
            )
            .fetch_all()
            .await
            .unwrap()
    }

    pub async fn stop(self) {
        self.root
            .query(&format!("DROP DATABASE {}", self.name))
            .execute()
            .await
            .unwrap();
    }
}

#[derive(Row, Deserialize)]
struct MessageRow {
    timestamp: String,
    id: String,
    channel_id: String,
    channel_login: String,
    user_id: String,
    user_login: String,
    display_name: String,
    color: Option<u32>,
    badges: Vec<String>,
    text: String,
}

/// An HTTP server on a random local port that answers the registered
/// paths (with their query) with canned JSON, and everything else with 404.
pub struct FakeRemote {
    pub base_url: String,
    responses: Arc<Mutex<HashMap<String, Value>>>,
}

impl FakeRemote {
    pub async fn start() -> Self {
        let responses = Arc::new(Mutex::new(HashMap::<String, Value>::new()));
        let router = Router::new().fallback({
            let responses = responses.clone();
            move |uri: Uri| {
                let key = uri.path_and_query().map_or("", |path| path.as_str());
                let body = responses.lock().unwrap().get(key).cloned();
                async move {
                    match body {
                        Some(body) => axum::Json(body).into_response(),
                        None => StatusCode::NOT_FOUND.into_response(),
                    }
                }
            }
        });

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

        Self {
            base_url: format!("http://{address}"),
            responses,
        }
    }

    pub fn respond(&self, path_and_query: &str, body: Value) {
        self.responses
            .lock()
            .unwrap()
            .insert(path_and_query.to_owned(), body);
    }
}

/// A chat message in channel `11111` as the JSON API of a remote instance
/// returns it with `?jsonBasic`.
pub struct RemoteMessage {
    json: Value,
}

impl RemoteMessage {
    pub fn new(timestamp: &str, user_id: &str, display_name: &str, text: &str) -> Self {
        Self {
            json: json!({
                "text": text,
                "displayName": display_name,
                "timestamp": timestamp,
                "tags": {
                    "room-id": "11111",
                    "user-id": user_id,
                    "display-name": display_name,
                },
            }),
        }
    }

    pub fn id(mut self, id: &str) -> Self {
        self.json["id"] = json!(id);
        self
    }

    pub fn tag(mut self, name: &str, value: &str) -> Self {
        self.json["tags"][name] = json!(value);
        self
    }

    pub fn without_tag(mut self, name: &str) -> Self {
        self.json["tags"].as_object_mut().unwrap().remove(name);
        self
    }
}

/// The body of a day of logs, as `/channel/{login}/{year}/{month}/{day}`
/// returns it.
pub fn day_of_logs(messages: impl IntoIterator<Item = RemoteMessage>) -> Value {
    let messages: Vec<Value> = messages.into_iter().map(|message| message.json).collect();
    json!({ "messages": messages })
}

/// Message ids that read well in snapshots.
pub fn message_id(number: u32) -> String {
    format!("00000000-0000-4000-8000-{number:012}")
}
