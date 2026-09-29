//! `cleanup-duplicate-ids`: report and remove rows that repeat a message id.

use crate::support::TestDb;
use insta::assert_snapshot;
use rustlog_tools::duplicates::{self, CleanupDuplicateIdsOptions};

/// In `testchan`: id 1 three times, id 2 twice, id 3 once and the nil id,
/// which stands for "no id", twice. In `otherchan`: id 7 twice.
const DUPLICATES: &str = "
INSERT INTO message_structured
    (channel_id, channel_login, timestamp, id, message_type, user_id, user_login, display_name, text)
VALUES
    ('11111', 'testchan', toDateTime64('2026-03-01 07:00:00.000', 3, 'UTC'), '00000000-0000-4000-8000-000000000001', 1, '22222', 'alice', 'Alice', 'first copy'),
    ('11111', 'testchan', toDateTime64('2026-03-01 07:00:01.000', 3, 'UTC'), '00000000-0000-4000-8000-000000000001', 1, '22222', 'alice', 'Alice', 'second copy'),
    ('11111', 'testchan', toDateTime64('2026-04-01 07:00:02.000', 3, 'UTC'), '00000000-0000-4000-8000-000000000001', 1, '22222', 'alice', 'Alice', 'third copy, next month'),
    ('11111', 'testchan', toDateTime64('2026-03-01 08:00:00.000', 3, 'UTC'), '00000000-0000-4000-8000-000000000002', 1, '33333', 'bob', 'Bob', 'bob first'),
    ('11111', 'testchan', toDateTime64('2026-03-01 08:00:05.000', 3, 'UTC'), '00000000-0000-4000-8000-000000000002', 1, '33333', 'bob', 'Bob', 'bob second'),
    ('11111', 'testchan', toDateTime64('2026-03-01 09:00:00.000', 3, 'UTC'), '00000000-0000-4000-8000-000000000003', 1, '33333', 'bob', 'Bob', 'unique'),
    ('11111', 'testchan', toDateTime64('2026-03-01 10:00:00.000', 3, 'UTC'), '00000000-0000-0000-0000-000000000000', 2, '', '', '', 'no id'),
    ('11111', 'testchan', toDateTime64('2026-03-01 10:00:01.000', 3, 'UTC'), '00000000-0000-0000-0000-000000000000', 2, '', '', '', 'no id either'),
    ('55555', 'otherchan', toDateTime64('2026-03-01 11:00:00.000', 3, 'UTC'), '00000000-0000-4000-8000-000000000007', 1, '22222', 'alice', 'Alice', 'other first'),
    ('55555', 'otherchan', toDateTime64('2026-03-01 11:00:01.000', 3, 'UTC'), '00000000-0000-4000-8000-000000000007', 1, '22222', 'alice', 'Alice', 'other second')
";

fn options(execute: bool) -> CleanupDuplicateIdsOptions {
    CleanupDuplicateIdsOptions {
        channels: vec!["testchan".to_owned()],
        year: Some(2026),
        execute,
        sample_limit: 30,
        wait_timeout: 600,
    }
}

#[tokio::test]
async fn dry_run_changes_nothing() {
    let test_db = TestDb::start().await;
    test_db.execute(DUPLICATES).await;

    duplicates::run(test_db.db.clone(), options(false))
        .await
        .unwrap();

    assert_snapshot!(test_db.messages().await);
    test_db.stop().await;
}

/// Keeps the earliest copy of each id, only in the requested channel, and
/// cleans up its work tables.
#[tokio::test]
async fn execute_keeps_one_row_per_id() {
    let test_db = TestDb::start().await;
    test_db.execute(DUPLICATES).await;
    let tables_before = test_db.tables().await;

    duplicates::run(test_db.db.clone(), options(true))
        .await
        .unwrap();

    assert_eq!(test_db.tables().await, tables_before);
    assert_snapshot!(test_db.messages().await);
    test_db.stop().await;
}
