use alloy::rpc::client::RpcClient;
use alloy::transports::layers::RetryBackoffLayer;
use raiko2_primitives::{RaikoError, RaikoResult};
use std::time::Duration;
use tower::{limit::ConcurrencyLimitLayer, timeout::TimeoutLayer};

const fn default_timeout_ms() -> u64 {
    10_000
}

const fn default_concurrency_limit() -> usize {
    32
}

const fn default_retry_max_attempts() -> u32 {
    3
}

const fn default_retry_initial_backoff_ms() -> u64 {
    500
}

const fn default_retry_cu_per_second() -> u64 {
    1_000
}

#[derive(Debug, Clone)]
pub struct RpcRetryConfig {
    pub max_attempts: u32,
    pub initial_backoff_ms: u64,
    pub compute_units_per_second: u64,
}

impl Default for RpcRetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: default_retry_max_attempts(),
            initial_backoff_ms: default_retry_initial_backoff_ms(),
            compute_units_per_second: default_retry_cu_per_second(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RpcClientConfig {
    pub timeout_ms: u64,
    pub concurrency_limit: usize,
    pub retry: RpcRetryConfig,
}

impl Default for RpcClientConfig {
    fn default() -> Self {
        Self {
            timeout_ms: default_timeout_ms(),
            concurrency_limit: default_concurrency_limit(),
            retry: RpcRetryConfig::default(),
        }
    }
}

pub fn build_rpc_client(rpc_url: &str, config: &RpcClientConfig) -> RaikoResult<RpcClient> {
    let url = reqwest::Url::parse(rpc_url)
        .map_err(|e| RaikoError::RPC(format!("Invalid RPC URL: {e}")))?;

    let mut builder = RpcClient::builder();

    if config.retry.max_attempts > 0 {
        builder = builder.layer(RetryBackoffLayer::new(
            config.retry.max_attempts,
            config.retry.initial_backoff_ms,
            config.retry.compute_units_per_second,
        ));
    }

    if config.timeout_ms > 0 {
        builder = builder.layer(TimeoutLayer::new(Duration::from_millis(config.timeout_ms)));
    }

    if config.concurrency_limit > 0 {
        builder = builder.layer(ConcurrencyLimitLayer::new(config.concurrency_limit));
    }

    Ok(builder.http(url))
}
