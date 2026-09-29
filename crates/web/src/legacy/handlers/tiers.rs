//! Chat tier tables and the SullyGnome stream history behind them.

use super::{normalize_month, parse_date, resolve_channel};
use crate::{
    cache_control::{no_cache, public_cache},
    legacy::{
        dto::{
            ChannelDayPath, ChannelMonthPath, ChannelYearPath, SullyStreamsResponse,
            TierDayResponse, TierEntry, TierModeQuery, TierResponse, TierYearResponse,
        },
        error::{Error, Result},
    },
};
use aide::axum::IntoApiResponse;
use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::Datelike;
use rustlog_app::{
    App,
    services::{self, sully, tiers::TierQuery},
};
use rustlog_domain::tiers::{RankedTiers, TIMEZONE, TierPeriod};

pub async fn get_channel_tiers_month(
    app: State<App>,
    Path(month_path): Path<ChannelMonthPath>,
    Query(mode_query): Query<TierModeQuery>,
) -> Result<impl IntoApiResponse> {
    let channel = &month_path.channel_info.channel;
    let channel_id =
        resolve_channel(&app, month_path.channel_info.channel_id_type, channel).await?;
    app.check_opted_out(&channel_id, None)?;

    let year: i32 = month_path.year.parse()?;
    let month = normalize_month(month_path.month.parse()?)?;
    let period = TierPeriod::Month { year, month };
    let tiers = compute_tiers(&app, &channel_id, channel, period, &mode_query).await?;

    let response = TierResponse {
        year,
        month,
        timezone: TIMEZONE,
        total_users: tiers.total_users,
        total_messages: tiers.total_messages,
        total_unique_messages: tiers.total_unique_messages,
        entries: tiers.entries.into_iter().map(TierEntry::from).collect(),
    };
    Ok((no_cache(), Json(response)))
}

pub async fn get_channel_tiers_day(
    app: State<App>,
    Path(day_path): Path<ChannelDayPath>,
    Query(mode_query): Query<TierModeQuery>,
) -> Result<impl IntoApiResponse> {
    let channel = &day_path.channel_info.channel;
    let channel_id = resolve_channel(&app, day_path.channel_info.channel_id_type, channel).await?;
    app.check_opted_out(&channel_id, None)?;

    let year: i32 = day_path.year.parse()?;
    let month = normalize_month(day_path.month.parse()?)?;
    let date = parse_date(year, month, day_path.day.parse()?)?;
    let tiers = compute_tiers(
        &app,
        &channel_id,
        channel,
        TierPeriod::Day(date),
        &mode_query,
    )
    .await?;

    let response = TierDayResponse {
        year,
        month,
        day: date.day(),
        timezone: TIMEZONE,
        total_users: tiers.total_users,
        total_messages: tiers.total_messages,
        total_unique_messages: tiers.total_unique_messages,
        entries: tiers.entries.into_iter().map(TierEntry::from).collect(),
    };
    Ok((no_cache(), Json(response)))
}

pub async fn get_channel_tiers_year(
    app: State<App>,
    Path(year_path): Path<ChannelYearPath>,
    Query(mode_query): Query<TierModeQuery>,
) -> Result<impl IntoApiResponse> {
    let channel = &year_path.channel_info.channel;
    let channel_id = resolve_channel(&app, year_path.channel_info.channel_id_type, channel).await?;
    app.check_opted_out(&channel_id, None)?;

    let year: i32 = year_path.year.parse()?;
    let period = TierPeriod::Year { year };
    let tiers = compute_tiers(&app, &channel_id, channel, period, &mode_query).await?;

    let response = TierYearResponse {
        year,
        timezone: TIMEZONE,
        total_users: tiers.total_users,
        total_messages: tiers.total_messages,
        total_unique_messages: tiers.total_unique_messages,
        entries: tiers.entries.into_iter().map(TierEntry::from).collect(),
    };
    Ok((no_cache(), Json(response)))
}

/// Computes a tier table and snapshots it to Supabase when enabled.
async fn compute_tiers(
    app: &App,
    channel_id: &str,
    channel: &str,
    period: TierPeriod,
    mode_query: &TierModeQuery,
) -> Result<RankedTiers> {
    let mode = mode_query.mode.into();
    let tiers = app
        .tiers
        .compute(TierQuery {
            channel_id,
            channel,
            period,
            mode,
            excluded_bots: &mode_query.exclude_bots,
        })
        .await?;
    services::supabase::spawn_tier_snapshot(&app.config, channel, period, mode, &tiers);
    Ok(tiers)
}

pub async fn get_sully_streams(
    app: State<App>,
    Path((channel, year)): Path<(String, i32)>,
) -> Result<impl IntoApiResponse> {
    // Prefer a live fetch (failures are logged), fall back to the cache.
    if let Ok((total, streams)) = app.sully.fetch(&channel, year).await {
        let list = sully::StreamList {
            channel,
            year,
            total,
            streams,
        };
        app.sully.write_cache(&list);
        return Ok((public_cache(600), Json(SullyStreamsResponse::from(list))));
    }

    if let Some(cached) = app
        .sully
        .read_cache(&channel, year)
        .filter(|list| !list.streams.is_empty())
    {
        return Ok((public_cache(3600), Json(SullyStreamsResponse::from(cached))));
    }

    Err(Error::NotFound)
}
