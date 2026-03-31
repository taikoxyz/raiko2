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
            "/v3/proof/batch/uzen",
            post(handlers::request_batch_uzen_proof),
        )
        .route(
            "/v3/proof/aggregate",
            post(handlers::request_aggregation_proof),
        )
        .route(
            "/v3/proof/aggregate/uzen",
            post(handlers::request_aggregation_uzen_proof),
        )
        .route("/v3/tasks/{id}", get(handlers::get_task))
        .route("/v3/tasks/{id}/cancel", post(handlers::cancel_task))
}
