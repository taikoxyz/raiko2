#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

//! Raiko V2 - Taiko zkVM Prover Server
//!
//! This binary provides a REST API for generating zero-knowledge proofs
//! of Taiko block execution using RISC0 or SP1 zkVMs.
//!
//! ## Usage
//!
//! ```bash
//! # Start the server
//! raiko2 --config config.toml
//!
//! # Or with environment variables
//! RAIKO2_L1_RPC=http://localhost:8545 \
//! RAIKO2_L2_RPC=http://localhost:9545 \
//! RAIKO2_PROVER_ROUTES=native/local \
//! raiko2
//! ```

mod cli;
mod config;
mod server;

use anyhow::Result;
use clap::Parser;
use tracing::{Level, info};
use tracing_subscriber::{
    EnvFilter,
    filter::{FilterExt, filter_fn},
    fmt,
    prelude::*,
};

use crate::cli::Cli;
#[cfg(feature = "fixture-server")]
use crate::cli::Command;
use crate::config::Config;
#[cfg(feature = "fixture-server")]
use crate::server::run_fixture_server;
use crate::server::{log_startup_summary, run_server};

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file if present
    dotenvy::dotenv().ok();

    // Parse CLI arguments
    let cli = Cli::parse();

    // Initialize logging
    init_logging(&cli);

    info!("Starting Raiko V2 Prover Server");

    #[cfg(feature = "fixture-server")]
    if let Some(Command::FixtureServer(args)) = &cli.command {
        run_fixture_server(args).await?;
        return Ok(());
    }

    // Load configuration
    let config = Config::load(&cli)?;
    log_startup_summary(&config, cli.json_logs);

    // Run the server
    run_server(config, cli.json_logs).await?;

    Ok(())
}

fn init_logging(cli: &Cli) {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        if cli.verbose {
            EnvFilter::new("debug")
        } else {
            EnvFilter::new("info")
        }
    });

    // These dependencies can render the configured provider URL before Raiko2
    // receives the error and can redact it. Keep this safety filter independent
    // of `RUST_LOG` so verbose logging cannot opt back into credential exposure.
    let dependency_filter =
        filter_fn(|metadata| credential_safe_dependency_log(metadata.target(), *metadata.level()));
    let log_filter = env_filter.and(dependency_filter);

    if cli.json_logs {
        tracing_subscriber::registry()
            .with(fmt::layer().json().with_filter(log_filter))
            .init();
    } else {
        tracing_subscriber::registry()
            .with(fmt::layer().with_ansi(false).with_filter(log_filter))
            .init();
    }
}

fn credential_safe_dependency_log(target: &str, level: Level) -> bool {
    if target.starts_with("boundless_market") {
        return false;
    }

    !target.starts_with("alloy_transport_http") || !matches!(level, Level::DEBUG | Level::TRACE)
}

#[cfg(test)]
mod tests {
    use tracing::Level;

    use super::credential_safe_dependency_log;

    #[test]
    fn credential_safety_filter_cannot_be_bypassed_by_verbose_dependency_logs() {
        for level in [
            Level::ERROR,
            Level::WARN,
            Level::INFO,
            Level::DEBUG,
            Level::TRACE,
        ] {
            assert!(!credential_safe_dependency_log(
                "boundless_market::request_builder",
                level
            ));
        }

        assert!(!credential_safe_dependency_log(
            "alloy_transport_http::reqwest_transport",
            Level::DEBUG
        ));
        assert!(!credential_safe_dependency_log(
            "alloy_transport_http::reqwest_transport",
            Level::TRACE
        ));
        assert!(credential_safe_dependency_log(
            "alloy_transport_http::reqwest_transport",
            Level::INFO
        ));
        assert!(credential_safe_dependency_log(
            "raiko2_prover::boundless",
            Level::TRACE
        ));
    }
}
