//! Legacy chat tier tables and the SullyGnome stream lists behind their
//! online and offline modes.

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

#[tokio::test]
async fn day_online() {
    assert_snapshot!(
        get("/channelid/11111/tiers/2026/3/1?mode=online")
            .send()
            .await
    );
}

#[tokio::test]
async fn day_offline() {
    assert_snapshot!(
        get("/channelid/11111/tiers/2026/3/1?mode=offline")
            .send()
            .await
    );
}

#[tokio::test]
async fn month_online() {
    assert_snapshot!(
        get("/channelid/11111/tiers/2026/3?mode=online")
            .send()
            .await
    );
}

#[tokio::test]
async fn month_offline() {
    assert_snapshot!(
        get("/channelid/11111/tiers/2026/3?mode=offline")
            .send()
            .await
    );
}

#[tokio::test]
async fn year_online() {
    assert_snapshot!(get("/channelid/11111/tiers/2026?mode=online").send().await);
}

/// No stream on 2026-03-02: nothing counts as online, everything as offline.
#[tokio::test]
async fn day_online_without_streams() {
    assert_snapshot!(
        get("/channelid/11111/tiers/2026/3/2?mode=online")
            .send()
            .await
    );
}

#[tokio::test]
async fn day_offline_without_streams() {
    assert_snapshot!(
        get("/channelid/11111/tiers/2026/3/2?mode=offline")
            .send()
            .await
    );
}

#[tokio::test]
async fn sully_streams_from_cache() {
    assert_snapshot!(get("/sully/11111/2026").send().await);
}

#[tokio::test]
async fn sully_streams_without_cache() {
    assert_snapshot!(get("/sully/99999/2026").send().await);
}

#[tokio::test]
async fn sully_streams_rejects_paths_outside_the_cache() {
    assert_snapshot!(get("/sully/..%2F..%2Fetc%2Fpasswd/2026").send().await);
}
