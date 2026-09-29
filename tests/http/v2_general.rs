//! API v2: routing, errors and documents.

use crate::support::{get, request};
use axum::http::Method;
use insta::assert_snapshot;

#[tokio::test]
async fn root() {
    assert_snapshot!(get("/api/v2").send().await);
}

#[tokio::test]
async fn root_trailing_slash() {
    assert_snapshot!(get("/api/v2/").send().await);
}

#[tokio::test]
async fn unknown_route() {
    assert_snapshot!(get("/api/v2/nonexistent").send().await);
}

#[tokio::test]
async fn wrong_method() {
    assert_snapshot!(request(Method::POST, "/api/v2/channels").send().await);
}

#[tokio::test]
async fn openapi() {
    assert_snapshot!(get("/api/v2/openapi.json").without_body().send().await);
}

#[tokio::test]
async fn docs() {
    assert_snapshot!(get("/api/v2/docs").without_body().send().await);
}
