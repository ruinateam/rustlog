//! Errors of the legacy API, answered with a plain text body.

use crate::{domain::opt_out::OptedOut, storage, twitch};
use aide::{OperationOutput, openapi::MediaType};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::num::ParseIntError;
use tracing::error;
use twitch_api::helix::ClientRequestError;

/// The messages are part of the frozen legacy API.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Twitch API error: {0}")]
    Helix(Box<ClientRequestError<reqwest::Error>>),
    #[error("Int parse error: {0}")]
    ParseInt(#[from] ParseIntError),
    #[error("Invalid param: {0}")]
    InvalidParam(String),
    #[error("Internal error")]
    Internal,
    #[error("Twitch token is not ready yet")]
    TwitchTokenUnavailable,
    #[error("Database error")]
    Database(Box<clickhouse::error::Error>),
    #[error("The requested channel has opted out of being logged")]
    ChannelOptedOut,
    #[error("The requested user has opted out of being logged")]
    UserOptedOut,
    #[error("Not found")]
    NotFound,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl From<storage::Error> for Error {
    fn from(error: storage::Error) -> Self {
        match error {
            storage::Error::Database(error) => Self::Database(error),
            storage::Error::NotFound => Self::NotFound,
        }
    }
}

impl From<twitch::Error> for Error {
    fn from(error: twitch::Error) -> Self {
        match error {
            twitch::Error::TokenUnavailable => Self::TwitchTokenUnavailable,
            twitch::Error::Helix(error) => Self::Helix(error),
            twitch::Error::NotFound => Self::NotFound,
        }
    }
}

impl From<OptedOut> for Error {
    fn from(opted_out: OptedOut) -> Self {
        match opted_out {
            OptedOut::Channel => Self::ChannelOptedOut,
            OptedOut::User => Self::UserOptedOut,
        }
    }
}

impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        error!("Error: {err}");
        Self::Internal
    }
}

impl Error {
    pub fn status(&self) -> StatusCode {
        match self {
            Error::Helix(_) | Error::Internal | Error::Database(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            Error::TwitchTokenUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            Error::ParseInt(_) | Error::InvalidParam(_) => StatusCode::BAD_REQUEST,
            Error::ChannelOptedOut | Error::UserOptedOut => StatusCode::FORBIDDEN,
            Error::NotFound => StatusCode::NOT_FOUND,
        }
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        if let Error::Database(error) = &self {
            error!("DB error: {error}");
        }

        (self.status(), self.to_string()).into_response()
    }
}

impl OperationOutput for Error {
    type Inner = Self;

    fn operation_response(
        _: &mut aide::generate::GenContext,
        _: &mut aide::openapi::Operation,
    ) -> Option<aide::openapi::Response> {
        Some(aide::openapi::Response {
            description: "Error response".into(),
            content: [("text/plain".into(), MediaType::default())]
                .into_iter()
                .collect(),
            ..Default::default()
        })
    }

    fn inferred_responses(
        ctx: &mut aide::generate::GenContext,
        operation: &mut aide::openapi::Operation,
    ) -> Vec<(Option<u16>, aide::openapi::Response)> {
        if let Some(res) = Self::operation_response(ctx, operation) {
            vec![
                (
                    Some(400),
                    aide::openapi::Response {
                        description: "The request is invalid".to_owned(),
                        ..res.clone()
                    },
                ),
                (
                    Some(403),
                    aide::openapi::Response {
                        description: "Channel or user has opted out".to_owned(),
                        ..res.clone()
                    },
                ),
                (
                    Some(404),
                    aide::openapi::Response {
                        description: "The requested data was not found".to_owned(),
                        ..res.clone()
                    },
                ),
                (
                    Some(500),
                    aide::openapi::Response {
                        description: "An internal server error occured".to_owned(),
                        ..res
                    },
                ),
            ]
        } else {
            Vec::new()
        }
    }
}
