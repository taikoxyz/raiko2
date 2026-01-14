use axum::{
    Router,
    routing::{get, post},
};

use super::super::handlers;
use super::super::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/proof/proposal", post(handlers::request_proposal_proof))
        .route("/v1/proof/:id", get(handlers::get_proof_status))
        .route("/v1/proof/:id/cancel", post(handlers::cancel_proof))
        .route("/v1/info", get(handlers::get_info))
}
