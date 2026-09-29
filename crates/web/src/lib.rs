//! The HTTP server: the legacy API at the root, v2 under `/api/v2`, the
//! embedded frontend, API docs and metrics.

mod cache_control;
mod frontend;
mod legacy;
mod logs_response;
mod openapi;
mod trace_layer;
mod v2;

use self::{
    cache_control::no_cache,
    legacy::{admin::AdminApiKey, docs::LegacySpec},
    openapi::enrich_openapi,
};
use aide::{
    axum::{ApiRouter, IntoApiResponse, routing::get},
    openapi::OpenApi,
};
use axum::{Extension, Router, ServiceExt, extract::Request, middleware};
use axum_prometheus::PrometheusMetricLayerBuilder;
use prometheus::TextEncoder;
use rustlog_app::{App, BotMessage, ShutdownRx};
use std::{
    net::{AddrParseError, SocketAddr},
    str::FromStr,
    sync::Arc,
};
use tokio::{net::TcpListener, sync::mpsc::Sender};
use tower_http::{
    CompressionLevel,
    compression::CompressionLayer,
    cors::CorsLayer,
    normalize_path::NormalizePath,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};
use tracing::{debug, info};

/// The HTTP routes together with the OpenAPI documents that describe them.
pub struct Api {
    /// Needs the [`App`] state and the extensions added in [`run`].
    pub router: Router<App>,
    pub legacy_openapi: Arc<OpenApi>,
    pub v2_openapi: Arc<OpenApi>,
}

pub async fn run(app: App, mut shutdown_rx: ShutdownRx, bot_tx: Sender<BotMessage>) {
    metrics_prometheus::install();

    let listen_address =
        parse_listen_addr(&app.config.listen_address).expect("invalid listen address");

    let app = service(app, bot_tx, shutdown_rx.clone());

    info!(address = %listen_address, "listening");

    let listener = TcpListener::bind(&listen_address)
        .await
        .expect("could not create TCP listener");

    axum::serve(listener, ServiceExt::<Request>::into_make_service(app))
        .with_graceful_shutdown(async move {
            shutdown_rx.changed().await.ok();
            debug!("shutting down the HTTP server");
        })
        .await
        .unwrap();
}

/// Builds the complete HTTP service that [`run`] serves.
pub fn service(
    app: App,
    bot_tx: Sender<BotMessage>,
    shutdown_rx: ShutdownRx,
) -> NormalizePath<Router> {
    let admin_api_key = AdminApiKey(app.config.admin_api_key.as_deref().map(Arc::from));

    let router = api()
        .router
        .layer(Extension(bot_tx))
        .layer(Extension(shutdown_rx))
        .layer(Extension(admin_api_key))
        .with_state(app)
        .layer(CorsLayer::permissive())
        .layer(CompressionLayer::new().quality(CompressionLevel::Fastest))
        // Outermost, so the request id exists before the trace span is made.
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid));
    NormalizePath::trim_trailing_slash(router)
}

/// Builds the routes and generates their OpenAPI documents.
pub fn api() -> Api {
    aide::generate::on_error(|error| {
        panic!("could not generate docs: {error}");
    });
    aide::generate::infer_responses(true);
    aide::generate::extract_schemas(true);

    let v2 = v2::api();

    let mut legacy_spec = legacy::openapi();
    let router = ApiRouter::new()
        .merge(Router::new().nest("/api/v2", v2.router))
        .merge(legacy::router())
        .route("/assets/{*asset}", get(frontend::static_asset))
        .fallback(frontend::static_asset)
        .layer(middleware::from_fn(legacy::capabilities_header))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(trace_layer::make_span_with)
                .on_response(trace_layer::on_response)
                .on_failure(trace_layer::on_failure),
        )
        // The default `axum_http_*` metric names are kept on purpose: older
        // axum-prometheus ignored a custom prefix, so deployed dashboards use
        // these names.
        .layer(PrometheusMetricLayerBuilder::new().build())
        .route("/metrics", get(metrics))
        .finish_api(&mut legacy_spec);
    enrich_openapi(&mut legacy_spec);
    let legacy_openapi = Arc::new(legacy_spec);

    Api {
        router: router.layer(Extension(LegacySpec(legacy_openapi.clone()))),
        legacy_openapi,
        v2_openapi: v2.spec,
    }
}

pub fn parse_listen_addr(addr: &str) -> Result<SocketAddr, AddrParseError> {
    if addr.starts_with(':') {
        SocketAddr::from_str(&format!("0.0.0.0{addr}"))
    } else {
        SocketAddr::from_str(addr)
    }
}

async fn metrics() -> impl IntoApiResponse {
    let metric_families = prometheus::gather();

    let encoder = TextEncoder::new();
    let metrics = encoder.encode_to_string(&metric_families).unwrap();
    (no_cache(), metrics)
}

#[cfg(test)]
mod tests {
    use super::api;
    use aide::openapi::OpenApi;
    use serde_json::Value;

    /// aide collects schemas in a global context, so a router built before
    /// another document is finished can leave its schemas in that document.
    #[test]
    fn schema_references_resolve() {
        let api = api();
        for (name, spec) in [("legacy", &api.legacy_openapi), ("v2", &api.v2_openapi)] {
            let dangling = dangling_references(spec);
            assert!(dangling.is_empty(), "{name} document: {dangling:?}");
        }
    }

    fn dangling_references(spec: &OpenApi) -> Vec<String> {
        let document = serde_json::to_value(spec).unwrap();
        let mut references = Vec::new();
        collect_references(&document, &mut references);
        references
            .into_iter()
            .filter(|reference| {
                let pointer = reference.trim_start_matches('#');
                document.pointer(pointer).is_none()
            })
            .collect()
    }

    fn collect_references(value: &Value, references: &mut Vec<String>) {
        match value {
            Value::Object(object) => {
                if let Some(Value::String(reference)) = object.get("$ref") {
                    references.push(reference.clone());
                }
                object
                    .values()
                    .for_each(|value| collect_references(value, references));
            }
            Value::Array(array) => array
                .iter()
                .for_each(|value| collect_references(value, references)),
            _ => {}
        }
    }
}
