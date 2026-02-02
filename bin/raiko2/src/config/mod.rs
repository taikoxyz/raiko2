//! Configuration management for Raiko V2.

use crate::cli::Cli;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

mod prover;
mod queue;
mod rpc;
mod server;
mod validation;

pub use prover::{ProverConfig, ProverType};
pub use queue::{QueueBackend, QueueConfig, RetryStrategy};
pub use rpc::RpcConfig;
pub use server::ServerConfig;

/// Full application configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub server: ServerConfig,
    pub rpc: RpcConfig,
    pub prover: ProverConfig,
    #[serde(default)]
    pub queue: QueueConfig,
}

impl Config {
    /// Load configuration from CLI arguments and optional config file.
    pub fn load(cli: &Cli) -> Result<Self> {
        let mut config = if let Some(config_path) = &cli.config {
            Self::from_file(config_path)?
        } else {
            Self::default()
        };

        // Override with CLI arguments
        config.server.host.clone_from(&cli.host);
        config.server.port = cli.port;

        if let Some(l1_rpc) = &cli.l1_rpc {
            config.rpc.l1_rpc.clone_from(l1_rpc);
        }
        if let Some(l2_rpc) = &cli.l2_rpc {
            config.rpc.l2_rpc.clone_from(l2_rpc);
        }
        config.rpc.l1_chain_id = cli.l1_chain_id;
        config.rpc.l2_chain_id = cli.l2_chain_id;
        if let Some(timeout_ms) = cli.rpc_timeout_ms {
            config.rpc.client.timeout_ms = timeout_ms;
        }
        if let Some(concurrency_limit) = cli.rpc_concurrency_limit {
            config.rpc.client.concurrency_limit = concurrency_limit;
        }
        if let Some(max_attempts) = cli.rpc_retry_max_attempts {
            config.rpc.client.retry.max_attempts = max_attempts;
        }
        if let Some(initial_backoff_ms) = cli.rpc_retry_initial_backoff_ms {
            config.rpc.client.retry.initial_backoff_ms = initial_backoff_ms;
        }
        if let Some(cu_per_second) = cli.rpc_retry_cu_per_second {
            config.rpc.client.retry.compute_units_per_second = cu_per_second;
        }

        config.prover.prover_type = cli.prover.parse().map_err(|e: String| anyhow::anyhow!(e))?;

        if let Some(queue_backend) = &cli.queue_backend {
            config.queue.backend = queue_backend
                .parse()
                .map_err(|e: String| anyhow::anyhow!(e))?;
        }
        if let Some(queue_namespace) = &cli.queue_namespace {
            config.queue.namespace.clone_from(queue_namespace);
        }
        if let Some(queue_workers) = cli.queue_workers {
            config.queue.workers = queue_workers;
        }
        if let Some(interval_ms) = cli.queue_maintenance_interval_ms {
            config.queue.maintenance_interval_ms = interval_ms;
        }
        if let Some(strategy) = &cli.queue_retry_strategy {
            config.queue.retry.strategy =
                strategy.parse().map_err(|e: String| anyhow::anyhow!(e))?;
        }
        if let Some(max_attempts) = cli.queue_retry_max_attempts {
            config.queue.retry.max_attempts = max_attempts;
        }
        if let Some(fixed_delay_ms) = cli.queue_retry_fixed_delay_ms {
            config.queue.retry.fixed_delay_ms = fixed_delay_ms;
        }
        if let Some(base_delay_ms) = cli.queue_retry_base_delay_ms {
            config.queue.retry.base_delay_ms = base_delay_ms;
        }
        if let Some(max_delay_ms) = cli.queue_retry_max_delay_ms {
            config.queue.retry.max_delay_ms = max_delay_ms;
        }
        if let Some(redis_url) = &cli.redis_url {
            config.queue.redis_url = Some(redis_url.clone());
        }

        // Validate configuration
        config.validate()?;

        Ok(config)
    }

    /// Load configuration from a TOML file.
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        toml::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))
    }

    /// Validate the entire configuration.
    pub fn validate(&self) -> Result<()> {
        self.server
            .validate()
            .context("Server configuration error")?;
        self.rpc.validate().context("RPC configuration error")?;
        self.queue.validate().context("Queue configuration error")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;
    use clap::Parser;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn write_temp_config(contents: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_nanos();
        path.push(format!("raiko2-config-{nanos}.toml"));
        std::fs::write(&path, contents).expect("write temp config");
        path
    }

    #[test]
    fn test_server_config_default() {
        let config = ServerConfig::default();
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.port, 8080);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_server_config_invalid_host() {
        let config = ServerConfig {
            host: "".to_string(),
            port: 8080,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_server_config_invalid_port() {
        let config = ServerConfig {
            host: "localhost".to_string(),
            port: 0,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_rpc_config_default() {
        let config = RpcConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_rpc_config_valid_urls() {
        let config = RpcConfig {
            l1_rpc: "https://eth.llamarpc.com".to_string(),
            l2_rpc: "wss://taiko-rpc.example.com".to_string(),
            l1_chain_id: 1,
            l2_chain_id: 167000,
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_rpc_config_invalid_l1_url() {
        let config = RpcConfig {
            l1_rpc: "not-a-valid-url".to_string(),
            l2_rpc: "http://localhost:9545".to_string(),
            l1_chain_id: 1,
            l2_chain_id: 167000,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("l1_rpc"));
    }

    #[test]
    fn test_rpc_config_invalid_chain_id() {
        let config = RpcConfig {
            l1_rpc: "http://localhost:8545".to_string(),
            l2_rpc: "http://localhost:9545".to_string(),
            l1_chain_id: 0,
            l2_chain_id: 167000,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_prover_type_from_str() {
        assert_eq!("risc0".parse::<ProverType>().unwrap(), ProverType::Risc0);
        assert_eq!("RISC0".parse::<ProverType>().unwrap(), ProverType::Risc0);
        assert_eq!("sp1".parse::<ProverType>().unwrap(), ProverType::Sp1);
        assert_eq!("SP1".parse::<ProverType>().unwrap(), ProverType::Sp1);
        assert_eq!("native".parse::<ProverType>().unwrap(), ProverType::Native);
        assert_eq!("NATIVE".parse::<ProverType>().unwrap(), ProverType::Native);
        assert!("invalid".parse::<ProverType>().is_err());
    }

    #[test]
    fn test_config_default_validates() {
        let config = Config::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_queue_backend_cli_overrides_config_file() {
        let config_toml = r#"
[server]
host = "0.0.0.0"
port = 8080

[rpc]
l1_rpc = "http://localhost:8545"
l2_rpc = "http://localhost:9545"
l1_chain_id = 1
l2_chain_id = 167000

[prover]
prover_type = "risc0"

[queue]
backend = "memory"
namespace = "raiko2:queue"
workers = 1
maintenance_interval_ms = 200
"#;
        let path = write_temp_config(config_toml);

        let cli = Cli::parse_from([
            "raiko2",
            "--config",
            path.to_str().expect("path utf8"),
            "--queue-backend",
            "redis",
            "--redis-url",
            "redis://localhost:6379/",
        ]);

        let config = Config::load(&cli).expect("config load");
        assert_eq!(config.queue.backend, QueueBackend::Redis);
        assert_eq!(
            config.queue.redis_url.as_deref(),
            Some("redis://localhost:6379/")
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_queue_backend_redis_requires_url() {
        let config_toml = r#"
[server]
host = "0.0.0.0"
port = 8080

[rpc]
l1_rpc = "http://localhost:8545"
l2_rpc = "http://localhost:9545"
l1_chain_id = 1
l2_chain_id = 167000

[prover]
prover_type = "risc0"

[queue]
backend = "memory"
namespace = "raiko2:queue"
workers = 1
maintenance_interval_ms = 200
"#;
        let path = write_temp_config(config_toml);

        let cli = Cli::parse_from([
            "raiko2",
            "--config",
            path.to_str().expect("path utf8"),
            "--queue-backend",
            "redis",
        ]);

        let err = Config::load(&cli).expect_err("expected config error");
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("Queue configuration error"),
            "unexpected error: {err_msg}"
        );
        assert!(
            err.chain().any(|e| e.to_string().contains("redis_url")),
            "missing redis_url detail in error chain: {err_msg}"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_is_valid_url() {
        assert!(super::rpc::is_valid_url("http://localhost:8545"));
        assert!(super::rpc::is_valid_url("https://eth.llamarpc.com"));
        assert!(super::rpc::is_valid_url("ws://localhost:8546"));
        assert!(super::rpc::is_valid_url("wss://rpc.example.com"));
        assert!(!super::rpc::is_valid_url("localhost:8545"));
        assert!(!super::rpc::is_valid_url("ftp://files.example.com"));
        assert!(!super::rpc::is_valid_url(""));
    }
}
