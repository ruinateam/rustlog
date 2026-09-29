//! Builds tier tables from the stored chat activity.

use super::sully::{Stream, SullyGnome};
use crate::{
    domain::{
        logs::TimeRange,
        tiers::{filter_bots, rank, RankedTiers, TierMode, TierPeriod, UserWindows},
    },
    storage::{
        tiers::{user_windows, CalendarPeriod, StreamFilter},
        Result,
    },
    twitch::Twitch,
};
use chrono::{DateTime, Days, FixedOffset, Months, NaiveDate, NaiveTime, Utc};
use clickhouse::Client;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

/// What to rank.
pub struct TierQuery<'a> {
    pub channel_id: &'a str,
    /// The channel as given by the client, a login or an id. Stream windows
    /// for the online and offline modes are looked up by it on SullyGnome.
    pub channel: &'a str,
    pub period: TierPeriod,
    pub mode: TierMode,
    pub excluded_bots: &'a [String],
}

/// Builds tier tables from stored messages, SullyGnome stream windows and
/// Twitch logins.
#[derive(Clone)]
pub struct Tiers {
    db: Arc<Client>,
    sully: SullyGnome,
    twitch: Twitch,
}

impl Tiers {
    pub fn new(db: Arc<Client>, sully: SullyGnome, twitch: Twitch) -> Self {
        Self { db, sully, twitch }
    }

    /// Ranks the users of a channel in a period.
    pub async fn compute(&self, query: TierQuery<'_>) -> Result<RankedTiers> {
        let TierQuery {
            channel_id,
            channel,
            period,
            mode,
            excluded_bots,
        } = query;
        let excluded: HashSet<String> =
            excluded_bots.iter().map(|bot| bot.to_lowercase()).collect();

        let streams = match mode {
            TierMode::All => Vec::new(),
            TierMode::Online | TierMode::Offline => self
                .sully
                .load(channel, period.year())
                .await
                .map(|list| list.streams)
                .unwrap_or_default(),
        };

        let mut rows_by_user: HashMap<String, UserWindows> = HashMap::new();
        match period {
            TierPeriod::Day(date) => {
                let period = CalendarPeriod::Day(date);
                for row in windows_in(&self.db, channel_id, period, mode, &streams).await? {
                    rows_by_user.insert(row.user_id.clone(), row);
                }
            }
            TierPeriod::Month { year, month } => {
                let period = CalendarPeriod::Month { year, month };
                for row in windows_in(&self.db, channel_id, period, mode, &streams).await? {
                    rows_by_user.insert(row.user_id.clone(), row);
                }
            }
            // A year sums up its months, so unique messages count per month.
            TierPeriod::Year { year } => {
                for month in 1..=12 {
                    let period = CalendarPeriod::Month { year, month };
                    for row in windows_in(&self.db, channel_id, period, mode, &streams).await? {
                        rows_by_user
                            .entry(row.user_id.clone())
                            .and_modify(|total| total.add(&row))
                            .or_insert(row);
                    }
                }
            }
        }

        let mut user_ids: Vec<String> = rows_by_user.keys().cloned().collect();
        user_ids.sort();
        let user_logins = self
            .twitch
            .get_users(user_ids, vec![], false)
            .await
            .unwrap_or_default();

        filter_bots(&mut rows_by_user, &user_logins, &excluded);

        Ok(rank(rows_by_user, user_logins))
    }
}

async fn windows_in(
    db: &Client,
    channel_id: &str,
    period: CalendarPeriod,
    mode: TierMode,
    streams: &[Stream],
) -> Result<Vec<UserWindows>> {
    let stream_ranges = streams_within(streams, period);
    let filter = match mode {
        TierMode::All => StreamFilter::All,
        TierMode::Online => StreamFilter::DuringStreams(&stream_ranges),
        TierMode::Offline => StreamFilter::OutsideStreams(&stream_ranges),
    };

    user_windows(db, channel_id, period, filter).await
}

/// The parts of the streams that fall into a Moscow calendar period.
fn streams_within(streams: &[Stream], period: CalendarPeriod) -> Vec<TimeRange> {
    let moscow = FixedOffset::east_opt(3 * 3600).unwrap();
    let (first_day, next_period) = match period {
        CalendarPeriod::Day(date) => (date, date.checked_add_days(Days::new(1))),
        CalendarPeriod::Month { year, month } => {
            let first_day = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
            (first_day, first_day.checked_add_months(Months::new(1)))
        }
    };
    let start_of = |date: NaiveDate| {
        date.and_time(NaiveTime::default())
            .and_local_timezone(moscow)
            .unwrap()
    };
    let period_start = start_of(first_day);
    let period_end = start_of(next_period.unwrap());

    streams
        .iter()
        .filter_map(|stream| {
            let stream_start = DateTime::parse_from_rfc3339(stream.start_iso.as_ref()?).ok()?;
            let stream_end =
                stream_start + chrono::Duration::minutes(stream.length_minutes? as i64);
            let from = std::cmp::max(stream_start, period_start);
            let to = std::cmp::min(stream_end, period_end);
            (to > from).then(|| TimeRange {
                from: from.with_timezone(&Utc),
                to: to.with_timezone(&Utc),
            })
        })
        .collect()
}
