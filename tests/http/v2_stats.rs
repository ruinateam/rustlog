//! API v2: message counts. Without Twitch the logins are left out.

use crate::support::get;
use insta::assert_snapshot;

#[tokio::test]
async fn channel_stats() {
    assert_snapshot!(get("/api/v2/channels/11111/stats").send().await);
}

#[tokio::test]
async fn channel_stats_in_range() {
    assert_snapshot!(
        get("/api/v2/channels/11111/stats?from=2026-03-01T00:00:00Z&to=2026-03-02T00:00:00Z")
            .send()
            .await
    );
}

#[tokio::test]
async fn stats_with_half_a_range() {
    assert_snapshot!(
        get("/api/v2/channels/11111/stats?from=2026-03-01T00:00:00Z")
            .send()
            .await
    );
}

#[tokio::test]
async fn user_stats() {
    assert_snapshot!(get("/api/v2/channels/11111/users/22222/stats").send().await);
}
