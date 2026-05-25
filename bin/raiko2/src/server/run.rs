//! HTTP server runtime.

use crate::config::Config;
use anyhow::Result;
use tokio::{net::TcpListener, signal};
use tracing::info;

use super::app;
use super::net;
use super::ready;
use super::{AppState, log_startup_readiness_passed};

/// Run the HTTP server.
pub async fn run_server(config: Config, json_logs: bool) -> Result<()> {
    ready::ensure_startup_ready(&config).await?;
    log_startup_readiness_passed(&config, json_logs);

    // Create application state
    let state = AppState::new(config.clone()).await?;

    // Build router
    let app = app::build_router(state);

    // Bind to address
    let addr = net::bind_addr(&config);
    let listener = TcpListener::bind(&addr).await?;

    info!("Server listening on http://{}", addr);

    // Run server
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = signal::ctrl_c().await {
            tracing::warn!(error = %err, "failed to install Ctrl-C handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match signal::unix::signal(signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(err) => {
                tracing::warn!(error = %err, "failed to install SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
