//! One log event per HTTP request, inside a span with the request details.

use axum::{
    extract::{MatchedPath, Request},
    response::Response,
};
use std::time::Duration;
use tower_http::classify::ServerErrorsFailureClass;
use tracing::{Span, error, info, info_span};

pub fn make_span_with(request: &Request) -> Span {
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str);
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok());

    info_span!(
        "http_request",
        method = %request.method(),
        route,
        uri = %request.uri(),
        request_id,
    )
}

pub fn on_response(response: &Response, latency: Duration, _span: &Span) {
    let status = response.status();
    let latency_ms = latency.as_millis() as u64;

    if status.is_server_error() {
        error!(status = status.as_u16(), latency_ms, "Request failed");
    } else {
        info!(status = status.as_u16(), latency_ms, "Request finished");
    }
}

/// Failures with a status code are already logged by [`on_response`]; this
/// only reports the rest, such as a response body stream that broke off.
pub fn on_failure(failure: ServerErrorsFailureClass, latency: Duration, _span: &Span) {
    if let ServerErrorsFailureClass::Error(error) = failure {
        error!(
            latency_ms = latency.as_millis() as u64,
            error, "Request failed while streaming the response"
        );
    }
}
