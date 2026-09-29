//! Message counts and name history.

use super::{resolve_channel, resolve_user_params};
use crate::{
    app::App,
    storage::stats,
    web::legacy::{
        dto::{
            ChannelLogsStats, LogRangeParams, LogsPathChannel, PreviousName, UserLogPathParams,
            UserLogsStats, UserNameHistoryParam,
        },
        error::Result,
    },
};
use axum::{
    Json,
    extract::{Path, Query, State},
};

pub async fn get_channel_stats(
    Path(LogsPathChannel {
        channel_id_type,
        channel,
    }): Path<LogsPathChannel>,
    Query(range_params): Query<LogRangeParams>,
    app: State<App>,
) -> Result<Json<ChannelLogsStats>> {
    let channel_id = resolve_channel(&app, channel_id_type, &channel).await?;
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

pub async fn get_user_name_history(
    app: State<App>,
    Path(UserNameHistoryParam { user_id }): Path<UserNameHistoryParam>,
) -> Result<Json<Vec<PreviousName>>> {
    app.check_user_opted_out(&user_id)?;

    let names: Vec<_> = stats::get_user_name_history(&app.db, &user_id)
        .await?
        .into_iter()
        .map(PreviousName::from)
        .collect();

    Ok(Json(names))
}
