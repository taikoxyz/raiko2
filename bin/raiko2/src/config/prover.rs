use anyhow::{Context, Result, bail};
use raiko2_pipeline::RunnerKind;
use raiko2_primitives::ProofType;
use raiko2_prover::{
    boundless_config::{
        DEFAULT_REBID_MAX_ATTEMPTS, DEFAULT_REBID_PRICE_STEP_BPS, DEFAULT_REBID_TIMEOUT_MS,
        DeploymentConfig, MIN_MEANINGFUL_REBID_PRICE_STEP_BPS, MIN_REBID_TIMEOUT_MS,
        OfferParamsConfig, QuoteSizing, REBID_MAX_ATTEMPTS_LIMIT, validate_offer_spec,
    },
    gaiko2::Gaiko2Config as Gaiko2ProverConfig,
    sp1_config::{
        ExecutionMode as Sp1ExecutionMode, ProverMode as Sp1ProverMode, Sp1Config, Sp1ConfigError,
    },
};
use serde::{Deserialize, Serialize};

use super::BoundlessPairConfig;

/// Prover configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ProverConfig {
    pub routes: ProverRoutesConfig,
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
    /// # Errors
    ///
    /// Returns an error if a configured route or prover setting is invalid.
    pub fn validate(&self) -> Result<()> {
        self.routes.validate()?;

        match self.routes.runner(ProofType::Risc0) {
            Some(RunnerKind::Network) => {
                if self.boundless.rpc_url.trim().is_empty() {
                    bail!("prover.boundless.rpc_url must not be empty");
                }
                if self.boundless.signer_key.trim().is_empty() {
                    bail!("prover.boundless.signer_key must not be empty");
                }
                self.boundless.validate()?;
                self.validate_risc0()?;
            }
            Some(RunnerKind::Local) => self.validate_risc0()?,
            None => {}
            Some(RunnerKind::Remote) => unreachable!("routes.validate rejects risc0/remote"),
        }

        if let Some(runner) = self.routes.runner(ProofType::Sp1) {
            match runner {
                RunnerKind::Network if self.sp1.prover != Sp1ProverMode::Network => {
                    bail!("prover.routes.sp1=network requires prover.sp1.prover=network");
                }
                RunnerKind::Local if self.sp1.prover == Sp1ProverMode::Network => {
                    bail!("prover.routes.sp1=local requires prover.sp1.prover=local or mock");
                }
                RunnerKind::Local | RunnerKind::Network => {}
                RunnerKind::Remote => unreachable!("routes.validate rejects sp1/remote"),
            }
            validate_sp1_config(&self.sp1)?;
            if matches!(self.sp1.mode, Sp1ExecutionMode::Prove) && !self.sp1.verify {
                bail!("prover.sp1.verify must be true when prover.sp1.mode=prove");
            }
        }

        let sgx_enabled = self.routes.is_enabled(ProofType::Sgx);
        let sgxgeth_enabled = self.routes.is_enabled(ProofType::SgxGeth);
        if sgx_enabled && self.remote_sgx.base_url.trim().is_empty() {
            bail!("prover.remote_sgx.base_url must not be empty when prover.routes.sgx=remote");
        }
        if sgxgeth_enabled && self.remote_sgx.sgxgeth_base_url.trim().is_empty() {
            bail!(
                "prover.remote_sgx.sgxgeth_base_url must not be empty when prover.routes.sgxgeth=remote"
            );
        }
        if sgx_enabled || sgxgeth_enabled {
            if self.remote_sgx.timeout_ms == 0 {
                bail!("prover.remote_sgx.timeout_ms must be greater than zero");
            }
        }

        self.zk_any.validate()?;
        if self.zk_any.sp1.is_some() && !self.routes.is_enabled(ProofType::Sp1) {
            bail!("prover.zk_any.sp1 requires enabled prover.routes.sp1");
        }
        if self.zk_any.risc0.is_some() && !self.routes.is_enabled(ProofType::Risc0) {
            bail!("prover.zk_any.risc0 requires enabled prover.routes.risc0");
        }

        Ok(())
    }

    fn validate_risc0(&self) -> Result<()> {
        if self.risc0.execution_po2 == 0 {
            bail!("prover.risc0.execution_po2 must be greater than zero");
        }
        Ok(())
    }
}

fn validate_sp1_config(config: &Sp1Config) -> Result<()> {
    config.validate().map_err(|error| match error {
        Sp1ConfigError::RpcUrlInvalid(_) => anyhow::anyhow!("prover.sp1.rpc_url is invalid"),
        Sp1ConfigError::RemoteVerifyRpcUrlInvalid(_) => {
            anyhow::anyhow!("prover.sp1.remote_verify.rpc_url is invalid")
        }
        error => anyhow::Error::msg(error),
    })
}

/// Explicit runner selection for each concrete proof type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProverRoutesConfig {
    pub risc0: Option<RunnerKind>,
    pub sp1: Option<RunnerKind>,
    pub native: Option<RunnerKind>,
    pub sgx: Option<RunnerKind>,
    pub sgxgeth: Option<RunnerKind>,
}

impl Default for ProverRoutesConfig {
    fn default() -> Self {
        Self {
            risc0: Some(RunnerKind::Local),
            sp1: None,
            native: None,
            sgx: None,
            sgxgeth: None,
        }
    }
}

impl ProverRoutesConfig {
    const STABLE_ORDER: [ProofType; 5] = [
        ProofType::Risc0,
        ProofType::Sp1,
        ProofType::Native,
        ProofType::Sgx,
        ProofType::SgxGeth,
    ];

    const fn empty() -> Self {
        Self {
            risc0: None,
            sp1: None,
            native: None,
            sgx: None,
            sgxgeth: None,
        }
    }

    /// Returns the configured runner for a concrete proof type.
    #[must_use]
    pub const fn runner(&self, proof_type: ProofType) -> Option<RunnerKind> {
        match proof_type {
            ProofType::Risc0 => self.risc0,
            ProofType::Sp1 => self.sp1,
            ProofType::Native => self.native,
            ProofType::Sgx => self.sgx,
            ProofType::SgxGeth => self.sgxgeth,
        }
    }

    /// Returns whether a concrete proof type has an enabled route.
    #[must_use]
    pub const fn is_enabled(&self, proof_type: ProofType) -> bool {
        self.runner(proof_type).is_some()
    }

    /// Iterates enabled routes in stable proof-type order.
    pub fn iter(&self) -> impl Iterator<Item = (ProofType, RunnerKind)> + '_ {
        Self::STABLE_ORDER
            .into_iter()
            .filter_map(|proof_type| self.runner(proof_type).map(|runner| (proof_type, runner)))
    }

    /// # Errors
    ///
    /// Returns an error if no route is enabled or a runner is unsupported for its proof type.
    pub fn validate(&self) -> Result<()> {
        let mut routes = self.iter().peekable();
        if routes.peek().is_none() {
            bail!("at least one prover route must be enabled");
        }
        for (proof_type, runner) in routes {
            validate_route(proof_type, runner).map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }

    fn insert(
        &mut self,
        proof_type: ProofType,
        runner: RunnerKind,
    ) -> std::result::Result<(), String> {
        let entry = match proof_type {
            ProofType::Risc0 => &mut self.risc0,
            ProofType::Sp1 => &mut self.sp1,
            ProofType::Native => &mut self.native,
            ProofType::Sgx => &mut self.sgx,
            ProofType::SgxGeth => &mut self.sgxgeth,
        };
        if entry.is_some() {
            return Err(format!("duplicate prover route for {proof_type}"));
        }
        *entry = Some(runner);
        Ok(())
    }
}

impl std::str::FromStr for ProverRoutesConfig {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        if value.trim().is_empty() {
            return Err("prover routes override must not be empty".to_string());
        }

        let mut routes = Self::empty();
        for raw_route in value.split(',') {
            let route = raw_route.trim();
            let (proof_type, runner) = route.split_once('/').ok_or_else(|| {
                format!("invalid prover route '{route}', expected <proof_type>/<runner>")
            })?;
            routes.insert(proof_type.trim().parse()?, runner.trim().parse()?)?;
        }
        routes.validate().map_err(|error| error.to_string())?;
        Ok(routes)
    }
}

fn validate_route(proof_type: ProofType, runner: RunnerKind) -> std::result::Result<(), String> {
    if matches!(
        (proof_type, runner),
        (
            ProofType::Risc0 | ProofType::Sp1,
            RunnerKind::Local | RunnerKind::Network
        ) | (ProofType::Native, RunnerKind::Local)
            | (ProofType::Sgx | ProofType::SgxGeth, RunnerKind::Remote)
    ) {
        Ok(())
    } else {
        Err(format!("unsupported prover route: {proof_type}/{runner}"))
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
        MIN_MEANINGFUL_REBID_PRICE_STEP_BPS, ProverConfig, ProverRoutesConfig,
        REBID_MAX_ATTEMPTS_LIMIT, Sp1ExecutionMode, ZkAnyConfig, ZkAnyTargetConfig,
    };
    use raiko2_pipeline::RunnerKind;
    use raiko2_primitives::ProofType;
    use raiko2_prover::sp1_config::ProverMode as Sp1ProverMode;

    fn routes(value: &str) -> ProverRoutesConfig {
        value.parse().expect("valid prover routes")
    }

    fn boundless_network_config() -> ProverConfig {
        let mut config = ProverConfig {
            routes: routes("risc0/network"),
            ..Default::default()
        };
        config.boundless.signer_key = "configured-by-secret-store".to_string();
        config
    }

    #[test]
    fn prover_routes_parse_explicit_table_and_iterate_in_stable_order() {
        let config: ProverConfig = toml::from_str(
            r#"
[routes]
sgxgeth = "remote"
native = "local"
sp1 = "network"
risc0 = "local"
sgx = "remote"
"#,
        )
        .expect("explicit routes should parse");

        assert_eq!(
            config.routes.iter().collect::<Vec<_>>(),
            vec![
                (ProofType::Risc0, RunnerKind::Local),
                (ProofType::Sp1, RunnerKind::Network),
                (ProofType::Native, RunnerKind::Local),
                (ProofType::Sgx, RunnerKind::Remote),
                (ProofType::SgxGeth, RunnerKind::Remote),
            ]
        );
        assert_eq!(
            config.routes.runner(ProofType::Sp1),
            Some(RunnerKind::Network)
        );
        assert!(config.routes.is_enabled(ProofType::SgxGeth));
    }

    #[test]
    fn prover_routes_default_is_programmatic_only() {
        // Rust defaults keep existing fixture construction ergonomic. Because `routes` has no
        // serde default on `ProverConfig`, deserialization still requires an explicit table.
        let default = ProverConfig::default();
        assert_eq!(
            default.routes.iter().collect::<Vec<_>>(),
            vec![(ProofType::Risc0, RunnerKind::Local)]
        );

        let err = toml::from_str::<ProverConfig>("")
            .expect_err("deserialization must require an explicit routes table");
        assert!(err.to_string().contains("missing field `routes`"));
    }

    #[test]
    fn prover_routes_reject_empty_table() {
        let config: ProverConfig =
            toml::from_str("[routes]").expect("an empty table should deserialize");

        let err = config
            .validate()
            .expect_err("at least one route must be enabled");
        assert!(err.to_string().contains("at least one prover route"));
    }

    #[test]
    fn prover_routes_accept_only_supported_runner_combinations() {
        for routes in [
            "risc0/local",
            "risc0/network",
            "sp1/local",
            "sp1/network",
            "native/local",
            "sgx/remote",
            "sgxgeth/remote",
        ] {
            routes
                .parse::<ProverRoutesConfig>()
                .unwrap_or_else(|err| panic!("{routes} should parse: {err}"))
                .validate()
                .unwrap_or_else(|err| panic!("{routes} should validate: {err}"));
        }

        for routes in [
            "risc0/remote",
            "sp1/remote",
            "native/network",
            "native/remote",
            "sgx/local",
            "sgx/network",
            "sgxgeth/local",
            "sgxgeth/network",
        ] {
            let err = routes
                .parse::<ProverRoutesConfig>()
                .expect_err("unsupported runner combination must fail");
            assert!(err.contains("unsupported prover route"), "{routes}: {err}");
        }
    }

    #[test]
    fn prover_routes_override_parser_rejects_invalid_input() {
        let duplicate = "risc0/local,risc0/network"
            .parse::<ProverRoutesConfig>()
            .expect_err("duplicate proof type must fail");
        assert!(duplicate.contains("duplicate prover route"));

        let unknown = "unknown/local"
            .parse::<ProverRoutesConfig>()
            .expect_err("unknown proof type must fail");
        assert!(unknown.contains("Unknown proof type"));

        let invalid_runner = "risc0/invalid"
            .parse::<ProverRoutesConfig>()
            .expect_err("unknown runner must fail");
        assert!(invalid_runner.contains("Unknown runner"));

        let empty = ""
            .parse::<ProverRoutesConfig>()
            .expect_err("empty override must fail");
        assert!(empty.contains("must not be empty"));
    }

    #[test]
    fn prover_config_rejects_removed_global_route_fields() {
        for legacy_field in ["guest_system = \"risc0\"", "runner = \"local\""] {
            let input = format!("{legacy_field}\n[routes]\nrisc0 = \"local\"\n");
            let err = toml::from_str::<ProverConfig>(&input)
                .expect_err("legacy global route field must fail");
            assert!(
                err.to_string().contains("unknown field"),
                "unexpected error for {legacy_field}: {err}"
            );
        }
    }

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
        let mut config = ProverConfig {
            routes: routes("risc0/local,sp1/local"),
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
        config.sp1.prover = Sp1ProverMode::Local;

        config.validate().expect("valid zk_any policy");
    }

    #[test]
    fn zk_any_rejects_disabled_sp1_target() {
        let config = ProverConfig {
            routes: routes("risc0/local"),
            zk_any: ZkAnyConfig {
                sp1: Some(ZkAnyTargetConfig {
                    probability: 0.5,
                    per_day: 0,
                }),
                risc0: None,
            },
            ..Default::default()
        };

        let err = config
            .validate()
            .expect_err("zk_any must not target a disabled SP1 route");
        assert!(
            err.to_string()
                .contains("prover.zk_any.sp1 requires enabled prover.routes.sp1")
        );
    }

    #[test]
    fn zk_any_rejects_disabled_risc0_target() {
        let mut config = ProverConfig {
            routes: routes("sp1/local"),
            zk_any: ZkAnyConfig {
                sp1: None,
                risc0: Some(ZkAnyTargetConfig {
                    probability: 0.5,
                    per_day: 0,
                }),
            },
            ..Default::default()
        };
        config.sp1.prover = Sp1ProverMode::Local;

        let err = config
            .validate()
            .expect_err("zk_any must not target a disabled RISC0 route");
        assert!(
            err.to_string()
                .contains("prover.zk_any.risc0 requires enabled prover.routes.risc0")
        );
    }

    #[test]
    fn risc0_network_requires_boundless_network_credentials() {
        let mut config = boundless_network_config();
        config.boundless.rpc_url.clear();

        let err = config
            .validate()
            .expect_err("risc0/network must require a Boundless RPC URL");
        assert!(err.to_string().contains("prover.boundless.rpc_url"));

        config.boundless.rpc_url = "https://boundless.example.com".to_string();
        config.boundless.signer_key.clear();
        let err = config
            .validate()
            .expect_err("risc0/network must require a Boundless signer");
        assert!(err.to_string().contains("prover.boundless.signer_key"));
    }

    #[test]
    fn risc0_local_skips_boundless_network_credentials() {
        let mut config = ProverConfig::default();
        config.boundless.rpc_url.clear();
        config.boundless.signer_key.clear();

        config
            .validate()
            .expect("risc0/local must not require Boundless credentials");
    }

    #[test]
    fn disabled_risc0_skips_boundless_backend_validation() {
        let mut config = ProverConfig {
            routes: routes("native/local"),
            ..Default::default()
        };
        config.risc0.execution_po2 = 0;
        config.boundless.rebid_timeout_ms = 0;

        config
            .validate()
            .expect("disabled RISC0 must not validate Boundless or RISC0 settings");
    }

    #[test]
    fn sgx_route_requires_lane_specific_base_url() {
        let mut config = ProverConfig {
            routes: routes("sgx/remote"),
            ..Default::default()
        };
        config.remote_sgx.base_url.clear();
        config.remote_sgx.sgxgeth_base_url = "https://sgxgeth.example.com".to_string();

        let err = config
            .validate()
            .expect_err("sgx/remote must require its own base URL");
        assert!(err.to_string().contains("prover.remote_sgx.base_url"));
    }

    #[test]
    fn sgxgeth_route_requires_lane_specific_base_url() {
        let mut config = ProverConfig {
            routes: routes("sgxgeth/remote"),
            ..Default::default()
        };
        config.remote_sgx.base_url = "https://sgx.example.com".to_string();
        config.remote_sgx.sgxgeth_base_url.clear();

        let err = config
            .validate()
            .expect_err("sgxgeth/remote must require its own base URL");
        assert!(
            err.to_string()
                .contains("prover.remote_sgx.sgxgeth_base_url")
        );
    }

    #[test]
    fn configured_sgx_urls_without_routes_do_not_enable_lanes() {
        let mut config = ProverConfig {
            routes: routes("native/local"),
            ..Default::default()
        };
        config.remote_sgx.base_url = "https://sgx.example.com".to_string();
        config.remote_sgx.sgxgeth_base_url = "https://sgxgeth.example.com".to_string();
        config.remote_sgx.timeout_ms = 0;

        config
            .validate()
            .expect("configured SGX URLs must not enable absent routes");
    }

    #[test]
    fn each_enabled_sgx_lane_requires_positive_timeout() {
        for route in ["sgx/remote", "sgxgeth/remote"] {
            let mut config = ProverConfig {
                routes: routes(route),
                ..Default::default()
            };
            config.remote_sgx.base_url = "https://sgx.example.com".to_string();
            config.remote_sgx.sgxgeth_base_url = "https://sgxgeth.example.com".to_string();
            config.remote_sgx.timeout_ms = 0;

            let err = config
                .validate()
                .expect_err("enabled SGX lanes must require a positive timeout");
            assert!(
                err.to_string()
                    .contains("prover.remote_sgx.timeout_ms must be greater than zero"),
                "unexpected error for {route}: {err}"
            );
        }
    }

    #[test]
    fn sp1_local_allows_local_or_mock_prover() {
        for prover in [Sp1ProverMode::Local, Sp1ProverMode::Mock] {
            let mut config = ProverConfig {
                routes: routes("sp1/local"),
                ..Default::default()
            };
            config.sp1.prover = prover;

            config
                .validate()
                .unwrap_or_else(|err| panic!("sp1/local should allow {prover:?}: {err}"));
        }
    }

    #[test]
    fn sp1_local_rejects_network_prover() {
        let config = ProverConfig {
            routes: routes("sp1/local"),
            ..Default::default()
        };

        let err = config
            .validate()
            .expect_err("sp1/local must reject the network prover");
        assert!(
            err.to_string()
                .contains("prover.routes.sp1=local requires prover.sp1.prover=local or mock")
        );
    }

    #[test]
    fn sp1_network_requires_network_prover() {
        let mut config = ProverConfig {
            routes: routes("sp1/network"),
            ..Default::default()
        };
        config.sp1.prover = Sp1ProverMode::Local;

        let err = config
            .validate()
            .expect_err("sp1/network must require the network prover");
        assert!(
            err.to_string()
                .contains("prover.routes.sp1=network requires prover.sp1.prover=network")
        );
    }

    #[test]
    fn disabled_sp1_skips_sp1_backend_validation() {
        let mut config = ProverConfig {
            routes: routes("native/local"),
            ..Default::default()
        };
        config.sp1.cycle_limit = 0;

        config
            .validate()
            .expect("disabled SP1 must not validate SP1 settings");
    }

    #[test]
    fn sp1_validation_error_omits_credential_bearing_url() {
        let mut config = ProverConfig {
            routes: routes("sp1/local"),
            ..Default::default()
        };
        config.sp1.prover = Sp1ProverMode::Local;
        let sensitive_url = "https://sample_user:sample_secret@[invalid";
        config.sp1.rpc_url = Some(sensitive_url.to_string());

        let err = config
            .validate()
            .expect_err("invalid SP1 URL must fail validation");
        let message = err.to_string();

        assert!(!message.contains(sensitive_url));
        assert!(!message.contains("sample_secret"));
    }

    #[test]
    fn prover_config_rejects_zero_aggregation_quote() {
        let mut config = boundless_network_config();
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
        let mut config = boundless_network_config();
        config.boundless.rebid_price_step_bps = 0;

        // 0 bps is a valid flat (no-escalation) ladder.
        config
            .validate()
            .expect("zero rebid price step bps should be accepted");
    }

    #[test]
    fn prover_config_rejects_sub_floor_boundless_rebid_price_step_bps() {
        // A value in 1..100 bps is almost always a bps/multiplier confusion (e.g. `2` meaning ×2).
        let mut config = boundless_network_config();
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
        let mut config = boundless_network_config();
        config.boundless.rebid_price_step_bps = MIN_MEANINGFUL_REBID_PRICE_STEP_BPS;

        config
            .validate()
            .expect("floor rebid price step bps should be accepted");
    }

    #[test]
    fn prover_config_rejects_excessive_boundless_rebid_max_attempts() {
        let mut config = boundless_network_config();
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
        let mut config = boundless_network_config();
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
        let mut config = boundless_network_config();
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
        let mut config = ProverConfig {
            routes: routes("sp1/local"),
            ..Default::default()
        };
        config.sp1.prover = Sp1ProverMode::Local;
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
