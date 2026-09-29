//! Legacy chat tier tables.

use insta::assert_snapshot;

use crate::support::get;

#[tokio::test]
async fn day() {
    assert_snapshot!(get("/channelid/11111/tiers/2026/3/1").send().await);
}

#[tokio::test]
async fn month() {
    assert_snapshot!(get("/channelid/11111/tiers/2026/3").send().await);
}

#[tokio::test]
async fn year() {
    assert_snapshot!(get("/channelid/11111/tiers/2026").send().await);
}

#[tokio::test]
async fn month_excluding_bots() {
    assert_snapshot!(
        get("/channelid/11111/tiers/2026/3?exclude_bots=nightbot")
            .send()
            .await
    );
}
