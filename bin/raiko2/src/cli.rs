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
    #[arg(long, env = "RAIKO2_HOST", default_value = "0.0.0.0")]
    pub host: String,

    /// Server port
    #[arg(long, env = "RAIKO2_PORT", default_value = "8080")]
    pub port: u16,

    /// Prover type (risc0, sp1)
    #[arg(long, env = "RAIKO2_PROVER", default_value = "risc0")]
    pub prover: String,

    /// Enable verbose logging
    #[arg(short, long)]
    pub verbose: bool,

    /// Output logs in JSON format
    #[arg(long)]
    pub json_logs: bool,

    /// L1 chain ID
    #[arg(long, env = "RAIKO2_L1_CHAIN_ID", default_value = "1")]
    pub l1_chain_id: u64,

    /// L2 chain ID
    #[arg(long, env = "RAIKO2_L2_CHAIN_ID", default_value = "167000")]
    pub l2_chain_id: u64,

    /// Queue backend (memory, redis)
    #[arg(long, env = "RAIKO2_QUEUE_BACKEND", default_value = "memory")]
    pub queue_backend: String,

    /// Redis URL for queue backend (e.g. redis://localhost:6379/)
    #[arg(long, env = "RAIKO2_REDIS_URL")]
    pub redis_url: Option<String>,

    /// Queue namespace/prefix for Redis keys
    #[arg(long, env = "RAIKO2_QUEUE_NAMESPACE", default_value = "raiko2:queue")]
    pub queue_namespace: String,

    /// Number of queue worker loops
    #[arg(long, env = "RAIKO2_QUEUE_WORKERS", default_value = "1")]
    pub queue_workers: usize,
}
