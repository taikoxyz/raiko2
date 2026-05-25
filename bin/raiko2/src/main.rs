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
//! raiko2
//! ```

mod cli;
mod config;
mod server;

use anyhow::Result;
use clap::Parser;
use tracing::{debug, info};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

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

    let registry = tracing_subscriber::registry().with(env_filter);
    if cli.json_logs {
        registry.with(fmt::layer().json()).init();
    } else {
        registry.with(fmt::layer().with_ansi(false)).init();
    }
}
