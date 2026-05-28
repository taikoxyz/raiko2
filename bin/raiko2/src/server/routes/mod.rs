//! API route definitions.

mod admin;
mod health;
mod metrics;
mod ready;
mod v3;

use axum::Router;

use super::state::AppState;

/// Build API routes.
pub fn api_routes() -> Router<AppState> {
    Router::new()
        .merge(admin::routes())
        .merge(health::routes())
        .merge(metrics::routes())
        .merge(ready::routes())
        .merge(v3::routes())
}
