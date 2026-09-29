//! `migrate`: import the raw IRC logs of a justlog instance.

use crate::support::TestDb;
use flate2::{Compression, write::GzEncoder};
use insta::assert_snapshot;
use rustlog_tools::migrate::{self, MigrateOptions};
use std::{fs, io::Write, path::Path};
use tempfile::TempDir;

/// 2026-03-01: a message, a CLEARCHAT without a user and a broken line.
const FIRST_DAY: &str = "\
@badge-info=;badges=subscriber/12;color=#FF0000;display-name=Alice;id=00000000-0000-4000-8000-000000000011;room-id=11111;tmi-sent-ts=1772348400000;user-id=22222 :alice!alice@alice.tmi.twitch.tv PRIVMSG #testchan :hello from justlog
@room-id=11111;tmi-sent-ts=1772348700000 :tmi.twitch.tv CLEARCHAT #testchan
this is not IRC
";

/// 2026-03-02, gzip compressed: one message.
const SECOND_DAY: &str = "\
@badge-info=;badges=;color=;display-name=Bob;id=00000000-0000-4000-8000-000000000012;room-id=11111;tmi-sent-ts=1772432100000;user-id=33333 :bob!bob@bob.tmi.twitch.tv PRIVMSG #testchan :good morning
";

/// Logs laid out as justlog stores them: `{channel id}/{year}/{month}/{day}/channel.txt[.gz]`.
fn justlog_logs() -> TempDir {
    let logs = TempDir::new().unwrap();
    write_day(logs.path(), "1", FIRST_DAY.as_bytes(), "channel.txt");

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(SECOND_DAY.as_bytes()).unwrap();
    write_day(
        logs.path(),
        "2",
        &encoder.finish().unwrap(),
        "channel.txt.gz",
    );

    logs
}

fn write_day(root: &Path, day: &str, contents: &[u8], file_name: &str) {
    let day_dir = root.join("11111/2026/3").join(day);
    fs::create_dir_all(&day_dir).unwrap();
    fs::write(day_dir.join(file_name), contents).unwrap();
}

#[tokio::test]
async fn imports_plain_and_compressed_days() {
    let test_db = TestDb::start().await;
    let logs = justlog_logs();

    migrate::run(
        test_db.db.clone(),
        MigrateOptions {
            source_dir: logs.path().to_owned(),
            channel_ids: Vec::new(),
            jobs: 2,
        },
    )
    .await
    .unwrap();

    assert_snapshot!(test_db.messages().await);
    test_db.stop().await;
}

#[tokio::test]
async fn only_the_requested_channels() {
    let test_db = TestDb::start().await;
    let logs = justlog_logs();

    migrate::run(
        test_db.db.clone(),
        MigrateOptions {
            source_dir: logs.path().to_owned(),
            channel_ids: vec!["99999".to_owned()],
            jobs: 1,
        },
    )
    .await
    .unwrap();

    assert_snapshot!(test_db.messages().await);
    test_db.stop().await;
}
