//! HTTP request handlers.

mod admin;
mod auth;
mod errors;
mod health;
mod metrics;
mod proof;
mod ready;

pub(crate) use admin::{get_ballot, set_ballot};
pub use health::health;
pub use metrics::metrics;
pub(crate) use proof::{migrate_legacy_queue_namespaces_on_startup, v3, v4};
pub use ready::ready;
