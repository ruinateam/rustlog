//! The v2 OpenAPI document at `/api/v2/openapi.json`.

use aide::{axum::IntoApiResponse, openapi::OpenApi};
use axum::{Extension, Json, response::IntoResponse};
use std::sync::Arc;

/// The v2 OpenAPI document, provided to [`serve_openapi`] as a request
/// extension because it only exists once the routes are built.
#[derive(Clone)]
pub struct V2Spec(pub Arc<OpenApi>);

pub async fn serve_openapi(Extension(V2Spec(spec)): Extension<V2Spec>) -> impl IntoApiResponse {
    Json(spec.as_ref()).into_response()
}
