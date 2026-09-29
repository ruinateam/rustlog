//! Admin routes, behind the `X-Api-Key` header.

mod firehose;

use crate::legacy::error::Error;
use aide::{
    axum::{
        ApiRouter,
        routing::{get_with, post_with},
    },
    openapi::{
        HeaderStyle, Parameter, ParameterData, ParameterSchemaOrContent, ReferenceOr, SchemaObject,
    },
    transform::TransformOperation,
};
use axum::http::StatusCode;
use axum::{
    Extension, Json,
    extract::{Request, State},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use rustlog_app::{App, BotMessage};
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

pub fn router() -> ApiRouter<App> {
    ApiRouter::new()
        .api_route(
            "/channels",
            post_with(add_channels, |mut op| {
                admin_auth_doc(&mut op);
                op.summary("Join channels for live logging")
                    .tag("Admin")
                    .description("Join the specified channels")
            })
            .delete_with(remove_channels, |mut op| {
                admin_auth_doc(&mut op);
                op.summary("Leave channels and stop live logging")
                    .tag("Admin")
                    .description("Leave the specified channels")
            }),
        )
        .api_route(
            "/firehose",
            get_with(firehose::firehose, |mut op| {
                admin_auth_doc(&mut op);
                op.summary("Stream live accepted chat events")
                    .tag("Admin")
                    .description("Open a WebSocket feed after authenticating with `X-Api-Key`. Messages are live, at-least-once deliveries accepted by the writer queue; reconnect and replay through the HTTP logs API after a `1013` close.")
            }),
        )
        .route_layer(middleware::from_fn(admin_auth))
}

/// The configured admin API key, provided to [`admin_auth`] as a request
/// extension so that the routes can be built without the app state.
#[derive(Clone)]
pub struct AdminApiKey(pub Option<Arc<str>>);

async fn admin_auth(
    Extension(AdminApiKey(admin_key)): Extension<AdminApiKey>,
    request: Request,
    next: Next,
) -> Result<Response, impl IntoResponse> {
    if let Some(admin_key) = admin_key
        && request
            .headers()
            .get("X-Api-Key")
            .and_then(|value| value.to_str().ok())
            == Some(&*admin_key)
    {
        let response = next.run(request).await;
        return Ok(response);
    }

    Err((StatusCode::FORBIDDEN, "No, I don't think so"))
}

fn admin_auth_doc(op: &mut TransformOperation) {
    let schema = aide::generate::in_context(|ctx| ctx.schema.subschema_for::<String>());

    op.inner_mut()
        .parameters
        .push(ReferenceOr::Item(Parameter::Header {
            parameter_data: ParameterData {
                name: "X-Api-Key".to_owned(),
                description: Some("Configured admin API key".to_owned()),
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
}

#[derive(Deserialize, JsonSchema)]
pub struct ChannelsRequest {
    /// List of channel ids
    pub channels: Vec<String>,
}

async fn add_channels(
    Extension(bot_tx): Extension<Sender<BotMessage>>,
    app: State<App>,
    Json(ChannelsRequest { channels }): Json<ChannelsRequest>,
) -> Result<(), Error> {
    let users = app.twitch.get_users(channels, vec![], false).await?;
    let names = users.into_values().collect();

    bot_tx.send(BotMessage::JoinChannels(names)).await.unwrap();

    Ok(())
}

async fn remove_channels(
    Extension(bot_tx): Extension<Sender<BotMessage>>,
    app: State<App>,
    Json(ChannelsRequest { channels }): Json<ChannelsRequest>,
) -> Result<(), Error> {
    let users = app.twitch.get_users(channels, vec![], false).await?;
    let names = users.into_values().collect();

    bot_tx.send(BotMessage::PartChannels(names)).await.unwrap();

    Ok(())
}
