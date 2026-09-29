//! API v2.

use insta::assert_snapshot;

use crate::support::get;

const LOGS: &str = "/api/v2/channels/11111/users/22222/logs";
const RANGE: &str = "from=2026-03-01T00:00:00Z&to=2026-04-01T00:00:00Z";

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
async fn channel_availability() {
    assert_snapshot!(get("/api/v2/channels/11111/availability").send().await);
}

#[tokio::test]
async fn user_availability() {
    assert_snapshot!(
        get("/api/v2/channels/11111/availability?userId=22222")
            .send()
            .await
    );
}

#[tokio::test]
async fn logs() {
    assert_snapshot!(get(format!("{LOGS}?{RANGE}")).send().await);
}

#[tokio::test]
async fn logs_full_json() {
    assert_snapshot!(get(format!("{LOGS}?{RANGE}&format=full-json")).send().await);
}

#[tokio::test]
async fn logs_ndjson() {
    assert_snapshot!(get(format!("{LOGS}?{RANGE}&format=ndjson")).send().await);
}

#[tokio::test]
async fn logs_text() {
    assert_snapshot!(get(format!("{LOGS}?{RANGE}&format=text")).send().await);
}

#[tokio::test]
async fn logs_raw() {
    assert_snapshot!(get(format!("{LOGS}?{RANGE}&format=raw")).send().await);
}

#[tokio::test]
async fn logs_reverse_limit() {
    assert_snapshot!(
        get(format!("{LOGS}?{RANGE}&reverse=true&limit=1"))
            .send()
            .await
    );
}

#[tokio::test]
async fn logs_invalid_format() {
    assert_snapshot!(get(format!("{LOGS}?{RANGE}&format=bogus")).send().await);
}

#[tokio::test]
async fn logs_inverted_range() {
    assert_snapshot!(
        get(format!(
            "{LOGS}?from=2026-04-01T00:00:00Z&to=2026-03-01T00:00:00Z"
        ))
        .send()
        .await
    );
}

#[tokio::test]
async fn logs_missing_range() {
    assert_snapshot!(get(LOGS).send().await);
}

#[tokio::test]
async fn resolve_login_needs_twitch() {
    assert_snapshot!(get("/api/v2/users/resolve?login=testchan").send().await);
}

#[tokio::test]
async fn docs() {
    assert_snapshot!(get("/api/v2/docs").without_body().send().await);
}

#[tokio::test]
async fn openapi() {
    assert_snapshot!(get("/api/v2/openapi.json").without_body().send().await);
}
