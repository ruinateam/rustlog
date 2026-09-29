//! Errors of the v2 API: `application/problem+json` bodies in the shape of
//! RFC 9457, with a machine-readable `code`.

use crate::{domain::opt_out::OptedOut, storage, twitch};
use aide::OperationOutput;
use axum::{
    Json,
    http::{HeaderValue, StatusCode, header::CONTENT_TYPE},
    response::{IntoResponse, Response},
};
use schemars::JsonSchema;
use serde::Serialize;
use tracing::error;

#[derive(Debug, Serialize, JsonSchema)]
pub struct ApiProblem {
    /// What went wrong, for program logic.
    pub code: ProblemCode,
    /// The HTTP status code.
    pub status: u16,
    /// A short summary of the code, for people.
    pub title: &'static str,
    /// What exactly is wrong with this request, for people.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProblemCode {
    InvalidRequest,
    Unauthorized,
    OptedOut,
    NotFound,
    MethodNotAllowed,
    UpstreamUnavailable,
    InternalError,
}

impl ProblemCode {
    fn status(self) -> StatusCode {
        match self {
            Self::InvalidRequest => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::OptedOut => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            Self::UpstreamUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::InternalError => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::InvalidRequest => "The request is invalid",
            Self::Unauthorized => "The API key is missing or wrong",
            Self::OptedOut => "The requested data is unavailable",
            Self::NotFound => "The requested data was not found",
            Self::MethodNotAllowed => "The endpoint does not support this method",
            Self::UpstreamUnavailable => "Twitch cannot be queried yet",
            Self::InternalError => "An internal error occurred",
        }
    }
}

impl ApiProblem {
    pub fn new(code: ProblemCode, detail: Option<String>) -> Self {
        Self {
            code,
            status: code.status().as_u16(),
            title: code.title(),
            detail,
        }
    }

    pub fn invalid(detail: impl Into<String>) -> Self {
        Self::new(ProblemCode::InvalidRequest, Some(detail.into()))
    }

    pub fn not_found(detail: impl Into<String>) -> Self {
        Self::new(ProblemCode::NotFound, Some(detail.into()))
    }

    pub fn method_not_allowed() -> Self {
        Self::new(ProblemCode::MethodNotAllowed, None)
    }

    pub fn unauthorized() -> Self {
        Self::new(ProblemCode::Unauthorized, None)
    }

    /// Logs the cause, which the client does not get to see.
    pub fn internal(cause: &dyn std::error::Error) -> Self {
        error!(error = %cause, "request failed");
        Self::new(ProblemCode::InternalError, None)
    }
}

impl From<storage::Error> for ApiProblem {
    fn from(error: storage::Error) -> Self {
        match error {
            storage::Error::NotFound => Self::new(ProblemCode::NotFound, None),
            storage::Error::Database(_) => Self::internal(&error),
        }
    }
}

impl From<twitch::Error> for ApiProblem {
    fn from(error: twitch::Error) -> Self {
        match error {
            twitch::Error::NotFound => Self::not_found("Twitch does not know the user"),
            twitch::Error::TokenUnavailable => Self::new(ProblemCode::UpstreamUnavailable, None),
            twitch::Error::Helix(_) => Self::internal(&error),
        }
    }
}

impl From<OptedOut> for ApiProblem {
    fn from(opted_out: OptedOut) -> Self {
        let detail = match opted_out {
            OptedOut::Channel => "the channel opted out of logging",
            OptedOut::User => "the user opted out of logging",
        };
        Self::new(ProblemCode::OptedOut, Some(detail.to_owned()))
    }
}

impl IntoResponse for ApiProblem {
    fn into_response(self) -> Response {
        (
            self.code.status(),
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
