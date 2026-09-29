//! Admin endpoints, behind the `X-Api-Key` header, and opt-out codes.

use super::{extract::Path, params::ChannelPath, problem::ApiProblem};
use crate::{
    app::{App, BotMessage},
    web::AdminApiKey,
};
use aide::{
    OperationInput, OperationOutput,
    generate::GenContext,
    openapi::{
        HeaderStyle, Operation, Parameter, ParameterData, ParameterSchemaOrContent, ReferenceOr,
        SchemaObject,
    },
};
use axum::{
    Extension, Json,
    extract::{FromRequestParts, State},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, NoContent, Response},
};
use chrono::{DateTime, Utc};
use rand::{RngExt, distr::Alphanumeric, rng};
use schemars::JsonSchema;
use serde::Serialize;
use std::time::Duration;
use tokio::sync::mpsc::Sender;
use tracing::debug;

const API_KEY_HEADER: &str = "X-Api-Key";
const OPT_OUT_CODE_LIFETIME: Duration = Duration::from_secs(60);

/// Proof that the request carries the configured admin API key.
pub struct Admin;

impl<S: Send + Sync> FromRequestParts<S> for Admin {
    type Rejection = ApiProblem;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let configured = parts
            .extensions
            .get::<AdminApiKey>()
            .and_then(|AdminApiKey(key)| key.clone());
        let given = parts
            .headers
            .get(API_KEY_HEADER)
            .and_then(|value| value.to_str().ok());

        match (configured, given) {
            (Some(configured), Some(given)) if equal_in_constant_time(&configured, given) => {
                Ok(Self)
            }
            _ => Err(ApiProblem::unauthorized()),
        }
    }
}

impl OperationInput for Admin {
    fn operation_input(ctx: &mut GenContext, operation: &mut Operation) {
        let schema = ctx.schema.subschema_for::<String>();
        operation
            .parameters
            .push(ReferenceOr::Item(Parameter::Header {
                parameter_data: ParameterData {
                    name: API_KEY_HEADER.to_owned(),
                    description: Some("The configured admin API key.".to_owned()),
                    required: true,
                    deprecated: None,
                    format: ParameterSchemaOrContent::Schema(SchemaObject {
                        json_schema: schema,
                        external_docs: None,
                        example: None,
                    }),
                    example: None,
                    examples: Default::default(),
                    explode: None,
                    extensions: Default::default(),
                },
                style: HeaderStyle::Simple,
            }));

        if let Some(response) = ApiProblem::operation_response(ctx, operation) {
            operation
                .responses
                .get_or_insert_with(Default::default)
                .responses
                .insert(
                    aide::openapi::StatusCode::Code(StatusCode::UNAUTHORIZED.as_u16()),
                    ReferenceOr::Item(response),
                );
        }
    }
}

/// Compares without revealing through timing how much of `given` matches.
fn equal_in_constant_time(expected: &str, given: &str) -> bool {
    expected.len() == given.len()
        && expected
            .bytes()
            .zip(given.bytes())
            .fold(0, |difference, (a, b)| difference | (a ^ b))
            == 0
}

pub async fn join_channel(
    _: Admin,
    State(app): State<App>,
    Extension(bot): Extension<Sender<BotMessage>>,
    Path(path): Path<ChannelPath>,
) -> Result<NoContent, ApiProblem> {
    let login = channel_login(&app, path.channel_id.as_str()).await?;
    bot.send(BotMessage::JoinChannels(vec![login]))
        .await
        .map_err(|error| ApiProblem::internal(&error))?;
    Ok(NoContent)
}

pub async fn leave_channel(
    _: Admin,
    State(app): State<App>,
    Extension(bot): Extension<Sender<BotMessage>>,
    Path(path): Path<ChannelPath>,
) -> Result<NoContent, ApiProblem> {
    let login = channel_login(&app, path.channel_id.as_str()).await?;
    bot.send(BotMessage::PartChannels(vec![login]))
        .await
        .map_err(|error| ApiProblem::internal(&error))?;
    Ok(NoContent)
}

/// The bot joins and leaves channels by login.
async fn channel_login(app: &App, channel_id: &str) -> Result<String, ApiProblem> {
    app.twitch
        .get_users(vec![channel_id.to_owned()], Vec::new(), false)
        .await?
        .into_values()
        .next()
        .ok_or_else(|| ApiProblem::not_found("Twitch does not know the channel"))
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OptOutCode {
    /// Write `!rustlog optout <code>` in a logged chat to opt out.
    pub code: String,
    pub expires_at: DateTime<Utc>,
}

pub async fn create_opt_out_code(State(app): State<App>) -> Created<Json<OptOutCode>> {
    let code: String = (0..5).map(|_| rng().sample(Alphanumeric) as char).collect();
    app.optout_codes.insert(code.clone());

    tokio::spawn({
        let codes = app.optout_codes.clone();
        let code = code.clone();
        async move {
            tokio::time::sleep(OPT_OUT_CODE_LIFETIME).await;
            if codes.remove(&code).is_some() {
                debug!(%code, "opt-out code expired");
            }
        }
    });

    let expires_at = Utc::now() + OPT_OUT_CODE_LIFETIME;
    Created(Json(OptOutCode { code, expires_at }))
}

/// A `201 Created` response.
pub struct Created<T>(T);

impl<T: IntoResponse> IntoResponse for Created<T> {
    fn into_response(self) -> Response {
        (StatusCode::CREATED, self.0).into_response()
    }
}

impl<T: OperationOutput> OperationOutput for Created<T> {
    type Inner = T::Inner;

    fn operation_response(
        ctx: &mut GenContext,
        operation: &mut Operation,
    ) -> Option<aide::openapi::Response> {
        T::operation_response(ctx, operation)
    }

    fn inferred_responses(
        ctx: &mut GenContext,
        operation: &mut Operation,
    ) -> Vec<(Option<u16>, aide::openapi::Response)> {
        T::operation_response(ctx, operation)
            .map(|response| vec![(Some(StatusCode::CREATED.as_u16()), response)])
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::equal_in_constant_time;

    #[test]
    fn compares_keys() {
        assert!(equal_in_constant_time("secret", "secret"));
        assert!(!equal_in_constant_time("secret", "secreT"));
        assert!(!equal_in_constant_time("secret", "secret2"));
        assert!(!equal_in_constant_time("secret", ""));
    }
}
