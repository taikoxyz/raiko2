use axum::{
    Router,
    routing::{get, post},
};

use super::super::handlers;
use super::super::state::AppState;

pub fn routes() -> Router<AppState> {
    // Legacy /proof/* routes share the v3 handler module so old clients keep the same behavior.
    Router::new()
        .route(
            "/v3/proof/batch/shasta",
            post(handlers::v3::request_batch_shasta_proof),
        )
        .route(
            "/v3/proof/aggregate",
            post(handlers::v3::request_aggregation_proof),
        )
        .route("/v3/proof/report", get(handlers::v3::report_proofs))
        .route("/v3/proof/list", get(handlers::v3::list_proofs))
        .route("/v3/proof/prune", post(handlers::v3::prune_proofs))
        .route("/v3/prover/status", get(handlers::v3::get_prover_status))
        .route("/v3/prover/clear", post(handlers::v3::clear_prover))
        .route(
            "/proof/batch/shasta",
            post(handlers::v3::request_batch_shasta_proof),
        )
        .route(
            "/proof/aggregate",
            post(handlers::v3::request_aggregation_proof),
        )
        .route("/proof/report", get(handlers::v3::report_proofs))
        .route("/proof/list", get(handlers::v3::list_proofs))
        .route("/proof/prune", post(handlers::v3::prune_proofs))
        .route("/v3/tasks/{id}", get(handlers::v3::get_task))
        .route("/v3/tasks/{id}/cancel", post(handlers::v3::cancel_task))
}
