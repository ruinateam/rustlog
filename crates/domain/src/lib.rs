//! Types of the problem domain, independent of HTTP and ClickHouse.
//!
//! The API layer maps them to its own response types, so changing a domain
//! type never silently changes an API response.

pub mod logs;
pub mod opt_out;
pub mod stats;
pub mod tiers;
