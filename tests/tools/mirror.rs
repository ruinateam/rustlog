//! `mirror`: import a channel from a remote instance or a local cache of one.

use crate::{
    first_day, second_day,
    support::{FakeRemote, TestDb, day_of_logs},
};
use insta::assert_snapshot;
use rustlog::{mirror, state::OperationalState};
use serde_json::json;
use std::{fs, path::Path, sync::Arc};
use tempfile::TempDir;

fn options(base_url: &str, local_cache: Option<&Path>) -> mirror::MirrorOptions {
    mirror::MirrorOptions {
        base_url: base_url.to_owned(),
        local_cache: local_cache.map(|path| path.display().to_string()),
        channel: "testchan".to_owned(),
        year: None,
        month: None,
        day: None,
        batch: 25_000,
        http_concurrency: 4,
        proxies: Vec::new(),
        rps: 1000.0,
    }
}

/// A cache laid out as `{channel}/daily/{year}/{month}/{day}.json`.
fn local_cache() -> TempDir {
    let cache = TempDir::new().unwrap();
    for (day, messages) in [("01", first_day()), ("02", second_day())] {
        let month_dir = cache.path().join("testchan/daily/2026/03");
        fs::create_dir_all(&month_dir).unwrap();
        fs::write(
            month_dir.join(format!("{day}.json")),
            day_of_logs(messages).to_string(),
        )
        .unwrap();
    }
    cache
}

#[tokio::test]
async fn from_local_cache() {
    let test_db = TestDb::start().await;
    let cache = local_cache();

    mirror::run(test_db.db.clone(), options("unused", Some(cache.path())))
        .await
        .unwrap();

    assert_snapshot!(test_db.messages().await);
    test_db.stop().await;
}

#[tokio::test]
async fn only_the_requested_day() {
    let test_db = TestDb::start().await;
    let cache = local_cache();

    let mut options = options("unused", Some(cache.path()));
    options.year = Some(2026);
    options.month = Some(3);
    options.day = Some(2);
    mirror::run(test_db.db.clone(), options).await.unwrap();

    assert_snapshot!(test_db.messages().await);
    test_db.stop().await;
}

#[tokio::test]
async fn second_run_adds_nothing() {
    let test_db = TestDb::start().await;
    let cache = local_cache();

    for _ in 0..2 {
        mirror::run(test_db.db.clone(), options("unused", Some(cache.path())))
            .await
            .unwrap();
    }

    assert_snapshot!(test_db.messages().await);
    test_db.stop().await;
}

#[tokio::test]
async fn skips_opted_out_users() {
    let test_db = TestDb::start().await;
    let state = OperationalState::load(Arc::new(test_db.db.clone()))
        .await
        .unwrap();
    state.optout_user("33333").await.unwrap();
    let cache = local_cache();

    mirror::run(test_db.db.clone(), options("unused", Some(cache.path())))
        .await
        .unwrap();

    assert_snapshot!(test_db.messages().await);
    test_db.stop().await;
}

/// Days the remote cannot serve are skipped after a retry.
#[tokio::test]
async fn from_remote_instance() {
    let test_db = TestDb::start().await;
    let remote = FakeRemote::start().await;
    remote.respond(
        "/list?channel=testchan",
        json!({ "availableLogs": [
            { "year": "2026", "month": "3", "day": "1" },
            { "year": "2026", "month": "3", "day": "2" },
            { "year": "2026", "month": "3" },
        ]}),
    );
    remote.respond(
        "/channel/testchan/2026/03/01?jsonBasic=1",
        day_of_logs(first_day()),
    );

    mirror::run(test_db.db.clone(), options(&remote.base_url, None))
        .await
        .unwrap();

    assert_snapshot!(test_db.messages().await);
    test_db.stop().await;
}
