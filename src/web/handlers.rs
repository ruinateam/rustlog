use super::{
    responders::logs::LogsResponse,
    schema::{
        AvailableLogDate, AvailableLogs, AvailableLogsParams, Channel, ChannelDayPath,
        ChannelIdType, ChannelLogsByDatePath, ChannelLogsStats, ChannelMonthPath, ChannelParam,
        ChannelYearPath, ChannelsList, ChatBadge, ChatBadgesResponse, LogsParams, LogsPathChannel,
        PreviousName, SearchParams, TierDayResponse, TierEntry, TierResponse, TierYearResponse,
        UserIdType, UserLogPathParams, UserLogsDatePath, UserLogsStats, UserNameHistoryParam,
        UserParam,
    },
};
use crate::web::error::{Error, Result};
use crate::{
    app::App,
    domain::tiers::{RankedTiers, TierPeriod, TIMEZONE},
    services::{self, sully, tiers::TierQuery},
    storage::{availability, logs, stats, stream::LogsStream},
    web::schema::{LogRangeParams, LogsPathDate, SullyStreamsResponse, TierModeQuery},
};
use aide::axum::IntoApiResponse;
use axum::{
    extract::{Path, Query, RawQuery, State},
    response::{IntoResponse, Redirect, Response},
    Json,
};
use axum_extra::{headers::CacheControl, TypedHeader};
use chrono::{DateTime, Datelike, Days, Months, NaiveDate, NaiveTime, Utc};
use rand::{distr::Alphanumeric, rng, Rng};
use std::time::Duration;
use tracing::debug;

pub async fn get_channels(app: State<App>) -> Result<impl IntoApiResponse> {
    let channel_ids = app.state.channel_ids();

    let channels = app
        .twitch
        .get_users(Vec::from_iter(channel_ids), vec![], false)
        .await?;

    let json = Json(ChannelsList {
        channels: channels
            .into_iter()
            .map(|(user_id, name)| Channel { name, user_id })
            .collect(),
    });
    Ok((no_cache_header(), json))
}

pub async fn get_chat_badges(
    Path(channel_id): Path<String>,
    app: State<App>,
) -> Result<impl IntoApiResponse> {
    // Only logged channels, so that arbitrary ids cannot drive Helix requests.
    if !app.state.is_channel_enabled(&channel_id) {
        return Err(Error::NotFound);
    }

    let (global, channel) = app.twitch.get_chat_badges(&channel_id).await?;
    let badges = global
        .into_iter()
        .chain(channel)
        .flat_map(|set| {
            let set_id = set.set_id.to_string();
            set.versions.into_iter().map(move |badge| ChatBadge {
                set_id: set_id.clone(),
                version: badge.id.to_string(),
                image_url_1x: badge.image_url_1x,
                image_url_2x: badge.image_url_2x,
                title: badge.title,
                description: badge.description,
            })
        })
        .collect();

    Ok((cache_header(3600), Json(ChatBadgesResponse { badges })))
}

pub async fn get_channel_logs(
    Path(LogsPathChannel {
        channel_id_type,
        channel,
    }): Path<LogsPathChannel>,
    Query(range_params): Query<LogRangeParams>,
    Query(logs_params): Query<LogsParams>,
    RawQuery(query): RawQuery,
    app: State<App>,
) -> Result<Response> {
    let channel_id = match channel_id_type {
        ChannelIdType::Name => app.twitch.get_user_id_by_name(&channel).await?,
        ChannelIdType::Id => channel.clone(),
    };

    if let Some(range) = range_params.range() {
        let logs = get_channel_logs_inner(&app, &channel_id, logs_params, range).await?;
        Ok(logs.into_response())
    } else {
        let available_logs =
            availability::read_available_channel_logs(&app.db, &channel_id).await?;
        let latest_log = AvailableLogDate::from(*available_logs.first().ok_or(Error::NotFound)?);

        let mut new_uri = format!("/{channel_id_type}/{channel}/{latest_log}");
        if let Some(query) = query {
            new_uri.push('?');
            new_uri.push_str(&query);
        }

        Ok(Redirect::to(&new_uri).into_response())
    }
}

pub async fn get_channel_stats(
    Path(LogsPathChannel {
        channel_id_type,
        channel,
    }): Path<LogsPathChannel>,
    Query(range_params): Query<LogRangeParams>,
    app: State<App>,
) -> Result<Json<ChannelLogsStats>> {
    let channel_id = match channel_id_type {
        ChannelIdType::Name => app.twitch.get_user_id_by_name(&channel).await?,
        ChannelIdType::Id => channel.clone(),
    };
    app.check_opted_out(&channel_id, None)?;

    let (message_count, stats_rows) =
        stats::get_channel_stats(&app.db, &channel_id, range_params.time_range()).await?;

    let user_ids = stats_rows.iter().map(|row| row.user_id.clone()).collect();
    let mut users = app.twitch.get_users(user_ids, vec![], false).await?;

    let top_chatters = stats_rows
        .into_iter()
        .map(|row| UserLogsStats {
            user_login: users.remove(&row.user_id),
            user_id: row.user_id,
            message_count: row.cnt,
        })
        .collect();

    Ok(Json(ChannelLogsStats {
        message_count,
        top_chatters,
    }))
}

pub async fn get_user_stats(
    Path(user_params): Path<UserLogPathParams>,
    Query(range_params): Query<LogRangeParams>,
    app: State<App>,
) -> Result<Json<UserLogsStats>> {
    let (channel_id, user_id) = resolve_user_params(&user_params, &app).await?;

    app.check_opted_out(&channel_id, Some(&user_id))?;

    let user_login = app
        .twitch
        .get_users(vec![user_id.clone()], vec![], false)
        .await?
        .into_values()
        .next();
    let stats = stats::get_user_stats(
        &app.db,
        &channel_id,
        user_id,
        user_login,
        range_params.time_range(),
    )
    .await?;

    Ok(Json(UserLogsStats::from(stats)))
}

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
    Ok((no_cache_header(), Json(response)))
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
    Ok((no_cache_header(), Json(response)))
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
    Ok((no_cache_header(), Json(response)))
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

async fn resolve_channel(app: &App, id_type: ChannelIdType, channel: &str) -> Result<String> {
    match id_type {
        ChannelIdType::Name => Ok(app.twitch.get_user_id_by_name(channel).await?),
        ChannelIdType::Id => Ok(channel.to_owned()),
    }
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
        let _ = app.sully.write_cache(&list);
        return Ok((cache_header(600), Json(SullyStreamsResponse::from(list))));
    }

    if let Some(cached) = app
        .sully
        .read_cache(&channel, year)
        .filter(|list| !list.streams.is_empty())
    {
        return Ok((cache_header(3600), Json(SullyStreamsResponse::from(cached))));
    }

    Err(Error::NotFound)
}

pub async fn get_channel_logs_by_date(
    app: State<App>,
    Path(channel_log_params): Path<ChannelLogsByDatePath>,
    Query(logs_params): Query<LogsParams>,
) -> Result<impl IntoApiResponse> {
    debug!("Params: {logs_params:?}");

    let channel_id = match channel_log_params.channel_info.channel_id_type {
        ChannelIdType::Name => {
            app.twitch
                .get_user_id_by_name(&channel_log_params.channel_info.channel)
                .await?
        }
        ChannelIdType::Id => channel_log_params.channel_info.channel.clone(),
    };

    let LogsPathDate { year, month, day } = channel_log_params.date;
    let year: i32 = year.parse()?;
    let month = normalize_month(month.parse()?)?;
    let from = parse_date(year, month, day.parse()?)?
        .and_time(NaiveTime::default())
        .and_utc();
    let to = from
        .checked_add_days(Days::new(1))
        .ok_or_else(|| Error::InvalidParam("Date out of range".to_owned()))?;

    get_channel_logs_inner(&app, &channel_id, logs_params, (from, to)).await
}

async fn get_channel_logs_inner(
    app: &App,
    channel_id: &str,
    params: LogsParams,
    range: (DateTime<Utc>, DateTime<Utc>),
) -> Result<impl IntoApiResponse> {
    app.check_opted_out(channel_id, None)?;

    let stream = logs::read_channel(
        &app.db,
        channel_id,
        params.query(),
        &app.flush_buffer,
        range,
    )
    .await?;

    let logs = LogsResponse {
        response_type: params.response_type(),
        stream,
    };

    // Historical log rows can be withdrawn by an opt-out mutation.
    let cache = no_cache_header();

    Ok((cache, logs))
}

pub async fn get_user_logs(
    Path(user_params): Path<UserLogPathParams>,
    Query(range_params): Query<LogRangeParams>,
    Query(logs_params): Query<LogsParams>,
    RawQuery(query): RawQuery,
    app: State<App>,
) -> Result<impl IntoApiResponse> {
    let (channel_id, user_id) = resolve_user_params(&user_params, &app).await?;

    app.check_opted_out(&channel_id, Some(&user_id))?;

    if let Some(range) = range_params.range() {
        let logs = get_user_logs_inner(&app, &channel_id, &user_id, logs_params, range).await?;
        Ok(logs.into_response())
    } else {
        let available_logs =
            availability::read_available_user_logs(&app.db, &channel_id, &user_id).await?;
        let latest_log = AvailableLogDate::from(*available_logs.first().ok_or(Error::NotFound)?);

        let UserLogPathParams {
            channel_id_type,
            channel,
            user_id_type,
            user,
        } = user_params;

        let mut new_uri =
            format!("/{channel_id_type}/{channel}/{user_id_type}/{user}/{latest_log}");
        if let Some(query) = query {
            new_uri.push('?');
            new_uri.push_str(&query);
        }
        Ok(Redirect::to(&new_uri).into_response())
    }
}

pub async fn get_user_logs_by_date(
    app: State<App>,
    Path(user_params): Path<UserLogPathParams>,
    Path(user_logs_date): Path<UserLogsDatePath>,
    Query(logs_params): Query<LogsParams>,
) -> Result<impl IntoApiResponse> {
    let (channel_id, user_id) = resolve_user_params(&user_params, &app).await?;

    app.check_opted_out(&channel_id, Some(&user_id))?;

    let year: i32 = user_logs_date.year.parse()?;
    let month = normalize_month(user_logs_date.month.parse()?)?;

    let from = NaiveDate::from_ymd_opt(year, month, 1)
        .ok_or_else(|| Error::InvalidParam("Invalid date".to_owned()))?
        .and_time(NaiveTime::default())
        .and_utc();
    let to = from
        .checked_add_months(Months::new(1))
        .ok_or_else(|| Error::InvalidParam("Date out of range".to_owned()))?;

    get_user_logs_inner(&app, &channel_id, &user_id, logs_params, (from, to)).await
}

async fn get_user_logs_inner(
    app: &App,
    channel_id: &str,
    user_id: &str,
    logs_params: LogsParams,
    range: (DateTime<Utc>, DateTime<Utc>),
) -> Result<impl IntoApiResponse> {
    let stream = logs::read_user(
        &app.db,
        channel_id,
        user_id,
        logs_params.query(),
        &app.flush_buffer,
        range,
    )
    .await?;

    let logs = LogsResponse {
        stream,
        response_type: logs_params.response_type(),
    };

    // Historical log rows can be withdrawn by an opt-out mutation.
    let cache = no_cache_header();

    Ok((cache, logs))
}

pub async fn list_available_logs(
    Query(AvailableLogsParams { user, channel }): Query<AvailableLogsParams>,
    app: State<App>,
) -> Result<impl IntoApiResponse> {
    let channel = channel.ok_or_else(|| {
        Error::InvalidParam("Specify query parameter: channel or channelid".to_owned())
    })?;
    let channel_id = match channel {
        ChannelParam::ChannelId(id) => id,
        ChannelParam::Channel(name) => app.twitch.get_user_id_by_name(&name).await?,
    };

    let available_logs = if let Some(user) = user {
        let user_id = match user {
            UserParam::UserId(id) => id,
            UserParam::User(name) => app.twitch.get_user_id_by_name(&name).await?,
        };
        app.check_opted_out(&channel_id, Some(&user_id))?;
        availability::read_available_user_logs(&app.db, &channel_id, &user_id).await?
    } else {
        app.check_opted_out(&channel_id, None)?;
        availability::read_available_channel_logs(&app.db, &channel_id).await?
    };

    if !available_logs.is_empty() {
        let available_logs = available_logs
            .into_iter()
            .map(AvailableLogDate::from)
            .collect();
        Ok((no_cache_header(), Json(AvailableLogs { available_logs })))
    } else {
        Err(Error::NotFound)
    }
}

pub async fn random_channel_line(
    app: State<App>,
    Path(LogsPathChannel {
        channel_id_type,
        channel,
    }): Path<LogsPathChannel>,
    Query(logs_params): Query<LogsParams>,
) -> Result<impl IntoApiResponse> {
    let channel_id = match channel_id_type {
        ChannelIdType::Name => app.twitch.get_user_id_by_name(&channel).await?,
        ChannelIdType::Id => channel,
    };
    app.check_opted_out(&channel_id, None)?;

    let random_line = logs::read_random_channel_line(&app.db, &channel_id).await?;
    let stream = LogsStream::new_provided(vec![random_line])?;

    let logs = LogsResponse {
        stream,
        response_type: logs_params.response_type(),
    };
    Ok((no_cache_header(), logs))
}

pub async fn random_user_line(
    app: State<App>,
    Path(user_params): Path<UserLogPathParams>,
    Query(logs_params): Query<LogsParams>,
) -> Result<impl IntoApiResponse> {
    let (channel_id, user_id) = resolve_user_params(&user_params, &app).await?;

    app.check_opted_out(&channel_id, Some(&user_id))?;

    let random_line = logs::read_random_user_line(&app.db, &channel_id, &user_id).await?;
    let stream = LogsStream::new_provided(vec![random_line])?;

    let logs = LogsResponse {
        stream,
        response_type: logs_params.response_type(),
    };
    Ok((no_cache_header(), logs))
}

pub async fn search_user_logs(
    app: State<App>,
    Path(user_params): Path<UserLogPathParams>,
    Query(search_params): Query<SearchParams>,
    Query(logs_params): Query<LogsParams>,
) -> Result<impl IntoApiResponse> {
    let (channel_id, user_id) = resolve_user_params(&user_params, &app).await?;

    app.check_opted_out(&channel_id, Some(&user_id))?;

    let stream = logs::search_user_logs(
        &app.db,
        &channel_id,
        &user_id,
        &search_params.q,
        logs_params.query(),
    )
    .await?;

    let logs = LogsResponse {
        stream,
        response_type: logs_params.response_type(),
    };
    Ok(logs)
}

pub async fn get_user_name_history(
    app: State<App>,
    Path(UserNameHistoryParam { user_id }): Path<UserNameHistoryParam>,
) -> Result<impl IntoApiResponse> {
    app.check_user_opted_out(&user_id)?;

    let names: Vec<_> = stats::get_user_name_history(&app.db, &user_id)
        .await?
        .into_iter()
        .map(PreviousName::from)
        .collect();

    Ok(Json(names))
}

pub async fn optout(app: State<App>) -> Json<String> {
    let mut rng = rng();
    let optout_code: String = (0..5).map(|_| rng.sample(Alphanumeric) as char).collect();

    app.optout_codes.insert(optout_code.clone());

    {
        let codes = app.optout_codes.clone();
        let optout_code = optout_code.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(60)).await;
            if codes.remove(&optout_code).is_some() {
                debug!("Dropping optout code {optout_code}");
            }
        });
    }

    Json(optout_code)
}

fn cache_header(secs: u64) -> TypedHeader<CacheControl> {
    TypedHeader(
        CacheControl::new()
            .with_public()
            .with_max_age(Duration::from_secs(secs)),
    )
}

pub fn no_cache_header() -> TypedHeader<CacheControl> {
    TypedHeader(CacheControl::new().with_no_cache())
}

async fn resolve_user_params(params: &UserLogPathParams, app: &App) -> Result<(String, String)> {
    let channel_id = match params.channel_id_type {
        ChannelIdType::Name => app.twitch.get_user_id_by_name(&params.channel).await?,
        ChannelIdType::Id => params.channel.clone(),
    };
    let user_id = match params.user_id_type {
        UserIdType::Name => app.twitch.get_user_id_by_name(&params.user).await?,
        UserIdType::Id => params.user.clone(),
    };
    Ok((channel_id, user_id))
}

fn normalize_month(month_raw: i32) -> Result<u32> {
    // Use strict 1-based months to avoid duplicated “13th” month (0..11 + 1..12).
    // Valid range: 1 (Jan) ..= 12 (Dec).
    if (1..=12).contains(&month_raw) {
        Ok(month_raw as u32)
    } else {
        Err(Error::InvalidParam(format!(
            "Invalid month: {month_raw} (expected 1-12)"
        )))
    }
}

fn parse_date(year: i32, month: u32, day_raw: i32) -> Result<NaiveDate> {
    NaiveDate::from_ymd_opt(year, month, day_raw as u32)
        .ok_or_else(|| Error::InvalidParam(format!("Invalid date: {year}-{month:02}-{day_raw:02}")))
}
