//! Builds tier tables from the stored chat activity.

use super::sully::Stream;
use crate::{
    app::App,
    domain::tiers::{filter_bots, rank, RankedTiers, TierMode, TierPeriod, UserWindows},
    storage::tiers::{
        get_day_windows, get_day_windows_with_ranges, get_month_windows,
        get_month_windows_with_ranges,
    },
    Result,
};
use chrono::{DateTime, Days, FixedOffset, Months, NaiveDate, NaiveTime};
use clickhouse::Client;
use std::collections::{HashMap, HashSet};

/// Ranks the users of a channel in a period.
///
/// `channel` is the channel as given by the client, a login or an id; stream
/// windows for the online and offline modes are looked up by it on
/// SullyGnome.
pub async fn compute(
    app: &App,
    channel_id: &str,
    channel: &str,
    period: TierPeriod,
    mode: TierMode,
    excluded_bots: &[String],
) -> Result<RankedTiers> {
    let excluded: HashSet<String> = excluded_bots.iter().map(|bot| bot.to_lowercase()).collect();

    let streams = match mode {
        TierMode::All => Vec::new(),
        TierMode::Online | TierMode::Offline => app
            .sully
            .load(channel, period.year())
            .await
            .map(|list| list.streams)
            .unwrap_or_default(),
    };

    let mut rows_by_user: HashMap<String, UserWindows> = HashMap::new();
    match period {
        TierPeriod::Day { year, month, day } => {
            let yyyymmdd = year * 10000 + (month as i32) * 100 + day as i32;
            let intervals = intervals_for_day(year, month, day, &streams);
            for row in day_windows(&app.db, channel_id, yyyymmdd, mode, &intervals).await? {
                rows_by_user.insert(row.user_id.clone(), row);
            }
        }
        TierPeriod::Month { year, month } => {
            let yyyymm = year * 100 + month as i32;
            let intervals = intervals_for_month(year, month, &streams);
            for row in month_windows(&app.db, channel_id, yyyymm, mode, &intervals).await? {
                rows_by_user.insert(row.user_id.clone(), row);
            }
        }
        TierPeriod::Year { year } => {
            for month in 1..=12 {
                let yyyymm = year * 100 + month as i32;
                let intervals = intervals_for_month(year, month, &streams);
                for row in month_windows(&app.db, channel_id, yyyymm, mode, &intervals).await? {
                    rows_by_user
                        .entry(row.user_id.clone())
                        .and_modify(|acc| acc.add(&row))
                        .or_insert(row);
                }
            }
        }
    }

    let mut user_ids: Vec<String> = rows_by_user.keys().cloned().collect();
    user_ids.sort();
    let user_logins = app
        .get_users(user_ids, vec![], false)
        .await
        .unwrap_or_default();

    filter_bots(&mut rows_by_user, &user_logins, &excluded);

    Ok(rank(rows_by_user, user_logins))
}

async fn day_windows(
    db: &Client,
    channel_id: &str,
    yyyymmdd: i32,
    mode: TierMode,
    intervals: &[(i64, i64)],
) -> Result<Vec<UserWindows>> {
    Ok(match mode {
        TierMode::All => get_day_windows(db, channel_id, yyyymmdd)
            .await?
            .into_iter()
            .map(UserWindows::from)
            .collect(),
        TierMode::Online | TierMode::Offline => {
            let online = mode == TierMode::Online;
            get_day_windows_with_ranges(db, channel_id, yyyymmdd, intervals, online)
                .await?
                .into_iter()
                .map(|(user_id, windows)| windows.into_user_windows(user_id))
                .collect()
        }
    })
}

async fn month_windows(
    db: &Client,
    channel_id: &str,
    yyyymm: i32,
    mode: TierMode,
    intervals: &[(i64, i64)],
) -> Result<Vec<UserWindows>> {
    Ok(match mode {
        TierMode::All => get_month_windows(db, channel_id, yyyymm)
            .await?
            .into_iter()
            .map(UserWindows::from)
            .collect(),
        TierMode::Online | TierMode::Offline => {
            let online = mode == TierMode::Online;
            get_month_windows_with_ranges(db, channel_id, yyyymm, intervals, online)
                .await?
                .into_iter()
                .map(|(user_id, windows)| windows.into_user_windows(user_id))
                .collect()
        }
    })
}

/// Stream intervals clipped to a Moscow calendar month, in Unix milliseconds.
fn intervals_for_month(year: i32, month: u32, streams: &[Stream]) -> Vec<(i64, i64)> {
    let moscow = FixedOffset::east_opt(3 * 3600).unwrap();
    let month_start = NaiveDate::from_ymd_opt(year, month, 1)
        .unwrap()
        .and_time(NaiveTime::default())
        .and_local_timezone(moscow)
        .unwrap();
    let month_end = month_start.checked_add_months(Months::new(1)).unwrap();

    clip_streams(streams, moscow, month_start, month_end)
}

/// Stream intervals clipped to a Moscow calendar day, in Unix milliseconds.
fn intervals_for_day(year: i32, month: u32, day: u32, streams: &[Stream]) -> Vec<(i64, i64)> {
    let moscow = FixedOffset::east_opt(3 * 3600).unwrap();
    let day_start = NaiveDate::from_ymd_opt(year, month, day)
        .unwrap()
        .and_time(NaiveTime::default())
        .and_local_timezone(moscow)
        .unwrap();
    let day_end = day_start.checked_add_days(Days::new(1)).unwrap();

    clip_streams(streams, moscow, day_start, day_end)
}

fn clip_streams(
    streams: &[Stream],
    timezone: FixedOffset,
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
) -> Vec<(i64, i64)> {
    streams
        .iter()
        .filter_map(|stream| {
            let stream_start = DateTime::parse_from_rfc3339(stream.start_iso.as_ref()?)
                .ok()?
                .with_timezone(&timezone);
            let stream_end =
                stream_start + chrono::Duration::minutes(stream.length_minutes? as i64);
            let from = std::cmp::max(stream_start, start);
            let to = std::cmp::min(stream_end, end);
            (to > from).then(|| (from.timestamp_millis(), to.timestamp_millis()))
        })
        .collect()
}
