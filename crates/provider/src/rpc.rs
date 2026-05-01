use alloy::rpc::client::RpcClient;
use alloy::transports::http::ReqwestTransport;
use alloy::transports::layers::RetryBackoffLayer;
use alloy::transports::{
    BoxTransport, Transport, TransportError, TransportErrorKind, TransportFut,
};
use alloy_json_rpc::{RequestPacket, ResponsePacket};
use raiko2_primitives::{RaikoError, RaikoResult};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tower::{Layer, Service, ServiceExt};

pub const DEFAULT_RPC_TIMEOUT_MS: u64 = 600_000;

const fn default_concurrency_limit() -> usize {
    32
}

const fn default_retry_max_attempts() -> u32 {
    4
}

const fn default_retry_initial_backoff_ms() -> u64 {
    1_000
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

const fn default_timeout_ms() -> u64 {
    DEFAULT_RPC_TIMEOUT_MS
}

#[derive(Clone)]
struct TimeoutTransport<T> {
    inner: T,
    timeout: Duration,
}

impl<T> TimeoutTransport<T> {
    const fn new(inner: T, timeout: Duration) -> Self {
        Self { inner, timeout }
    }
}

impl<T> Service<RequestPacket> for TimeoutTransport<T>
where
    T: Transport + Clone,
{
    type Response = ResponsePacket;
    type Error = TransportError;
    type Future = TransportFut<'static>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: RequestPacket) -> Self::Future {
        let timeout = self.timeout;
        let fut = self.inner.call(request);
        Box::pin(async move {
            match tokio::time::timeout(timeout, fut).await {
                Ok(res) => res,
                Err(elapsed) => Err(TransportErrorKind::custom(elapsed)),
            }
        })
    }
}

#[derive(Clone)]
struct ConcurrencyLimitTransport<T> {
    inner: T,
    semaphore: Arc<Semaphore>,
}

impl<T> ConcurrencyLimitTransport<T> {
    fn new(inner: T, max: usize) -> Self {
        Self {
            inner,
            semaphore: Arc::new(Semaphore::new(max)),
        }
    }
}

impl<T> Service<RequestPacket> for ConcurrencyLimitTransport<T>
where
    T: Transport + Clone,
{
    type Response = ResponsePacket;
    type Error = TransportError;
    type Future = TransportFut<'static>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: RequestPacket) -> Self::Future {
        let semaphore = Arc::clone(&self.semaphore);
        let mut inner = self.inner.clone();
        Box::pin(async move {
            let permit = semaphore
                .acquire_owned()
                .await
                .map_err(TransportErrorKind::custom)?;
            let _permit = permit;

            inner.ready().await?;
            inner.call(request).await
        })
    }
}

/// Build an [`RpcClient`] for the given RPC URL and configuration.
///
/// # Errors
///
/// Returns an error if `rpc_url` is not a valid URL.
pub fn build_rpc_client(rpc_url: &str, config: &RpcClientConfig) -> RaikoResult<RpcClient> {
    let url = reqwest::Url::parse(rpc_url)
        .map_err(|e| RaikoError::RPC(format!("Invalid RPC URL: {e}")))?;

    let base = ReqwestTransport::new(url);
    let is_local = base.guess_local();
    let mut transport = BoxTransport::new(base);

    if config.concurrency_limit > 0 {
        transport = BoxTransport::new(ConcurrencyLimitTransport::new(
            transport,
            config.concurrency_limit,
        ));
    }

    if config.timeout_ms > 0 {
        transport = BoxTransport::new(TimeoutTransport::new(
            transport,
            Duration::from_millis(config.timeout_ms),
        ));
    }

    if config.retry.max_attempts > 0 {
        let retry = RetryBackoffLayer::new(
            config.retry.max_attempts,
            config.retry.initial_backoff_ms,
            config.retry.compute_units_per_second,
        );
        transport = BoxTransport::new(retry.layer(transport));
    }

    Ok(RpcClient::new(transport, is_local))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_rpc_timeout_uses_exported_constant() {
        assert_eq!(
            RpcClientConfig::default().timeout_ms,
            DEFAULT_RPC_TIMEOUT_MS
        );
    }
}
