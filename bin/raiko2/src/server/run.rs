//! HTTP server runtime.

use crate::config::Config;
use anyhow::Result;
use tokio::net::TcpListener;
use tracing::info;

use super::AppState;
use super::app;
use super::net;

/// Run the HTTP server.
pub async fn run_server(config: Config) -> Result<()> {
    // Create application state
    let state = AppState::new(config.clone()).await?;

    // Build router
    let app = app::build_router(state);

    // Bind to address
    let addr = net::bind_addr(&config);
    let listener = TcpListener::bind(&addr).await?;

    info!("Server listening on http://{}", addr);

    // Run server
    axum::serve(listener, app).await?;

    Ok(())
}
