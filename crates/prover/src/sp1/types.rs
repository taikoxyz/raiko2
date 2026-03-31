use alloy_primitives::B256;
use raiko2_primitives::Proof;
use serde::{Deserialize, Serialize};
use sp1_sdk::{
    ExecutionReport, SP1ProofMode, SP1ProofWithPublicValues, SP1VerifyingKey,
    network::{FulfillmentStrategy, NetworkMode},
};
use tracing::error;

/// SP1 prover configuration parameters.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sp1Config {
    /// Proof mode (Core, Compressed, Plonk).
    #[serde(default)]
    pub recursion: RecursionMode,
    /// Prover mode (Mock, Local, Network).
    #[serde(default = "default_prover_mode")]
    pub prover: ProverMode,
    /// Execution mode (prove or execute-only).
    #[serde(default)]
    pub mode: ExecutionMode,
    /// Whether to verify the proof after generation.
    #[serde(default = "default_true")]
    pub verify: bool,
    /// Succinct network mode to use for remote proving.
    #[serde(default)]
    pub network_mode: Sp1NetworkMode,
    /// Succinct fulfillment strategy to use for remote proving.
    #[serde(default)]
    pub fulfillment_strategy: Sp1FulfillmentStrategy,
    /// Skip local simulation before submitting a network prove request.
    #[serde(default = "default_true")]
    pub skip_simulation: bool,
    /// Cycle limit to attach to network prove requests.
    #[serde(default = "default_cycle_limit")]
    pub cycle_limit: u64,
    /// Timeout to use when waiting for network proofs.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// Optional override for the Succinct network RPC URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rpc_url: Option<String>,
}

const fn default_true() -> bool {
    true
}

const fn default_prover_mode() -> ProverMode {
    ProverMode::Network
}

const fn default_cycle_limit() -> u64 {
    1_000_000_000_000
}

const fn default_timeout_secs() -> u64 {
    3_600
}

impl Default for Sp1Config {
    fn default() -> Self {
        Self {
            recursion: RecursionMode::Plonk,
            prover: default_prover_mode(),
            mode: ExecutionMode::Prove,
            verify: true,
            network_mode: Sp1NetworkMode::Reserved,
            fulfillment_strategy: Sp1FulfillmentStrategy::Reserved,
            skip_simulation: true,
            cycle_limit: default_cycle_limit(),
            timeout_secs: default_timeout_secs(),
            rpc_url: None,
        }
    }
}

/// Request-scoped SP1 overrides accepted via `prover_args.sp1`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sp1ConfigOverrides {
    #[serde(default)]
    pub recursion: Option<RecursionMode>,
    #[serde(default)]
    pub prover: Option<ProverMode>,
    #[serde(default)]
    pub mode: Option<ExecutionMode>,
    #[serde(default)]
    pub verify: Option<bool>,
    #[serde(default)]
    pub network_mode: Option<Sp1NetworkMode>,
    #[serde(default)]
    pub fulfillment_strategy: Option<Sp1FulfillmentStrategy>,
    #[serde(default)]
    pub skip_simulation: Option<bool>,
    #[serde(default)]
    pub cycle_limit: Option<u64>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

impl Sp1Config {
    #[must_use]
    pub fn merged_with(&self, overrides: &Sp1ConfigOverrides) -> Self {
        Self {
            recursion: overrides.recursion.unwrap_or(self.recursion),
            prover: overrides.prover.unwrap_or(self.prover),
            mode: overrides.mode.unwrap_or(self.mode),
            verify: overrides.verify.unwrap_or(self.verify),
            network_mode: overrides.network_mode.unwrap_or(self.network_mode),
            fulfillment_strategy: overrides
                .fulfillment_strategy
                .unwrap_or(self.fulfillment_strategy),
            skip_simulation: overrides.skip_simulation.unwrap_or(self.skip_simulation),
            cycle_limit: overrides.cycle_limit.unwrap_or(self.cycle_limit),
            timeout_secs: overrides.timeout_secs.unwrap_or(self.timeout_secs),
            rpc_url: self.rpc_url.clone(),
        }
    }

    /// # Errors
    ///
    /// Returns an error when the SP1 prover configuration is internally inconsistent.
    pub fn validate(&self) -> Result<(), String> {
        if self.cycle_limit == 0 {
            return Err("sp1.cycle_limit must be greater than 0".to_string());
        }
        if self.timeout_secs == 0 {
            return Err("sp1.timeout_secs must be greater than 0".to_string());
        }
        if self
            .rpc_url
            .as_deref()
            .is_some_and(|rpc_url| rpc_url.trim().is_empty())
        {
            return Err("sp1.rpc_url must not be empty".to_string());
        }
        if self.mode == ExecutionMode::Execute && self.prover == ProverMode::Network {
            return Err("sp1.mode=execute does not support sp1.prover=network".to_string());
        }
        if self.prover == ProverMode::Network {
            match (self.network_mode, self.fulfillment_strategy) {
                (Sp1NetworkMode::Mainnet, Sp1FulfillmentStrategy::Auction)
                | (
                    Sp1NetworkMode::Reserved,
                    Sp1FulfillmentStrategy::Reserved | Sp1FulfillmentStrategy::Hosted,
                ) => {}
                (Sp1NetworkMode::Mainnet, strategy) => {
                    return Err(format!(
                        "sp1.network_mode=mainnet requires sp1.fulfillment_strategy=auction, got {}",
                        strategy.as_str()
                    ));
                }
                (Sp1NetworkMode::Reserved, strategy) => {
                    return Err(format!(
                        "sp1.network_mode=reserved requires sp1.fulfillment_strategy=reserved or hosted, got {}",
                        strategy.as_str()
                    ));
                }
            }
        }

        Ok(())
    }
}

impl Sp1ConfigOverrides {
    #[must_use]
    pub const fn has_network_overrides(&self) -> bool {
        self.network_mode.is_some()
            || self.fulfillment_strategy.is_some()
            || self.skip_simulation.is_some()
            || self.cycle_limit.is_some()
            || self.timeout_secs.is_some()
    }
}

/// SP1 proof recursion mode.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RecursionMode {
    /// Core proof (no recursion).
    Core,
    /// Compressed proof.
    Compressed,
    /// Plonk proof (on-chain verifiable).
    #[default]
    Plonk,
}

impl From<RecursionMode> for SP1ProofMode {
    fn from(value: RecursionMode) -> Self {
        match value {
            RecursionMode::Core => SP1ProofMode::Core,
            RecursionMode::Compressed => SP1ProofMode::Compressed,
            RecursionMode::Plonk => SP1ProofMode::Plonk,
        }
    }
}

/// SP1 proposal execution mode.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    /// Execute the guest without producing a proof.
    Execute,
    /// Produce a proof through the configured prover.
    #[default]
    Prove,
}

impl ExecutionMode {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Execute => "execute",
            Self::Prove => "prove",
        }
    }
}

/// SP1 prover mode.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ProverMode {
    /// Mock prover for testing.
    Mock,
    /// Local CPU prover.
    Local,
    /// Network prover (Succinct network).
    Network,
}

/// SP1 network mode.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Sp1NetworkMode {
    /// Reserved capacity network for hosted or reserved proving.
    #[default]
    Reserved,
    /// Mainnet auction-based network.
    Mainnet,
}

impl Sp1NetworkMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Mainnet => "mainnet",
        }
    }
}

impl From<Sp1NetworkMode> for NetworkMode {
    fn from(value: Sp1NetworkMode) -> Self {
        match value {
            Sp1NetworkMode::Reserved => Self::Reserved,
            Sp1NetworkMode::Mainnet => Self::Mainnet,
        }
    }
}

/// SP1 fulfillment strategy.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Sp1FulfillmentStrategy {
    /// Reserved capacity strategy.
    #[default]
    Reserved,
    /// Hosted proving strategy.
    Hosted,
    /// Auction-based proving strategy.
    Auction,
}

impl Sp1FulfillmentStrategy {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Hosted => "hosted",
            Self::Auction => "auction",
        }
    }
}

impl From<Sp1FulfillmentStrategy> for FulfillmentStrategy {
    fn from(value: Sp1FulfillmentStrategy) -> Self {
        match value {
            Sp1FulfillmentStrategy::Reserved => Self::Reserved,
            Sp1FulfillmentStrategy::Hosted => Self::Hosted,
            Sp1FulfillmentStrategy::Auction => Self::Auction,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Sp1NetworkSubmissionProgress {
    pub provider_request_id: String,
    pub network_mode: Sp1NetworkMode,
    pub fulfillment_strategy: Sp1FulfillmentStrategy,
    pub skip_simulation: bool,
    pub cycle_limit: u64,
    pub timeout_secs: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Sp1NetworkMetadata {
    pub request_id: String,
    pub network_mode: Sp1NetworkMode,
    pub fulfillment_strategy: Sp1FulfillmentStrategy,
    pub skip_simulation: bool,
    pub cycle_limit: u64,
    pub timeout_secs: u64,
}

impl Sp1NetworkMetadata {
    #[must_use]
    pub const fn from_config(request_id: String, config: &Sp1Config) -> Self {
        Self {
            request_id,
            network_mode: config.network_mode,
            fulfillment_strategy: config.fulfillment_strategy,
            skip_simulation: config.skip_simulation,
            cycle_limit: config.cycle_limit,
            timeout_secs: config.timeout_secs,
        }
    }
}

/// SP1 proof response.
#[derive(Clone, Serialize, Deserialize)]
pub struct Sp1Response {
    /// Hex-encoded serialized proof
    pub proof: Option<String>,
    /// Verifying key hash (bytes32)
    pub vkey_hash: Option<String>,
    /// Public input commitment
    pub input: B256,
    /// For aggregation
    pub sp1_proof: Option<SP1ProofWithPublicValues>,
    /// Verifying key for verification
    #[serde(skip)]
    pub vkey: Option<SP1VerifyingKey>,
    /// Additional fork/backend metadata.
    pub extra_data: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Sp1CycleMetadata {
    pub label: String,
    pub cycles: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Sp1CountMetadata {
    pub label: String,
    pub count: u64,
}

/// Serializable SP1 execute metadata exposed by async `v3` tasks.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Sp1ExecutionMetadata {
    pub zkvm: String,
    pub mode: String,
    pub public_values: String,
    pub exit_code: u64,
    pub gas: Option<u64>,
    pub total_instruction_count: u64,
    pub total_syscall_count: u64,
    pub touched_memory_addresses: u64,
    pub cycle_tracker: Vec<Sp1CycleMetadata>,
    pub invocation_tracker: Vec<Sp1CountMetadata>,
    pub opcode_counts: Vec<Sp1CountMetadata>,
    pub syscall_counts: Vec<Sp1CountMetadata>,
}

fn count_entries(entries: impl Iterator<Item = (String, u64)>) -> Vec<Sp1CountMetadata> {
    let mut entries = entries
        .filter_map(|(label, count)| (count > 0).then_some(Sp1CountMetadata { label, count }))
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.label.cmp(&right.label));
    entries
}

impl Sp1ExecutionMetadata {
    #[must_use]
    pub fn from_execution_report(
        public_values: String,
        execution_report: &ExecutionReport,
    ) -> Self {
        let mut cycle_tracker = execution_report
            .cycle_tracker
            .iter()
            .map(|(label, cycles)| Sp1CycleMetadata {
                label: label.clone(),
                cycles: *cycles,
            })
            .collect::<Vec<_>>();
        cycle_tracker.sort_by(|left, right| left.label.cmp(&right.label));

        Self {
            zkvm: "sp1".to_string(),
            mode: ExecutionMode::Execute.as_str().to_string(),
            public_values,
            exit_code: execution_report.exit_code,
            gas: execution_report.gas(),
            total_instruction_count: execution_report.total_instruction_count(),
            total_syscall_count: execution_report.total_syscall_count(),
            touched_memory_addresses: execution_report.touched_memory_addresses,
            cycle_tracker,
            invocation_tracker: count_entries(
                execution_report
                    .invocation_tracker
                    .iter()
                    .map(|(label, count)| (label.clone(), *count)),
            ),
            opcode_counts: count_entries(
                execution_report
                    .opcode_counts
                    .iter()
                    .map(|(label, count)| (format!("{label:?}"), *count)),
            ),
            syscall_counts: count_entries(
                execution_report
                    .syscall_counts
                    .iter()
                    .map(|(label, count)| (format!("{label:?}"), *count)),
            ),
        }
    }
}

impl From<Sp1Response> for Proof {
    fn from(value: Sp1Response) -> Self {
        let quote = match value.sp1_proof.as_ref() {
            Some(proof) => match serde_json::to_string(&proof.proof) {
                Ok(serialized) => Some(serialized),
                Err(err) => {
                    error!(error = %err, "failed to serialize sp1 proof");
                    None
                }
            },
            None => None,
        };

        Self {
            proof: value.proof,
            quote,
            input: Some(value.input),
            uuid: value.vkey_hash,
            kzg_proof: None,
            extra_data: value.extra_data,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ExecutionMode, ProverMode, Sp1Config, Sp1ConfigOverrides, Sp1FulfillmentStrategy,
        Sp1NetworkMode,
    };

    #[test]
    fn sp1_default_config_is_valid() {
        assert!(Sp1Config::default().validate().is_ok());
    }

    #[test]
    fn sp1_execute_rejects_network_prover() {
        let config = Sp1Config {
            mode: ExecutionMode::Execute,
            prover: ProverMode::Network,
            ..Sp1Config::default()
        };

        let err = config
            .validate()
            .expect_err("execute should reject network");
        assert!(err.contains("sp1.mode=execute"));
    }

    #[test]
    fn sp1_mainnet_requires_auction_strategy() {
        let config = Sp1Config {
            network_mode: Sp1NetworkMode::Mainnet,
            fulfillment_strategy: Sp1FulfillmentStrategy::Reserved,
            ..Sp1Config::default()
        };

        let err = config
            .validate()
            .expect_err("mainnet should reject reserved strategy");
        assert!(err.contains("sp1.network_mode=mainnet"));
    }

    #[test]
    fn sp1_network_override_detection_ignores_local_fields() {
        let overrides = Sp1ConfigOverrides {
            network_mode: Some(Sp1NetworkMode::Mainnet),
            ..Sp1ConfigOverrides::default()
        };

        assert!(overrides.has_network_overrides());
    }
}
