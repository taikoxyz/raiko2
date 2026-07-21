use crate::config::{Config, ResolvedNetworkPair};
use anyhow::Result;
use raiko2_primitives::{
    PreflightRpcClientConfig, PreflightRpcRetryConfig, ProofContext, ProofRequest, ProofType,
};
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
        serde_json::json!({}),
    );
    context.preflight.resolved_l1_chain_spec = Some(pair.l1_spec.clone());
    context.preflight.resolved_l2_chain_spec = Some(pair.l2_spec.clone());
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
    if let Some(verify_rpc) = config
        .preflight
        .verify_checkpoint_l2_rpc_for_network(&pair.network)
    {
        context.preflight.verify_checkpoint_l2_rpc = Some(verify_rpc.to_owned());
        context.preflight.rpc_client_config = Some(PreflightRpcClientConfig {
            timeout_ms: config.rpc.client.timeout_ms,
            concurrency_limit: config.rpc.client.concurrency_limit,
            retry: PreflightRpcRetryConfig {
                max_attempts: config.rpc.client.retry.max_attempts,
                initial_backoff_ms: config.rpc.client.retry.initial_backoff_ms,
                compute_units_per_second: config.rpc.client.retry.compute_units_per_second,
            },
        });
    }
    Ok(context)
}

pub(crate) fn build_provider(
    config: &Config,
    pair: &ResolvedNetworkPair,
) -> Result<NetworkProvider> {
    let rpc_config = config.rpc.provider_client_config();
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
        retry: RetryPolicy::None,
    }
}

#[allow(clippy::missing_const_for_fn)]
#[cfg(feature = "host")]
pub(crate) fn sp1_scheduler_config(config: &Config) -> SchedulerConfig {
    SchedulerConfig {
        lease_duration: task_lease_duration(config),
        retry: RetryPolicy::Fixed {
            max_attempts: 21,
            delay: Duration::from_secs(5 * 60),
        },
    }
}

#[allow(clippy::missing_const_for_fn)]
#[cfg(feature = "host")]
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
    // especially when provider retries are enabled. Keep worker lease renewal aligned with the
    // effective RPC wait window; proof backends own any stage-specific timeout or retry policy.
    Duration::from_millis(lease_ms.max(60_000))
}

#[cfg(feature = "local-provers")]
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

#[cfg(feature = "host")]
#[allow(clippy::missing_const_for_fn)]
pub(crate) fn sp1_prover_config(config: &Config) -> raiko2_prover::sp1::Sp1Config {
    config.prover.sp1.config.clone()
}

#[cfg(feature = "host")]
pub(crate) fn boundless_prover_config(
    config: &Config,
    pair: &ResolvedNetworkPair,
) -> raiko2_prover::boundless::BoundlessConfig {
    let boundless = config
        .prover
        .risc0
        .boundless
        .apply_pair_override(&pair.boundless)
        .expect("validated boundless config must merge cleanly");
    raiko2_prover::boundless::BoundlessConfig {
        execution_po2: config.prover.risc0.execution_po2,
        offchain: boundless.offchain,
        rpc_url: boundless.rpc_url,
        signer_key: boundless.signer_key,
        deployment: boundless.deployment,
        batch_quote: boundless.batch_quote,
        aggregation_quote: boundless.aggregation_quote,
        offer_params: boundless.offer_params,
        poll_interval_ms: boundless.poll_interval_ms,
        timeout_ms: boundless.timeout_ms,
        rebid_timeout_ms: boundless.rebid_timeout_ms,
        rebid_price_step_bps: boundless.rebid_price_step_bps,
        rebid_max_attempts: boundless.rebid_max_attempts,
    }
}

pub(crate) const fn remote_sgx_prover_config(
    base_url: String,
    timeout_ms: u64,
) -> raiko2_prover::gaiko2::Gaiko2Config {
    raiko2_prover::gaiko2::Gaiko2Config {
        base_url,
        timeout_ms,
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "local-provers")]
    use super::sp1_scheduler_config;
    #[cfg(feature = "host")]
    use super::{boundless_prover_config, boundless_scheduler_config};
    use super::{build_context, scheduler_config};
    use crate::config::{BoundlessPairConfig, Config, ResolvedNetworkPair};
    use raiko2_primitives::{ProofType, SupportedChainSpecs};
    use raiko2_provider::L2ProviderKind;
    use raiko2_queue::RetryPolicy;
    use std::time::Duration;

    fn resolved_pair(network: &str, l1_network: &str) -> ResolvedNetworkPair {
        let specs = SupportedChainSpecs::default();
        let l1_spec = specs
            .get_chain_spec(l1_network)
            .expect("known l1 network")
            .clone();
        let l2_spec = specs
            .get_chain_spec(network)
            .expect("known l2 network")
            .clone();

        ResolvedNetworkPair {
            key: format!("{network}/{l1_network}"),
            network: network.to_string(),
            l1_network: l1_network.to_string(),
            l1_rpc: l1_spec.rpc.clone(),
            l2_rpc: l2_spec.rpc.clone(),
            l2_provider: L2ProviderKind::Reth,
            l2_witness_rpc: l2_spec.rpc.clone(),
            sp1_verifier_rpc_url: None,
            sp1_verifier_address: None,
            boundless: BoundlessPairConfig::default(),
            l1_spec,
            l2_spec,
        }
    }

    #[test]
    fn build_context_carries_network_pair_without_full_prover_config() {
        let config = Config::default();
        let pair = resolved_pair("taiko_hoodi", "hoodi");

        let context =
            build_context(&config, &pair, ProofType::Risc0).expect("context should build");

        assert_eq!(context.config["hoodi_network"], "taiko_hoodi");
        assert_eq!(context.config["hoodi_l1_network"], "hoodi");
        assert!(context.config.get("sp1").is_none());
    }

    #[cfg(feature = "host")]
    #[test]
    fn boundless_scheduler_uses_general_task_policy() {
        let config = Config::default();

        assert_eq!(
            boundless_scheduler_config(&config),
            scheduler_config(&config)
        );
    }

    #[test]
    fn scheduler_lease_duration_uses_rpc_retry_config() {
        let mut config = Config::default();
        config.rpc.client.timeout_ms = 20_000;
        config.rpc.client.retry.max_attempts = 1;
        config.rpc.client.retry.initial_backoff_ms = 500;

        let scheduler = scheduler_config(&config);
        assert_eq!(scheduler.lease_duration, Duration::from_millis(70_500));
    }

    #[test]
    fn scheduler_disables_queue_retry() {
        let config = Config::default();
        let scheduler = scheduler_config(&config);

        assert_eq!(scheduler.retry, RetryPolicy::None);
    }

    #[cfg(feature = "local-provers")]
    #[test]
    fn sp1_scheduler_retries_pre_checkpoint_failures() {
        let scheduler = sp1_scheduler_config(&Config::default());
        assert_eq!(
            scheduler.retry,
            RetryPolicy::Fixed {
                max_attempts: 21,
                delay: Duration::from_secs(5 * 60),
            }
        );
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

    #[cfg(feature = "host")]
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

        let boundless = boundless_prover_config(&config, &pair);

        assert_eq!(boundless.execution_po2, 24);
    }

    #[cfg(feature = "host")]
    #[test]
    fn boundless_prover_applies_pair_specific_overrides() {
        let mut config = Config::default();
        config.prover.risc0.boundless.offchain = true;
        config.prover.risc0.boundless.rpc_url = "https://boundless.example.com".to_string();
        config.prover.risc0.boundless.signer_key = "configured-by-secret-store".to_string();
        config.prover.risc0.boundless.rebid_timeout_ms = 900_000;
        config.prover.risc0.boundless.rebid_price_step_bps = 3000;
        config.prover.risc0.boundless.rebid_max_attempts = 5;
        config.rpc.pairs[0].boundless.batch_quote =
            Some(raiko2_prover::boundless::QuoteSizing::Fixed { mcycles: 5_000 });
        config.rpc.pairs[0].boundless.aggregation_quote =
            Some(raiko2_prover::boundless::QuoteSizing::Evaluated);
        config.rpc.pairs[0].boundless.poll_interval_ms = Some(15_000);
        config.rpc.pairs[0].boundless.timeout_ms = Some(4_200_000);
        config.rpc.pairs[0].boundless.rebid_timeout_ms = Some(450_000);
        config.rpc.pairs[0].boundless.rebid_price_step_bps = Some(4000);
        config.rpc.pairs[0].boundless.rebid_max_attempts = Some(2);
        config.rpc.pairs[0].boundless.offer_params.batch =
            Some(raiko2_prover::boundless::BoundlessOfferParams {
                timeouts: raiko2_prover::boundless::TimeoutPolicy::PerMcycle {
                    lock_timeout_ms_per_mcycle: 300,
                    timeout_ms_per_mcycle: 500,
                    dynamic_pricing_timeout_modifier: None,
                },
                ..config.prover.risc0.boundless.offer_params.batch.clone()
            });
        config.rpc.pairs[0].boundless.offer_params.aggregation =
            Some(raiko2_prover::boundless::BoundlessOfferParams {
                timeouts: raiko2_prover::boundless::TimeoutPolicy::PerMcycle {
                    lock_timeout_ms_per_mcycle: 3_000,
                    timeout_ms_per_mcycle: 7_000,
                    dynamic_pricing_timeout_modifier: None,
                },
                ..config
                    .prover
                    .risc0
                    .boundless
                    .offer_params
                    .aggregation
                    .clone()
            });
        let pair = config
            .rpc
            .resolved_pairs()
            .expect("resolved rpc pairs")
            .pop()
            .expect("default pair");

        let boundless = boundless_prover_config(&config, &pair);

        assert!(boundless.offchain);
        assert_eq!(boundless.rpc_url, "https://boundless.example.com");
        assert_eq!(boundless.signer_key, "configured-by-secret-store");
        assert!(boundless.deployment.is_some());
        assert_eq!(
            boundless.batch_quote,
            raiko2_prover::boundless::QuoteSizing::Fixed { mcycles: 5_000 }
        );
        assert_eq!(
            boundless.aggregation_quote,
            raiko2_prover::boundless::QuoteSizing::Evaluated
        );
        assert_eq!(
            boundless.offer_params.batch.timeouts,
            raiko2_prover::boundless::TimeoutPolicy::PerMcycle {
                lock_timeout_ms_per_mcycle: 300,
                timeout_ms_per_mcycle: 500,
                dynamic_pricing_timeout_modifier: None,
            }
        );
        assert_eq!(
            boundless.offer_params.aggregation.timeouts,
            raiko2_prover::boundless::TimeoutPolicy::PerMcycle {
                lock_timeout_ms_per_mcycle: 3_000,
                timeout_ms_per_mcycle: 7_000,
                dynamic_pricing_timeout_modifier: None,
            }
        );
        assert_eq!(boundless.poll_interval_ms, 15_000);
        assert_eq!(boundless.timeout_ms, 4_200_000);
        assert_eq!(boundless.rebid_timeout_ms, 450_000);
        assert_eq!(boundless.rebid_price_step_bps, 4000);
        assert_eq!(boundless.rebid_max_attempts, 2);
    }

    #[test]
    fn build_context_uses_network_specific_preflight_verify_rpc_and_rpc_client_config() {
        let mut config = Config::default();
        config.preflight.verify_checkpoint_l2_rpcs.insert(
            "taiko_hoodi".to_string(),
            "https://verify.hoodi.example".to_string(),
        );
        config.rpc.client.timeout_ms = 1234;
        config.rpc.client.concurrency_limit = 7;
        config.rpc.client.retry.max_attempts = 9;
        config.rpc.client.retry.initial_backoff_ms = 88;
        config.rpc.client.retry.compute_units_per_second = 77;

        let pair = resolved_pair("taiko_hoodi", "hoodi");
        let context = build_context(&config, &pair, ProofType::Risc0).expect("context");

        assert_eq!(
            context.preflight.verify_checkpoint_l2_rpc.as_deref(),
            Some("https://verify.hoodi.example")
        );
        assert_eq!(
            context
                .preflight
                .rpc_client_config
                .as_ref()
                .map(|cfg| cfg.timeout_ms),
            Some(1234)
        );
        assert_eq!(
            context
                .preflight
                .rpc_client_config
                .as_ref()
                .map(|cfg| cfg.concurrency_limit),
            Some(7)
        );
        assert_eq!(
            context
                .preflight
                .rpc_client_config
                .as_ref()
                .map(|cfg| cfg.retry.max_attempts),
            Some(9)
        );
    }

    #[test]
    fn build_context_carries_resolved_chain_specs() {
        let config = Config::default();
        let mut pair = resolved_pair("taiko_dev", "taiko_dev_l1");
        pair.l1_spec.beacon_rpc = Some("https://beacon.example.test/".to_string());

        let context = build_context(&config, &pair, ProofType::Sp1).expect("context");
        let l1_spec = context
            .preflight
            .resolved_l1_chain_spec
            .as_ref()
            .expect("resolved l1 chain spec");

        assert_eq!(l1_spec.chain_id, pair.l1_spec.chain_id);
        assert_eq!(
            l1_spec.beacon_rpc.as_deref(),
            Some("https://beacon.example.test/")
        );
        assert_eq!(
            context.preflight.resolved_l2_chain_spec.as_ref(),
            Some(&pair.l2_spec)
        );
    }
}
