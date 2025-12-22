//! Configuration management for Raiko V2.

use crate::cli::Cli;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Configuration validation error messages.
mod validation {
    pub const INVALID_HOST: &str = "Invalid host address";
    pub const INVALID_PORT: &str = "Port must be between 1 and 65535";
    pub const INVALID_RPC_URL: &str = "Invalid RPC URL format";
    pub const INVALID_CHAIN_ID: &str = "Chain ID must be greater than 0";
}

/// Validate that a string is a valid URL.
fn is_valid_url(url: &str) -> bool {
    url.starts_with("http://")
        || url.starts_with("https://")
        || url.starts_with("ws://")
        || url.starts_with("wss://")
}

/// Server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8080,
        }
    }
}

impl ServerConfig {
    /// Validate server configuration.
    pub fn validate(&self) -> Result<()> {
        if self.host.is_empty() {
            bail!("{}", validation::INVALID_HOST);
        }
        if self.port == 0 {
            bail!("{}", validation::INVALID_PORT);
        }
        Ok(())
    }
}

/// RPC configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcConfig {
    pub l1_rpc: String,
    pub l2_rpc: String,
    pub l1_chain_id: u64,
    pub l2_chain_id: u64,
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            l1_rpc: "http://localhost:8545".to_string(),
            l2_rpc: "http://localhost:9545".to_string(),
            l1_chain_id: 1,
            l2_chain_id: 167000,
        }
    }
}

impl RpcConfig {
    /// Validate RPC configuration.
    pub fn validate(&self) -> Result<()> {
        if !is_valid_url(&self.l1_rpc) {
            bail!(
                "{}: l1_rpc = '{}'",
                validation::INVALID_RPC_URL,
                self.l1_rpc
            );
        }
        if !is_valid_url(&self.l2_rpc) {
            bail!(
                "{}: l2_rpc = '{}'",
                validation::INVALID_RPC_URL,
                self.l2_rpc
            );
        }
        if self.l1_chain_id == 0 {
            bail!("{}: l1_chain_id", validation::INVALID_CHAIN_ID);
        }
        if self.l2_chain_id == 0 {
            bail!("{}: l2_chain_id", validation::INVALID_CHAIN_ID);
        }
        Ok(())
    }
}

/// Prover type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProverType {
    #[default]
    Risc0,
    Sp1,
    Native,
}

impl std::str::FromStr for ProverType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "risc0" => Ok(ProverType::Risc0),
            "sp1" => Ok(ProverType::Sp1),
            "native" => Ok(ProverType::Native),
            _ => Err(format!("Unknown prover type: {}", s)),
        }
    }
}

/// Prover configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProverConfig {
    pub prover_type: ProverType,
    /// RISC0 specific configuration.
    #[serde(default)]
    pub risc0: Risc0Config,
    /// SP1 specific configuration.
    #[serde(default)]
    pub sp1: Sp1Config,
}

/// RISC0 configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Risc0Config {
    pub bonsai: bool,
    pub snark: bool,
}

impl Default for Risc0Config {
    fn default() -> Self {
        Self {
            bonsai: true,
            snark: true,
        }
    }
}

/// SP1 configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sp1Config {
    pub network: bool,
    pub plonk: bool,
}

impl Default for Sp1Config {
    fn default() -> Self {
        Self {
            network: true,
            plonk: true,
        }
    }
}

fn default_queue_namespace() -> String {
    "raiko2:queue".to_string()
}

const fn default_queue_maintenance_interval_ms() -> u64 {
    200
}

const fn default_queue_retry_max_attempts() -> u32 {
    3
}

const fn default_queue_retry_fixed_delay_ms() -> u64 {
    1_000
}

const fn default_queue_retry_base_delay_ms() -> u64 {
    1_000
}

const fn default_queue_retry_max_delay_ms() -> u64 {
    30_000
}

/// Queue retry strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RetryStrategy {
    None,
    Fixed,
    #[default]
    Exponential,
}

impl std::str::FromStr for RetryStrategy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "none" => Ok(RetryStrategy::None),
            "fixed" => Ok(RetryStrategy::Fixed),
            "exponential" | "exp" => Ok(RetryStrategy::Exponential),
            _ => Err(format!("Unknown retry strategy: {s}")),
        }
    }
}

/// Queue retry configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    #[serde(default)]
    pub strategy: RetryStrategy,
    #[serde(default = "default_queue_retry_max_attempts")]
    pub max_attempts: u32,
    #[serde(default = "default_queue_retry_fixed_delay_ms")]
    pub fixed_delay_ms: u64,
    #[serde(default = "default_queue_retry_base_delay_ms")]
    pub base_delay_ms: u64,
    #[serde(default = "default_queue_retry_max_delay_ms")]
    pub max_delay_ms: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            strategy: RetryStrategy::default(),
            max_attempts: default_queue_retry_max_attempts(),
            fixed_delay_ms: default_queue_retry_fixed_delay_ms(),
            base_delay_ms: default_queue_retry_base_delay_ms(),
            max_delay_ms: default_queue_retry_max_delay_ms(),
        }
    }
}

/// Queue backend type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum QueueBackend {
    #[default]
    Memory,
    Redis,
}

impl std::str::FromStr for QueueBackend {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "memory" => Ok(QueueBackend::Memory),
            "redis" => Ok(QueueBackend::Redis),
            _ => Err(format!("Unknown queue backend: {}", s)),
        }
    }
}

/// Queue configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueConfig {
    #[serde(default)]
    pub backend: QueueBackend,
    #[serde(default = "default_queue_namespace")]
    pub namespace: String,
    #[serde(default)]
    pub redis_url: Option<String>,
    #[serde(default = "default_queue_workers")]
    pub workers: usize,
    #[serde(default = "default_queue_maintenance_interval_ms")]
    pub maintenance_interval_ms: u64,
    #[serde(default)]
    pub retry: RetryConfig,
}

const fn default_queue_workers() -> usize {
    1
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            backend: QueueBackend::Memory,
            namespace: default_queue_namespace(),
            redis_url: None,
            workers: default_queue_workers(),
            maintenance_interval_ms: default_queue_maintenance_interval_ms(),
            retry: RetryConfig::default(),
        }
    }
}

impl QueueConfig {
    pub fn validate(&self) -> Result<()> {
        if self.workers == 0 {
            bail!("Queue workers must be > 0");
        }

        if self.maintenance_interval_ms == 0 {
            bail!("Queue maintenance_interval_ms must be > 0");
        }

        if self.retry.strategy != RetryStrategy::None && self.retry.max_attempts == 0 {
            bail!("Queue retry max_attempts must be > 0 when retry is enabled");
        }

        match self.backend {
            QueueBackend::Memory => Ok(()),
            QueueBackend::Redis => {
                if self.redis_url.as_deref().unwrap_or_default().is_empty() {
                    bail!("Redis queue backend requires redis_url to be set");
                }
                if self.namespace.is_empty() {
                    bail!("Redis queue backend requires namespace to be non-empty");
                }
                Ok(())
            }
        }
    }
}

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
        config.server.host = cli.host.clone();
        config.server.port = cli.port;

        if let Some(l1_rpc) = &cli.l1_rpc {
            config.rpc.l1_rpc = l1_rpc.clone();
        }
        if let Some(l2_rpc) = &cli.l2_rpc {
            config.rpc.l2_rpc = l2_rpc.clone();
        }
        config.rpc.l1_chain_id = cli.l1_chain_id;
        config.rpc.l2_chain_id = cli.l2_chain_id;

        config.prover.prover_type = cli.prover.parse().map_err(|e: String| anyhow::anyhow!(e))?;

        if let Some(queue_backend) = &cli.queue_backend {
            config.queue.backend = queue_backend
                .parse()
                .map_err(|e: String| anyhow::anyhow!(e))?;
        }
        if let Some(queue_namespace) = &cli.queue_namespace {
            config.queue.namespace = queue_namespace.clone();
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
            .with_context(|| format!("Failed to read config file: {:?}", path))?;
        toml::from_str(&content).with_context(|| format!("Failed to parse config file: {:?}", path))
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
    fn test_is_valid_url() {
        assert!(is_valid_url("http://localhost:8545"));
        assert!(is_valid_url("https://eth.llamarpc.com"));
        assert!(is_valid_url("ws://localhost:8546"));
        assert!(is_valid_url("wss://rpc.example.com"));
        assert!(!is_valid_url("localhost:8545"));
        assert!(!is_valid_url("ftp://files.example.com"));
        assert!(!is_valid_url(""));
    }
}
