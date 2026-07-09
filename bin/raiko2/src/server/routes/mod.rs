//! API route definitions.

mod admin;
mod health;
mod metrics;
mod ready;
mod v3;
mod v4;

use axum::Router;

use super::state::AppState;

// Keep the legacy v3 route graph type-checked while it is intentionally unmounted.
// Remove this reference together with the v3 handlers if v4 remains the only public API.
#[allow(dead_code)]
const LEGACY_V3_ROUTES_TYPECHECK: fn() -> Router<AppState> = v3::routes;

/// Build API routes.
pub fn api_routes() -> Router<AppState> {
    Router::new()
        .merge(admin::routes())
        .merge(health::routes())
        .merge(metrics::routes())
        .merge(ready::routes())
        .merge(v4::routes())
}

#[cfg(all(test, feature = "fixture-server"))]
pub fn api_routes_with_legacy_v3_for_tests() -> Router<AppState> {
    api_routes().merge(v3::routes())
}
