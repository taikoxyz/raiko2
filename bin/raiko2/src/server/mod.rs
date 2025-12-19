//! HTTP API server for Raiko V2.

mod handlers;
mod routes;
mod state;

use crate::config::Config;
use anyhow::Result;
use axum::Router;
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::info;

pub use state::AppState;

fn bind_addr(config: &Config) -> String {
    format!("{}:{}", config.server.host, config.server.port)
}

/// Run the HTTP server.
pub async fn run_server(config: Config) -> Result<()> {
    // Create application state
    let state = AppState::new(config.clone()).await?;

    // Build router
    let app = Router::new()
        .merge(routes::api_routes())
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(state);

    // Bind to address
    let addr = bind_addr(&config);
    let listener = TcpListener::bind(&addr).await?;

    info!("Server listening on http://{}", addr);

    // Run server
    axum::serve(listener, app).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServerConfig;

    #[test]
    fn bind_addr_respects_config_host() {
        let mut config = Config::default();
        config.server = ServerConfig {
            host: "127.0.0.1".to_string(),
            port: 1234,
        };

        assert_eq!(bind_addr(&config), "127.0.0.1:1234");
    }
}
