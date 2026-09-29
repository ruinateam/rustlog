//! `Cache-Control` headers shared by the API versions.

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
