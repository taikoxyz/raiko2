use anyhow::{Result, bail};
use raiko2_provider::{
    RpcClientConfig as ProviderRpcClientConfig, RpcRetryConfig as ProviderRpcRetryConfig,
};
use serde::{Deserialize, Serialize};

const fn default_rpc_timeout_ms() -> u64 {
    10_000
}

const fn default_rpc_concurrency_limit() -> usize {
    32
}

const fn default_rpc_retry_max_attempts() -> u32 {
    3
}

const fn default_rpc_retry_initial_backoff_ms() -> u64 {
    500
}

const fn default_rpc_retry_cu_per_second() -> u64 {
    1_000
}

use super::validation;

/// Validate that a string is a valid URL.
pub(crate) fn is_valid_url(url: &str) -> bool {
    url.starts_with("http://")
        || url.starts_with("https://")
        || url.starts_with("ws://")
        || url.starts_with("wss://")
}

/// RPC configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcConfig {
    pub l1_rpc: String,
    pub l2_rpc: String,
    pub l1_chain_id: u64,
    pub l2_chain_id: u64,
    #[serde(default)]
    pub client: RpcClientConfig,
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            l1_rpc: "http://localhost:8545".to_string(),
            l2_rpc: "http://localhost:9545".to_string(),
            l1_chain_id: 1,
            l2_chain_id: 167_000,
            client: RpcClientConfig::default(),
        }
    }
}

/// RPC client configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcClientConfig {
    #[serde(default = "default_rpc_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_rpc_concurrency_limit")]
    pub concurrency_limit: usize,
    #[serde(default)]
    pub retry: RpcRetryConfig,
}

impl Default for RpcClientConfig {
    fn default() -> Self {
        Self {
            timeout_ms: default_rpc_timeout_ms(),
            concurrency_limit: default_rpc_concurrency_limit(),
            retry: RpcRetryConfig::default(),
        }
    }
}

/// RPC retry configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRetryConfig {
    #[serde(default = "default_rpc_retry_max_attempts")]
    pub max_attempts: u32,
    #[serde(default = "default_rpc_retry_initial_backoff_ms")]
    pub initial_backoff_ms: u64,
    #[serde(default = "default_rpc_retry_cu_per_second")]
    pub compute_units_per_second: u64,
}

impl Default for RpcRetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: default_rpc_retry_max_attempts(),
            initial_backoff_ms: default_rpc_retry_initial_backoff_ms(),
            compute_units_per_second: default_rpc_retry_cu_per_second(),
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
        if self.client.timeout_ms == 0 {
            bail!("rpc client timeout_ms must be > 0");
        }
        if self.client.concurrency_limit == 0 {
            bail!("rpc client concurrency_limit must be > 0");
        }
        if self.client.retry.max_attempts == 0 {
            // Allow disabling retries by setting 0.
        } else {
            if self.client.retry.initial_backoff_ms == 0 {
                bail!("rpc client retry initial_backoff_ms must be > 0");
            }
            if self.client.retry.compute_units_per_second == 0 {
                bail!("rpc client retry compute_units_per_second must be > 0");
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn provider_client_config(&self) -> ProviderRpcClientConfig {
        ProviderRpcClientConfig {
            timeout_ms: self.client.timeout_ms,
            concurrency_limit: self.client.concurrency_limit,
            retry: ProviderRpcRetryConfig {
                max_attempts: self.client.retry.max_attempts,
                initial_backoff_ms: self.client.retry.initial_backoff_ms,
                compute_units_per_second: self.client.retry.compute_units_per_second,
            },
        }
    }
}
