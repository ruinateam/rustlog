//! Legacy log routes: availability, channel and user logs in every format,
//! search and random lines.

use insta::assert_snapshot;

use crate::support::get;

#[tokio::test]
async fn list_all_channels() {
    assert_snapshot!(get("/list").send().await);
}

#[tokio::test]
async fn list_channel() {
    assert_snapshot!(get("/list?channelid=11111").send().await);
}

#[tokio::test]
async fn list_channel_user() {
    assert_snapshot!(get("/list?channelid=11111&userid=22222").send().await);
}

#[tokio::test]
async fn channel_redirects_to_latest_day() {
    assert_snapshot!(get("/channelid/11111").send().await);
}

#[tokio::test]
async fn channel_day() {
    assert_snapshot!(get("/channelid/11111/2026/3/1").send().await);
}

#[tokio::test]
async fn channel_day_json() {
    assert_snapshot!(get("/channelid/11111/2026/3/1?json").send().await);
}

#[tokio::test]
async fn channel_day_json_basic() {
    assert_snapshot!(get("/channelid/11111/2026/3/1?jsonBasic").send().await);
}

#[tokio::test]
async fn channel_day_raw() {
    assert_snapshot!(get("/channelid/11111/2026/3/1?raw").send().await);
}

#[tokio::test]
async fn channel_day_ndjson() {
    assert_snapshot!(get("/channelid/11111/2026/3/1?ndjson").send().await);
}

#[tokio::test]
async fn channel_day_reverse_limit() {
    assert_snapshot!(
        get("/channelid/11111/2026/3/1?reverse&limit=1")
            .send()
            .await
    );
}

#[tokio::test]
async fn channel_day_offset() {
    assert_snapshot!(
        get("/channelid/11111/2026/3/1?limit=1&offset=1&json")
            .send()
            .await
    );
}

#[tokio::test]
async fn channel_range() {
    assert_snapshot!(
        get("/channelid/11111?from=2026-03-01T00:00:00Z&to=2026-03-02T12:00:00Z")
            .send()
            .await
    );
}

#[tokio::test]
async fn channel_range_json() {
    assert_snapshot!(
        get("/channelid/11111?from=2026-03-01T00:00:00Z&to=2026-03-02T12:00:00Z&json")
            .send()
            .await
    );
}

#[tokio::test]
async fn channel_range_invalid() {
    assert_snapshot!(
        get("/channelid/11111?from=garbage&to=2026-03-02T12:00:00Z")
            .send()
            .await
    );
}

#[tokio::test]
async fn channel_invalid_month() {
    assert_snapshot!(get("/channelid/11111/2026/13/1").send().await);
}

#[tokio::test]
async fn channel_unknown() {
    assert_snapshot!(get("/channelid/99999/2026/3/1").send().await);
}

#[tokio::test]
async fn channel_by_login_needs_twitch() {
    assert_snapshot!(get("/channel/testchan/2026/3/1").send().await);
}

#[tokio::test]
async fn user_redirects_to_latest_month() {
    assert_snapshot!(get("/channelid/11111/userid/22222").send().await);
}

#[tokio::test]
async fn user_month() {
    assert_snapshot!(get("/channelid/11111/userid/22222/2026/3").send().await);
}

#[tokio::test]
async fn user_month_json() {
    assert_snapshot!(
        get("/channelid/11111/userid/22222/2026/3?json")
            .send()
            .await
    );
}

#[tokio::test]
async fn user_month_raw() {
    assert_snapshot!(get("/channelid/11111/userid/22222/2026/3?raw").send().await);
}

#[tokio::test]
async fn user_by_login_needs_twitch() {
    assert_snapshot!(get("/channel/testchan/user/alice/2026/3").send().await);
}

#[tokio::test]
async fn user_search() {
    assert_snapshot!(
        get("/channelid/11111/userid/22222/search?q=hello")
            .send()
            .await
    );
}

#[tokio::test]
async fn user_search_json() {
    assert_snapshot!(
        get("/channelid/11111/userid/22222/search?q=hello&json")
            .send()
            .await
    );
}

#[tokio::test]
async fn channel_random() {
    assert_snapshot!(get("/channelid/11111/random").without_body().send().await);
}

#[tokio::test]
async fn user_random() {
    assert_snapshot!(
        get("/channelid/11111/userid/22222/random")
            .without_body()
            .send()
            .await
    );
}
