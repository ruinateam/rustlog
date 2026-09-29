use chrono::{DateTime, Utc};

/// Ordering and paging of a log query.
#[derive(Debug, Clone, Copy, Default)]
pub struct LogsQuery {
    /// Newest messages first.
    pub reverse: bool,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
}

/// A half-open time range `[from, to)`.
#[derive(Debug, Clone, Copy)]
pub struct TimeRange {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

/// A UTC day or month for which logs exist.
#[derive(Debug, Clone, Copy)]
pub struct LogDate {
    pub year: u16,
    pub month: u8,
    /// `None` when the date stands for a whole month.
    pub day: Option<u8>,
}
