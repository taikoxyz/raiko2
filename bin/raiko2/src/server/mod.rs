//! HTTP API server for Raiko V2.

mod app;
#[cfg(feature = "fixture-server")]
mod fixture;
mod handlers;
mod net;
mod proof_artifact;
mod ready;
mod request_identity;
mod routes;
mod run;
mod sampling;
mod startup;
mod state;
mod task_cleanup;
mod task_metadata;
mod telemetry;

#[cfg(feature = "fixture-server")]
pub use fixture::run_fixture_server;
pub use run::run_server;
pub(crate) use startup::{log_startup_readiness_passed, log_startup_summary};
pub use state::AppState;

#[cfg(all(test, feature = "fixture-server"))]
mod e2e;
