use axum::{Router, routing::get};

use super::super::handlers;
use super::super::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/admin/ballot",
        get(handlers::get_ballot).post(handlers::set_ballot),
    )
}
