use axum::{
    Router,
    routing::{get, post},
};

use super::super::{handlers, state::AppState};

pub fn routes() -> Router<AppState> {
    // Keep the v4 surface isolated so path shape changes do not leak into legacy proof routes.
    Router::new()
        .route(
            "/v4/proof/proposal",
            post(handlers::v4_request_proposal_proof),
        )
        .route(
            "/v4/proof/aggregation",
            post(handlers::v4_request_aggregation_proof),
        )
        .route("/v4/tasks/{id}", get(handlers::v4_get_task))
        .route("/v4/prover/status", get(handlers::v4_get_prover_status))
        .route("/v4/prover/clear", post(handlers::v4_clear_prover))
}
