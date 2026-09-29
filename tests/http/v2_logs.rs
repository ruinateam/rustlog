//! API v2: log dates, messages, random messages and search.

use crate::support::get;
use insta::assert_snapshot;

const CHANNEL_LOGS: &str = "/api/v2/channels/11111/logs";
const USER_LOGS: &str = "/api/v2/channels/11111/users/22222/logs";
const MARCH: &str = "from=2026-03-01T00:00:00Z&to=2026-04-01T00:00:00Z";

#[tokio::test]
async fn channel_log_dates() {
    assert_snapshot!(get("/api/v2/channels/11111/log-dates").send().await);
}

#[tokio::test]
async fn log_dates_of_unknown_channel() {
    assert_snapshot!(get("/api/v2/channels/99999/log-dates").send().await);
}

#[tokio::test]
async fn user_log_months() {
    assert_snapshot!(
        get("/api/v2/channels/11111/users/22222/log-months")
            .send()
            .await
    );
}

#[tokio::test]
async fn channel_logs() {
    assert_snapshot!(get(format!("{CHANNEL_LOGS}?{MARCH}")).send().await);
}

#[tokio::test]
async fn channel_logs_text() {
    assert_snapshot!(
        get(format!("{CHANNEL_LOGS}?{MARCH}&format=text"))
            .send()
            .await
    );
}

#[tokio::test]
async fn user_logs() {
    assert_snapshot!(get(format!("{USER_LOGS}?{MARCH}")).send().await);
}

#[tokio::test]
async fn user_logs_full_json() {
    assert_snapshot!(
        get(format!("{USER_LOGS}?{MARCH}&format=full-json"))
            .send()
            .await
    );
}

#[tokio::test]
async fn user_logs_ndjson() {
    assert_snapshot!(
        get(format!("{USER_LOGS}?{MARCH}&format=ndjson"))
            .send()
            .await
    );
}

#[tokio::test]
async fn user_logs_raw() {
    assert_snapshot!(get(format!("{USER_LOGS}?{MARCH}&format=raw")).send().await);
}

#[tokio::test]
async fn user_logs_reverse_limit_offset() {
    assert_snapshot!(
        get(format!("{USER_LOGS}?{MARCH}&reverse=true&limit=1&offset=1"))
            .send()
            .await
    );
}

#[tokio::test]
async fn logs_invalid_format() {
    assert_snapshot!(
        get(format!("{USER_LOGS}?{MARCH}&format=bogus"))
            .send()
            .await
    );
}

#[tokio::test]
async fn logs_zero_limit() {
    assert_snapshot!(get(format!("{USER_LOGS}?{MARCH}&limit=0")).send().await);
}

#[tokio::test]
async fn logs_inverted_range() {
    assert_snapshot!(
        get(format!(
            "{USER_LOGS}?from=2026-04-01T00:00:00Z&to=2026-03-01T00:00:00Z"
        ))
        .send()
        .await
    );
}

#[tokio::test]
async fn logs_missing_range() {
    assert_snapshot!(get(USER_LOGS).send().await);
}

#[tokio::test]
async fn random_channel_message() {
    assert_snapshot!(
        get("/api/v2/channels/11111/logs/random")
            .without_body()
            .send()
            .await
    );
}

#[tokio::test]
async fn random_user_message_of_one() {
    assert_snapshot!(
        get("/api/v2/channels/11111/users/33333/logs/random?format=text")
            .without_body()
            .send()
            .await
    );
}

#[tokio::test]
async fn random_message_of_silent_user() {
    assert_snapshot!(
        get("/api/v2/channels/11111/users/99999/logs/random")
            .send()
            .await
    );
}

#[tokio::test]
async fn search() {
    assert_snapshot!(
        get("/api/v2/channels/11111/users/22222/logs/search?q=HELLO")
            .send()
            .await
    );
}

#[tokio::test]
async fn search_without_text() {
    assert_snapshot!(
        get("/api/v2/channels/11111/users/22222/logs/search?q=%20")
            .send()
            .await
    );
}

#[tokio::test]
async fn search_without_matches() {
    assert_snapshot!(
        get("/api/v2/channels/11111/users/22222/logs/search?q=nothing-matches")
            .send()
            .await
    );
}

#[tokio::test]
async fn empty_range_in_every_format() {
    let empty = "from=2020-01-01T00:00:00Z&to=2020-01-02T00:00:00Z";
    for format in ["basic-json", "full-json", "ndjson", "text", "raw"] {
        assert_snapshot!(
            format!("empty_{format}"),
            get(format!("{CHANNEL_LOGS}?{empty}&format={format}"))
                .send()
                .await
        );
    }
}
