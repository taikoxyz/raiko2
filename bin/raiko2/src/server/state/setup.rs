use crate::config::{Config, RetryStrategy};
use anyhow::Result;
use raiko2_primitives::{ProofContext, ProofRequest};
use raiko2_provider::NetworkProvider;
use raiko2_queue::{RetryPolicy, SchedulerConfig};
use std::time::Duration;

pub(crate) fn build_context(config: &Config, proof_type: &str) -> ProofContext {
    ProofContext::new(
        ProofRequest {
            l1_chain_id: config.rpc.l1_chain_id,
            l2_chain_id: config.rpc.l2_chain_id,
            proposal_id: 0,
            l2_block_range: None,
            proof_type: proof_type.to_string(),
            blob_proof_type: None,
            prover: None,
            graffiti: None,
        },
        raiko2_primitives::ProverConfig::default(),
    )
}

pub(crate) fn build_provider(config: &Config) -> Result<NetworkProvider> {
    let rpc_config = config.rpc.provider_client_config();
    NetworkProvider::new_with_config(&config.rpc.l2_rpc, &rpc_config)
        .map_err(|e| anyhow::anyhow!(e))
}

#[allow(clippy::missing_const_for_fn)]
pub(crate) fn scheduler_config(config: &Config) -> SchedulerConfig {
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
        lease_duration: Duration::from_secs(60),
        retry: retry_policy,
    }
}

#[allow(clippy::missing_const_for_fn)]
pub(crate) fn agent_scheduler_config(config: &Config) -> SchedulerConfig {
    let timeout_ms = config.prover.agent.timeout_ms;
    let poll_interval_ms = config.prover.agent.poll_interval_ms;
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

pub(crate) fn agent_prover_config(config: &Config) -> raiko2_prover::agent::AgentConfig {
    raiko2_prover::agent::AgentConfig {
        base_url: config.prover.agent.url.clone(),
        prover_type: config.prover.agent.prover_type.clone(),
        api_key: config.prover.agent.api_key.clone(),
        poll_interval_ms: config.prover.agent.poll_interval_ms,
        timeout_ms: config.prover.agent.timeout_ms,
    }
}

#[cfg(feature = "redis-queue")]
use raiko2_pipeline::PipelineKey;

#[cfg(feature = "redis-queue")]
pub(crate) fn queue_namespace(base: &str, key: PipelineKey) -> String {
    let base = base.trim_end_matches('/');
    format!("{}/{}", base, key.as_str())
}

#[cfg(test)]
mod tests {
    use super::agent_scheduler_config;
    use crate::config::Config;
    use std::time::Duration;

    #[test]
    fn agent_scheduler_lease_covers_agent_timeout() {
        let mut config = Config::default();
        config.prover.agent.timeout_ms = 300_000;
        config.prover.agent.poll_interval_ms = 1_000;

        let scheduler = agent_scheduler_config(&config);
        assert_eq!(scheduler.lease_duration, Duration::from_millis(331_000));
    }

    #[test]
    fn agent_scheduler_lease_has_one_minute_floor() {
        let mut config = Config::default();
        config.prover.agent.timeout_ms = 5_000;
        config.prover.agent.poll_interval_ms = 1_000;

        let scheduler = agent_scheduler_config(&config);
        assert_eq!(scheduler.lease_duration, Duration::from_secs(60));
    }
}
