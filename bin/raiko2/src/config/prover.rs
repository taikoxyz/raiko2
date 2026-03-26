use anyhow::{Result, bail};
use raiko2_prover::boundless::{DeploymentConfig, OfferParamsConfig};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Guest execution system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum GuestSystem {
    #[default]
    Risc0,
    Sp1,
    Native,
}

impl GuestSystem {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            GuestSystem::Risc0 => "risc0",
            GuestSystem::Sp1 => "sp1",
            GuestSystem::Native => "native",
        }
    }
}

impl fmt::Display for GuestSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for GuestSystem {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "risc0" => Ok(Self::Risc0),
            "sp1" => Ok(Self::Sp1),
            "native" => Ok(Self::Native),
            _ => Err(format!("Unknown guest_system: {s}")),
        }
    }
}

/// Prover runner implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RunnerKind {
    #[default]
    Local,
    Boundless,
}

impl RunnerKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            RunnerKind::Local => "local",
            RunnerKind::Boundless => "boundless",
        }
    }
}

impl fmt::Display for RunnerKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RunnerKind {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "local" => Ok(Self::Local),
            "boundless" => Ok(Self::Boundless),
            _ => Err(format!("Unknown runner: {s}")),
        }
    }
}

/// Canonical route for a proving request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PipelineRoute {
    pub guest_system: GuestSystem,
    pub runner: RunnerKind,
}

impl PipelineRoute {
    #[must_use]
    pub const fn new(guest_system: GuestSystem, runner: RunnerKind) -> Self {
        Self {
            guest_system,
            runner,
        }
    }
}

impl fmt::Display for PipelineRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.guest_system, self.runner)
    }
}

impl FromStr for PipelineRoute {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let (guest_system, runner) = s
            .split_once('/')
            .ok_or_else(|| format!("Invalid route '{s}', expected <guest_system>/<runner>"))?;
        Ok(Self::new(guest_system.parse()?, runner.parse()?))
    }
}

/// Prover configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
}

impl ProverConfig {
    #[must_use]
    pub const fn route(&self) -> PipelineRoute {
        PipelineRoute::new(self.guest_system, self.runner)
    }

    /// # Errors
    ///
    /// Returns an error if the configured guest system and runner are incompatible.
    pub fn validate(&self) -> Result<()> {
        match self.route() {
            PipelineRoute {
                guest_system: GuestSystem::Risc0,
                ..
            }
            | PipelineRoute {
                guest_system: GuestSystem::Sp1 | GuestSystem::Native,
                runner: RunnerKind::Local,
            } => {}
            PipelineRoute {
                guest_system: GuestSystem::Sp1,
                runner: RunnerKind::Boundless,
            } => bail!("sp1/boundless is not supported"),
            PipelineRoute {
                guest_system: GuestSystem::Native,
                runner: RunnerKind::Boundless,
            } => bail!("native/boundless is not supported"),
        }

        if matches!(self.runner, RunnerKind::Boundless) {
            if self.boundless.rpc_url.trim().is_empty() {
                bail!("prover.boundless.rpc_url must not be empty");
            }
            if self.boundless.signer_key.trim().is_empty() {
                bail!("prover.boundless.signer_key must not be empty");
            }
        }

        Ok(())
    }
}

/// RISC0 configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Risc0Config {
    pub bonsai: bool,
    pub snark: bool,
    #[serde(default)]
    pub mock: bool,
}

impl Default for Risc0Config {
    fn default() -> Self {
        Self {
            bonsai: true,
            snark: true,
            mock: false,
        }
    }
}

/// SP1 configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sp1Config {
    pub network: bool,
    pub plonk: bool,
}

impl Default for Sp1Config {
    fn default() -> Self {
        Self {
            network: true,
            plonk: true,
        }
    }
}

/// Boundless configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundlessConfig {
    #[serde(default = "default_boundless_offchain")]
    pub offchain: bool,
    pub rpc_url: String,
    pub signer_key: String,
    #[serde(default)]
    pub deployment: Option<DeploymentConfig>,
    pub offer_params: OfferParamsConfig,
    #[serde(default = "default_boundless_poll_interval_ms")]
    pub poll_interval_ms: u64,
    #[serde(default = "default_boundless_timeout_ms")]
    pub timeout_ms: u64,
}

impl Default for BoundlessConfig {
    fn default() -> Self {
        Self {
            offchain: raiko2_prover::boundless::BoundlessConfig::default().offchain,
            rpc_url: raiko2_prover::boundless::BoundlessConfig::default().rpc_url,
            signer_key: String::new(),
            deployment: raiko2_prover::boundless::BoundlessConfig::default().deployment,
            offer_params: raiko2_prover::boundless::BoundlessConfig::default().offer_params,
            poll_interval_ms: default_boundless_poll_interval_ms(),
            timeout_ms: default_boundless_timeout_ms(),
        }
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
