use alloy_primitives::B256;
use raiko2_primitives::Proof;
use serde::{Deserialize, Serialize};
use sp1_sdk::{ExecutionReport, SP1ProofMode, SP1ProofWithPublicValues, SP1VerifyingKey};
use tracing::error;

/// SP1 prover configuration parameters.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sp1Config {
    /// Proof mode (Core, Compressed, Plonk).
    #[serde(default)]
    pub recursion: RecursionMode,
    /// Prover mode (Mock, Local, Network).
    #[serde(default)]
    pub prover: Option<ProverMode>,
    /// Execution mode (prove or execute-only).
    #[serde(default)]
    pub mode: ExecutionMode,
    /// Whether to verify the proof after generation.
    #[serde(default = "default_true")]
    pub verify: bool,
}

const fn default_true() -> bool {
    true
}

impl Default for Sp1Config {
    fn default() -> Self {
        Self {
            recursion: RecursionMode::Plonk,
            prover: None,
            mode: ExecutionMode::Prove,
            verify: true,
        }
    }
}

/// Request-scoped SP1 overrides accepted via `prover_args.sp1`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Sp1ConfigOverrides {
    #[serde(default)]
    pub recursion: Option<RecursionMode>,
    #[serde(default)]
    pub prover: Option<ProverMode>,
    #[serde(default)]
    pub mode: Option<ExecutionMode>,
    #[serde(default)]
    pub verify: Option<bool>,
}

impl Sp1Config {
    #[must_use]
    pub fn merged_with(&self, overrides: &Sp1ConfigOverrides) -> Self {
        Self {
            recursion: overrides
                .recursion
                .clone()
                .unwrap_or_else(|| self.recursion.clone()),
            prover: overrides.prover.clone().or_else(|| self.prover.clone()),
            mode: overrides.mode.clone().unwrap_or_else(|| self.mode.clone()),
            verify: overrides.verify.unwrap_or(self.verify),
        }
    }
}

/// SP1 proof recursion mode.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
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
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
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
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ProverMode {
    /// Mock prover for testing.
    Mock,
    /// Local CPU prover.
    Local,
    /// Network prover (Succinct network).
    Network,
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
