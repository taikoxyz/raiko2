//! Command-line interface for Raiko V2.

use clap::Parser;
#[cfg(feature = "fixture-server")]
use clap::{Args, Subcommand};
use std::path::PathBuf;

/// Raiko V2 - Taiko zkVM Prover Server
#[derive(Parser, Debug)]
#[command(name = "raiko2")]
#[command(version, about, long_about = None)]
pub struct Cli {
    #[cfg(feature = "fixture-server")]
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Path to configuration file
    #[arg(short, long, env = "RAIKO2_CONFIG")]
    pub config: Option<PathBuf>,

    /// L1 RPC endpoint URL
    #[arg(long, env = "RAIKO2_L1_RPC")]
    pub l1_rpc: Option<String>,

    /// L2 RPC endpoint URL
    #[arg(long, env = "RAIKO2_L2_RPC")]
    pub l2_rpc: Option<String>,

    /// Server host address
    #[arg(long, env = "RAIKO2_HOST")]
    pub host: Option<String>,

    /// Server port
    #[arg(long, env = "RAIKO2_PORT")]
    pub port: Option<u16>,

    /// Canonical proving route (`<guest_system>/<runner>`)
    #[arg(long, env = "RAIKO2_PROVER")]
    pub prover: Option<String>,

    /// Remote SGX prover base URL used by the `sgx/remote` route
    #[arg(long = "remote-sgx-base-url", env = "RAIKO2_REMOTE_SGX_BASE_URL")]
    pub remote_sgx_base_url: Option<String>,

    /// Remote SGXGETH prover base URL used by the `sgxgeth` lane
    #[arg(
        long = "remote-sgx-sgxgeth-base-url",
        env = "RAIKO2_REMOTE_SGX_SGXGETH_BASE_URL"
    )]
    pub remote_sgx_sgxgeth_base_url: Option<String>,

    /// Remote SGX prover timeout in milliseconds used by the `sgx/remote` route
    #[arg(long = "remote-sgx-timeout-ms", env = "RAIKO2_REMOTE_SGX_TIMEOUT_MS")]
    pub remote_sgx_timeout_ms: Option<u64>,

    /// Enable verbose logging
    #[arg(short, long)]
    pub verbose: bool,

    /// Output logs in JSON format
    #[arg(long)]
    pub json_logs: bool,

    /// RPC request timeout in milliseconds
    #[arg(long, env = "RAIKO2_RPC_TIMEOUT_MS")]
    pub rpc_timeout_ms: Option<u64>,

    /// RPC concurrency limit
    #[arg(long, env = "RAIKO2_RPC_CONCURRENCY_LIMIT")]
    pub rpc_concurrency_limit: Option<usize>,

    /// RPC retry max attempts (0 disables retry)
    #[arg(long, env = "RAIKO2_RPC_RETRY_MAX_ATTEMPTS")]
    pub rpc_retry_max_attempts: Option<u32>,

    /// RPC retry initial backoff in milliseconds
    #[arg(long, env = "RAIKO2_RPC_RETRY_INITIAL_BACKOFF_MS")]
    pub rpc_retry_initial_backoff_ms: Option<u64>,

    /// RPC retry compute units per second budget
    #[arg(long, env = "RAIKO2_RPC_RETRY_CU_PER_SECOND")]
    pub rpc_retry_cu_per_second: Option<u64>,

    /// Queue backend (memory, redis)
    #[arg(long, env = "RAIKO2_QUEUE_BACKEND")]
    pub queue_backend: Option<String>,

    /// Redis URL for queue backend (e.g. <redis://localhost:6379/>)
    #[arg(long, env = "RAIKO2_REDIS_URL")]
    pub redis_url: Option<String>,

    /// Queue namespace/prefix for Redis keys
    #[arg(long, env = "RAIKO2_QUEUE_NAMESPACE")]
    pub queue_namespace: Option<String>,

    /// Number of queue worker loops
    #[arg(long, env = "RAIKO2_QUEUE_WORKERS")]
    pub queue_workers: Option<usize>,

    /// Scheduler maintenance tick interval in milliseconds
    #[arg(long, env = "RAIKO2_QUEUE_MAINTENANCE_INTERVAL_MS")]
    pub queue_maintenance_interval_ms: Option<u64>,

    /// Task execution timeout in seconds, applied regardless of proof type
    #[arg(long, env = "RAIKO2_QUEUE_TASK_TIMEOUT_SECS")]
    pub queue_task_timeout_secs: Option<u64>,
}

#[cfg(feature = "fixture-server")]
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run a local fixture-backed HTTP server for manual v3 testing
    FixtureServer(FixtureServerArgs),
}

#[cfg(feature = "fixture-server")]
#[derive(Args, Debug, Clone)]
pub struct FixtureServerArgs {
    /// Fixture server host address
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,

    /// Fixture server port
    #[arg(long, default_value_t = 8080)]
    pub port: u16,

    /// Number of in-process queue workers
    #[arg(long, default_value_t = 1)]
    pub workers: usize,
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;

    #[cfg(not(feature = "fixture-server"))]
    #[test]
    fn fixture_server_command_is_rejected_without_feature() {
        let result = Cli::try_parse_from(["raiko2", "fixture-server"]);

        assert!(
            result.is_err(),
            "fixture-server should require an explicit feature"
        );
    }

    #[cfg(feature = "fixture-server")]
    #[test]
    fn fixture_server_command_parses_with_feature() {
        let result = Cli::try_parse_from(["raiko2", "fixture-server", "--port", "8087"])
            .expect("fixture-server should parse when the feature is enabled");

        assert!(matches!(
            result.command,
            Some(super::Command::FixtureServer(super::FixtureServerArgs {
                port: 8087,
                ..
            }))
        ));
    }
}
