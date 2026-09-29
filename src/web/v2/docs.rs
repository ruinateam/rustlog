//! The v2 OpenAPI document and its Scalar page at `/api/v2/docs`.

use aide::{axum::IntoApiResponse, openapi::OpenApi, scalar::Scalar};
use axum::{
    Extension, Json,
    response::{Html, IntoResponse},
};
use std::sync::{Arc, OnceLock};

const SPEC_URL: &str = "/api/v2/openapi.json";

/// The v2 OpenAPI document, provided to [`serve_openapi`] as a request
/// extension because it only exists once the routes are built.
#[derive(Clone)]
pub struct V2Spec(pub Arc<OpenApi>);

pub async fn serve_openapi(Extension(V2Spec(spec)): Extension<V2Spec>) -> impl IntoApiResponse {
    Json(spec.as_ref()).into_response()
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
