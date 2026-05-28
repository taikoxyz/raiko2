use crate::config::{Config, GuestSystem, PipelineRoute, QueueBackend, QueueConfig, RunnerKind};
use alloy::providers::{Provider as AlloyProvider, ProviderBuilder};
use alloy_signer_local::PrivateKeySigner;
use anyhow::{Context, Result, bail};
use raiko2_prover::sp1::{
    ExecutionMode as Sp1ExecutionMode, ProverMode as Sp1ProverMode, Sp1Config,
    Sp1RemoteVerifyConfig, Sp1SystemConfig,
};
use raiko2_provider::rpc::build_rpc_client;
use serde::Serialize;
use url::Url;

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
            error: Some(format_error_chain(err)),
        }
    }
}

fn format_error_chain(err: &anyhow::Error) -> String {
    let mut chain = err.chain();
    let Some(root) = chain.next() else {
        return String::new();
    };

    let mut message = root.to_string();
    for source in chain {
        let source = source.to_string();
        if !source.is_empty() {
            message.push_str(": ");
            message.push_str(&source);
        }
    }
    message
}

#[derive(Debug, Serialize)]
pub struct ReadyResponse {
    pub status: &'static str,
    /// Legacy field name retained for compatibility; this covers configured L1/L2 RPC readiness.
    pub reth: ReadyCheck,
    pub queue: ReadyCheck,
    pub prover: ReadyCheck,
}

pub async fn evaluate_readiness(config: &Config) -> ReadyResponse {
    let reth = match check_rpc_pairs(config).await {
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

    let prover = match check_prover(config) {
        Ok(()) => ReadyCheck::ok(),
        Err(err) => ReadyCheck::err(&err),
    };

    let status = if reth.ok && queue.ok && prover.ok {
        "ok"
    } else {
        "error"
    };

    ReadyResponse {
        status,
        reth,
        queue,
        prover,
    }
}

pub async fn ensure_startup_ready(config: &Config) -> Result<()> {
    check_rpc_pairs(config)
        .await
        .context("reth readiness failed")?;
    #[cfg(feature = "redis-queue")]
    check_queue(config)
        .await
        .context("queue readiness failed")?;

    #[cfg(not(feature = "redis-queue"))]
    check_queue(config).context("queue readiness failed")?;
    check_prover(config).context("prover readiness failed")?;
    Ok(())
}

async fn check_rpc_pairs(config: &Config) -> Result<()> {
    let client_config = config.rpc.provider_client_config();
    for pair in config.rpc.resolved_pairs()? {
        check_rpc_chain_id(
            "l1",
            &pair.key,
            &pair.l1_rpc,
            pair.l1_chain_id(),
            &client_config,
        )
        .await?;
        check_rpc_chain_id(
            "l2",
            &pair.key,
            &pair.l2_rpc,
            pair.l2_chain_id(),
            &client_config,
        )
        .await?;
        if pair.l2_witness_rpc != pair.l2_rpc {
            check_rpc_chain_id(
                "l2_witness",
                &pair.key,
                &pair.l2_witness_rpc,
                pair.l2_chain_id(),
                &client_config,
            )
            .await?;
        }
    }
    Ok(())
}

async fn check_rpc_chain_id(
    kind: &str,
    pair_key: &str,
    rpc_url: &str,
    expected_chain_id: u64,
    client_config: &raiko2_provider::RpcClientConfig,
) -> Result<()> {
    let rpc_client = build_rpc_client(rpc_url, client_config)
        .with_context(|| format!("failed to build {kind} RPC client for {pair_key}"))?;
    let provider = ProviderBuilder::new().connect_client(rpc_client);
    let chain_id = provider
        .get_chain_id()
        .await
        .with_context(|| format!("{kind} eth_chainId failed for {pair_key}"))?;
    if chain_id != expected_chain_id {
        bail!(
            "{kind} chain_id mismatch for {pair_key}: expected {expected_chain_id}, got {chain_id}"
        );
    }
    Ok(())
}

fn check_prover(config: &Config) -> Result<()> {
    config
        .prover
        .validate()
        .context("configured proving capabilities are invalid")?;

    if requires_risc0_capability_check(config) {
        check_risc0_capability(config).context("risc0 capability is invalid")?;
    }
    if requires_sp1_capability_check(config) {
        check_sp1_capability(config).context("sp1 capability is invalid")?;
    }

    Ok(())
}

const fn requires_risc0_capability_check(config: &Config) -> bool {
    !config.prover.is_remote_sgx_route() && !config.prover.is_tdx_route()
}

fn check_risc0_capability(config: &Config) -> Result<()> {
    match config.prover.route() {
        PipelineRoute {
            guest_system: GuestSystem::Risc0,
            runner: RunnerKind::Network,
        } => check_boundless_prover(config),
        _ => Ok(()),
    }
}

const fn requires_sp1_capability_check(config: &Config) -> bool {
    !config.prover.is_remote_sgx_route() && !config.prover.is_tdx_route()
}

fn check_sp1_capability(config: &Config) -> Result<()> {
    check_sp1_prover(config)
}

fn check_boundless_prover(config: &Config) -> Result<()> {
    let boundless = &config.prover.boundless;
    Url::parse(&boundless.rpc_url).context("boundless rpc_url is not a valid URL")?;
    let _: PrivateKeySigner = boundless
        .signer_key
        .parse()
        .context("boundless signer_key is invalid")?;

    if let Some(order_stream_url) = boundless
        .deployment
        .as_ref()
        .and_then(|deployment| deployment.overrides.as_ref())
        .and_then(|overrides| overrides.get("order_stream_url"))
        .and_then(serde_json::Value::as_str)
    {
        Url::parse(order_stream_url)
            .context("boundless deployment overrides.order_stream_url is not a valid URL")?;
    }

    Ok(())
}

fn check_sp1_prover(config: &Config) -> Result<()> {
    let sp1 = &config.prover.sp1;
    sp1.validate().map_err(anyhow::Error::msg)?;

    if sp1.prover == Sp1ProverMode::Network {
        if matches!(sp1.mode, Sp1ExecutionMode::Prove) && sp1.verify {
            for pair in config.rpc.resolved_pairs()? {
                if sp1_effective_pair_config(sp1, &pair)
                    .remote_verify
                    .is_none()
                {
                    bail!(
                        "sp1 network verification is not enabled for network pair {}",
                        pair.key
                    );
                }
            }
        }
        let private_key = std::env::var("NETWORK_PRIVATE_KEY")
            .context("NETWORK_PRIVATE_KEY must be set for sp1.prover=network")?;
        let _: PrivateKeySigner = private_key
            .parse()
            .context("NETWORK_PRIVATE_KEY is invalid")?;
    }

    if let Some(rpc_url) = sp1.rpc_url.as_deref() {
        Url::parse(rpc_url).context("sp1.rpc_url is not a valid URL")?;
    }

    Ok(())
}

fn sp1_effective_pair_config(
    global: &Sp1Config,
    pair: &crate::config::ResolvedNetworkPair,
) -> Sp1Config {
    match (
        pair.sp1_verifier_rpc_url.as_ref(),
        pair.sp1_verifier_address.as_ref(),
    ) {
        (Some(rpc_url), Some(verifier_address)) => Sp1SystemConfig {
            remote_verify: Some(Sp1RemoteVerifyConfig {
                rpc_url: rpc_url.clone(),
                verifier_address: verifier_address.clone(),
            }),
        }
        .applied_to(global),
        _ => global.clone(),
    }
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

#[cfg(test)]
mod tests {
    use super::{
        Sp1RemoteVerifyConfig, check_prover, requires_sp1_capability_check,
        sp1_effective_pair_config,
    };
    use crate::config::{Config, GuestSystem, RunnerKind};

    #[test]
    fn sp1_effective_pair_config_honors_global_remote_verify() {
        let mut config = Config::default();
        config.prover.sp1.remote_verify = Some(Sp1RemoteVerifyConfig {
            rpc_url: "https://global-verifier.example.com".to_string(),
            verifier_address: "0x0000000000000000000000000000000000000001".to_string(),
        });
        let pair = config
            .rpc
            .resolved_pairs()
            .expect("resolve default pair")
            .remove(0);

        let effective = sp1_effective_pair_config(&config.prover.sp1, &pair);

        assert_eq!(
            effective
                .remote_verify
                .expect("global remote verifier")
                .rpc_url,
            "https://global-verifier.example.com"
        );
    }

    #[test]
    fn sp1_effective_pair_config_uses_pair_remote_verify_when_present() {
        let mut config = Config::default();
        config.prover.sp1.remote_verify = Some(Sp1RemoteVerifyConfig {
            rpc_url: "https://global-verifier.example.com".to_string(),
            verifier_address: "0x0000000000000000000000000000000000000001".to_string(),
        });
        let mut pair = config
            .rpc
            .resolved_pairs()
            .expect("resolve default pair")
            .remove(0);
        pair.sp1_verifier_rpc_url = Some("https://pair-verifier.example.com".to_string());
        pair.sp1_verifier_address = Some("0x0000000000000000000000000000000000000002".to_string());

        let effective = sp1_effective_pair_config(&config.prover.sp1, &pair);

        assert_eq!(
            effective
                .remote_verify
                .expect("pair remote verifier")
                .rpc_url,
            "https://pair-verifier.example.com"
        );
    }

    #[test]
    fn remote_sgx_route_does_not_require_sp1_capability_check() {
        let mut config = Config::default();
        config.prover.guest_system = GuestSystem::Sgx;
        config.prover.runner = RunnerKind::Remote;
        config.prover.remote_sgx.base_url = "http://127.0.0.1:9090".to_string();

        assert!(!requires_sp1_capability_check(&config));
        assert!(check_prover(&config).is_ok());
    }
}
