//! API v2: admin endpoints and opt-out codes.

use crate::support::request;
use axum::http::Method;
use insta::assert_snapshot;

const CHANNEL: &str = "/api/v2/admin/channels/44444";

#[tokio::test]
async fn join_without_key() {
    assert_snapshot!(request(Method::PUT, CHANNEL).send().await);
}

#[tokio::test]
async fn join_with_wrong_key() {
    assert_snapshot!(
        request(Method::PUT, CHANNEL)
            .header("x-api-key", "wrong")
            .send()
            .await
    );
}

#[tokio::test]
async fn join_needs_twitch() {
    assert_snapshot!(request(Method::PUT, CHANNEL).admin_key().send().await);
}

#[tokio::test]
async fn leave_without_key() {
    assert_snapshot!(request(Method::DELETE, CHANNEL).send().await);
}

#[tokio::test]
async fn leave_needs_twitch() {
    assert_snapshot!(request(Method::DELETE, CHANNEL).admin_key().send().await);
}

#[tokio::test]
async fn join_with_invalid_id() {
    assert_snapshot!(
        request(Method::PUT, "/api/v2/admin/channels/xqc")
            .admin_key()
            .send()
            .await
    );
}

#[tokio::test]
async fn opt_out_code() {
    assert_snapshot!(
        request(Method::POST, "/api/v2/opt-out-codes")
            .without_body()
            .send()
            .await
    );
}
