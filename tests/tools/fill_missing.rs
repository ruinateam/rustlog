//! `fill-missing`: find the days logs.zonian.dev knows about and import the
//! missing (or, with `--repair-existing`, incomplete) ones from mirrors.

use crate::{
    first_day,
    mirror::transfer_options,
    second_day,
    support::{FakeRemote, TestDb, day_of_logs},
};
use insta::assert_snapshot;
use rustlog::tools::fill_missing::{self, FillMissingOptions};
use serde_json::json;

/// Only the first message of 2026-03-01 is stored locally.
const INCOMPLETE_FIRST_DAY: &str = "
INSERT INTO message_structured
    (channel_id, channel_login, timestamp, id, message_type, user_id, user_login, display_name, text)
VALUES
    ('11111', 'testchan', toDateTime64('2026-03-01 07:00:00.000', 3, 'UTC'), '00000000-0000-4000-8000-000000000001', 1, '22222', 'alice', 'Alice', 'hello')
";

fn options(api_base: &str) -> FillMissingOptions {
    FillMissingOptions {
        channels: vec!["testchan".to_owned()],
        year: 2026,
        api_base: api_base.to_owned(),
        transfer: transfer_options(),
        exclude_instances: Vec::new(),
        dry_run: false,
        repair_existing: false,
        deep: false,
    }
}

/// A fake that is both the zonian API, listing `days` of 2026-03, and the
/// only mirror, serving the first two days.
async fn remote_with_days(days: &[&str]) -> FakeRemote {
    let remote = FakeRemote::start().await;
    let list: Vec<_> = days
        .iter()
        .map(|day| json!({ "year": "2026", "month": "3", "day": day }))
        .collect();
    remote.respond(
        "/api/testchan",
        json!({
            "loggedData": { "list": list },
            "channelLogs": { "instances": [format!("{}/", remote.base_url)] },
        }),
    );
    remote.respond(
        "/channel/testchan/2026/03/01?jsonBasic=1",
        day_of_logs(first_day()),
    );
    remote.respond(
        "/channel/testchan/2026/03/02?jsonBasic=1",
        day_of_logs(second_day()),
    );
    remote
}

#[tokio::test]
async fn imports_missing_days_only() {
    let test_db = TestDb::start().await;
    test_db.execute(INCOMPLETE_FIRST_DAY).await;
    let remote = remote_with_days(&["1", "2"]).await;

    fill_missing::run(test_db.db.clone(), options(&remote.base_url))
        .await
        .unwrap();

    assert_snapshot!(test_db.messages().await);
    test_db.stop().await;
}

#[tokio::test]
async fn repairs_incomplete_days() {
    let test_db = TestDb::start().await;
    test_db.execute(INCOMPLETE_FIRST_DAY).await;
    let remote = remote_with_days(&["1", "2"]).await;

    let mut options = options(&remote.base_url);
    options.repair_existing = true;
    fill_missing::run(test_db.db.clone(), options)
        .await
        .unwrap();

    assert_snapshot!(test_db.messages().await);
    test_db.stop().await;
}

#[tokio::test]
async fn dry_run_imports_nothing() {
    let test_db = TestDb::start().await;
    test_db.execute(INCOMPLETE_FIRST_DAY).await;
    let remote = remote_with_days(&["1", "2"]).await;

    let mut options = options(&remote.base_url);
    options.dry_run = true;
    options.repair_existing = true;
    fill_missing::run(test_db.db.clone(), options)
        .await
        .unwrap();

    assert_snapshot!(test_db.messages().await);
    test_db.stop().await;
}

#[tokio::test]
async fn fails_when_no_mirror_has_a_day() {
    let test_db = TestDb::start().await;
    let remote = remote_with_days(&["2", "3"]).await;

    let error = fill_missing::run(test_db.db.clone(), options(&remote.base_url))
        .await
        .unwrap_err();

    assert_snapshot!(format!("{error:#}\n---\n{}", test_db.messages().await));
    test_db.stop().await;
}
