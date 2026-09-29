//! Legacy channel listing, chat badges, capabilities and opt-out.

use axum::http::Method;
use insta::assert_snapshot;

use crate::support::{get, request};

#[tokio::test]
async fn capabilities() {
    assert_snapshot!(get("/capabilities").send().await);
}

#[tokio::test]
async fn channels_need_twitch() {
    assert_snapshot!(get("/channels").send().await);
}

#[tokio::test]
async fn channels_trailing_slash() {
    assert_snapshot!(get("/channels/").send().await);
}

#[tokio::test]
async fn channels_wrong_method() {
    assert_snapshot!(request(Method::PUT, "/channels").send().await);
}

#[tokio::test]
async fn badges_need_twitch() {
    assert_snapshot!(get("/badges/11111").send().await);
}

#[tokio::test]
async fn badges_of_unlogged_channel() {
    assert_snapshot!(get("/badges/99999").send().await);
}

#[tokio::test]
async fn optout_code() {
    assert_snapshot!(request(Method::POST, "/optout").without_body().send().await);
}
