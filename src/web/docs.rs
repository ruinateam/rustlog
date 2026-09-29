//! The documentation page at `/docs`: both OpenAPI documents in Scalar, v2
//! first, the deprecated legacy API behind the document switcher.

use axum::response::{Html, Redirect};
use serde_json::json;
use std::sync::LazyLock;

/// Scalar from the CDN, pinned and checked by its hash. The Scalar that aide
/// bundles is too old for several documents on one page. To update, change
/// the version and set the hash to the output of
/// `curl -s <url> | openssl dgst -sha384 -binary | openssl base64 -A`.
const SCRIPT_URL: &str =
    "https://cdn.jsdelivr.net/npm/@scalar/api-reference@1.72.2/dist/browser/standalone.js";
const SCRIPT_INTEGRITY: &str =
    "sha384-mc6GgHVwdYe1ZSU5XmJBa2pe6QzCRDSd0Pk8TqiqM7iiAOFMKtpBTOOa2RZDHkYN";

static PAGE: LazyLock<String> = LazyLock::new(|| {
    let configuration = json!({
        "theme": "default",
        "layout": "modern",
        "withDefaultFonts": false,
        "operationTitleSource": "summary",
        "defaultHttpClient": { "targetKey": "shell", "clientKey": "curl" },
        "searchHotKey": "k",
        // Nothing leaves the page but the requests the reader tries out.
        "telemetry": false,
        "showDeveloperTools": "never",
        "agent": { "disabled": true },
        "mcp": { "disabled": true },
        "metaData": {
            "title": "ChatTiers Rustlog API",
            "description": "Browse the endpoints and try them out on this rustlog instance.",
        },
        "sources": [
            {
                "title": "API v2",
                "slug": "v2",
                "url": "/api/v2/openapi.json",
                "default": true,
            },
            {
                "title": "Legacy API (deprecated)",
                "slug": "legacy",
                "url": "/openapi.json",
            },
        ],
    });

    include_str!("docs.html")
        .replace("{script_url}", SCRIPT_URL)
        .replace("{script_integrity}", SCRIPT_INTEGRITY)
        .replace("{configuration}", &script_literal(&configuration))
});

pub async fn page() -> Html<&'static str> {
    Html(PAGE.as_str())
}

/// The v2 page of earlier versions.
pub async fn redirect_to_page() -> Redirect {
    Redirect::permanent("/docs")
}

/// JSON that is safe inside a `<script>` element, which `</` would end.
fn script_literal(value: &serde_json::Value) -> String {
    value.to_string().replace("</", "<\\/")
}

#[cfg(test)]
mod tests {
    use super::{PAGE, script_literal};
    use serde_json::json;

    #[test]
    fn page_lists_both_documents() {
        assert!(PAGE.contains(r#""url":"/api/v2/openapi.json""#));
        assert!(PAGE.contains(r#""url":"/openapi.json""#));
        assert!(!PAGE.contains("{configuration}"));
        assert!(!PAGE.contains("{script_url}"));
        assert!(!PAGE.contains("{script_integrity}"));
    }

    #[test]
    fn json_cannot_end_the_script() {
        assert_eq!(
            script_literal(&json!({ "text": "</script>" })),
            r#"{"text":"<\/script>"}"#
        );
    }
}
