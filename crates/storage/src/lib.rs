//! ClickHouse storage: schema migrations, stored messages and their IRC
//! form, the operational state (logged channels and opt-outs), the write
//! buffer and queries.

pub mod availability;
mod error;
pub mod irc;
pub mod logs;
pub mod message;
mod migrations;
pub mod state;
pub mod stats;
pub mod stream;
pub mod tiers;
pub mod writer;

pub use error::{Error, Result};
pub use migrations::run as setup_db;

// Deletion mutations are asynchronous, so every read excludes opt-outs until
// ClickHouse has physically removed their historical rows.
const ACTIVE_USER_OPT_OUT_PREDICATE: &str = "user_id NOT IN (SELECT subject_id FROM (SELECT subject_id, argMax(opted_out, revision) AS opted_out FROM opt_out_state WHERE scope IN ('user', 'legacy') GROUP BY scope, subject_id) WHERE opted_out = 1)";
