//! Chat messages from the JSON API of a remote instance, converted to the
//! stored form.

use chrono::DateTime;
use rustlog_storage::message::{MessageFlags, MessageType, StructuredMessage};
use serde::Deserialize;
use std::{borrow::Cow, collections::HashMap};
use uuid::Uuid;

/// Older timestamps are bogus: 2020-01-01T00:00:00Z.
const MIN_VALID_TIMESTAMP_MS: i64 = 1_577_836_800_000;

/// Tags stored in their own columns or not at all; the others go to
/// `extra_tags`.
const COLUMN_TAGS: &[&str] = &[
    "user-id",
    "display-name",
    "color",
    "user-type",
    "badges",
    "badge-info",
    "client-nonce",
    "emotes",
    "flags",
    "room-id",
    "id",
    "tmi-sent-ts",
];

/// A message as `?jsonBasic` returns it.
#[derive(Deserialize)]
pub struct RemoteMessage {
    text: Option<String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    timestamp: Option<String>,
    id: Option<String>,
    #[serde(default)]
    tags: HashMap<String, String>,
}

/// Why a message is not stored.
#[derive(Debug, PartialEq, Eq)]
pub enum Skipped {
    /// Without an id the message cannot be told apart from stored copies.
    MissingId,
    /// No valid timestamp, channel or user.
    Invalid,
}

impl RemoteMessage {
    /// The message as a PRIVMSG of `channel_login`.
    pub fn into_structured(
        self,
        channel_login: &str,
    ) -> Result<StructuredMessage<'static>, Skipped> {
        let id = self.id().ok_or(Skipped::MissingId)?;
        let timestamp = self.timestamp_ms().ok_or(Skipped::Invalid)?;
        let mut tags = self.tags;
        let channel_id = tags.remove("room-id").ok_or(Skipped::Invalid)?;
        let user_id = tags
            .remove("user-id")
            .filter(|user_id| !user_id.is_empty())
            .ok_or(Skipped::Invalid)?;

        let user_login = self
            .display_name
            .as_deref()
            .unwrap_or_default()
            .to_lowercase();
        let display_name = tags
            .remove("display-name")
            .or(self.display_name)
            .unwrap_or_default();
        let color = tags.get("color").and_then(|color| {
            u32::from_str_radix(color.strip_prefix('#').unwrap_or(color), 16).ok()
        });
        let badges = tags
            .get("badges")
            .map(|badges| {
                badges
                    .split(',')
                    .filter(|badge| !badge.is_empty())
                    .map(|badge| Cow::Owned(badge.to_owned()))
                    .collect()
            })
            .unwrap_or_default();
        let mut take = |name: &str| Cow::Owned(tags.remove(name).unwrap_or_default());
        let user_type = take("user-type");
        let badge_info = take("badge-info");
        let client_nonce = take("client-nonce");
        let emotes = take("emotes");
        let automod_flags = take("flags");
        let extra_tags = tags
            .into_iter()
            .filter(|(name, _)| !COLUMN_TAGS.contains(&name.as_str()))
            .map(|(name, value)| (Cow::Owned(name), Cow::Owned(value)))
            .collect();

        Ok(StructuredMessage {
            channel_id: channel_id.into(),
            channel_login: channel_login.to_owned().into(),
            timestamp,
            id,
            message_type: MessageType::PrivMsg,
            user_id: user_id.into(),
            user_login: user_login.into(),
            display_name: display_name.into(),
            color,
            user_type,
            badges,
            badge_info,
            client_nonce,
            emotes,
            automod_flags,
            text: self.text.unwrap_or_default().into(),
            message_flags: MessageFlags::empty(),
            extra_tags,
        })
    }

    /// The `id` field, or else the `id` tag.
    fn id(&self) -> Option<Uuid> {
        let id = self
            .id
            .as_deref()
            .or_else(|| self.tags.get("id").map(String::as_str))?
            .trim();
        Uuid::parse_str(id).ok()
    }

    fn timestamp_ms(&self) -> Option<u64> {
        let timestamp = DateTime::parse_from_rfc3339(self.timestamp.as_deref()?).ok()?;
        let timestamp_ms = timestamp.timestamp_millis();
        (timestamp_ms >= MIN_VALID_TIMESTAMP_MS).then_some(timestamp_ms as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::{RemoteMessage, Skipped};
    use std::collections::HashMap;
    use uuid::Uuid;

    fn message() -> RemoteMessage {
        RemoteMessage {
            text: Some("hi".into()),
            display_name: Some("Alice".into()),
            timestamp: Some("2024-01-02T03:04:05.678Z".into()),
            id: Some("00000000-0000-4000-8000-000000000001".into()),
            tags: HashMap::from([
                ("room-id".into(), "123".into()),
                ("user-id".into(), "456".into()),
                ("display-name".into(), "Alice".into()),
            ]),
        }
    }

    #[test]
    fn requires_an_id() {
        let mut message = message();
        message.id = None;
        assert_eq!(
            message.into_structured("chan").unwrap_err(),
            Skipped::MissingId
        );
    }

    #[test]
    fn takes_the_id_from_the_tags() {
        let mut message = message();
        message.id = None;
        message
            .tags
            .insert("id".into(), "00000000-0000-4000-8000-000000000002".into());
        let stored = message.into_structured("chan").unwrap();
        assert_eq!(
            stored.id,
            Uuid::parse_str("00000000-0000-4000-8000-000000000002").unwrap()
        );
    }

    #[test]
    fn requires_a_user_id() {
        let mut message = message();
        message.tags.remove("user-id");
        assert_eq!(
            message.into_structured("chan").unwrap_err(),
            Skipped::Invalid
        );
    }

    #[test]
    fn keeps_unknown_tags() {
        let mut message = message();
        message
            .tags
            .insert("reply-parent-msg-id".into(), "x".into());
        message.tags.insert("tmi-sent-ts".into(), "1".into());
        let stored = message.into_structured("chan").unwrap();
        let extra_tags: Vec<_> = stored
            .extra_tags
            .iter()
            .map(|(name, value)| (name.as_ref(), value.as_ref()))
            .collect();
        assert_eq!(extra_tags, [("reply-parent-msg-id", "x")]);
    }
}
