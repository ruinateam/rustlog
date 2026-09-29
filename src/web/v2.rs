use super::{
    handlers::no_cache_header,
    responders::logs::{JsonResponseType, LogsResponse, LogsResponseType},
    schema::{AvailableLogDate, AvailableLogs},
};
use crate::{
    app::App,
    domain::logs::LogsQuery,
    domain::opt_out::OptedOut,
    storage,
    storage::{availability, logs},
    twitch,
    web::error::Error,
};
use aide::{
    axum::{routing::get_with, ApiRouter, IntoApiResponse},
    openapi::OpenApi,
    scalar::Scalar,
    OperationOutput,
};
use axum::{
    extract::{Path, Query, State},
    http::{header::CONTENT_TYPE, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
    Extension, Json,
};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};
use tracing::error;

const SPEC_URL: &str = "/api/v2/openapi.json";

#[derive(Clone)]
pub struct V2OpenApi(pub Arc<OpenApi>);

pub fn router() -> ApiRouter<App> {
    ApiRouter::new()
        .api_route(
            "/users/resolve",
            get_with(resolve_user, |op| {
                op.id("resolveUser")
                    .tag("Users")
                    .summary("Resolve a Twitch login")
                    .description("Resolve one Twitch login to the canonical numeric user id.")
            }),
        )
        .api_route(
            "/channels/{channel_id}/availability",
            get_with(availability, |op| {
                op.id("getChannelAvailability")
                    .tag("Logs")
                    .summary("List available log periods")
                    .description("List day buckets for a channel or month buckets for one canonical user id.")
            }),
        )
        .api_route(
            "/channels/{channel_id}/users/{user_id}/logs",
            get_with(user_logs, |op| {
                op.id("getUserLogs")
                    .tag("Logs")
                    .summary("Get user logs for an explicit range")
                    .description("Both RFC 3339 range endpoints are required. `from` is inclusive and `to` is exclusive.")
            }),
        )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResolveUserQuery {
    pub login: String,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedUser {
    pub id: String,
    pub login: String,
}

async fn resolve_user(
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AvailabilityQuery {
    pub user_id: Option<String>,
}

async fn availability(
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
    Ok((no_cache_header(), Json(AvailableLogs { available_logs })))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LogFormat {
    BasicJson,
    FullJson,
    Ndjson,
    Text,
    Raw,
}

impl Default for LogFormat {
    fn default() -> Self {
        Self::BasicJson
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserLogsQuery {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    #[serde(default)]
    pub format: LogFormat,
    #[serde(default)]
    pub reverse: bool,
    #[schemars(range(min = 1))]
    pub limit: Option<u64>,
    #[schemars(range(min = 0))]
    pub offset: Option<u64>,
}

async fn user_logs(
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
    let response_type = match query.format {
        LogFormat::BasicJson => LogsResponseType::Json(JsonResponseType::Basic),
        LogFormat::FullJson => LogsResponseType::Json(JsonResponseType::Full),
        LogFormat::Ndjson => LogsResponseType::NdJson,
        LogFormat::Text => LogsResponseType::Text,
        LogFormat::Raw => LogsResponseType::Raw,
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
        no_cache_header(),
        LogsResponse {
            stream,
            response_type,
        },
    ))
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiProblem {
    pub code: String,
    pub status: u16,
    pub title: String,
}

impl ApiProblem {
    fn invalid(_: &str) -> Self {
        Self {
            code: "invalid_request".to_owned(),
            status: StatusCode::BAD_REQUEST.as_u16(),
            title: "The request is invalid".to_owned(),
        }
    }
}

// Errors of the lower layers reach v2 through the legacy `Error`, which knows
// how they map to HTTP.

impl From<storage::Error> for ApiProblem {
    fn from(error: storage::Error) -> Self {
        Error::from(error).into()
    }
}

impl From<twitch::Error> for ApiProblem {
    fn from(error: twitch::Error) -> Self {
        Error::from(error).into()
    }
}

impl From<OptedOut> for ApiProblem {
    fn from(opted_out: OptedOut) -> Self {
        Error::from(opted_out).into()
    }
}

impl From<Error> for ApiProblem {
    fn from(error_value: Error) -> Self {
        let (status, code, title) = match error_value {
            Error::ParseInt(_) | Error::InvalidParam(_) => (
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "The request is invalid",
            ),
            Error::ChannelOptedOut | Error::UserOptedOut => (
                StatusCode::FORBIDDEN,
                "opted_out",
                "The requested data is unavailable",
            ),
            Error::NotFound => (
                StatusCode::NOT_FOUND,
                "not_found",
                "The requested data was not found",
            ),
            Error::TwitchTokenUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "upstream_unavailable",
                "The Twitch token is not ready",
            ),
            Error::Helix(_) | Error::Internal | Error::Database(_) => {
                error!("v2 request failed: {error_value}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "An internal error occurred",
                )
            }
        };

        Self {
            code: code.to_owned(),
            status: status.as_u16(),
            title: title.to_owned(),
        }
    }
}

impl IntoResponse for ApiProblem {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (
            status,
            [(
                CONTENT_TYPE,
                HeaderValue::from_static("application/problem+json"),
            )],
            Json(self),
        )
            .into_response()
    }
}

impl OperationOutput for ApiProblem {
    type Inner = Self;

    fn operation_response(
        ctx: &mut aide::generate::GenContext,
        operation: &mut aide::openapi::Operation,
    ) -> Option<aide::openapi::Response> {
        let mut response = Json::<Self>::operation_response(ctx, operation)?;
        response.description = "Problem response".to_owned();
        if let Some(content) = response.content.shift_remove("application/json") {
            response
                .content
                .insert("application/problem+json".to_owned(), content);
        }
        Some(response)
    }

    fn inferred_responses(
        ctx: &mut aide::generate::GenContext,
        operation: &mut aide::openapi::Operation,
    ) -> Vec<(Option<u16>, aide::openapi::Response)> {
        let Some(response) = Self::operation_response(ctx, operation) else {
            return Vec::new();
        };

        [400, 403, 404, 500, 503]
            .into_iter()
            .map(|status| (Some(status), response.clone()))
            .collect()
    }
}

pub async fn serve_openapi(Extension(api): Extension<V2OpenApi>) -> impl IntoApiResponse {
    Json(api.0.as_ref()).into_response()
}

pub async fn scalar_page() -> Html<&'static str> {
    static HTML: OnceLock<&'static str> = OnceLock::new();
    Html(HTML.get_or_init(|| {
        Box::leak(
            Scalar::new(SPEC_URL)
                .with_title("Rustlog API v2")
                .html()
                .into_boxed_str(),
        )
    }))
}
