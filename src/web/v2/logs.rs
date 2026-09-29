//! Chat logs: the days and months that have logs, ranges of messages,
//! random messages and search.

use super::{
    extract::{Path, Query},
    params::{ChannelPath, ChannelUserPath, Format, Paging, Range},
    problem::ApiProblem,
};
use crate::{
    app::App,
    domain::logs::LogDate,
    storage::{self, availability, logs, stream::LogsStream},
    web::{
        cache_control::Cached,
        logs_response::{
            LogsResponse,
            message::{BasicMessage, FullMessage},
        },
    },
};
use aide::{
    OperationOutput,
    generate::GenContext,
    openapi::{MediaType, Operation, Response as ApiResponse, SchemaObject},
};
use axum::{
    Json,
    extract::State,
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;
use indexmap::IndexMap;
use schemars::{JsonSchema, Schema, json_schema};
use serde::{Deserialize, Serialize};

#[derive(Serialize, JsonSchema)]
pub struct LogDates {
    /// UTC days with messages, newest first.
    pub dates: Vec<NaiveDate>,
}

#[derive(Serialize, JsonSchema)]
pub struct LogMonths {
    /// UTC months with messages as `YYYY-MM`, newest first.
    pub months: Vec<String>,
}

pub async fn channel_log_dates(
    State(app): State<App>,
    Path(path): Path<ChannelPath>,
) -> Result<Cached<Json<LogDates>>, ApiProblem> {
    let channel_id = path.channel_id.as_str();
    app.check_opted_out(channel_id, None)?;

    let dates = availability::read_available_channel_logs(&app.db, channel_id)
        .await?
        .into_iter()
        .filter_map(day_of)
        .collect();
    Ok(Cached::no_cache(Json(LogDates { dates })))
}

pub async fn user_log_months(
    State(app): State<App>,
    Path(path): Path<ChannelUserPath>,
) -> Result<Cached<Json<LogMonths>>, ApiProblem> {
    let (channel_id, user_id) = (path.channel_id.as_str(), path.user_id.as_str());
    app.check_opted_out(channel_id, Some(user_id))?;

    let months = availability::read_available_user_logs(&app.db, channel_id, user_id)
        .await?
        .into_iter()
        .map(|date| format!("{:04}-{:02}", date.year, date.month))
        .collect();
    Ok(Cached::no_cache(Json(LogMonths { months })))
}

fn day_of(date: LogDate) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(date.year.into(), date.month.into(), date.day?.into())
}

pub async fn channel_logs(
    State(app): State<App>,
    Path(path): Path<ChannelPath>,
    Query(range): Query<Range>,
    Query(format): Query<Format>,
    Query(paging): Query<Paging>,
) -> Result<Cached<Messages>, ApiProblem> {
    let range = range.validate()?;
    let query = paging.validate()?;
    let channel_id = path.channel_id.as_str();
    app.check_opted_out(channel_id, None)?;

    let stream = logs::read_channel(
        &app.db,
        channel_id,
        query,
        &app.flush_buffer,
        (range.from, range.to),
    )
    .await?;
    Ok(messages(stream, format))
}

pub async fn user_logs(
    State(app): State<App>,
    Path(path): Path<ChannelUserPath>,
    Query(range): Query<Range>,
    Query(format): Query<Format>,
    Query(paging): Query<Paging>,
) -> Result<Cached<Messages>, ApiProblem> {
    let range = range.validate()?;
    let query = paging.validate()?;
    let (channel_id, user_id) = (path.channel_id.as_str(), path.user_id.as_str());
    app.check_opted_out(channel_id, Some(user_id))?;

    let stream = logs::read_user(
        &app.db,
        channel_id,
        user_id,
        query,
        &app.flush_buffer,
        (range.from, range.to),
    )
    .await?;
    Ok(messages(stream, format))
}

pub async fn random_channel_message(
    State(app): State<App>,
    Path(path): Path<ChannelPath>,
    Query(format): Query<Format>,
) -> Result<Cached<Messages>, ApiProblem> {
    let channel_id = path.channel_id.as_str();
    app.check_opted_out(channel_id, None)?;

    let message = logs::read_random_channel_line(&app.db, channel_id)
        .await
        .map_err(nothing_to_pick)?;
    Ok(messages(LogsStream::new_provided(vec![message])?, format))
}

pub async fn random_user_message(
    State(app): State<App>,
    Path(path): Path<ChannelUserPath>,
    Query(format): Query<Format>,
) -> Result<Cached<Messages>, ApiProblem> {
    let (channel_id, user_id) = (path.channel_id.as_str(), path.user_id.as_str());
    app.check_opted_out(channel_id, Some(user_id))?;

    let message = logs::read_random_user_line(&app.db, channel_id, user_id)
        .await
        .map_err(nothing_to_pick)?;
    Ok(messages(LogsStream::new_provided(vec![message])?, format))
}

fn nothing_to_pick(error: storage::Error) -> ApiProblem {
    match error {
        storage::Error::NotFound => ApiProblem::not_found("there are no messages to pick from"),
        error => error.into(),
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct SearchQuery {
    /// Text to find in the messages, ignoring case.
    pub q: String,
}

pub async fn search_user_logs(
    State(app): State<App>,
    Path(path): Path<ChannelUserPath>,
    Query(search): Query<SearchQuery>,
    Query(format): Query<Format>,
    Query(paging): Query<Paging>,
) -> Result<Cached<Messages>, ApiProblem> {
    let query = paging.validate()?;
    if search.q.trim().is_empty() {
        return Err(ApiProblem::invalid("`q` must not be empty"));
    }
    let (channel_id, user_id) = (path.channel_id.as_str(), path.user_id.as_str());
    app.check_opted_out(channel_id, Some(user_id))?;

    let stream = logs::search_user_logs(&app.db, channel_id, user_id, &search.q, query).await?;
    Ok(messages(stream, format))
}

/// Log rows can be withdrawn by an opt-out at any time, hence `no-cache`.
fn messages(stream: LogsStream, format: Format) -> Cached<Messages> {
    Cached::no_cache(Messages(LogsResponse {
        stream,
        response_type: format.format.into(),
    }))
}

/// Messages in the requested format, streamed.
pub struct Messages(LogsResponse);

impl IntoResponse for Messages {
    fn into_response(self) -> Response {
        self.0.into_response()
    }
}

/// The `basic-json` body; only for the documentation.
#[derive(JsonSchema)]
#[allow(dead_code)]
struct BasicMessages<'a> {
    messages: Vec<BasicMessage<'a>>,
}

/// The `full-json` body; only for the documentation.
#[derive(JsonSchema)]
#[allow(dead_code)]
struct FullMessages<'a> {
    messages: Vec<FullMessage<'a>>,
}

impl OperationOutput for Messages {
    type Inner = Self;

    fn operation_response(ctx: &mut GenContext, _: &mut Operation) -> Option<ApiResponse> {
        let basic = ctx.schema.subschema_for::<BasicMessages>();
        let full = ctx.schema.subschema_for::<FullMessages>();
        let line = ctx.schema.subschema_for::<BasicMessage>();
        let media_type = |schema: Schema| MediaType {
            schema: Some(SchemaObject {
                json_schema: schema,
                external_docs: None,
                example: None,
            }),
            ..MediaType::default()
        };

        Some(ApiResponse {
            description: "The messages in the requested `format`: `basic-json` or `full-json` \
                          as JSON, `ndjson` as one basic message per line, `text` and `raw` as \
                          text lines."
                .to_owned(),
            content: IndexMap::from([
                (
                    "application/json".to_owned(),
                    media_type(json_schema!({ "oneOf": [basic, full] })),
                ),
                ("application/x-ndjson".to_owned(), media_type(line)),
                (
                    "text/plain; charset=utf-8".to_owned(),
                    media_type(json_schema!({ "type": "string" })),
                ),
            ]),
            ..ApiResponse::default()
        })
    }

    fn inferred_responses(
        ctx: &mut GenContext,
        operation: &mut Operation,
    ) -> Vec<(Option<u16>, ApiResponse)> {
        Self::operation_response(ctx, operation)
            .map(|response| vec![(Some(200), response)])
            .unwrap_or_default()
    }
}
