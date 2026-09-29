//! `Cache-Control` headers shared by the API versions.

use aide::{OperationOutput, generate::GenContext, openapi::Operation};
use axum::response::{IntoResponse, Response};
use axum_extra::{TypedHeader, headers::CacheControl};
use std::time::Duration;

/// Lets clients and proxies cache the response for `secs` seconds.
pub fn public_cache(secs: u64) -> TypedHeader<CacheControl> {
    TypedHeader(
        CacheControl::new()
            .with_public()
            .with_max_age(Duration::from_secs(secs)),
    )
}

/// For responses that can change at any time, such as logs, which an
/// opt-out can withdraw.
pub fn no_cache() -> TypedHeader<CacheControl> {
    TypedHeader(CacheControl::new().with_no_cache())
}

/// A response with a `Cache-Control` header, documented as its body is.
pub struct Cached<T> {
    header: TypedHeader<CacheControl>,
    body: T,
}

impl<T> Cached<T> {
    pub fn no_cache(body: T) -> Self {
        Self {
            header: no_cache(),
            body,
        }
    }

    pub fn public(secs: u64, body: T) -> Self {
        Self {
            header: public_cache(secs),
            body,
        }
    }
}

impl<T: IntoResponse> IntoResponse for Cached<T> {
    fn into_response(self) -> Response {
        (self.header, self.body).into_response()
    }
}

impl<T: OperationOutput> OperationOutput for Cached<T> {
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
        T::inferred_responses(ctx, operation)
    }
}
