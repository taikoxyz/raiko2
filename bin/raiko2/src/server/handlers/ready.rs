use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};

use crate::server::ready::evaluate_readiness;
use crate::server::state::AppState;

/// Readiness check endpoint.
pub async fn ready(State(state): State<AppState>) -> impl IntoResponse {
    let readiness = evaluate_readiness(&state.config).await;
    let status = if readiness.status == "ok" {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(readiness))
}
