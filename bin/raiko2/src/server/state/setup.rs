use crate::config::{Config, ResolvedNetworkPair};
use anyhow::Result;
use raiko2_primitives::{ProofContext, ProofRequest, ProofType};
use raiko2_provider::NetworkProvider;
use raiko2_queue::{RetryPolicy, SchedulerConfig};
use std::time::Duration;

pub(crate) fn build_context(
    config: &Config,
    pair: &ResolvedNetworkPair,
    proof_type: ProofType,
) -> Result<ProofContext> {
    let mut context = ProofContext::new(
        ProofRequest {
            l1_chain_id: pair.l1_chain_id(),
            l2_chain_id: pair.l2_chain_id(),
            proposal_id: 0,
            l2_block_range: None,
            shasta: None,
            proof_type,
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
    NetworkProvider::new_pair_with_l2_provider_kind_and_chain_specs_and_config(
        &pair.l1_rpc,
        &pair.l2_rpc,
        pair.l2_provider,
        Some(pair.l1_spec.clone()),
        Some(pair.l2_spec.clone()),
        Some(&pair.l2_witness_rpc),
        &rpc_config,
    )
    .map_err(|e| anyhow::anyhow!(e))
}

#[allow(clippy::missing_const_for_fn)]
pub(crate) fn scheduler_config(config: &Config) -> SchedulerConfig {
    SchedulerConfig {
        lease_duration: task_lease_duration(config),
        task_timeout: Duration::from_secs(config.queue.task_timeout_secs),
        retry: RetryPolicy::None,
    }
}

#[allow(clippy::missing_const_for_fn)]
#[cfg(any(feature = "boundless", test))]
pub(crate) fn boundless_scheduler_config(config: &Config) -> SchedulerConfig {
    scheduler_config(config)
}

#[allow(clippy::missing_const_for_fn)]
fn task_lease_duration(config: &Config) -> Duration {
    let retries = u64::from(config.rpc.client.retry.max_attempts);
    let total_attempts = retries.saturating_add(1);
    let timeout_ms = config.rpc.client.timeout_ms;
    let mut backoff_ms = 0u64;
    let mut next_backoff_ms = config.rpc.client.retry.initial_backoff_ms;
    for _ in 0..retries {
        backoff_ms = backoff_ms.saturating_add(next_backoff_ms);
        next_backoff_ms = next_backoff_ms.saturating_mul(2);
    }
    let lease_ms = timeout_ms
        .saturating_mul(total_attempts)
        .saturating_add(backoff_ms)
        .saturating_add(30_000);

    // Preflight and local witness generation can run longer than a single RPC timeout,
    // especially when provider retries are enabled. Keep lease renewal aligned with the
    // effective RPC wait window, while execution timeout remains the queue-level task timeout.
    Duration::from_millis(lease_ms.max(60_000))
}

#[allow(clippy::missing_const_for_fn)]
pub(crate) fn risc0_prover_config(config: &Config) -> raiko2_prover::risc0::Risc0Config {
    raiko2_prover::risc0::Risc0Config {
        bonsai: config.prover.risc0.bonsai,
        snark: config.prover.risc0.snark,
        mock: config.prover.risc0.mock,
        profile: false,
        execution_po2: config.prover.risc0.execution_po2,
        verify: true,
    }
}

#[allow(clippy::missing_const_for_fn)]
pub(crate) fn sp1_prover_config(config: &Config) -> raiko2_prover::sp1::Sp1Config {
    config.prover.sp1.clone()
}

#[cfg(any(feature = "boundless", test))]
pub(crate) fn boundless_prover_config(
    config: &Config,
    pair: &ResolvedNetworkPair,
) -> raiko2_prover::boundless::BoundlessConfig {
    let boundless = config
        .prover
        .boundless
        .apply_pair_override(&pair.boundless)
        .expect("validated boundless config must merge cleanly");
    raiko2_prover::boundless::BoundlessConfig {
        execution_po2: config.prover.risc0.execution_po2,
        offchain: boundless.offchain,
        rpc_url: boundless.rpc_url,
        signer_key: boundless.signer_key,
        deployment: boundless.deployment,
        batch_quoted_mcycles: boundless.batch_quoted_mcycles,
        batch_quote_strategy: boundless.batch_quote_strategy,
        aggregation_quoted_mcycles: boundless.aggregation_quoted_mcycles,
        offer_params: boundless.offer_params,
        poll_interval_ms: boundless.poll_interval_ms,
        timeout_ms: boundless.timeout_ms,
    }
}

#[cfg(feature = "tdx")]
pub(crate) fn tdx_prover_config(config: &Config) -> raiko2_prover::tdx::TdxConfig {
    raiko2_prover::tdx::TdxConfig {
        instance_id: config.prover.tdx.instance_id,
        socket_path: config.prover.tdx.socket_path.clone(),
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
    use super::{
        boundless_prover_config, boundless_scheduler_config, risc0_prover_config, scheduler_config,
    };
    use crate::config::Config;
    use raiko2_queue::RetryPolicy;
    use std::time::Duration;

    #[test]
    fn boundless_scheduler_uses_general_task_policy() {
        let mut config = Config::default();
        config.queue.task_timeout_secs = 321;

        assert_eq!(
            boundless_scheduler_config(&config),
            scheduler_config(&config)
        );
    }

    #[test]
    fn scheduler_task_timeout_uses_queue_config() {
        let mut config = Config::default();
        config.queue.task_timeout_secs = 321;

        let scheduler = scheduler_config(&config);
        assert_eq!(scheduler.task_timeout, Duration::from_secs(321));
    }

    #[test]
    fn scheduler_disables_queue_retry() {
        let config = Config::default();
        let scheduler = scheduler_config(&config);

        assert_eq!(scheduler.retry, RetryPolicy::None);
    }

    #[test]
    fn scheduler_lease_covers_rpc_timeout_window() {
        let mut config = Config::default();
        config.rpc.client.timeout_ms = 60_000;
        config.rpc.client.retry.max_attempts = 3;
        config.rpc.client.retry.initial_backoff_ms = 500;

        let scheduler = scheduler_config(&config);
        assert_eq!(scheduler.lease_duration, Duration::from_millis(273_500));
    }

    #[test]
    fn scheduler_lease_has_one_minute_floor() {
        let mut config = Config::default();
        config.rpc.client.timeout_ms = 5_000;
        config.rpc.client.retry.max_attempts = 0;

        let scheduler = scheduler_config(&config);
        assert_eq!(scheduler.lease_duration, Duration::from_secs(60));
    }

    #[test]
    fn boundless_prover_inherits_risc0_execution_po2() {
        let mut config = Config::default();
        config.prover.risc0.execution_po2 = 24;
        let pair = config
            .rpc
            .resolved_pairs()
            .expect("resolved rpc pairs")
            .pop()
            .expect("default pair");

        let risc0 = risc0_prover_config(&config);
        let boundless = boundless_prover_config(&config, &pair);

        assert_eq!(risc0.execution_po2, 24);
        assert_eq!(boundless.execution_po2, risc0.execution_po2);
    }

    #[test]
    fn boundless_prover_applies_pair_specific_overrides() {
        let mut config = Config::default();
        config.rpc.pairs[0].boundless.batch_quoted_mcycles = Some(5_000);
        config.rpc.pairs[0].boundless.aggregation_quoted_mcycles = Some(320);
        config.rpc.pairs[0].boundless.offer_params.batch =
            Some(raiko2_prover::boundless::BoundlessOfferParams {
                timeout_ms_per_mcycle: 500,
                ..config.prover.boundless.offer_params.batch.clone()
            });
        config.rpc.pairs[0].boundless.offer_params.aggregation =
            Some(raiko2_prover::boundless::BoundlessOfferParams {
                timeout_ms_per_mcycle: 7_000,
                ..config.prover.boundless.offer_params.aggregation.clone()
            });
        let pair = config
            .rpc
            .resolved_pairs()
            .expect("resolved rpc pairs")
            .pop()
            .expect("default pair");

        let boundless = boundless_prover_config(&config, &pair);

        assert_eq!(boundless.batch_quoted_mcycles, Some(5_000));
        assert_eq!(boundless.aggregation_quoted_mcycles, 320);
        assert_eq!(boundless.offer_params.batch.timeout_ms_per_mcycle, 500);
        assert_eq!(
            boundless.offer_params.aggregation.timeout_ms_per_mcycle,
            7_000
        );
    }
}
