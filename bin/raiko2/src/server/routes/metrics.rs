use axum::{Router, routing::get};

use super::super::handlers;
use super::super::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/metrics", get(handlers::metrics))
}
