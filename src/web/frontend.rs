use axum::{
    http::{StatusCode, Uri, header},
    response::{Html, IntoResponse, Response},
};

const INDEX_HTML: &str = "index.html";

const PLACEHOLDER_HTML: &str = r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>rustlog</title></head>
<body>
  <h1>rustlog</h1>
  <p>This build does not include the web frontend (built without the <code>embed-frontend</code> feature).</p>
  <p>API documentation: <a href="/api/v2/docs">v2</a>, <a href="/docs">legacy</a>.</p>
</body>
</html>
"#;

#[cfg(feature = "embed-frontend")]
mod assets {
    use rust_embed::RustEmbed;
    use std::borrow::Cow;

    #[derive(RustEmbed)]
    #[folder = "$CARGO_MANIFEST_DIR/web/dist"]
    struct Assets;

    pub fn get(path: &str) -> Option<Cow<'static, [u8]>> {
        Assets::get(path).map(|file| file.data)
    }
}

#[cfg(not(feature = "embed-frontend"))]
mod assets {
    use std::borrow::Cow;

    pub fn get(_path: &str) -> Option<Cow<'static, [u8]>> {
        None
    }
}

pub async fn static_asset(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    if path.is_empty() || path == INDEX_HTML {
        return index_html();
    }

    match assets::get(path) {
        Some(data) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            ([(header::CONTENT_TYPE, mime.as_ref())], data).into_response()
        }
        None if path.contains('.') => not_found(),
        None => index_html(),
    }
}

fn index_html() -> Response {
    match assets::get(INDEX_HTML) {
        Some(data) => ([(header::CONTENT_TYPE, "text/html")], data).into_response(),
        None if cfg!(feature = "embed-frontend") => not_found(),
        None => Html(PLACEHOLDER_HTML).into_response(),
    }
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "404").into_response()
}
