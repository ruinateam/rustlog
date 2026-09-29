//! Snapshot tests of the HTTP API against a real ClickHouse.
//!
//! Every test sends one request to a freshly seeded database and pins the
//! status, the relevant headers and the body, so refactoring cannot silently
//! change the API. They need ClickHouse and are excluded from the default
//! nextest profile: run them with `just test-integration`, or set
//! `RUSTLOG_TEST_CLICKHOUSE_URL` (plus `RUSTLOG_TEST_CLICKHOUSE_USER` and
//! `RUSTLOG_TEST_CLICKHOUSE_PASSWORD` if needed) and run
//! `cargo nextest run --profile integration`. The ClickHouse server must run
//! in UTC, as the Docker image does.

mod support;

mod frontend_and_docs;
mod legacy_admin;
mod legacy_channels;
mod legacy_logs;
mod legacy_stats;
mod legacy_tiers;
mod v2;
