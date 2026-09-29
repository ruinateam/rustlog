//! Tests of the maintenance commands (`mirror`, `fill-missing`,
//! `cleanup-duplicate-ids`, `migrate`) against a real ClickHouse.
//!
//! Each test works on a fresh database and pins the stored messages. Remote
//! instances are faked by a local HTTP server, so nothing reaches the
//! network. Like the HTTP tests they need ClickHouse in UTC: run them with
//! `just test-integration`.

mod support;

mod duplicates;
mod fill_missing;
mod migrate;
mod mirror;

use support::{RemoteMessage, message_id};

/// 2026-03-01 in channel `11111` (`testchan`): two good messages, one with
/// its id only in the tags, and messages the import must skip: a repeated
/// id, a missing id, a timestamp before 2020 and a missing user id.
fn first_day() -> Vec<RemoteMessage> {
    vec![
        RemoteMessage::new("2026-03-01T07:00:00Z", "22222", "Alice", "hello")
            .id(&message_id(1))
            .tag("color", "#FF0000")
            .tag("badges", "subscriber/12,premium/1"),
        RemoteMessage::new("2026-03-01T08:30:00.5Z", "33333", "Bob", "hi alice").id(&message_id(2)),
        RemoteMessage::new("2026-03-01T08:31:00Z", "22222", "Alice", "hello").id(&message_id(1)),
        RemoteMessage::new("2026-03-01T09:00:00Z", "44444", "Carol", "no id"),
        RemoteMessage::new("2019-12-31T23:59:59Z", "22222", "Alice", "too old").id(&message_id(3)),
        RemoteMessage::new("2026-03-01T09:30:00Z", "22222", "Alice", "no user id")
            .id(&message_id(4))
            .without_tag("user-id"),
        RemoteMessage::new("2026-03-01T10:00:00Z", "33333", "Bob", "id in tags")
            .tag("id", &message_id(5)),
    ]
}

/// 2026-03-02 in channel `11111`: one message.
fn second_day() -> Vec<RemoteMessage> {
    vec![
        RemoteMessage::new("2026-03-02T06:15:00Z", "33333", "Bob", "good morning")
            .id(&message_id(6)),
    ]
}
