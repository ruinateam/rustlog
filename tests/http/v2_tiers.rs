//! API v2: tier tables and streams. Without Twitch the tables fall back to
//! the channel id, by which the SullyGnome cache of the fixture knows it.

use crate::support::get;
use insta::assert_snapshot;

#[tokio::test]
async fn day() {
    assert_snapshot!(get("/api/v2/channels/11111/tiers/2026-03-01").send().await);
}

#[tokio::test]
async fn month() {
    assert_snapshot!(get("/api/v2/channels/11111/tiers/2026-03").send().await);
}

#[tokio::test]
async fn year() {
    assert_snapshot!(get("/api/v2/channels/11111/tiers/2026").send().await);
}

#[tokio::test]
async fn online_day() {
    assert_snapshot!(
        get("/api/v2/channels/11111/tiers/2026-03-01?mode=online")
            .send()
            .await
    );
}

#[tokio::test]
async fn offline_day() {
    assert_snapshot!(
        get("/api/v2/channels/11111/tiers/2026-03-01?mode=offline")
            .send()
            .await
    );
}

/// Bots are matched by login, which needs Twitch: without it nobody is left
/// out, but the parameter is accepted.
#[tokio::test]
async fn excluded_bots_need_logins() {
    assert_snapshot!(
        get("/api/v2/channels/11111/tiers/2026-03?excludeBots=bob&excludeBots=nightbot")
            .send()
            .await
    );
}

#[tokio::test]
async fn invalid_period() {
    assert_snapshot!(get("/api/v2/channels/11111/tiers/2026-02-30").send().await);
}

#[tokio::test]
async fn invalid_mode() {
    assert_snapshot!(
        get("/api/v2/channels/11111/tiers/2026-03?mode=sometimes")
            .send()
            .await
    );
}

#[tokio::test]
async fn streams_need_twitch() {
    assert_snapshot!(get("/api/v2/channels/11111/streams?year=2026").send().await);
}
