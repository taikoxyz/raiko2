//! HTTP app wiring.

use axum::{Router, extract::DefaultBodyLimit};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use super::AppState;
use super::routes;

const API_BODY_LIMIT_BYTES: usize = 1 << 20;

pub fn build_router(state: AppState) -> Router {
    build_router_with_api_routes(state, routes::api_routes())
}

#[cfg(all(test, feature = "fixture-server"))]
pub fn build_router_with_legacy_v3_for_tests(state: AppState) -> Router {
    build_router_with_api_routes(state, routes::api_routes_with_legacy_v3_for_tests())
}

fn build_router_with_api_routes(state: AppState, api_routes: Router<AppState>) -> Router {
    Router::new()
        .merge(api_routes)
        .layer(DefaultBodyLimit::max(API_BODY_LIMIT_BYTES))
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(state)
}
