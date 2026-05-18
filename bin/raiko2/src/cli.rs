//! Command-line interface for Raiko V2.

use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

/// Raiko V2 - Taiko zkVM Prover Server
#[derive(Parser, Debug)]
#[command(name = "raiko2")]
#[command(version, about, long_about = None)]
pub struct Cli {
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

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run a local fixture-backed HTTP server for manual v3 testing
    FixtureServer(FixtureServerArgs),

    /// TDX prover registration utilities (requires `--features tdx`).
    #[cfg(feature = "tdx")]
    Tdx {
        #[command(subcommand)]
        sub: TdxCommand,
    },
}

/// Subcommands for `raiko2 tdx`.
#[cfg(feature = "tdx")]
#[derive(Subcommand, Debug)]
pub enum TdxCommand {
    /// Register this prover's TDX instance on a `TdxVerifier` contract.
    ///
    /// Reads `~/.config/raiko2/tdx/bootstrap.json` (written during prover bootstrap),
    /// extracts the on-chain attestation parameters, and submits transactions.
    Register(TdxRegisterArgs),
}

#[cfg(feature = "tdx")]
#[derive(Args, Debug, Clone)]
pub struct TdxRegisterArgs {
    /// Address of the deployed `TdxVerifier` contract.
    #[arg(long, env = "TDX_VERIFIER")]
    pub verifier: alloy_primitives::Address,

    /// L1 RPC URL where the verifier lives.
    #[arg(long, env = "TDX_RPC")]
    pub rpc: String,

    /// Private key used to sign the registration transactions (hex, with or without 0x).
    ///
    /// For `--trust`, this must be the verifier owner. For `--register` alone, any funded
    /// account works.
    #[arg(long, env = "PRIVATE_KEY")]
    pub private_key: String,

    /// Trusted params slot to write/read. Use the same index for `--trust` and `--register`
    /// in one run; pick a fresh index for new image versions.
    #[arg(long, default_value_t = 0)]
    pub trusted_params_index: u64,

    /// PCR bitmap (24-bit mask of TPM PCR indices to bind into trusted params).
    /// Default `0xBA10` = PCRs 4, 9, 11, 12, 13, 15 (matches the Azure paravisor reference set).
    #[arg(long, default_value_t = 0xBA10)]
    pub pcr_bitmap: u32,

    /// Owner-only: also call `setTrustedParams` with the measurements extracted from this VM.
    #[arg(long)]
    pub trust: bool,

    /// Permissionless: call `registerInstance`. Defaults to true when neither flag is set
    /// so a bare `raiko2 tdx register` does the common case.
    #[arg(long)]
    pub register: bool,

    /// Print the transactions without broadcasting.
    #[arg(long)]
    pub dry_run: bool,
}

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
