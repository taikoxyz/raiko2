//! Command-line interface for Raiko V2.

use clap::Parser;
use std::path::PathBuf;

/// Raiko V2 - Taiko zkVM Prover Server
#[derive(Parser, Debug)]
#[command(name = "raiko2")]
#[command(version, about, long_about = None)]
pub struct Cli {
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

    /// L1 chain ID
    #[arg(long, env = "RAIKO2_L1_CHAIN_ID")]
    pub l1_chain_id: Option<u64>,

    /// L2 chain ID
    #[arg(long, env = "RAIKO2_L2_CHAIN_ID")]
    pub l2_chain_id: Option<u64>,

    /// RPC request timeout in milliseconds
    #[arg(long, env = "RAIKO2_RPC_TIMEOUT_MS")]
    pub rpc_timeout_ms: Option<u64>,

    /// RPC concurrency limit
    #[arg(long, env = "RAIKO2_RPC_CONCURRENCY_LIMIT")]
    pub rpc_concurrency_limit: Option<usize>,

    /// Local witness generation concurrency limit
    #[arg(long, env = "RAIKO2_RPC_LOCAL_WITNESS_CONCURRENCY_LIMIT")]
    pub rpc_local_witness_concurrency_limit: Option<usize>,

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

    /// Queue retry strategy (none, fixed, exponential)
    #[arg(long, env = "RAIKO2_QUEUE_RETRY_STRATEGY")]
    pub queue_retry_strategy: Option<String>,

    /// Maximum attempts for retry (when retry is enabled)
    #[arg(long, env = "RAIKO2_QUEUE_RETRY_MAX_ATTEMPTS")]
    pub queue_retry_max_attempts: Option<u32>,

    /// Fixed retry delay in milliseconds (when `retry_strategy=fixed`)
    #[arg(long, env = "RAIKO2_QUEUE_RETRY_FIXED_DELAY_MS")]
    pub queue_retry_fixed_delay_ms: Option<u64>,

    /// Exponential retry base delay in milliseconds (when `retry_strategy=exponential`)
    #[arg(long, env = "RAIKO2_QUEUE_RETRY_BASE_DELAY_MS")]
    pub queue_retry_base_delay_ms: Option<u64>,

    /// Exponential retry maximum delay in milliseconds (when `retry_strategy=exponential`)
    #[arg(long, env = "RAIKO2_QUEUE_RETRY_MAX_DELAY_MS")]
    pub queue_retry_max_delay_ms: Option<u64>,
}
