//! The legacy OpenAPI document and its Scalar page at `/docs`.

use aide::{axum::IntoApiResponse, openapi::OpenApi, scalar::Scalar};
use axum::{
    Extension, Json,
    response::{Html, IntoResponse},
};
use std::sync::{Arc, OnceLock};

const SCALAR_SPEC_URL: &str = "/openapi.json";
const SCALAR_PAGE_TITLE: &str = "ChatTiers Rustlog API";
const SCALAR_HEAD_INJECT: &str = r#"
<link rel="preconnect" href="https://fonts.googleapis.com" />
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin />
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&family=JetBrains+Mono:wght@400;500;600&display=swap" rel="stylesheet" />
<style>
  :root {
    --scalar-font: 'Inter', sans-serif;
    --scalar-font-code: 'JetBrains Mono', monospace;
  }
</style>
"#;

/// The legacy OpenAPI document, provided to [`serve_openapi`] as a request
/// extension because it only exists once the routes are built.
#[derive(Clone)]
pub struct LegacySpec(pub Arc<OpenApi>);

pub async fn serve_openapi(
    Extension(LegacySpec(spec)): Extension<LegacySpec>,
) -> impl IntoApiResponse {
    Json(spec.as_ref()).into_response()
}

pub async fn scalar_page() -> Html<&'static str> {
    static HTML: OnceLock<&'static str> = OnceLock::new();

    Html(HTML.get_or_init(build_scalar_html))
}

fn build_scalar_html() -> &'static str {
    let html = Scalar::new(SCALAR_SPEC_URL)
        .with_title(SCALAR_PAGE_TITLE)
        .html()
        .replace(
            "<style>",
            &format!("{SCALAR_HEAD_INJECT}\n<style>"),
        )
        .replace(
            "theme: 'purple',",
            r#"theme: 'default',
                    layout: 'modern',
                    withDefaultFonts: false,
                    operationTitleSource: 'summary',
                    defaultHttpClient: { targetKey: 'shell', clientKey: 'curl' },
                    searchHotKey: 'k',
                    hideDarkModeToggle: false,
                    metaData: {
                      title: 'ChatTiers Rustlog API',
                      description: 'Browse endpoints, inspect request and response shapes, and test the current rustlog instance.'
                    },"#,
        );

    Box::leak(html.into_boxed_str())
}
