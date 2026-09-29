use super::{
    dto::{
        AvailabilityQuery, AvailableLogDate, AvailableLogs, ResolveUserQuery, ResolvedUser,
        UserLogsQuery,
    },
    problem::ApiProblem,
};
use crate::{
    app::App,
    domain::logs::LogsQuery,
    storage::{availability, logs},
    web::{cache_control::no_cache, logs_response::LogsResponse},
};
use aide::axum::IntoApiResponse;
use axum::{
    Json,
    extract::{Path, Query, State},
};

pub async fn resolve_user(
    State(app): State<App>,
    Query(query): Query<ResolveUserQuery>,
) -> Result<Json<ResolvedUser>, ApiProblem> {
    let login = query.login.trim();
    if login.is_empty() {
        return Err(ApiProblem::invalid("login must not be empty"));
    }

    let id = app.twitch.get_user_id_by_name(login).await?;
    Ok(Json(ResolvedUser {
        id,
        login: login.to_owned(),
    }))
}

pub async fn availability(
    State(app): State<App>,
    Path(channel_id): Path<String>,
    Query(query): Query<AvailabilityQuery>,
) -> Result<impl IntoApiResponse, ApiProblem> {
    let available_logs = if let Some(user_id) = query.user_id {
        app.check_opted_out(&channel_id, Some(&user_id))?;
        availability::read_available_user_logs(&app.db, &channel_id, &user_id).await?
    } else {
        app.check_opted_out(&channel_id, None)?;
        availability::read_available_channel_logs(&app.db, &channel_id).await?
    };

    let available_logs = available_logs
        .into_iter()
        .map(AvailableLogDate::from)
        .collect();
    Ok((no_cache(), Json(AvailableLogs { available_logs })))
}

pub async fn user_logs(
    State(app): State<App>,
    Path((channel_id, user_id)): Path<(String, String)>,
    Query(query): Query<UserLogsQuery>,
) -> Result<impl IntoApiResponse, ApiProblem> {
    if query.to <= query.from {
        return Err(ApiProblem::invalid("to must be later than from"));
    }
    if query.limit == Some(0) {
        return Err(ApiProblem::invalid("limit must be at least 1"));
    }

    app.check_opted_out(&channel_id, Some(&user_id))?;

    let logs_query = LogsQuery {
        reverse: query.reverse,
        limit: query.limit,
        offset: query.offset,
    };
    let stream = logs::read_user(
        &app.db,
        &channel_id,
        &user_id,
        logs_query,
        &app.flush_buffer,
        (query.from, query.to),
    )
    .await?;

    Ok((
        no_cache(),
        LogsResponse {
            stream,
            response_type: query.format.into(),
        },
    ))
}
