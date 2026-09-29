//! API v2: admin endpoints and opt-out codes.

use crate::support::{get, in_sequence, request};
use axum::http::Method;
use insta::assert_snapshot;

const CHANNEL: &str = "/api/v2/admin/channels/44444";
const CHANNEL_OPT_OUT: &str = "/api/v2/admin/channels/11111/opt-out";
const USER_OPT_OUT: &str = "/api/v2/admin/users/22222/opt-out";

#[tokio::test]
async fn opt_out_without_key() {
    assert_snapshot!(request(Method::PUT, CHANNEL_OPT_OUT).send().await);
}

/// A channel that opts out is hidden, without Twitch even from the channel
/// list, and its logs come back when it opts back in.
#[tokio::test]
async fn channel_opt_out_hides_the_channel_until_it_opts_in() {
    assert_snapshot!(
        in_sequence([
            request(Method::PUT, CHANNEL_OPT_OUT).admin_key(),
            get("/api/v2/channels/11111/log-dates"),
            get("/api/v2/channels/11111/tiers/2026-03"),
            get("/api/v2/channels"),
            request(Method::DELETE, CHANNEL_OPT_OUT).admin_key(),
            get("/api/v2/channels/11111/log-dates"),
        ])
        .await
    );
}

/// The deletion of the user's messages runs in the background, so only
/// what does not depend on it is pinned after the opt-in.
#[tokio::test]
async fn user_opt_out_hides_the_user_until_they_opt_in() {
    assert_snapshot!(
        in_sequence([
            request(Method::PUT, USER_OPT_OUT).admin_key(),
            get("/api/v2/channels/11111/users/22222/log-months"),
            get("/api/v2/channels/11111/logs?from=2026-03-01T00:00:00Z&to=2026-03-02T00:00:00Z&format=text"),
            request(Method::DELETE, USER_OPT_OUT).admin_key(),
            get("/api/v2/channels/11111/users/22222/logs/search?q=nothing-matches"),
        ])
        .await
    );
}

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
