//! API route definitions.

use axum::{
    Router,
    routing::{get, post},
};

use super::handlers;
use super::state::AppState;

/// Build API routes.
pub fn api_routes() -> Router<AppState> {
    Router::new()
        // Health check
        .route("/health", get(handlers::health))
        // API v1 routes
        .route("/v1/proof/proposal", post(handlers::request_proposal_proof))
        .route("/v1/proof/:id", get(handlers::get_proof_status))
        .route("/v1/proof/:id/cancel", post(handlers::cancel_proof))
        // Info routes
        .route("/v1/info", get(handlers::get_info))
}
