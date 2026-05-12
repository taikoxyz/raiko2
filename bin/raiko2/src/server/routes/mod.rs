//! API route definitions.

mod admin;
mod health;
mod metrics;
mod ready;
#[cfg(feature = "tdx")]
mod tdx;
mod v3;

use axum::Router;

use super::state::AppState;

/// Build API routes.
pub fn api_routes() -> Router<AppState> {
    let router = Router::new()
        .merge(admin::routes())
        .merge(health::routes())
        .merge(metrics::routes())
        .merge(ready::routes())
        .merge(v3::routes());
    #[cfg(feature = "tdx")]
    let router = router.merge(tdx::routes());
    router
}
