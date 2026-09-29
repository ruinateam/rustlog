//! The legacy OpenAPI document at `/openapi.json`.

use aide::{axum::IntoApiResponse, openapi::OpenApi};
use axum::{Extension, Json, response::IntoResponse};
use std::sync::Arc;

/// The legacy OpenAPI document, provided to [`serve_openapi`] as a request
/// extension because it only exists once the routes are built.
#[derive(Clone)]
pub struct LegacySpec(pub Arc<OpenApi>);

pub async fn serve_openapi(
    Extension(LegacySpec(spec)): Extension<LegacySpec>,
) -> impl IntoApiResponse {
    Json(spec.as_ref()).into_response()
}
