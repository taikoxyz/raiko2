use crate::config::{Config, QueueBackend, QueueConfig};
use alloy::providers::{Provider as AlloyProvider, ProviderBuilder};
use anyhow::{Context, Result, bail};
use raiko2_provider::rpc::build_rpc_client;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ReadyCheck {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ReadyCheck {
    const fn ok() -> Self {
        Self {
            ok: true,
            error: None,
        }
    }

    fn err(err: &anyhow::Error) -> Self {
        Self {
            ok: false,
            error: Some(err.to_string()),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ReadyResponse {
    pub status: &'static str,
    pub reth: ReadyCheck,
    pub queue: ReadyCheck,
}

pub async fn evaluate_readiness(config: &Config) -> ReadyResponse {
    let reth = match check_reth(config).await {
        Ok(()) => ReadyCheck::ok(),
        Err(err) => ReadyCheck::err(&err),
    };

    let queue = {
        #[cfg(feature = "redis-queue")]
        {
            match check_queue(config).await {
                Ok(()) => ReadyCheck::ok(),
                Err(err) => ReadyCheck::err(&err),
            }
        }

        #[cfg(not(feature = "redis-queue"))]
        {
            match check_queue(config) {
                Ok(()) => ReadyCheck::ok(),
                Err(err) => ReadyCheck::err(&err),
            }
        }
    };

    let status = if reth.ok && queue.ok { "ok" } else { "error" };

    ReadyResponse {
        status,
        reth,
        queue,
    }
}

pub async fn ensure_startup_ready(config: &Config) -> Result<()> {
    check_reth(config).await.context("reth readiness failed")?;
    #[cfg(feature = "redis-queue")]
    check_queue(config)
        .await
        .context("queue readiness failed")?;

    #[cfg(not(feature = "redis-queue"))]
    check_queue(config).context("queue readiness failed")?;
    Ok(())
}

async fn check_reth(config: &Config) -> Result<()> {
    for pair in config.rpc.resolved_pairs()? {
        let rpc_client = build_rpc_client(&pair.l2_rpc, &config.rpc.provider_client_config())
            .with_context(|| format!("failed to build RPC client for {}", pair.key))?;
        let provider = ProviderBuilder::new().connect_client(rpc_client);
        let chain_id = provider
            .get_chain_id()
            .await
            .with_context(|| format!("eth_chainId failed for {}", pair.key))?;
        if chain_id != pair.l2_chain_id() {
            bail!(
                "l2 chain_id mismatch for {}: expected {}, got {}",
                pair.key,
                pair.l2_chain_id(),
                chain_id
            );
        }
    }
    Ok(())
}

#[cfg(feature = "redis-queue")]
async fn check_queue(config: &Config) -> Result<()> {
    match config.queue.backend {
        QueueBackend::Memory => Ok(()),
        QueueBackend::Redis => check_redis_queue(&config.queue).await,
    }
}

#[cfg(not(feature = "redis-queue"))]
fn check_queue(config: &Config) -> Result<()> {
    match config.queue.backend {
        QueueBackend::Memory => Ok(()),
        QueueBackend::Redis => check_redis_queue(&config.queue),
    }
}

#[cfg(feature = "redis-queue")]
async fn check_redis_queue(config: &QueueConfig) -> Result<()> {
    let url = config.redis_url.clone().unwrap_or_default();
    let namespace = config.namespace.clone();
    let _store = raiko2_queue::RedisStore::<(), (), ()>::connect(
        &url,
        &namespace,
        std::time::Duration::from_secs(60),
    )
    .await
    .context("failed to connect to redis queue")?;
    Ok(())
}

#[cfg(not(feature = "redis-queue"))]
fn check_redis_queue(_config: &QueueConfig) -> Result<()> {
    bail!("redis queue requires building raiko2 with `--features redis-queue`");
}
