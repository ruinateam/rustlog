//! Chat logs of a channel or of one user in it, by date or range, plus
//! random lines, search and the list of available dates.

use super::{normalize_month, parse_date, resolve_channel, resolve_user_params};
use crate::{
    cache_control::no_cache,
    legacy::{
        dto::{
            AvailableLogDate, AvailableLogs, AvailableLogsParams, ChannelLogsByDatePath,
            ChannelParam, LogRangeParams, LogsParams, LogsPathChannel, LogsPathDate, SearchParams,
            UserLogPathParams, UserLogsDatePath, UserParam,
        },
        error::{Error, Result},
    },
    logs_response::LogsResponse,
};
use aide::axum::IntoApiResponse;
use axum::{
    Json,
    extract::{Path, Query, RawQuery, State},
    response::{IntoResponse, Redirect, Response},
};
use chrono::{DateTime, Days, Months, NaiveDate, NaiveTime, Utc};
use rustlog_app::App;
use rustlog_storage::{availability, logs, stream::LogsStream};

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
    let channel_id = resolve_channel(&app, channel_id_type, &channel).await?;

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

pub async fn get_channel_logs_by_date(
    app: State<App>,
    Path(channel_log_params): Path<ChannelLogsByDatePath>,
    Query(logs_params): Query<LogsParams>,
) -> Result<impl IntoApiResponse> {
    let channel_id = resolve_channel(
        &app,
        channel_log_params.channel_info.channel_id_type,
        &channel_log_params.channel_info.channel,
    )
    .await?;

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
) -> Result<impl IntoApiResponse + use<>> {
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
    let cache = no_cache();

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
) -> Result<impl IntoApiResponse + use<>> {
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
    let cache = no_cache();

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
        Ok((no_cache(), Json(AvailableLogs { available_logs })))
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
    let channel_id = resolve_channel(&app, channel_id_type, &channel).await?;
    app.check_opted_out(&channel_id, None)?;

    let random_line = logs::read_random_channel_line(&app.db, &channel_id).await?;
    let stream = LogsStream::new_provided(vec![random_line])?;

    let logs = LogsResponse {
        stream,
        response_type: logs_params.response_type(),
    };
    Ok((no_cache(), logs))
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
    Ok((no_cache(), logs))
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
