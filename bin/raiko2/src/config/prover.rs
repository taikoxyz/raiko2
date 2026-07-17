use anyhow::{Context, Result, bail};
use raiko2_pipeline::{GuestSystem, PipelineRoute, RunnerKind};
use raiko2_primitives::ProofType;
use raiko2_prover::{
    boundless_config::{
        DEFAULT_REBID_MAX_ATTEMPTS, DEFAULT_REBID_PRICE_STEP_BPS, DEFAULT_REBID_TIMEOUT_MS,
        DeploymentConfig, MIN_MEANINGFUL_REBID_PRICE_STEP_BPS, MIN_REBID_TIMEOUT_MS,
        OfferParamsConfig, QuoteSizing, REBID_MAX_ATTEMPTS_LIMIT, validate_offer_spec,
    },
    gaiko2::Gaiko2Config as Gaiko2ProverConfig,
    sp1_config::{ExecutionMode as Sp1ExecutionMode, ProverMode as Sp1ProverMode, Sp1Config},
};
use serde::{Deserialize, Serialize};

use super::BoundlessPairConfig;

/// Prover configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ProverConfig {
    #[serde(default)]
    pub guest_system: GuestSystem,
    #[serde(default)]
    pub runner: RunnerKind,
    /// RISC0 specific configuration.
    #[serde(default)]
    pub risc0: Risc0Config,
    /// SP1 specific configuration.
    #[serde(default)]
    pub sp1: Sp1Config,
    /// Boundless runner configuration.
    #[serde(default)]
    pub boundless: BoundlessConfig,
    /// Request sampling policy for `proof_type=zk_any`.
    #[serde(default)]
    pub zk_any: ZkAnyConfig,
    /// Remote SGX prover configuration.
    #[serde(default)]
    pub remote_sgx: RemoteSgxConfig,
}

impl ProverConfig {
    #[must_use]
    pub const fn route(&self) -> PipelineRoute {
        PipelineRoute::new(self.guest_system, self.runner)
    }

    #[must_use]
    pub const fn sp1_route(&self) -> PipelineRoute {
        let runner = match self.sp1.prover {
            Sp1ProverMode::Network => RunnerKind::Network,
            Sp1ProverMode::Mock | Sp1ProverMode::Local => RunnerKind::Local,
        };
        PipelineRoute::new(GuestSystem::Sp1, runner)
    }

    #[must_use]
    pub const fn is_remote_sgx_route(&self) -> bool {
        matches!(
            self.route(),
            PipelineRoute {
                guest_system: GuestSystem::Sgx,
                runner: RunnerKind::Remote
            }
        )
    }

    /// Applies the canonical server route to backend-specific prover defaults.
    pub fn normalize_route(&mut self) {
        match self.route() {
            PipelineRoute {
                guest_system: GuestSystem::Sp1,
                runner: RunnerKind::Network,
            } => {
                self.sp1.prover = Sp1ProverMode::Network;
            }
            PipelineRoute {
                guest_system: GuestSystem::Sp1,
                runner: RunnerKind::Local,
            } if self.sp1.prover == Sp1ProverMode::Network => {
                self.sp1.prover = Sp1ProverMode::Local;
            }
            _ => {}
        }
    }

    /// # Errors
    ///
    /// Returns an error if the configured guest system and runner are incompatible.
    pub fn validate(&self) -> Result<()> {
        self.route().pipeline_key().map_err(anyhow::Error::msg)?;

        if matches!(
            self.route(),
            PipelineRoute {
                guest_system: GuestSystem::Risc0,
                runner: RunnerKind::Network,
            }
        ) {
            if self.boundless.rpc_url.trim().is_empty() {
                bail!("prover.boundless.rpc_url must not be empty");
            }
            if self.boundless.signer_key.trim().is_empty() {
                bail!("prover.boundless.signer_key must not be empty");
            }
        }
        self.boundless.validate()?;
        if self.risc0.execution_po2 == 0 {
            bail!("prover.risc0.execution_po2 must be greater than zero");
        }
        if matches!(
            self.route(),
            PipelineRoute {
                guest_system: GuestSystem::Sgx,
                runner: RunnerKind::Remote
            }
        ) {
            if self.remote_sgx.base_url.trim().is_empty()
                && self.remote_sgx.sgxgeth_base_url.trim().is_empty()
            {
                bail!(
                    "either prover.remote_sgx.base_url or prover.remote_sgx.sgxgeth_base_url must not be empty"
                );
            }
            if self.remote_sgx.timeout_ms == 0 {
                bail!("prover.remote_sgx.timeout_ms must be greater than zero");
            }
        }
        self.sp1.validate().map_err(anyhow::Error::msg)?;
        if matches!(self.sp1.mode, Sp1ExecutionMode::Prove) && !self.sp1.verify {
            bail!("prover.sp1.verify must be true when prover.sp1.mode=prove");
        }
        self.zk_any.validate()?;

        Ok(())
    }
}

/// Server-side request sampling configuration for `proof_type=zk_any`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ZkAnyConfig {
    pub sp1: Option<ZkAnyTargetConfig>,
    pub risc0: Option<ZkAnyTargetConfig>,
}

impl ZkAnyConfig {
    pub fn validate(&self) -> Result<()> {
        let mut total_probability = 0.0f64;

        for (proof_type, target) in self.targets() {
            if !target.probability.is_finite() {
                bail!("prover.zk_any.{proof_type}.probability must be finite");
            }
            if !(0.0..=1.0).contains(&target.probability) {
                bail!("prover.zk_any.{proof_type}.probability must be between 0 and 1");
            }
            total_probability += target.probability;
        }

        if total_probability > 1.0 {
            bail!("prover.zk_any total probability must be less than or equal to 1");
        }

        Ok(())
    }

    #[must_use]
    pub fn targets(&self) -> Vec<(&'static str, &ZkAnyTargetConfig)> {
        let mut targets = Vec::new();
        if let Some(sp1) = self.sp1.as_ref() {
            targets.push(("sp1", sp1));
        }
        if let Some(risc0) = self.risc0.as_ref() {
            targets.push(("risc0", risc0));
        }
        targets
    }

    #[must_use]
    pub fn sampling_entries(&self) -> Vec<(ProofType, ZkAnyTargetConfig)> {
        let mut entries = Vec::new();
        if let Some(sp1) = self.sp1.as_ref() {
            entries.push((ProofType::Sp1, sp1.clone()));
        }
        if let Some(risc0) = self.risc0.as_ref() {
            entries.push((ProofType::Risc0, risc0.clone()));
        }
        entries
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ZkAnyTargetConfig {
    pub probability: f64,
    pub per_day: u64,
}

impl Default for ZkAnyTargetConfig {
    fn default() -> Self {
        Self {
            probability: 0.0,
            per_day: 0,
        }
    }
}

/// Remote SGX prover configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RemoteSgxConfig {
    pub base_url: String,
    pub sgxgeth_base_url: String,
    pub timeout_ms: u64,
}

impl Default for RemoteSgxConfig {
    fn default() -> Self {
        let defaults = Gaiko2ProverConfig::default();
        Self {
            base_url: defaults.base_url,
            sgxgeth_base_url: String::new(),
            timeout_ms: if defaults.timeout_ms == 0 {
                300_000
            } else {
                defaults.timeout_ms
            },
        }
    }
}

/// RISC0 configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Risc0Config {
    pub bonsai: bool,
    pub snark: bool,
    #[serde(default)]
    pub mock: bool,
    #[serde(default = "default_risc0_execution_po2")]
    pub execution_po2: u32,
}

impl Default for Risc0Config {
    fn default() -> Self {
        Self {
            bonsai: true,
            snark: true,
            mock: false,
            execution_po2: default_risc0_execution_po2(),
        }
    }
}

const fn default_risc0_execution_po2() -> u32 {
    20
}

/// Boundless configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundlessConfig {
    #[serde(default = "default_boundless_offchain")]
    pub offchain: bool,
    pub rpc_url: String,
    pub signer_key: String,
    #[serde(default)]
    pub deployment: Option<DeploymentConfig>,
    #[serde(default)]
    pub batch_quote: QuoteSizing,
    #[serde(default)]
    pub aggregation_quote: QuoteSizing,
    pub offer_params: OfferParamsConfig,
    #[serde(default = "default_boundless_poll_interval_ms")]
    pub poll_interval_ms: u64,
    #[serde(default = "default_boundless_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_boundless_rebid_timeout_ms")]
    pub rebid_timeout_ms: u64,
    #[serde(default = "default_boundless_rebid_price_step_bps")]
    pub rebid_price_step_bps: u32,
    #[serde(default = "default_boundless_rebid_max_attempts")]
    pub rebid_max_attempts: u32,
}

impl Default for BoundlessConfig {
    fn default() -> Self {
        Self {
            offchain: raiko2_prover::boundless_config::BoundlessConfig::default().offchain,
            rpc_url: raiko2_prover::boundless_config::BoundlessConfig::default().rpc_url,
            signer_key: String::new(),
            deployment: raiko2_prover::boundless_config::BoundlessConfig::default().deployment,
            batch_quote: raiko2_prover::boundless_config::BoundlessConfig::default().batch_quote,
            aggregation_quote: raiko2_prover::boundless_config::BoundlessConfig::default()
                .aggregation_quote,
            offer_params: raiko2_prover::boundless_config::BoundlessConfig::default().offer_params,
            poll_interval_ms: default_boundless_poll_interval_ms(),
            timeout_ms: default_boundless_timeout_ms(),
            rebid_timeout_ms: default_boundless_rebid_timeout_ms(),
            rebid_price_step_bps: default_boundless_rebid_price_step_bps(),
            rebid_max_attempts: default_boundless_rebid_max_attempts(),
        }
    }
}

impl BoundlessConfig {
    /// Validate the effective Boundless config.
    pub fn validate(&self) -> Result<()> {
        self.batch_quote
            .validate("prover.boundless.batch_quote")
            .map_err(anyhow::Error::msg)?;
        self.aggregation_quote
            .validate("prover.boundless.aggregation_quote")
            .map_err(anyhow::Error::msg)?;
        if self.rebid_timeout_ms < MIN_REBID_TIMEOUT_MS {
            bail!("prover.boundless.rebid_timeout_ms must be >= {MIN_REBID_TIMEOUT_MS}");
        }
        if self.rebid_max_attempts > REBID_MAX_ATTEMPTS_LIMIT {
            bail!("prover.boundless.rebid_max_attempts must be <= {REBID_MAX_ATTEMPTS_LIMIT}");
        }
        if (1..MIN_MEANINGFUL_REBID_PRICE_STEP_BPS).contains(&self.rebid_price_step_bps) {
            bail!(
                "prover.boundless.rebid_price_step_bps = {} is below the {MIN_MEANINGFUL_REBID_PRICE_STEP_BPS} bps (1%) floor; \
                 values in 1..{MIN_MEANINGFUL_REBID_PRICE_STEP_BPS} are almost always a basis-points/multiplier confusion \
                 (e.g. `2` meaning ×2). Use 0 for an explicit flat ladder or >= {MIN_MEANINGFUL_REBID_PRICE_STEP_BPS} to escalate.",
                self.rebid_price_step_bps
            );
        }
        validate_offer_spec(&self.offer_params.batch)
            .map_err(anyhow::Error::msg)
            .context("prover.boundless.offer_params.batch")?;
        validate_offer_spec(&self.offer_params.aggregation)
            .map_err(anyhow::Error::msg)
            .context("prover.boundless.offer_params.aggregation")?;
        Ok(())
    }

    /// Merge a pair-specific Boundless override into the global default config.
    pub fn apply_pair_override(&self, pair: &BoundlessPairConfig) -> Result<Self> {
        let mut merged = self.clone();
        if let Some(batch_quote) = pair.batch_quote.clone() {
            merged.batch_quote = batch_quote;
        }
        if let Some(aggregation_quote) = pair.aggregation_quote.clone() {
            merged.aggregation_quote = aggregation_quote;
        }
        if let Some(poll_interval_ms) = pair.poll_interval_ms {
            merged.poll_interval_ms = poll_interval_ms;
        }
        if let Some(timeout_ms) = pair.timeout_ms {
            merged.timeout_ms = timeout_ms;
        }
        if let Some(rebid_timeout_ms) = pair.rebid_timeout_ms {
            merged.rebid_timeout_ms = rebid_timeout_ms;
        }
        if let Some(rebid_price_step_bps) = pair.rebid_price_step_bps {
            merged.rebid_price_step_bps = rebid_price_step_bps;
        }
        if let Some(rebid_max_attempts) = pair.rebid_max_attempts {
            merged.rebid_max_attempts = rebid_max_attempts;
        }
        if let Some(batch) = &pair.offer_params.batch {
            merged.offer_params.batch = batch.clone();
        }
        if let Some(aggregation) = &pair.offer_params.aggregation {
            merged.offer_params.aggregation = aggregation.clone();
        }
        merged.validate()?;
        Ok(merged)
    }
}

const fn default_boundless_offchain() -> bool {
    false
}

const fn default_boundless_poll_interval_ms() -> u64 {
    10_000
}

const fn default_boundless_timeout_ms() -> u64 {
    3_600_000
}

const fn default_boundless_rebid_timeout_ms() -> u64 {
    DEFAULT_REBID_TIMEOUT_MS
}

const fn default_boundless_rebid_price_step_bps() -> u32 {
    DEFAULT_REBID_PRICE_STEP_BPS
}

const fn default_boundless_rebid_max_attempts() -> u32 {
    DEFAULT_REBID_MAX_ATTEMPTS
}

#[cfg(test)]
mod tests {
    use super::{
        MIN_MEANINGFUL_REBID_PRICE_STEP_BPS, ProverConfig, REBID_MAX_ATTEMPTS_LIMIT,
        Sp1ExecutionMode, ZkAnyConfig, ZkAnyTargetConfig,
    };

    #[test]
    fn zk_any_config_rejects_probability_above_one() {
        let config = ZkAnyConfig {
            sp1: Some(ZkAnyTargetConfig {
                probability: 1.1,
                per_day: 0,
            }),
            risc0: None,
        };

        assert!(
            config
                .validate()
                .expect_err("invalid probability should fail")
                .to_string()
                .contains("prover.zk_any.sp1.probability")
        );
    }

    #[test]
    fn zk_any_config_rejects_total_probability_above_one() {
        let config = ZkAnyConfig {
            sp1: Some(ZkAnyTargetConfig {
                probability: 0.6,
                per_day: 0,
            }),
            risc0: Some(ZkAnyTargetConfig {
                probability: 0.5,
                per_day: 0,
            }),
        };

        assert!(
            config
                .validate()
                .expect_err("invalid probability sum should fail")
                .to_string()
                .contains("total probability")
        );
    }

    #[test]
    fn prover_config_accepts_valid_zk_any_policy() {
        let config = ProverConfig {
            zk_any: ZkAnyConfig {
                sp1: Some(ZkAnyTargetConfig {
                    probability: 0.3,
                    per_day: 100,
                }),
                risc0: Some(ZkAnyTargetConfig {
                    probability: 0.4,
                    per_day: 0,
                }),
            },
            ..Default::default()
        };

        config.validate().expect("valid zk_any policy");
    }

    #[test]
    fn prover_config_rejects_zero_aggregation_quote() {
        let mut config = ProverConfig::default();
        config.boundless.aggregation_quote =
            raiko2_prover::boundless_config::QuoteSizing::Fixed { mcycles: 0 };

        assert!(
            config
                .validate()
                .expect_err("zero aggregation quote should fail")
                .to_string()
                .contains("prover.boundless.aggregation_quote")
        );
    }

    #[test]
    fn prover_config_accepts_zero_boundless_rebid_price_step_bps() {
        let mut config = ProverConfig::default();
        config.boundless.rebid_price_step_bps = 0;

        // 0 bps is a valid flat (no-escalation) ladder.
        config
            .validate()
            .expect("zero rebid price step bps should be accepted");
    }

    #[test]
    fn prover_config_rejects_sub_floor_boundless_rebid_price_step_bps() {
        // A value in 1..100 bps is almost always a bps/multiplier confusion (e.g. `2` meaning ×2).
        let mut config = ProverConfig::default();
        config.boundless.rebid_price_step_bps = 2;

        assert!(
            config
                .validate()
                .expect_err("sub-floor rebid price step bps should fail")
                .to_string()
                .contains("rebid_price_step_bps")
        );
    }

    #[test]
    fn prover_config_accepts_floor_boundless_rebid_price_step_bps() {
        let mut config = ProverConfig::default();
        config.boundless.rebid_price_step_bps = MIN_MEANINGFUL_REBID_PRICE_STEP_BPS;

        config
            .validate()
            .expect("floor rebid price step bps should be accepted");
    }

    #[test]
    fn prover_config_rejects_excessive_boundless_rebid_max_attempts() {
        let mut config = ProverConfig::default();
        config.boundless.rebid_max_attempts = REBID_MAX_ATTEMPTS_LIMIT + 1;

        assert!(
            config
                .validate()
                .expect_err("excessive rebid attempts should fail")
                .to_string()
                .contains("rebid_max_attempts")
        );
    }

    #[test]
    fn prover_config_rejects_zero_boundless_rebid_timeout() {
        let mut config = ProverConfig::default();
        config.boundless.rebid_timeout_ms = 0;

        assert!(
            config
                .validate()
                .expect_err("zero rebid timeout should fail")
                .to_string()
                .contains("prover.boundless.rebid_timeout_ms")
        );
    }

    #[test]
    fn prover_config_rejects_subsecond_boundless_rebid_timeout() {
        let mut config = ProverConfig::default();
        config.boundless.rebid_timeout_ms = 999;

        assert!(
            config
                .validate()
                .expect_err("subsecond rebid timeout should fail")
                .to_string()
                .contains("prover.boundless.rebid_timeout_ms")
        );
    }

    #[test]
    fn prover_config_rejects_sp1_prove_without_verification() {
        let mut config = ProverConfig::default();
        config.sp1.mode = Sp1ExecutionMode::Prove;
        config.sp1.verify = false;

        assert!(
            config
                .validate()
                .expect_err("invalid sp1 production posture should fail")
                .to_string()
                .contains("prover.sp1.verify must be true")
        );
    }
}
