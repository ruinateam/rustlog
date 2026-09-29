use chrono::{DateTime, Utc};

use crate::{
    db::{schema::StructuredMessage, writer::FlushBuffer},
    domain::logs::LogsQuery,
};

#[derive(Debug)]
pub struct FlushBufferResponse {
    pub messages: Vec<StructuredMessage<'static>>,
    pub query: LogsQuery,
}

impl FlushBufferResponse {
    pub fn empty(query: LogsQuery) -> Self {
        Self {
            messages: vec![],
            query,
        }
    }

    pub async fn new(
        buffer: &FlushBuffer,
        channel_id: &str,
        user_id: Option<&str>,
        query: LogsQuery,
        (from, to): (DateTime<Utc>, DateTime<Utc>),
    ) -> Self {
        let timestamp_range = (from.timestamp_millis() as u64)..(to.timestamp_millis() as u64);

        let mut messages = if let Some(user_id) = user_id {
            buffer
                .messages_by_channel_and_user(timestamp_range, channel_id, user_id)
                .await
        } else {
            buffer
                .messages_by_channel(timestamp_range, channel_id)
                .await
        };

        if query.reverse {
            messages.reverse();
        }

        if let Some(offset) = query.offset {
            if offset as usize > messages.len() {
                messages.clear();
            } else {
                messages = messages.into_iter().skip(offset as usize).collect();
            }
        }

        Self { messages, query }
    }

    pub fn normalized_limit(&self) -> Option<u64> {
        let count = self.messages.len() as u64;
        let limit = self.query.limit;

        if self.query.reverse {
            limit.map(|limit| limit.saturating_sub(count))
        } else {
            limit
        }
    }

    pub fn normalized_offset(&self) -> Option<u64> {
        let count = self.len() as u64;
        let offset = self.query.offset;

        if self.query.reverse {
            offset.map(|offset| offset.saturating_sub(count))
        } else {
            offset
        }
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn is_at_start(&self) -> bool {
        self.query.reverse
    }
}
