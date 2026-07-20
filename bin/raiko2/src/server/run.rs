//! HTTP server runtime.

use crate::config::Config;
use anyhow::Result;
use std::future::IntoFuture;
use std::sync::Arc;
use std::time::Duration;
use tokio::{net::TcpListener, signal, sync::Notify};
use tracing::{info, warn};

use super::app;
use super::net;
use super::ready;
use super::{AppState, log_startup_readiness_passed};

const HTTP_GRACEFUL_DRAIN_TIMEOUT: Duration = Duration::from_secs(15);

/// Run the HTTP server.
pub async fn run_server(config: Config, json_logs: bool) -> Result<()> {
    ready::ensure_startup_ready(&config).await?;
    log_startup_readiness_passed(&config, json_logs);

    // Create application state
    let state = AppState::new(config.clone()).await?;

    // Build router
    let app = app::build_router(state.clone());

    // Bind to address
    let addr = net::bind_addr(&config);
    let listener = TcpListener::bind(&addr).await?;

    info!("Server listening on http://{}", addr);

    // Run server
    let drain_started = Arc::new(Notify::new());
    let shutdown_state = state.clone();
    let shutdown_started = Arc::clone(&drain_started);
    let server = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            shutdown_state.begin_shutdown().await;
            shutdown_started.notify_one();
        })
        .into_future();
    tokio::pin!(server);
    let server_result = tokio::select! {
        result = &mut server => result,
        () = async {
            drain_started.notified().await;
            tokio::time::sleep(HTTP_GRACEFUL_DRAIN_TIMEOUT).await;
        } => {
            warn!(
                timeout_secs = HTTP_GRACEFUL_DRAIN_TIMEOUT.as_secs(),
                "timed out waiting for HTTP connections to drain"
            );
            Ok(())
        }
    };
    state.shutdown().await;
    server_result?;

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
