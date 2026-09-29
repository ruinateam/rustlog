use chrono::{DateTime, Utc};

/// How many messages a user sent.
#[derive(Debug, Clone)]
pub struct UserMessageCount {
    pub user_id: String,
    pub user_login: Option<String>,
    pub message_count: u64,
}

/// A login a user id was seen with, and when.
#[derive(Debug, Clone)]
pub struct NameHistoryEntry {
    pub user_login: String,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}
