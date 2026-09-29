//! Errors of the v2 API, answered as `application/problem+json`.

use aide::OperationOutput;
use axum::{
    Json,
    http::{HeaderValue, StatusCode, header::CONTENT_TYPE},
    response::{IntoResponse, Response},
};
use rustlog_domain::opt_out::OptedOut;
use rustlog_storage::{self as storage};
use rustlog_twitch::{self as twitch};
use schemars::JsonSchema;
use serde::Serialize;
use tracing::error;

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiProblem {
    pub code: String,
    pub status: u16,
    pub title: String,
}

impl ApiProblem {
    fn new(status: StatusCode, code: &str, title: &str) -> Self {
        Self {
            code: code.to_owned(),
            status: status.as_u16(),
            title: title.to_owned(),
        }
    }

    /// The reason is not exposed yet: v2 answers every invalid request the
    /// same way.
    pub fn invalid(_reason: &str) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "The request is invalid",
        )
    }

    fn not_found() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "The requested data was not found",
        )
    }

    /// Logs the cause, which the client does not get to see.
    fn internal(cause: &dyn std::error::Error) -> Self {
        error!(error = %cause, "request failed");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "An internal error occurred",
        )
    }
}

impl From<storage::Error> for ApiProblem {
    fn from(error: storage::Error) -> Self {
        match error {
            storage::Error::NotFound => Self::not_found(),
            storage::Error::Database(_) => Self::internal(&error),
        }
    }
}

impl From<twitch::Error> for ApiProblem {
    fn from(error: twitch::Error) -> Self {
        match error {
            twitch::Error::NotFound => Self::not_found(),
            twitch::Error::TokenUnavailable => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "upstream_unavailable",
                "The Twitch token is not ready",
            ),
            twitch::Error::Helix(_) => Self::internal(&error),
        }
    }
}

impl From<OptedOut> for ApiProblem {
    fn from(_: OptedOut) -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "opted_out",
            "The requested data is unavailable",
        )
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
