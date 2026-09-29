//! Rows of the message tables.

use bitflags::bitflags;
use clickhouse::Row;
use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};
use std::borrow::Cow;
use strum::{Display, EnumString};
use uuid::Uuid;

pub const MESSAGES_STRUCTURED_TABLE: &str = "message_structured";

mod datetime64_millis_u64 {
    use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(timestamp: &u64, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        i64::try_from(*timestamp)
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u64, D::Error>
    where
        D: Deserializer<'de>,
    {
        let timestamp = i64::deserialize(deserializer)?;
        u64::try_from(timestamp).map_err(D::Error::custom)
    }
}

bitflags! {
    #[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Default, Clone, Copy)]
    #[serde(transparent)]
    pub struct MessageFlags: u16 {
        const SUBSCRIBER        = 1;
        const VIP               = 2;
        const MOD               = 4;
        const TURBO             = 8;
        const FIRST_MSG         = 16;
        const RETURNING_CHATTER = 32;
        const EMOTE_ONLY        = 64;
        const R9K               = 128;
        const SUBS_ONLY         = 256;
        const SLOW_MODE         = 512;
    }
}

#[derive(Row, Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct StructuredMessage<'a> {
    pub channel_id: Cow<'a, str>,
    pub channel_login: Cow<'a, str>,
    #[serde(with = "datetime64_millis_u64")]
    pub timestamp: u64,
    #[serde(with = "clickhouse::serde::uuid")]
    pub id: Uuid,
    pub message_type: MessageType,
    pub user_id: Cow<'a, str>,
    pub user_login: Cow<'a, str>,
    pub display_name: Cow<'a, str>,
    pub color: Option<u32>,
    pub user_type: Cow<'a, str>,
    pub badges: Vec<Cow<'a, str>>,
    pub badge_info: Cow<'a, str>,
    pub client_nonce: Cow<'a, str>,
    pub emotes: Cow<'a, str>,
    pub automod_flags: Cow<'a, str>,
    pub text: Cow<'a, str>,
    pub message_flags: MessageFlags,
    pub extra_tags: Vec<(Cow<'a, str>, Cow<'a, str>)>,
}

#[derive(Row, Serialize, Deserialize, Debug)]
pub struct UnstructuredMessage<'a> {
    pub channel_id: &'a str,
    pub user_id: &'a str,
    #[serde(with = "datetime64_millis_u64")]
    pub timestamp: u64,
    pub raw: &'a str,
}

impl<'a> StructuredMessage<'a> {
    pub fn id(&self) -> Option<String> {
        if self.id.is_nil() {
            None
        } else {
            Some(self.id.to_string())
        }
    }

    pub fn display_name(&self) -> &str {
        if !self.display_name.is_empty() {
            &self.display_name
        } else {
            &self.user_login
        }
    }

    pub fn into_owned(self) -> StructuredMessage<'static> {
        StructuredMessage {
            channel_id: Cow::Owned(self.channel_id.into_owned()),
            channel_login: Cow::Owned(self.channel_login.into_owned()),
            timestamp: self.timestamp,
            id: self.id,
            message_type: self.message_type,
            user_id: Cow::Owned(self.user_id.into_owned()),
            user_login: Cow::Owned(self.user_login.into_owned()),
            display_name: Cow::Owned(self.display_name.into_owned()),
            color: self.color,
            user_type: Cow::Owned(self.user_type.into_owned()),
            badges: self
                .badges
                .into_iter()
                .map(|value| Cow::Owned(value.into_owned()))
                .collect(),
            badge_info: Cow::Owned(self.badge_info.into_owned()),
            client_nonce: Cow::Owned(self.client_nonce.into_owned()),
            emotes: Cow::Owned(self.emotes.into_owned()),
            automod_flags: Cow::Owned(self.automod_flags.into_owned()),
            text: Cow::Owned(self.text.into_owned()),
            message_flags: self.message_flags,
            extra_tags: self
                .extra_tags
                .into_iter()
                .map(|(k, v)| (Cow::Owned(k.into_owned()), Cow::Owned(v.into_owned())))
                .collect(),
        }
    }
}

#[derive(Serialize_repr, Deserialize_repr, EnumString, Debug, PartialEq, Display, Clone, Copy)]
#[repr(u8)]
#[strum(serialize_all = "UPPERCASE")]
pub enum MessageType {
    Whisper = 0,
    PrivMsg = 1,
    ClearChat = 2,
    RoomState = 3,
    UserNotice = 4,
    UserState = 5,
    Notice = 6,
    Join = 7,
    Part = 8,
    Reconnect = 9,
    Names = 10,
    Ping = 11,
    Pong = 12,
    ClearMsg = 13,
    GlobalUserState = 14,
}
