//! The embedded frontend with its fallback, the API docs and metrics.

use insta::assert_snapshot;

use crate::support::get;

#[tokio::test]
async fn root() {
    assert_snapshot!(get("/").send().await);
}

#[tokio::test]
async fn index() {
    assert_snapshot!(get("/index.html").send().await);
}

#[tokio::test]
async fn spa_path() {
    assert_snapshot!(get("/some/spa/path").send().await);
}

#[tokio::test]
async fn missing_asset() {
    assert_snapshot!(get("/assets/app.js").send().await);
}

#[tokio::test]
async fn missing_file() {
    assert_snapshot!(get("/missing.png").send().await);
}

#[tokio::test]
async fn legacy_docs() {
    assert_snapshot!(get("/docs").without_body().send().await);
}

#[tokio::test]
async fn legacy_openapi() {
    assert_snapshot!(get("/openapi.json").without_body().send().await);
}

#[tokio::test]
async fn metrics() {
    assert_snapshot!(get("/metrics").without_body().send().await);
}
