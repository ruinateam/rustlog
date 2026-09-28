use super::{
    responders::logs::LogsResponse,
    schema::{
        AvailableLogs, AvailableLogsParams, Channel, ChannelDayPath, ChannelIdType,
        ChannelLogsByDatePath, ChannelLogsStats, ChannelMonthPath, ChannelParam, ChannelYearPath,
        ChannelsList, ChatBadge, ChatBadgesResponse, LogsParams, LogsPathChannel, SearchParams,
        TierDayResponse, TierEntry, TierResponse, TierYearResponse, UserIdType, UserLogPathParams,
        UserLogsDatePath, UserLogsStats, UserNameHistoryParam, UserParam,
    },
};
use crate::{
    app::App,
    db::{
        self, get_day_windows, get_day_windows_with_ranges, get_month_windows,
        get_month_windows_with_ranges, read_available_channel_logs, read_available_user_logs,
        read_channel, read_random_channel_line, read_random_user_line, read_user, WindowsAggRow,
    },
    error::Error,
    logs::{schema::LogRangeParams, stream::LogsStream},
    supabase::{write_tier_snapshot, SupabaseConfig, TierSnapshotPayload},
    web::schema::{LogsPathDate, SullyStreamEntry, SullyStreamsResponse, TierMode, TierModeQuery},
    Result,
};
use aide::axum::IntoApiResponse;
use axum::{
    extract::{Path, Query, RawQuery, State},
    response::{IntoResponse, Redirect, Response},
    Json,
};
use axum_extra::{headers::CacheControl, TypedHeader};
use chrono::offset::FixedOffset;
use chrono::{DateTime, Datelike, Days, Months, NaiveDate, NaiveTime, Utc};
use rand::{distr::Alphanumeric, rng, Rng};
use reqwest::Client as HttpClient;
use serde::Deserialize;
use serde_json;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use tracing::{debug, error, warn};
// use std::process::Command; // not used
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};

pub async fn get_channels(app: State<App>) -> impl IntoApiResponse {
    let channel_ids = app.state.channel_ids();

    let channels = app
        .get_users(Vec::from_iter(channel_ids), vec![], false)
        .await
        .unwrap();

    let json = Json(ChannelsList {
        channels: channels
            .into_iter()
            .map(|(user_id, name)| Channel { name, user_id })
            .collect(),
    });
    (no_cache_header(), json)
}

pub async fn get_chat_badges(
    Path(channel_id): Path<String>,
    app: State<App>,
) -> Result<impl IntoApiResponse> {
    let (global, channel) = app.get_chat_badges(&channel_id).await?;
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
        ChannelIdType::Name => app.get_user_id_by_name(&channel).await?,
        ChannelIdType::Id => channel.clone(),
    };

    if let Some(range) = range_params.range() {
        let logs = get_channel_logs_inner(&app, &channel_id, logs_params, range).await?;
        Ok(logs.into_response())
    } else {
        let available_logs = read_available_channel_logs(&app.db, &channel_id).await?;
        let latest_log = available_logs.first().ok_or(Error::NotFound)?;

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
        ChannelIdType::Name => app.get_user_id_by_name(&channel).await?,
        ChannelIdType::Id => channel.clone(),
    };
    app.check_opted_out(&channel_id, None)?;

    let (message_count, stats_rows) =
        db::get_channel_stats(&app.db, &channel_id, range_params).await?;

    let user_ids = stats_rows.iter().map(|row| row.user_id.clone()).collect();
    let mut users = app.get_users(user_ids, vec![], false).await?;

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
        .get_users(vec![user_id.clone()], vec![], false)
        .await?
        .into_values()
        .next();
    let stats = db::get_user_stats(&app.db, &channel_id, user_id, user_login, range_params).await?;

    Ok(Json(stats))
}

fn build_http_client() -> Result<HttpClient> {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("rustlog/1.0 (https://localhost)"),
    );
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    let client = HttpClient::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| Error::Internal)?;
    Ok(client)
}

async fn load_sully_streams(channel: &str, year: i32) -> Option<SullyStreamsResponse> {
    // Try live fetch first; on failure fall back to cache.
    if let Ok(http) = build_http_client() {
        match fetch_sully_id(&http, channel).await {
            Ok(internal_id) => match fetch_sully_streams(&http, &internal_id, year).await {
                Ok((_, data)) => {
                    let resp = SullyStreamsResponse {
                        channel: channel.to_string(),
                        year,
                        total: data.len() as u32,
                        streams: data.clone(),
                    };
                    let _ = write_sully_cache(channel, year, &resp);
                    return Some(resp);
                }
                Err(e) => {
                    error!(
                        "sully fetch streams failed channel={} year={} err={:?}",
                        channel, year, e
                    );
                }
            },
            Err(e) => {
                error!(
                    "sully fetch internal_id failed channel={} err={:?}",
                    channel, e
                );
            }
        };
    }
    if let Some(cached) = read_sully_cache(channel, year) {
        warn!(
            "sully fetch falling back to cache channel={} year={} streams={}",
            channel,
            year,
            cached.streams.len()
        );
        Some(cached)
    } else {
        warn!(
            "sully fetch failed and no cache available channel={} year={}",
            channel, year
        );
        None
    }
}

pub async fn get_channel_tiers_month(
    app: State<App>,
    Path(month_path): Path<ChannelMonthPath>,
    Query(mode_query): Query<TierModeQuery>,
) -> Result<impl IntoApiResponse> {
    let channel_id = match month_path.channel_info.channel_id_type {
        ChannelIdType::Name => {
            app.get_user_id_by_name(&month_path.channel_info.channel)
                .await?
        }
        ChannelIdType::Id => month_path.channel_info.channel.clone(),
    };

    app.check_opted_out(&channel_id, None)?;

    let year: i32 = month_path.year.parse()?;
    let month = normalize_month(month_path.month.parse()?)?;
    let yyyymm = year * 100 + month as i32;

    let mode = mode_query.mode;
    let bot_filter: HashSet<String> = mode_query
        .exclude_bots
        .iter()
        .map(|bot| bot.to_lowercase())
        .collect();

    // Determine intervals from Sully for this month. Always try live fetch, fallback to cache.
    let streams = load_sully_streams(&month_path.channel_info.channel, year).await;

    let intervals = streams
        .as_ref()
        .map(|resp| intervals_for_month(year, month, &resp.streams))
        .unwrap_or_default();

    let rows = match mode {
        TierMode::All => get_month_windows(&app.db, &channel_id, yyyymm).await?,
        TierMode::Online => {
            get_month_windows_with_ranges(&app.db, &channel_id, yyyymm, &intervals, true)
                .await?
                .into_iter()
                .map(|(user_id, agg)| WindowsAggRow {
                    user_id,
                    messages: agg.messages,
                    uniq_messages: agg.uniq_messages,
                    w1: agg.w1,
                    w5: agg.w5,
                    w30: agg.w30,
                    w15: agg.w15,
                    w60: agg.w60,
                })
                .collect()
        }
        TierMode::Offline => {
            get_month_windows_with_ranges(&app.db, &channel_id, yyyymm, &intervals, false)
                .await?
                .into_iter()
                .map(|(user_id, agg)| WindowsAggRow {
                    user_id,
                    messages: agg.messages,
                    uniq_messages: agg.uniq_messages,
                    w1: agg.w1,
                    w5: agg.w5,
                    w15: agg.w15,
                    w30: agg.w30,
                    w60: agg.w60,
                })
                .collect()
        }
    };

    let mut rows_by_user: HashMap<String, WindowsAggRow> = HashMap::new();
    for row in rows {
        rows_by_user.insert(row.user_id.clone(), row);
    }

    // Resolve logins once
    let mut user_ids_all: Vec<String> = rows_by_user.keys().cloned().collect();
    user_ids_all.sort();
    let user_logins = app
        .get_users(user_ids_all.clone(), vec![], false)
        .await
        .unwrap_or_default();

    crate::tiers::filter_bots(&mut rows_by_user, &user_logins, &bot_filter);

    let ranked = crate::tiers::rank(rows_by_user, user_logins);

    let response = TierResponse {
        year,
        month,
        timezone: crate::tiers::TIMEZONE,
        total_users: ranked.total_users,
        total_messages: ranked.total_messages,
        total_unique_messages: ranked.total_unique_messages,
        entries: ranked.entries,
    };

    spawn_supabase_tiers(
        &app,
        &month_path.channel_info.channel,
        "month",
        format!("{year:04}{month:02}"),
        mode,
        response.total_users,
        response.total_messages,
        response.total_unique_messages,
        &response.entries,
    );

    Ok((no_cache_header(), Json(response)))
}

pub async fn get_channel_tiers_day(
    app: State<App>,
    Path(day_path): Path<ChannelDayPath>,
    Query(mode_query): Query<TierModeQuery>,
) -> Result<impl IntoApiResponse> {
    let channel_id = match day_path.channel_info.channel_id_type {
        ChannelIdType::Name => {
            app.get_user_id_by_name(&day_path.channel_info.channel)
                .await?
        }
        ChannelIdType::Id => day_path.channel_info.channel.clone(),
    };

    app.check_opted_out(&channel_id, None)?;

    let year: i32 = day_path.year.parse()?;
    let month = normalize_month(day_path.month.parse()?)?;
    let day = normalize_day(year, month, day_path.day.parse()?)?;
    let yyyymmdd = year * 10000 + (month as i32) * 100 + day as i32;

    let mode = mode_query.mode;
    let bot_filter: HashSet<String> = mode_query
        .exclude_bots
        .iter()
        .map(|bot| bot.to_lowercase())
        .collect();

    // Determine intervals from Sully for this day. Always try live fetch, fallback to cache.
    let streams = load_sully_streams(&day_path.channel_info.channel, year).await;

    let intervals = streams
        .as_ref()
        .map(|resp| intervals_for_day(year, month, day, &resp.streams))
        .unwrap_or_default();

    let rows = match mode {
        TierMode::All => get_day_windows(&app.db, &channel_id, yyyymmdd).await?,
        TierMode::Online => {
            get_day_windows_with_ranges(&app.db, &channel_id, yyyymmdd, &intervals, true)
                .await?
                .into_iter()
                .map(|(user_id, agg)| WindowsAggRow {
                    user_id,
                    messages: agg.messages,
                    uniq_messages: agg.uniq_messages,
                    w1: agg.w1,
                    w5: agg.w5,
                    w15: agg.w15,
                    w30: agg.w30,
                    w60: agg.w60,
                })
                .collect()
        }
        TierMode::Offline => {
            get_day_windows_with_ranges(&app.db, &channel_id, yyyymmdd, &intervals, false)
                .await?
                .into_iter()
                .map(|(user_id, agg)| WindowsAggRow {
                    user_id,
                    messages: agg.messages,
                    uniq_messages: agg.uniq_messages,
                    w1: agg.w1,
                    w5: agg.w5,
                    w15: agg.w15,
                    w30: agg.w30,
                    w60: agg.w60,
                })
                .collect()
        }
    };

    let mut rows_by_user: HashMap<String, WindowsAggRow> = HashMap::new();
    for row in rows {
        rows_by_user.insert(row.user_id.clone(), row);
    }

    // Resolve logins once
    let mut user_ids_all: Vec<String> = rows_by_user.keys().cloned().collect();
    user_ids_all.sort();
    let user_logins = app
        .get_users(user_ids_all.clone(), vec![], false)
        .await
        .unwrap_or_default();

    crate::tiers::filter_bots(&mut rows_by_user, &user_logins, &bot_filter);

    let ranked = crate::tiers::rank(rows_by_user, user_logins);

    let response = TierDayResponse {
        year,
        month,
        day,
        timezone: crate::tiers::TIMEZONE,
        total_users: ranked.total_users,
        total_messages: ranked.total_messages,
        total_unique_messages: ranked.total_unique_messages,
        entries: ranked.entries,
    };

    spawn_supabase_tiers(
        &app,
        &day_path.channel_info.channel,
        "day",
        format!("{year:04}{month:02}{day:02}"),
        mode,
        response.total_users,
        response.total_messages,
        response.total_unique_messages,
        &response.entries,
    );

    Ok((no_cache_header(), Json(response)))
}

pub async fn get_channel_tiers_year(
    app: State<App>,
    Path(year_path): Path<ChannelYearPath>,
    Query(mode_query): Query<TierModeQuery>,
) -> Result<impl IntoApiResponse> {
    let channel_id = match year_path.channel_info.channel_id_type {
        ChannelIdType::Name => {
            app.get_user_id_by_name(&year_path.channel_info.channel)
                .await?
        }
        ChannelIdType::Id => year_path.channel_info.channel.clone(),
    };

    app.check_opted_out(&channel_id, None)?;

    let year: i32 = year_path.year.parse()?;
    let mode = mode_query.mode;
    let bot_filter: HashSet<String> = mode_query
        .exclude_bots
        .iter()
        .map(|bot| bot.to_lowercase())
        .collect();

    // Fetch sully streams once for all months (for online/offline mode) - prefer live fetch.
    let streams: Option<Vec<SullyStreamEntry>> =
        load_sully_streams(&year_path.channel_info.channel, year)
            .await
            .map(|resp| resp.streams);

    // For each month aggregate windows
    let mut rows_by_user: HashMap<String, WindowsAggRow> = HashMap::new();
    for month in 1..=12 {
        let yyyymm = year * 100 + month as i32;
        let month_intervals = streams
            .as_ref()
            .map(|data| intervals_for_month(year, month, data))
            .unwrap_or_default();

        let month_rows: Vec<WindowsAggRow> = match mode {
            TierMode::All => get_month_windows(&app.db, &channel_id, yyyymm).await?,
            TierMode::Online => {
                get_month_windows_with_ranges(&app.db, &channel_id, yyyymm, &month_intervals, true)
                    .await?
                    .into_iter()
                    .map(|(user_id, agg)| WindowsAggRow {
                        user_id,
                        messages: agg.messages,
                        uniq_messages: agg.uniq_messages,
                        w1: agg.w1,
                        w5: agg.w5,
                        w15: agg.w15,
                        w30: agg.w30,
                        w60: agg.w60,
                    })
                    .collect()
            }
            TierMode::Offline => {
                get_month_windows_with_ranges(&app.db, &channel_id, yyyymm, &month_intervals, false)
                    .await?
                    .into_iter()
                    .map(|(user_id, agg)| WindowsAggRow {
                        user_id,
                        messages: agg.messages,
                        uniq_messages: agg.uniq_messages,
                        w1: agg.w1,
                        w5: agg.w5,
                        w15: agg.w15,
                        w30: agg.w30,
                        w60: agg.w60,
                    })
                    .collect()
            }
        };

        for row in month_rows {
            rows_by_user
                .entry(row.user_id.clone())
                .and_modify(|acc| {
                    acc.messages += row.messages;
                    acc.uniq_messages += row.uniq_messages;
                    acc.w1 += row.w1;
                    acc.w5 += row.w5;
                    acc.w15 += row.w15;
                    acc.w30 += row.w30;
                    acc.w60 += row.w60;
                })
                .or_insert(row);
        }
    }

    // Resolve logins once
    let mut user_ids_all: Vec<String> = rows_by_user.keys().cloned().collect();
    user_ids_all.sort();
    let user_logins = app
        .get_users(user_ids_all.clone(), vec![], false)
        .await
        .unwrap_or_default();

    crate::tiers::filter_bots(&mut rows_by_user, &user_logins, &bot_filter);

    let ranked = crate::tiers::rank(rows_by_user, user_logins);

    let response = TierYearResponse {
        year,
        timezone: crate::tiers::TIMEZONE,
        total_users: ranked.total_users,
        total_messages: ranked.total_messages,
        total_unique_messages: ranked.total_unique_messages,
        entries: ranked.entries,
    };

    spawn_supabase_tiers(
        &app,
        &year_path.channel_info.channel,
        "year",
        format!("{year:04}"),
        mode,
        response.total_users,
        response.total_messages,
        response.total_unique_messages,
        &response.entries,
    );

    Ok((no_cache_header(), Json(response)))
}

pub async fn get_sully_streams(
    Path((channel, year)): Path<(String, i32)>,
) -> Result<impl IntoApiResponse> {
    // Prefer live fetch, log failures, fall back to cache.
    match build_http_client() {
        Ok(http) => match fetch_sully_id(&http, &channel).await {
            Ok(internal_id) => match fetch_sully_streams(&http, &internal_id, year).await {
                Ok((total, data)) => {
                    let response = SullyStreamsResponse {
                        channel: channel.clone(),
                        year,
                        total,
                        streams: data.clone(),
                    };
                    let _ = write_sully_cache(&channel, year, &response);
                    return Ok((cache_header(600), Json(response)));
                }
                Err(e) => error!(
                    "sully endpoint: fetch streams failed channel={} year={} err={:?}",
                    channel, year, e
                ),
            },
            Err(e) => error!(
                "sully endpoint: fetch internal_id failed channel={} err={:?}",
                channel, e
            ),
        },
        Err(e) => {
            error!("sully endpoint: build_http_client failed err={:?}", e);
        }
    }

    if let Some(cached) = read_sully_cache(&channel, year).filter(|resp| !resp.streams.is_empty()) {
        return Ok((cache_header(3600), Json(cached)));
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
            app.get_user_id_by_name(&channel_log_params.channel_info.channel)
                .await?
        }
        ChannelIdType::Id => channel_log_params.channel_info.channel.clone(),
    };

    let LogsPathDate { year, month, day } = channel_log_params.date;
    let year: i32 = year.parse()?;
    let month = normalize_month(month.parse()?)?;
    let day = normalize_day(year, month, day.parse()?)?;

    let from = NaiveDate::from_ymd_opt(year, month, day)
        .ok_or_else(|| Error::InvalidParam("Invalid date".to_owned()))?
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

    let stream = read_channel(&app.db, channel_id, params, &app.flush_buffer, range).await?;

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
        let available_logs = read_available_user_logs(&app.db, &channel_id, &user_id).await?;
        let latest_log = available_logs.first().ok_or(Error::NotFound)?;

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
    let stream = read_user(
        &app.db,
        channel_id,
        user_id,
        logs_params,
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
        ChannelParam::Channel(name) => app.get_user_id_by_name(&name).await?,
    };

    let available_logs = if let Some(user) = user {
        let user_id = match user {
            UserParam::UserId(id) => id,
            UserParam::User(name) => app.get_user_id_by_name(&name).await?,
        };
        app.check_opted_out(&channel_id, Some(&user_id))?;
        read_available_user_logs(&app.db, &channel_id, &user_id).await?
    } else {
        app.check_opted_out(&channel_id, None)?;
        read_available_channel_logs(&app.db, &channel_id).await?
    };

    if !available_logs.is_empty() {
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
        ChannelIdType::Name => app.get_user_id_by_name(&channel).await?,
        ChannelIdType::Id => channel,
    };
    app.check_opted_out(&channel_id, None)?;

    let random_line = read_random_channel_line(&app.db, &channel_id).await?;
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

    let random_line = read_random_user_line(&app.db, &channel_id, &user_id).await?;
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

    let stream = db::search_user_logs(
        &app.db,
        &channel_id,
        &user_id,
        &search_params.q,
        logs_params,
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

    let names = db::get_user_name_history(&app.db, &user_id).await?;

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
        ChannelIdType::Name => app.get_user_id_by_name(&params.channel).await?,
        ChannelIdType::Id => params.channel.clone(),
    };
    let user_id = match params.user_id_type {
        UserIdType::Name => app.get_user_id_by_name(&params.user).await?,
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

fn normalize_day(year: i32, month: u32, day_raw: i32) -> Result<u32> {
    if let Some(date) = NaiveDate::from_ymd_opt(year, month, day_raw as u32) {
        Ok(date.day())
    } else {
        Err(Error::InvalidParam(format!(
            "Invalid date: {year}-{month:02}-{day_raw:02}"
        )))
    }
}

fn tier_mode_str(mode: TierMode) -> &'static str {
    match mode {
        TierMode::All => "all",
        TierMode::Online => "online",
        TierMode::Offline => "offline",
    }
}

fn spawn_supabase_tiers(
    app: &App,
    channel: &str,
    scope: &str,
    period_key: String,
    mode: TierMode,
    total_users: u64,
    total_messages: u64,
    total_unique: u64,
    entries: &[TierEntry],
) {
    if !app.config.enable_tier_snapshots {
        return;
    }

    let Some(url) = app.config.supabase_url.clone() else {
        return;
    };
    let Some(service_key) = app.config.supabase_service_key.clone() else {
        return;
    };

    let p_total_users = match i32::try_from(total_users) {
        Ok(v) => v,
        Err(_) => {
            warn!("supabase skip: total_users overflow ({}).", total_users);
            return;
        }
    };
    let p_total_messages = match i32::try_from(total_messages) {
        Ok(v) => v,
        Err(_) => {
            warn!(
                "supabase skip: total_messages overflow ({}).",
                total_messages
            );
            return;
        }
    };
    let p_total_unique_messages = match i32::try_from(total_unique) {
        Ok(v) => v,
        Err(_) => {
            warn!(
                "supabase skip: total_unique_messages overflow ({}).",
                total_unique
            );
            return;
        }
    };

    let entries_json = match serde_json::to_value(entries) {
        Ok(v) => v,
        Err(e) => {
            warn!("supabase skip: serialize entries failed: {:?}", e);
            return;
        }
    };

    let payload = TierSnapshotPayload {
        p_channel: channel.to_string(),
        p_scope: scope.to_string(),
        p_period_key: period_key,
        p_mode: tier_mode_str(mode).to_string(),
        p_total_users,
        p_total_messages,
        p_total_unique_messages,
        p_entries: entries_json,
    };

    let cfg = SupabaseConfig { url, service_key };
    tokio::spawn(async move {
        if let Err(e) = write_tier_snapshot(cfg, payload).await {
            warn!("supabase upsert failed: {:?}", e);
        }
    });
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SullyId {
    Str(String),
    Num(i64),
}

impl SullyId {
    fn into_string(self) -> String {
        match self {
            SullyId::Str(s) => s,
            SullyId::Num(n) => n.to_string(),
        }
    }
}

#[derive(Deserialize)]
struct SullySearchItem {
    value: SullyId,
    itemtype: i32,
}

#[derive(Deserialize)]
struct SullyStreamsRaw {
    #[serde(rename = "recordsTotal")]
    records_total: u32,
    data: Vec<SullyStreamRawEntry>,
}

#[derive(Deserialize)]
struct SullyStreamRawEntry {
    #[serde(rename = "streamId")]
    stream_id: String,
    #[serde(rename = "startDateTime")]
    start_iso: Option<String>,
    #[serde(rename = "starttime")]
    start_human: Option<String>,
    #[serde(rename = "endtime")]
    end_human: Option<String>,
    #[serde(rename = "length")]
    length: Option<f64>,
    gamesplayed: Option<String>,
}

async fn fetch_sully_id(http: &HttpClient, login: &str) -> Result<String> {
    let url = format!("https://sullygnome.com/api/standardsearch/{login}");
    let resp = http.get(&url).send().await.map_err(|e| {
        error!("sully id http error login={} err={:?}", login, e);
        Error::Internal
    })?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        error!(
            "sully id non-success status={} login={} body_snippet={}",
            status,
            login,
            body.chars().take(500).collect::<String>()
        );
        return Err(Error::NotFound);
    }
    let items: Vec<SullySearchItem> = serde_json::from_str(&body).map_err(|e| {
        error!(
            "sully id parse error login={} err={:?} body_snippet={}",
            login,
            e,
            body.chars().take(500).collect::<String>()
        );
        Error::Internal
    })?;
    let id = items
        .into_iter()
        .find(|it| it.itemtype == 1)
        .map(|it| it.value.into_string())
        .ok_or(Error::NotFound)?;
    Ok(id)
}

async fn fetch_sully_streams(
    http: &HttpClient,
    internal_id: &str,
    year: i32,
) -> Result<(u32, Vec<SullyStreamEntry>)> {
    let mut offset: u32 = 0;
    let limit: u32 = 2000;
    let mut streams = Vec::new();

    let total = loop {
        let url = format!(
            "https://sullygnome.com/api/tables/channeltables/streams/{year}/{internal_id}/%20/1/1/desc/{offset}/{limit}"
        );
        let resp_raw = http.get(&url).send().await.map_err(|e| {
            error!("sully streams http error url={} err={:?}", url, e);
            Error::Internal
        })?;
        let status = resp_raw.status();
        let body = resp_raw.text().await.unwrap_or_default();
        if !status.is_success() {
            error!(
                "sully streams non-success status={} url={} body_snippet={}",
                status,
                url,
                body.chars().take(500).collect::<String>()
            );
            return Err(Error::NotFound);
        }
        let parsed = parse_sully_body("", year, &body).map_err(|e| {
            error!(
                "sully streams parse error url={} err={:?} body_snippet={}",
                url,
                e,
                body.chars().take(500).collect::<String>()
            );
            Error::Internal
        })?;
        let total = parsed.total;
        let count_added = parsed.streams.len() as u32;
        streams.extend(parsed.streams);
        offset += limit;
        if offset >= total || count_added == 0 {
            break total;
        }
    };

    Ok((total, streams))
}

fn parse_sully_body(channel: &str, year: i32, body: &str) -> Result<SullyStreamsResponse> {
    // Try raw first
    if let Ok(raw) = serde_json::from_str::<SullyStreamsRaw>(body) {
        let streams = raw
            .data
            .into_iter()
            .map(|row| {
                let len_minutes = row.length.map(|v| v.round() as u32);
                SullyStreamEntry {
                    stream_id: row.stream_id,
                    start_iso: row.start_iso,
                    start_human: row.start_human,
                    end_human: row.end_human,
                    length_minutes: len_minutes,
                    gamesplayed: row.gamesplayed,
                }
            })
            .collect();
        return Ok(SullyStreamsResponse {
            channel: channel.to_string(),
            year,
            total: raw.records_total,
            streams,
        });
    }
    if let Ok(resp) = serde_json::from_str::<SullyStreamsResponse>(body) {
        return Ok(resp);
    }
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(body) {
        let records_total = val
            .get("recordsTotal")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let mut streams = Vec::new();
        if let Some(arr) = val.get("data").and_then(|v| v.as_array()) {
            for row in arr {
                let len_minutes = row
                    .get("length")
                    .and_then(|v| v.as_f64())
                    .map(|v| v.round() as u32);
                streams.push(SullyStreamEntry {
                    stream_id: row
                        .get("streamId")
                        .and_then(|v| v.as_i64())
                        .map(|v| v.to_string())
                        .or_else(|| {
                            row.get("streamId")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                        })
                        .unwrap_or_default(),
                    start_iso: row
                        .get("startDateTime")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    start_human: row
                        .get("starttime")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    end_human: row
                        .get("endtime")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    length_minutes: len_minutes,
                    gamesplayed: row
                        .get("gamesplayed")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                });
            }
        }
        return Ok(SullyStreamsResponse {
            channel: channel.to_string(),
            year,
            total: records_total,
            streams,
        });
    }
    Err(Error::NotFound)
}

fn sully_cache_path(channel: &str, year: i32) -> PathBuf {
    PathBuf::from("cache")
        .join("sullygnome")
        .join(format!("{channel}-{year}.json"))
}

fn read_sully_cache(channel: &str, year: i32) -> Option<SullyStreamsResponse> {
    let path = sully_cache_path(channel, year);
    let data = fs::read_to_string(path).ok()?;
    parse_sully_body(channel, year, &data).ok()
}

fn write_sully_cache(channel: &str, year: i32, resp: &SullyStreamsResponse) -> std::io::Result<()> {
    let path = sully_cache_path(channel, year);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let data = serde_json::to_string_pretty(resp)?;
    fs::write(path, data)
}

fn intervals_for_month(year: i32, month: u32, streams: &[SullyStreamEntry]) -> Vec<(i64, i64)> {
    let moscow = FixedOffset::east_opt(3 * 3600).unwrap();
    let month_start = NaiveDate::from_ymd_opt(year, month, 1)
        .unwrap()
        .and_time(NaiveTime::default())
        .and_local_timezone(moscow)
        .unwrap();
    let month_end = month_start.checked_add_months(Months::new(1)).unwrap();

    streams
        .iter()
        .filter_map(|s| {
            let start_iso = s.start_iso.as_ref()?;
            let length = s.length_minutes?;
            let start = DateTime::parse_from_rfc3339(start_iso)
                .ok()?
                .with_timezone(&moscow);
            let end = start + chrono::Duration::minutes(length as i64);
            let from = std::cmp::max(start, month_start);
            let to = std::cmp::min(end, month_end);
            if to > from {
                Some((from.timestamp_millis(), to.timestamp_millis()))
            } else {
                None
            }
        })
        .collect()
}

fn intervals_for_day(
    year: i32,
    month: u32,
    day: u32,
    streams: &[SullyStreamEntry],
) -> Vec<(i64, i64)> {
    let moscow = FixedOffset::east_opt(3 * 3600).unwrap();
    let day_start = NaiveDate::from_ymd_opt(year, month, day)
        .unwrap()
        .and_time(NaiveTime::default())
        .and_local_timezone(moscow)
        .unwrap();
    let day_end = day_start.checked_add_days(Days::new(1)).unwrap();

    streams
        .iter()
        .filter_map(|s| {
            let start_iso = s.start_iso.as_ref()?;
            let length = s.length_minutes?;
            let start = DateTime::parse_from_rfc3339(start_iso)
                .ok()?
                .with_timezone(&moscow);
            let end = start + chrono::Duration::minutes(length as i64);
            let from = std::cmp::max(start, day_start);
            let to = std::cmp::min(end, day_end);
            if to > from {
                Some((from.timestamp_millis(), to.timestamp_millis()))
            } else {
                None
            }
        })
        .collect()
}
