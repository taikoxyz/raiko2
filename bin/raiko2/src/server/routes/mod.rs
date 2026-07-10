//! API route definitions.

mod admin;
mod health;
mod metrics;
mod ready;
mod v3;
mod v4;

use axum::Router;

use super::state::AppState;

/// Build API routes.
///
/// Temporary mainnet branch: remount legacy v3 alongside v4 so existing
/// taiko-client / ops callers keep working while they migrate to v4.
pub fn api_routes() -> Router<AppState> {
    Router::new()
        .merge(admin::routes())
        .merge(health::routes())
        .merge(metrics::routes())
        .merge(ready::routes())
        .merge(v3::routes())
        .merge(v4::routes())
}

#[cfg(all(test, feature = "fixture-server"))]
pub fn api_routes_with_legacy_v3_for_tests() -> Router<AppState> {
    // Production api_routes already mounts v3 on this temporary branch.
    api_routes()
}
