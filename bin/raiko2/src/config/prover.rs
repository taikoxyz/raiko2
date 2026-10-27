use anyhow::{Result, bail};
use raiko2_pipeline::RunnerKind;
use raiko2_primitives::ProofType;
use raiko2_prover::sp1_config::{
    ExecutionMode as Sp1ExecutionMode, ProverMode as Sp1ProverMode, Sp1Config, Sp1ConfigError,
};
use serde::{Deserialize, Serialize};
use std::ops::{Deref, DerefMut};

/// Prover configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProverConfig {
    /// RISC0 specific configuration.
    pub risc0: Risc0Config,
    /// SP1 specific configuration.
    pub sp1: Sp1ProverConfig,
    /// Native execution configuration.
    pub native: NativeProverConfig,
    /// Remote SGX prover configuration.
    pub sgx: RemoteProverConfig,
    /// Remote SGXGETH prover configuration.
    pub sgxgeth: RemoteProverConfig,
    /// Request sampling policy for `proof_type=zk_any`.
    pub zk_any: ZkAnyConfig,
}

impl ProverConfig {
    const STABLE_ORDER: [ProofType; 5] = [
        ProofType::Risc0,
        ProofType::Sp1,
        ProofType::Native,
        ProofType::Sgx,
        ProofType::SgxGeth,
    ];

    /// Returns the configured runner when the proof type is enabled.
    #[must_use]
    pub const fn runner(&self, proof_type: ProofType) -> Option<RunnerKind> {
        match proof_type {
            ProofType::Risc0 if self.risc0.enabled => Some(self.risc0.runner.runner_kind()),
            ProofType::Sp1 if self.sp1.enabled => match self.sp1.config.prover {
                Sp1ProverMode::Network => Some(RunnerKind::Network),
                Sp1ProverMode::Mock | Sp1ProverMode::Local => Some(RunnerKind::Local),
            },
            ProofType::Native if self.native.enabled => Some(RunnerKind::Local),
            ProofType::Sgx if self.sgx.enabled => Some(RunnerKind::Remote),
            ProofType::SgxGeth if self.sgxgeth.enabled => Some(RunnerKind::Remote),
            _ => None,
        }
    }

    /// Returns whether a concrete proof type is enabled.
    #[must_use]
    pub const fn is_enabled(&self, proof_type: ProofType) -> bool {
        self.runner(proof_type).is_some()
    }

    /// Iterates enabled routes in stable proof-type order.
    pub fn iter_routes(&self) -> impl Iterator<Item = (ProofType, RunnerKind)> + '_ {
        Self::STABLE_ORDER
            .into_iter()
            .filter_map(|proof_type| self.runner(proof_type).map(|runner| (proof_type, runner)))
    }

    /// Atomically replace enabled proof types using the operational route override.
    pub(crate) fn apply_routes_override(&mut self, routes: &ProverRoutesOverride) {
        self.risc0.enabled = false;
        self.sp1.enabled = false;
        self.native.enabled = false;
        self.sgx.enabled = false;
        self.sgxgeth.enabled = false;

        for (proof_type, runner) in routes.iter() {
            match proof_type {
                ProofType::Risc0 => {
                    self.risc0.enabled = true;
                    self.risc0.runner = match runner {
                        RunnerKind::Local => Risc0Runner::Local,
                        RunnerKind::Network | RunnerKind::Remote => {
                            unreachable!("route override is validated")
                        }
                    };
                }
                ProofType::Sp1 => {
                    self.sp1.enabled = true;
                    self.sp1.config.prover = match runner {
                        RunnerKind::Network => Sp1ProverMode::Network,
                        RunnerKind::Local if self.sp1.config.prover == Sp1ProverMode::Mock => {
                            Sp1ProverMode::Mock
                        }
                        RunnerKind::Local => Sp1ProverMode::Local,
                        RunnerKind::Remote => unreachable!("route override is validated"),
                    };
                }
                ProofType::Native => self.native.enabled = true,
                ProofType::Sgx => self.sgx.enabled = true,
                ProofType::SgxGeth => self.sgxgeth.enabled = true,
            }
        }
    }

    /// # Errors
    ///
    /// Returns an error if a configured route or prover setting is invalid.
    pub fn validate(&self) -> Result<()> {
        if self.iter_routes().next().is_none() {
            bail!("at least one prover must be enabled");
        }

        match self.runner(ProofType::Risc0) {
            Some(RunnerKind::Local) => self.validate_risc0()?,
            None => {}
            Some(RunnerKind::Network | RunnerKind::Remote) => {
                unreachable!("RISC0 runner is restricted by its type")
            }
        }

        if self.sp1.enabled {
            validate_sp1_config(&self.sp1)?;
            if matches!(self.sp1.mode, Sp1ExecutionMode::Prove) && !self.sp1.verify {
                bail!("prover.sp1.verify must be true when prover.sp1.mode=prove");
            }
        }

        self.sgx.validate("prover.sgx")?;
        self.sgxgeth.validate("prover.sgxgeth")?;

        self.zk_any.validate()?;
        if self.zk_any.sp1.is_some() && !self.is_enabled(ProofType::Sp1) {
            bail!("prover.zk_any.sp1 requires prover.sp1.enabled=true");
        }
        if self.zk_any.risc0.is_some() && !self.is_enabled(ProofType::Risc0) {
            bail!("prover.zk_any.risc0 requires prover.risc0.enabled=true");
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

/// Atomic operational override for proof-type enablement and runner selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProverRoutesOverride {
    risc0: Option<RunnerKind>,
    sp1: Option<RunnerKind>,
    native: Option<RunnerKind>,
    sgx: Option<RunnerKind>,
    sgxgeth: Option<RunnerKind>,
}

impl ProverRoutesOverride {
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

impl std::str::FromStr for ProverRoutesOverride {
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
        (ProofType::Sp1, RunnerKind::Local | RunnerKind::Network)
            | (ProofType::Risc0 | ProofType::Native, RunnerKind::Local)
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

/// SP1 configuration plus host-level enablement.
#[derive(Debug, Clone, Default)]
pub struct Sp1ProverConfig {
    pub enabled: bool,
    pub config: Sp1Config,
}

impl Serialize for Sp1ProverConfig {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut value = serde_json::to_value(&self.config).map_err(serde::ser::Error::custom)?;
        let fields = value
            .as_object_mut()
            .ok_or_else(|| serde::ser::Error::custom("SP1 config must serialize as an object"))?;
        fields.insert("enabled".to_string(), serde_json::Value::Bool(self.enabled));
        value.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Sp1ProverConfig {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut fields = serde_json::Map::<String, serde_json::Value>::deserialize(deserializer)?;
        let enabled = fields
            .remove("enabled")
            .map(serde_json::from_value)
            .transpose()
            .map_err(serde::de::Error::custom)?
            .unwrap_or(false);
        let config = serde_json::from_value(serde_json::Value::Object(fields))
            .map_err(serde::de::Error::custom)?;
        Ok(Self { enabled, config })
    }
}

impl Deref for Sp1ProverConfig {
    type Target = Sp1Config;

    fn deref(&self) -> &Self::Target {
        &self.config
    }
}

impl DerefMut for Sp1ProverConfig {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.config
    }
}

/// Native execution configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NativeProverConfig {
    pub enabled: bool,
}

impl Default for NativeProverConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// One remote prover lane.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RemoteProverConfig {
    pub enabled: bool,
    pub base_url: String,
    pub timeout_ms: u64,
}

impl Default for RemoteProverConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: String::new(),
            timeout_ms: 300_000,
        }
    }
}

impl RemoteProverConfig {
    fn validate(&self, field: &str) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        if self.base_url.trim().is_empty() {
            bail!("{field}.base_url must not be empty when enabled");
        }
        if self.timeout_ms == 0 {
            bail!("{field}.timeout_ms must be greater than zero");
        }
        Ok(())
    }
}

/// RISC0 configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct Risc0Config {
    pub enabled: bool,
    pub runner: Risc0Runner,
    pub snark: bool,
    #[serde(default)]
    pub mock: bool,
    #[serde(default = "default_risc0_execution_po2")]
    pub execution_po2: u32,
}

impl Default for Risc0Config {
    fn default() -> Self {
        Self {
            enabled: false,
            runner: Risc0Runner::Local,
            snark: true,
            mock: false,
            execution_po2: default_risc0_execution_po2(),
        }
    }
}

/// Supported RISC0 execution backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Risc0Runner {
    #[default]
    Local,
}

impl Risc0Runner {
    const fn runner_kind(self) -> RunnerKind {
        match self {
            Self::Local => RunnerKind::Local,
        }
    }
}

const fn default_risc0_execution_po2() -> u32 {
    20
}

#[cfg(test)]
mod tests {
    use super::{
        ProverConfig, ProverRoutesOverride, Sp1ExecutionMode, ZkAnyConfig, ZkAnyTargetConfig,
    };
    use raiko2_pipeline::RunnerKind;
    use raiko2_primitives::ProofType;
    use raiko2_prover::sp1_config::ProverMode as Sp1ProverMode;

    #[test]
    fn removed_market_route_and_settings_are_rejected() {
        assert!("risc0/network".parse::<ProverRoutesOverride>().is_err());
        assert!(toml::from_str::<ProverConfig>("[risc0]\nrunner = \"network\"\n").is_err());
        assert!(toml::from_str::<ProverConfig>("[risc0.boundless]\noffchain = true\n").is_err());
    }

    #[test]
    fn removed_market_pair_override_is_rejected() {
        let error = toml::from_str::<crate::config::NetworkPairConfig>(
            "network = \"taiko_hoodi\"\nl1_network = \"hoodi\"\n[boundless]\noffchain = true\n",
        )
        .expect_err("removed pair override must fail ordinary schema validation");
        assert!(error.to_string().contains("unknown field"));
    }

    fn routes(value: &str) -> ProverRoutesOverride {
        value.parse().expect("valid prover routes")
    }

    fn config_with_routes(value: &str) -> ProverConfig {
        let mut config = ProverConfig::default();
        config.apply_routes_override(&routes(value));
        config
    }

    #[test]
    fn self_contained_prover_tables_derive_routes() {
        let config: ProverConfig = toml::from_str(
            r#"
[risc0]
enabled = true
runner = "local"
snark = true
mock = false
execution_po2 = 20

[sp1]
enabled = true
prover = "network"

[native]
enabled = true

[sgx]
enabled = true
base_url = "http://sgx-prover:8080"
timeout_ms = 300000

[sgxgeth]
enabled = true
base_url = "http://sgxgeth-prover:8080"
timeout_ms = 300001
"#,
        )
        .expect("self-contained prover tables should parse");

        assert_eq!(
            config.iter_routes().collect::<Vec<_>>(),
            vec![
                (ProofType::Risc0, RunnerKind::Local),
                (ProofType::Sp1, RunnerKind::Network),
                (ProofType::Native, RunnerKind::Local),
                (ProofType::Sgx, RunnerKind::Remote),
                (ProofType::SgxGeth, RunnerKind::Remote),
            ]
        );
        assert_eq!(config.runner(ProofType::Sp1), Some(RunnerKind::Network));
        assert!(config.is_enabled(ProofType::SgxGeth));
        assert_eq!(config.sgx.base_url, "http://sgx-prover:8080");
        assert_eq!(config.sgxgeth.timeout_ms, 300_001);
    }

    #[test]
    fn self_contained_prover_config_rejects_superseded_sections() {
        for input in [
            "[routes]\nrisc0 = \"local\"\n",
            "[boundless]\nrpc_url = \"https://boundless.example.com\"\n",
            "[remote_sgx]\nbase_url = \"http://sgx-prover:8080\"\n",
        ] {
            let err = toml::from_str::<ProverConfig>(input)
                .expect_err("superseded prover section must be rejected");
            assert!(err.to_string().contains("unknown field"), "{err}");
        }
    }

    #[test]
    fn rejects_removed_risc0_bonsai_setting() {
        let err = toml::from_str::<ProverConfig>(
            r#"
[risc0]
enabled = true
runner = "local"
bonsai = true
"#,
        )
        .expect_err("the obsolete Bonsai selector must be rejected");

        assert!(err.to_string().contains("unknown field `bonsai`"), "{err}");
    }

    #[test]
    fn sp1_table_rejects_unknown_backend_fields() {
        let err = toml::from_str::<ProverConfig>(
            r#"
[sp1]
enabled = true
prover = "local"
unexpected = true
"#,
        )
        .expect_err("unknown SP1 backend field must fail closed");

        assert!(err.to_string().contains("unknown field"), "{err}");
    }

    #[test]
    fn risc0_table_rejects_remote_runner_during_parsing() {
        let err = toml::from_str::<ProverConfig>(
            r#"
[risc0]
enabled = true
runner = "remote"
"#,
        )
        .expect_err("RISC0 supports only the local runner");

        assert!(err.to_string().contains("unknown variant"), "{err}");
    }

    #[test]
    fn prover_config_defaults_to_native_local() {
        let default = ProverConfig::default();
        assert_eq!(
            default.iter_routes().collect::<Vec<_>>(),
            vec![(ProofType::Native, RunnerKind::Local)]
        );
        default.validate().expect("default native route is valid");

        let parsed = toml::from_str::<ProverConfig>("").expect("empty prover table uses defaults");
        assert!(parsed.native.enabled);

        let overridden = config_with_routes("risc0/local");
        assert!(!overridden.native.enabled);
        assert_eq!(
            overridden.iter_routes().collect::<Vec<_>>(),
            vec![(ProofType::Risc0, RunnerKind::Local)]
        );
    }

    #[test]
    fn route_override_replaces_all_enablement() {
        let mut config = config_with_routes("risc0/local,sgx/remote");
        config.apply_routes_override(&routes("native/local"));

        assert_eq!(
            config.iter_routes().collect::<Vec<_>>(),
            vec![(ProofType::Native, RunnerKind::Local)]
        );
    }

    #[test]
    fn prover_routes_accept_only_supported_runner_combinations() {
        for routes in [
            "risc0/local",
            "sp1/local",
            "sp1/network",
            "native/local",
            "sgx/remote",
            "sgxgeth/remote",
        ] {
            routes
                .parse::<ProverRoutesOverride>()
                .unwrap_or_else(|err| panic!("{routes} should parse: {err}"))
                .validate()
                .unwrap_or_else(|err| panic!("{routes} should validate: {err}"));
        }

        for routes in [
            "risc0/network",
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
                .parse::<ProverRoutesOverride>()
                .expect_err("unsupported runner combination must fail");
            assert!(err.contains("unsupported prover route"), "{routes}: {err}");
        }
    }

    #[test]
    fn prover_routes_override_parser_rejects_invalid_input() {
        let duplicate = "risc0/local,risc0/local"
            .parse::<ProverRoutesOverride>()
            .expect_err("duplicate proof type must fail");
        assert!(duplicate.contains("duplicate prover route"));

        let unknown = "unknown/local"
            .parse::<ProverRoutesOverride>()
            .expect_err("unknown proof type must fail");
        assert!(unknown.contains("Unknown proof type"));

        let invalid_runner = "risc0/invalid"
            .parse::<ProverRoutesOverride>()
            .expect_err("unknown runner must fail");
        assert!(invalid_runner.contains("Unknown runner"));

        let empty = ""
            .parse::<ProverRoutesOverride>()
            .expect_err("empty override must fail");
        assert!(empty.contains("must not be empty"));
    }

    #[test]
    fn prover_config_rejects_removed_global_route_fields() {
        for legacy_field in ["guest_system = \"risc0\"", "runner = \"local\""] {
            let input = format!("{legacy_field}\n[native]\nenabled = true\n");
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
        let mut config = config_with_routes("risc0/local,sp1/local");
        config.zk_any = ZkAnyConfig {
            sp1: Some(ZkAnyTargetConfig {
                probability: 0.3,
                per_day: 100,
            }),
            risc0: Some(ZkAnyTargetConfig {
                probability: 0.4,
                per_day: 0,
            }),
        };
        config.sp1.prover = Sp1ProverMode::Local;

        config.validate().expect("valid zk_any policy");
    }

    #[test]
    fn zk_any_rejects_disabled_sp1_target() {
        let mut config = config_with_routes("risc0/local");
        config.zk_any = ZkAnyConfig {
            sp1: Some(ZkAnyTargetConfig {
                probability: 0.5,
                per_day: 0,
            }),
            risc0: None,
        };

        let err = config
            .validate()
            .expect_err("zk_any must not target a disabled SP1 route");
        assert!(
            err.to_string()
                .contains("prover.zk_any.sp1 requires prover.sp1.enabled=true")
        );
    }

    #[test]
    fn zk_any_rejects_disabled_risc0_target() {
        let mut config = config_with_routes("sp1/local");
        config.zk_any = ZkAnyConfig {
            sp1: None,
            risc0: Some(ZkAnyTargetConfig {
                probability: 0.5,
                per_day: 0,
            }),
        };
        config.sp1.prover = Sp1ProverMode::Local;

        let err = config
            .validate()
            .expect_err("zk_any must not target a disabled RISC0 route");
        assert!(
            err.to_string()
                .contains("prover.zk_any.risc0 requires prover.risc0.enabled=true")
        );
    }

    #[test]
    fn sgx_route_requires_lane_specific_base_url() {
        let mut config = config_with_routes("sgx/remote");
        config.sgx.base_url.clear();
        config.sgxgeth.base_url = "https://sgxgeth.example.com".to_string();

        let err = config
            .validate()
            .expect_err("sgx/remote must require its own base URL");
        assert!(err.to_string().contains("prover.sgx.base_url"));
    }

    #[test]
    fn sgxgeth_route_requires_lane_specific_base_url() {
        let mut config = config_with_routes("sgxgeth/remote");
        config.sgx.base_url = "https://sgx.example.com".to_string();
        config.sgxgeth.base_url.clear();

        let err = config
            .validate()
            .expect_err("sgxgeth/remote must require its own base URL");
        assert!(err.to_string().contains("prover.sgxgeth.base_url"));
    }

    #[test]
    fn configured_sgx_urls_without_routes_do_not_enable_lanes() {
        let mut config = config_with_routes("native/local");
        config.sgx.base_url = "https://sgx.example.com".to_string();
        config.sgxgeth.base_url = "https://sgxgeth.example.com".to_string();
        config.sgx.timeout_ms = 0;
        config.sgxgeth.timeout_ms = 0;

        config
            .validate()
            .expect("configured SGX URLs must not enable absent routes");
    }

    #[test]
    fn each_enabled_sgx_lane_requires_positive_timeout() {
        for route in ["sgx/remote", "sgxgeth/remote"] {
            let mut config = config_with_routes(route);
            config.sgx.base_url = "https://sgx.example.com".to_string();
            config.sgxgeth.base_url = "https://sgxgeth.example.com".to_string();
            config.sgx.timeout_ms = 0;
            config.sgxgeth.timeout_ms = 0;

            let err = config
                .validate()
                .expect_err("enabled SGX lanes must require a positive timeout");
            assert!(
                err.to_string()
                    .contains("timeout_ms must be greater than zero")
            );
        }
    }

    #[test]
    fn sp1_local_allows_local_or_mock_prover() {
        for prover in [Sp1ProverMode::Local, Sp1ProverMode::Mock] {
            let mut config = config_with_routes("sp1/local");
            config.sp1.prover = prover;

            config
                .validate()
                .unwrap_or_else(|err| panic!("sp1/local should allow {prover:?}: {err}"));
        }
    }

    #[test]
    fn sp1_prover_mode_is_the_runner_source_of_truth() {
        let mut config = config_with_routes("sp1/network");
        assert_eq!(config.runner(ProofType::Sp1), Some(RunnerKind::Network));

        config.sp1.prover = Sp1ProverMode::Local;
        assert_eq!(config.runner(ProofType::Sp1), Some(RunnerKind::Local));

        config.sp1.prover = Sp1ProverMode::Mock;
        assert_eq!(config.runner(ProofType::Sp1), Some(RunnerKind::Local));
    }

    #[test]
    fn disabled_sp1_skips_sp1_backend_validation() {
        let mut config = config_with_routes("native/local");
        config.sp1.cycle_limit = 0;

        config
            .validate()
            .expect("disabled SP1 must not validate SP1 settings");
    }

    #[test]
    fn sp1_validation_error_omits_credential_bearing_url() {
        let mut config = config_with_routes("sp1/local");
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
    fn prover_config_rejects_sp1_prove_without_verification() {
        let mut config = config_with_routes("sp1/local");
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
