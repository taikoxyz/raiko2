use crate::config::{Config, ResolvedNetworkPair, RetryStrategy};
use anyhow::Result;
use raiko2_primitives::{ProofContext, ProofRequest};
use raiko2_provider::NetworkProvider;
use raiko2_queue::{RetryPolicy, SchedulerConfig};
use std::time::Duration;

pub(crate) fn build_context(
    config: &Config,
    pair: &ResolvedNetworkPair,
    proof_type: &str,
) -> Result<ProofContext> {
    let mut context = ProofContext::new(
        ProofRequest {
            l1_chain_id: pair.l1_chain_id(),
            l2_chain_id: pair.l2_chain_id(),
            proposal_id: 0,
            l2_block_range: None,
            proof_type: proof_type.to_string(),
            blob_proof_type: None,
            prover: None,
            graffiti: None,
        },
        raiko2_primitives::ProverConfig::default(),
    );
    context.l2_chain_spec = pair.l2_spec.to_taiko_chain_spec()?;
    if !context.config.is_object() {
        context.config = serde_json::json!({});
    }
    if let Some(config_obj) = context.config.as_object_mut() {
        config_obj.insert("hoodi_network".to_string(), serde_json::json!(pair.network));
        config_obj.insert(
            "hoodi_l1_network".to_string(),
            serde_json::json!(pair.l1_network),
        );
    }
    let _ = config;
    Ok(context)
}

pub(crate) fn build_provider(
    config: &Config,
    pair: &ResolvedNetworkPair,
) -> Result<NetworkProvider> {
    let rpc_config = config.rpc.provider_client_config();
    let _ = config;
    NetworkProvider::new_with_config(&pair.l2_rpc, &rpc_config).map_err(|e| anyhow::anyhow!(e))
}

#[allow(clippy::missing_const_for_fn)]
pub(crate) fn scheduler_config(config: &Config) -> SchedulerConfig {
    let attempts = u64::from(config.rpc.client.retry.max_attempts.max(1));
    let timeout_ms = config.rpc.client.timeout_ms;
    let backoff_ms = config
        .rpc
        .client
        .retry
        .initial_backoff_ms
        .saturating_mul(attempts.saturating_sub(1));
    let lease_ms = timeout_ms
        .saturating_mul(attempts)
        .saturating_add(backoff_ms)
        .saturating_add(30_000);
    let retry_policy = match config.queue.retry.strategy {
        RetryStrategy::None => RetryPolicy::None,
        RetryStrategy::Fixed => RetryPolicy::Fixed {
            max_attempts: config.queue.retry.max_attempts,
            delay: Duration::from_millis(config.queue.retry.fixed_delay_ms),
        },
        RetryStrategy::Exponential => RetryPolicy::Exponential {
            max_attempts: config.queue.retry.max_attempts,
            base_delay: Duration::from_millis(config.queue.retry.base_delay_ms),
            max_delay: Duration::from_millis(config.queue.retry.max_delay_ms),
        },
    };

    SchedulerConfig {
        // Preflight and local witness generation can run longer than a single RPC timeout,
        // especially when provider retries are enabled. Keep the lease aligned with the
        // effective RPC wait window so maintenance does not requeue the same long-running task.
        lease_duration: Duration::from_millis(lease_ms.max(60_000)),
        retry: retry_policy,
    }
}

#[allow(clippy::missing_const_for_fn)]
pub(crate) fn boundless_scheduler_config(config: &Config) -> SchedulerConfig {
    let timeout_ms = config.prover.boundless.timeout_ms;
    let poll_interval_ms = config.prover.boundless.poll_interval_ms;
    let lease_ms = timeout_ms
        .saturating_add(poll_interval_ms)
        .saturating_add(30_000);

    SchedulerConfig {
        lease_duration: Duration::from_millis(lease_ms.max(60_000)),
        // Agent requests are externally stateful. Once submitted, the outer queue must not
        // retry or lease-expire into a second remote proof request for the same logical task.
        retry: RetryPolicy::None,
    }
}

#[allow(clippy::missing_const_for_fn)]
pub(crate) fn risc0_prover_config(config: &Config) -> raiko2_prover::risc0::Risc0Config {
    raiko2_prover::risc0::Risc0Config {
        bonsai: config.prover.risc0.bonsai,
        snark: config.prover.risc0.snark,
        mock: config.prover.risc0.mock,
        profile: false,
        execution_po2: 20,
        verify: true,
    }
}

#[allow(clippy::missing_const_for_fn)]
pub(crate) fn sp1_prover_config(config: &Config) -> raiko2_prover::sp1::Sp1Config {
    raiko2_prover::sp1::Sp1Config {
        recursion: if config.prover.sp1.plonk {
            raiko2_prover::sp1::RecursionMode::Plonk
        } else {
            raiko2_prover::sp1::RecursionMode::Compressed
        },
        prover: if config.prover.sp1.network {
            Some(raiko2_prover::sp1::ProverMode::Network)
        } else {
            Some(raiko2_prover::sp1::ProverMode::Local)
        },
        verify: true,
    }
}

pub(crate) fn boundless_prover_config(
    config: &Config,
) -> raiko2_prover::boundless::BoundlessConfig {
    raiko2_prover::boundless::BoundlessConfig {
        offchain: config.prover.boundless.offchain,
        rpc_url: config.prover.boundless.rpc_url.clone(),
        signer_key: config.prover.boundless.signer_key.clone(),
        deployment: config.prover.boundless.deployment.clone(),
        offer_params: config.prover.boundless.offer_params.clone(),
        poll_interval_ms: config.prover.boundless.poll_interval_ms,
        timeout_ms: config.prover.boundless.timeout_ms,
    }
}

#[cfg(feature = "redis-queue")]
use raiko2_pipeline::PipelineKey;

#[cfg(feature = "redis-queue")]
pub(crate) fn queue_namespace(base: &str, pair: &ResolvedNetworkPair, key: PipelineKey) -> String {
    let base = base.trim_end_matches('/');
    format!("{}/{}/{}", base, pair.key, key.as_str())
}

#[cfg(test)]
mod tests {
    use super::{boundless_scheduler_config, scheduler_config};
    use crate::config::Config;
    use std::time::Duration;

    #[test]
    fn boundless_scheduler_lease_covers_boundless_timeout() {
        let mut config = Config::default();
        config.prover.boundless.timeout_ms = 300_000;
        config.prover.boundless.poll_interval_ms = 1_000;

        let scheduler = boundless_scheduler_config(&config);
        assert_eq!(scheduler.lease_duration, Duration::from_millis(331_000));
    }

    #[test]
    fn boundless_scheduler_lease_has_one_minute_floor() {
        let mut config = Config::default();
        config.prover.boundless.timeout_ms = 5_000;
        config.prover.boundless.poll_interval_ms = 1_000;

        let scheduler = boundless_scheduler_config(&config);
        assert_eq!(scheduler.lease_duration, Duration::from_secs(60));
    }

    #[test]
    fn scheduler_lease_covers_rpc_timeout_window() {
        let mut config = Config::default();
        config.rpc.client.timeout_ms = 60_000;
        config.rpc.client.retry.max_attempts = 3;
        config.rpc.client.retry.initial_backoff_ms = 500;

        let scheduler = scheduler_config(&config);
        assert_eq!(scheduler.lease_duration, Duration::from_millis(211_000));
    }

    #[test]
    fn scheduler_lease_has_one_minute_floor() {
        let mut config = Config::default();
        config.rpc.client.timeout_ms = 5_000;
        config.rpc.client.retry.max_attempts = 0;

        let scheduler = scheduler_config(&config);
        assert_eq!(scheduler.lease_duration, Duration::from_secs(60));
    }
}
