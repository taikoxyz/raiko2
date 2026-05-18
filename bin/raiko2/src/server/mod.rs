//! HTTP API server for Raiko V2.

mod app;
mod fixture;
mod handlers;
mod net;
mod proof_artifact;
mod ready;
mod routes;
mod run;
mod sampling;
mod startup;
mod state;
mod task_cleanup;
mod task_metadata;
mod telemetry;

pub use fixture::run_fixture_server;
pub use run::run_server;
pub(crate) use startup::{log_startup_readiness_passed, log_startup_summary};
pub use state::AppState;

#[cfg(test)]
mod e2e;
