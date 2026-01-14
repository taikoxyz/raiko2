//! HTTP app wiring.

use axum::Router;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use super::AppState;
use super::routes;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .merge(routes::api_routes())
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(state)
}
