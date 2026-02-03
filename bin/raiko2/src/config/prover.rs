use serde::{Deserialize, Serialize};

/// Prover type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProverType {
    #[default]
    Risc0,
    Sp1,
    Native,
    AgentRisc0,
}

impl std::str::FromStr for ProverType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "risc0" => Ok(ProverType::Risc0),
            "sp1" => Ok(ProverType::Sp1),
            "native" => Ok(ProverType::Native),
            "agent-risc0" => Ok(ProverType::AgentRisc0),
            _ => Err(format!("Unknown prover type: {s}")),
        }
    }
}

/// Prover configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProverConfig {
    pub prover_type: ProverType,
    /// RISC0 specific configuration.
    #[serde(default)]
    pub risc0: Risc0Config,
    /// SP1 specific configuration.
    #[serde(default)]
    pub sp1: Sp1Config,
    /// Agent-specific configuration.
    #[serde(default)]
    pub agent: AgentConfig,
}

/// RISC0 configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Risc0Config {
    pub bonsai: bool,
    pub snark: bool,
}

impl Default for Risc0Config {
    fn default() -> Self {
        Self {
            bonsai: true,
            snark: true,
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

/// Agent configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub url: String,
    pub api_key: Option<String>,
    pub poll_interval_ms: u64,
    pub timeout_ms: u64,
    pub prover_type: String,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:9999".to_string(),
            api_key: None,
            poll_interval_ms: 1_000,
            timeout_ms: 300_000,
            prover_type: "boundless".to_string(),
        }
    }
}
