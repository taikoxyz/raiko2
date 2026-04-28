use axum::{
    Router,
    routing::{get, post},
};

use super::super::handlers;
use super::super::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/get_ballot", get(handlers::get_ballot))
        .route("/admin/set_ballot", post(handlers::set_ballot))
}
