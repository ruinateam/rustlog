//! Admin routes. The API key check runs before method matching, so requests
//! without a key get 403 even for unsupported methods.

use axum::http::Method;
use insta::assert_snapshot;

use crate::support::{get, request};

const CHANNELS: &str = r#"{"channels":["11111"]}"#;

#[tokio::test]
async fn join_channels_without_key() {
    assert_snapshot!(
        request(Method::POST, "/admin/channels")
            .json(CHANNELS)
            .send()
            .await
    );
}

#[tokio::test]
async fn join_channels_with_wrong_key() {
    assert_snapshot!(
        request(Method::POST, "/admin/channels")
            .header("x-api-key", "wrong")
            .json(CHANNELS)
            .send()
            .await
    );
}

#[tokio::test]
async fn join_channels_invalid_body() {
    assert_snapshot!(
        request(Method::POST, "/admin/channels")
            .admin_key()
            .json(r#"{"bad":true}"#)
            .send()
            .await
    );
}

#[tokio::test]
async fn leave_channels_without_key() {
    assert_snapshot!(
        request(Method::DELETE, "/admin/channels")
            .json(CHANNELS)
            .send()
            .await
    );
}

#[tokio::test]
async fn channels_wrong_method_without_key() {
    assert_snapshot!(get("/admin/channels").send().await);
}

#[tokio::test]
async fn channels_wrong_method() {
    assert_snapshot!(get("/admin/channels").admin_key().send().await);
}

#[tokio::test]
async fn channels_trailing_slash_without_key() {
    assert_snapshot!(get("/admin/channels/").send().await);
}

#[tokio::test]
async fn firehose_without_key() {
    assert_snapshot!(get("/admin/firehose").send().await);
}

#[tokio::test]
async fn firehose_without_upgrade() {
    assert_snapshot!(get("/admin/firehose").admin_key().send().await);
}

#[tokio::test]
async fn firehose_wrong_method_without_key() {
    assert_snapshot!(request(Method::PATCH, "/admin/firehose").send().await);
}

#[tokio::test]
async fn unknown_route() {
    assert_snapshot!(get("/admin/unknown").send().await);
}

#[tokio::test]
async fn unknown_route_with_key() {
    assert_snapshot!(get("/admin/unknown").admin_key().send().await);
}

#[tokio::test]
async fn removed_badges_route() {
    assert_snapshot!(get("/admin/badges/11111").admin_key().send().await);
}
