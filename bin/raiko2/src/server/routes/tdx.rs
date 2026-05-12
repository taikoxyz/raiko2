use axum::{Router, routing::get};

use super::super::handlers;
use super::super::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/v3/proof/tdx/bootstrap", get(handlers::tdx_bootstrap))
}
