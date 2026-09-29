/// One user's activity in a tier table: message counts, active windows per
/// window size, and the rank and tier within each window size where ranked.
#[derive(Debug, Clone)]
pub struct TierEntry {
    pub user_id: String,
    pub user_login: Option<String>,
    pub messages: u64,
    pub unique_messages: u64,
    pub windows_1m: u64,
    pub windows_5m: u64,
    pub windows_15m: u64,
    pub windows_30m: u64,
    pub windows_60m: u64,
    pub rank_1m: Option<u32>,
    pub tier_1m: Option<String>,
    pub rank_5m: Option<u32>,
    pub tier_5m: Option<String>,
    pub rank_15m: Option<u32>,
    pub tier_15m: Option<String>,
    pub rank_30m: Option<u32>,
    pub tier_30m: Option<String>,
    pub rank_60m: Option<u32>,
    pub tier_60m: Option<String>,
    pub tier_score: u32,
}
