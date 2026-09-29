//! Chat tier tables: users ranked by how many fixed time windows they were
//! active in, per window size.

use chrono::{Datelike, NaiveDate};
use std::collections::{HashMap, HashSet};

pub const RESPONSE_LIMIT: usize = 500;
pub const TIMEZONE: &str = "Europe/Moscow";

/// Bot logins left out of tier tables unless a request says otherwise.
pub const DEFAULT_EXCLUDED_BOTS: &[&str] = &[
    "twirapp",
    "streamelements",
    "nightbot",
    "moobot",
    "mejkizbot",
    "supibot",
    "potatbotat",
    "fossabot",
];

/// Which messages count towards a tier table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TierMode {
    All,
    /// Only messages sent while the stream was live.
    Online,
    /// Only messages sent while the stream was offline.
    Offline,
}

impl TierMode {
    pub fn as_str(self) -> &'static str {
        match self {
            TierMode::All => "all",
            TierMode::Online => "online",
            TierMode::Offline => "offline",
        }
    }
}

/// A calendar period in [`TIMEZONE`].
#[derive(Debug, Clone, Copy)]
pub enum TierPeriod {
    Day(NaiveDate),
    Month { year: i32, month: u32 },
    Year { year: i32 },
}

impl TierPeriod {
    pub fn year(self) -> i32 {
        match self {
            TierPeriod::Day(date) => date.year(),
            TierPeriod::Month { year, .. } | TierPeriod::Year { year } => year,
        }
    }

    /// `day`, `month` or `year`.
    pub fn scope(self) -> &'static str {
        match self {
            TierPeriod::Day(_) => "day",
            TierPeriod::Month { .. } => "month",
            TierPeriod::Year { .. } => "year",
        }
    }

    /// `YYYYMMDD`, `YYYYMM` or `YYYY`.
    pub fn key(self) -> String {
        match self {
            TierPeriod::Day(date) => date.format("%Y%m%d").to_string(),
            TierPeriod::Month { year, month } => format!("{year:04}{month:02}"),
            TierPeriod::Year { year } => format!("{year:04}"),
        }
    }
}

/// A user's message counts and the number of active windows per window size.
#[derive(Debug, Clone)]
pub struct UserWindows {
    pub user_id: String,
    pub messages: u64,
    pub unique_messages: u64,
    pub windows_1m: u64,
    pub windows_5m: u64,
    pub windows_15m: u64,
    pub windows_30m: u64,
    pub windows_60m: u64,
}

impl UserWindows {
    /// Adds another period's counts of the same user.
    pub fn add(&mut self, other: &UserWindows) {
        self.messages += other.messages;
        self.unique_messages += other.unique_messages;
        self.windows_1m += other.windows_1m;
        self.windows_5m += other.windows_5m;
        self.windows_15m += other.windows_15m;
        self.windows_30m += other.windows_30m;
        self.windows_60m += other.windows_60m;
    }
}

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

pub struct RankedTiers {
    pub total_users: u64,
    pub total_messages: u64,
    pub total_unique_messages: u64,
    pub entries: Vec<TierEntry>,
}

pub fn filter_bots(
    rows: &mut HashMap<String, UserWindows>,
    user_logins: &HashMap<String, String>,
    excluded: &HashSet<String>,
) {
    if excluded.is_empty() {
        return;
    }

    rows.retain(|user_id, _| {
        !excluded.contains(&user_id.to_lowercase())
            && !user_logins
                .get(user_id)
                .is_some_and(|login| excluded.contains(&login.to_lowercase()))
    });
}

pub fn rank(
    rows_by_user: HashMap<String, UserWindows>,
    user_logins: HashMap<String, String>,
) -> RankedTiers {
    let total_users = rows_by_user.len() as u64;
    let total_messages = rows_by_user.values().map(|row| row.messages).sum();
    let total_unique_messages = rows_by_user.values().map(|row| row.unique_messages).sum();

    let ranks_1m = rank_window(&rows_by_user, |row| row.windows_1m);
    let ranks_5m = rank_window(&rows_by_user, |row| row.windows_5m);
    let ranks_15m = rank_window(&rows_by_user, |row| row.windows_15m);
    let ranks_30m = rank_window(&rows_by_user, |row| row.windows_30m);
    let ranks_60m = rank_window(&rows_by_user, |row| row.windows_60m);

    let mut included_ids: HashSet<String> = ranks_1m.keys().cloned().collect();
    included_ids.extend(ranks_5m.keys().cloned());
    included_ids.extend(ranks_15m.keys().cloned());
    included_ids.extend(ranks_30m.keys().cloned());
    included_ids.extend(ranks_60m.keys().cloned());

    let mut entries = included_ids
        .into_iter()
        .filter_map(|user_id| {
            let row = rows_by_user.get(&user_id)?;
            let ranked_1m = ranks_1m.get(&user_id);
            let ranked_5m = ranks_5m.get(&user_id);
            let ranked_15m = ranks_15m.get(&user_id);
            let ranked_30m = ranks_30m.get(&user_id);
            let ranked_60m = ranks_60m.get(&user_id);

            Some(TierEntry {
                user_login: user_logins.get(&user_id).cloned(),
                user_id,
                messages: row.messages,
                unique_messages: row.unique_messages,
                windows_1m: row.windows_1m,
                windows_5m: row.windows_5m,
                windows_15m: row.windows_15m,
                windows_30m: row.windows_30m,
                windows_60m: row.windows_60m,
                rank_1m: ranked_1m.map(|rank| rank.position),
                tier_1m: ranked_1m.map(|rank| rank.tier.to_owned()),
                rank_5m: ranked_5m.map(|rank| rank.position),
                tier_5m: ranked_5m.map(|rank| rank.tier.to_owned()),
                rank_15m: ranked_15m.map(|rank| rank.position),
                tier_15m: ranked_15m.map(|rank| rank.tier.to_owned()),
                rank_30m: ranked_30m.map(|rank| rank.position),
                tier_30m: ranked_30m.map(|rank| rank.tier.to_owned()),
                rank_60m: ranked_60m.map(|rank| rank.position),
                tier_60m: ranked_60m.map(|rank| rank.tier.to_owned()),
                tier_score: score([ranked_1m, ranked_5m, ranked_15m, ranked_30m, ranked_60m]),
            })
        })
        .collect::<Vec<_>>();

    entries.sort_by(compare_entries);
    entries.truncate(RESPONSE_LIMIT);

    RankedTiers {
        total_users,
        total_messages,
        total_unique_messages,
        entries,
    }
}

/// A user's place among all users for one window size.
struct WindowRank {
    /// 1-based.
    position: u32,
    tier: &'static str,
}

fn rank_window(
    rows_by_user: &HashMap<String, UserWindows>,
    value: impl Fn(&UserWindows) -> u64,
) -> HashMap<String, WindowRank> {
    let mut rows = rows_by_user.values().collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        value(right)
            .cmp(&value(left))
            .then_with(|| left.user_id.cmp(&right.user_id))
    });

    rows.into_iter()
        .enumerate()
        .filter_map(|(index, row)| {
            let position = (index + 1) as u32;
            tier_for_position(position)
                .map(|tier| (row.user_id.clone(), WindowRank { position, tier }))
        })
        .collect()
}

fn tier_for_position(position: u32) -> Option<&'static str> {
    match position {
        1 => Some("HT1"),
        2..=4 => Some("LT1"),
        5..=10 => Some("HT2"),
        11..=20 => Some("LT2"),
        21..=40 => Some("HT3"),
        41..=80 => Some("LT3"),
        81..=140 => Some("HT4"),
        141..=200 => Some("LT4"),
        _ => Some("LT5"),
    }
}

fn tier_value(tier: &str) -> u8 {
    match tier {
        "HT1" => 10,
        "LT1" => 9,
        "HT2" => 8,
        "LT2" => 7,
        "HT3" => 6,
        "LT3" => 5,
        "HT4" => 4,
        "LT4" => 3,
        "LT5" => 1,
        _ => 0,
    }
}

fn score(ranks: [Option<&WindowRank>; 5]) -> u32 {
    ranks
        .into_iter()
        .flatten()
        .map(|rank| u32::from(tier_value(rank.tier)))
        .sum()
}

fn compare_entries(left: &TierEntry, right: &TierEntry) -> std::cmp::Ordering {
    right
        .tier_score
        .cmp(&left.tier_score)
        .then_with(|| right.messages.cmp(&left.messages))
        .then_with(|| right.unique_messages.cmp(&left.unique_messages))
        .then_with(|| right.windows_1m.cmp(&left.windows_1m))
        .then_with(|| right.windows_5m.cmp(&left.windows_5m))
        .then_with(|| right.windows_15m.cmp(&left.windows_15m))
        .then_with(|| right.windows_30m.cmp(&left.windows_30m))
        .then_with(|| right.windows_60m.cmp(&left.windows_60m))
        .then_with(|| left.user_id.cmp(&right.user_id))
}

#[cfg(test)]
mod tests {
    use super::UserWindows;
    use super::{filter_bots, rank};
    use std::collections::{HashMap, HashSet};

    fn row(user_id: &str, windows: u64) -> UserWindows {
        UserWindows {
            user_id: user_id.to_owned(),
            messages: windows,
            unique_messages: windows,
            windows_1m: windows,
            windows_5m: windows,
            windows_15m: windows,
            windows_30m: windows,
            windows_60m: windows,
        }
    }

    #[test]
    fn ranks_equal_values_by_user_id() {
        let rows = HashMap::from([
            ("zulu".to_owned(), row("zulu", 10)),
            ("alpha".to_owned(), row("alpha", 10)),
        ]);

        let ranked = rank(rows, HashMap::new());

        assert_eq!(ranked.entries[0].user_id, "alpha");
        assert_eq!(ranked.entries[0].rank_1m, Some(1));
        assert_eq!(ranked.entries[0].tier_1m.as_deref(), Some("HT1"));
        assert_eq!(ranked.entries[1].user_id, "zulu");
        assert_eq!(ranked.entries[1].rank_1m, Some(2));
        assert_eq!(ranked.entries[1].tier_1m.as_deref(), Some("LT1"));
    }

    #[test]
    fn preserves_tier_boundaries_and_response_limit() {
        let rows = (1..=501)
            .map(|position| {
                let id = format!("{position:04}");
                (id.clone(), row(&id, 1_000 - position))
            })
            .collect();

        let ranked = rank(rows, HashMap::new());

        assert_eq!(ranked.total_users, 501);
        assert_eq!(ranked.entries.len(), 500);
        assert_eq!(ranked.entries[0].tier_1m.as_deref(), Some("HT1"));
        assert_eq!(ranked.entries[4].tier_1m.as_deref(), Some("HT2"));
        assert_eq!(ranked.entries[140].tier_1m.as_deref(), Some("LT4"));
        assert_eq!(ranked.entries[200].tier_1m.as_deref(), Some("LT5"));
    }

    #[test]
    fn excludes_ids_and_logins_case_insensitively() {
        let mut rows = HashMap::from([
            ("bot-id".to_owned(), row("bot-id", 5)),
            ("person-id".to_owned(), row("person-id", 4)),
            ("other-id".to_owned(), row("other-id", 3)),
        ]);
        let logins = HashMap::from([
            ("person-id".to_owned(), "NightBot".to_owned()),
            ("other-id".to_owned(), "person".to_owned()),
        ]);
        let excluded = HashSet::from(["bot-id".to_owned(), "nightbot".to_owned()]);

        filter_bots(&mut rows, &logins, &excluded);

        assert_eq!(rows.len(), 1);
        assert!(rows.contains_key("other-id"));
    }
}
