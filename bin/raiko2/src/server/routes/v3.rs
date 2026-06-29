use axum::{
    Router,
    routing::{get, post},
};

use super::super::handlers;
use super::super::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v3/proof/batch/shasta",
            post(handlers::request_batch_shasta_proof),
        )
        .route(
            "/v3/proof/aggregate",
            post(handlers::request_aggregation_proof),
        )
        .route("/v3/proof/report", get(handlers::report_proofs))
        .route("/v3/proof/list", get(handlers::list_proofs))
        .route("/v3/proof/prune", post(handlers::prune_proofs))
        .route("/v3/prover/status", get(handlers::get_prover_status))
        .route("/v3/prover/clear", post(handlers::clear_prover))
        .route(
            "/proof/batch/shasta",
            post(handlers::request_batch_shasta_proof),
        )
        .route(
            "/proof/aggregate",
            post(handlers::request_aggregation_proof),
        )
        .route("/proof/report", get(handlers::report_proofs))
        .route("/proof/list", get(handlers::list_proofs))
        .route("/proof/prune", post(handlers::prune_proofs))
        .route("/v3/tasks/{id}", get(handlers::get_task))
        .route("/v3/tasks/{id}/cancel", post(handlers::cancel_task))
}
