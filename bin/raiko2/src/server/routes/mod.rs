//! API route definitions.

mod health;
mod v1;

use axum::Router;

use super::state::AppState;

/// Build API routes.
pub fn api_routes() -> Router<AppState> {
    Router::new().merge(health::routes()).merge(v1::routes())
}
