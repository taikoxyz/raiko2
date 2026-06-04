use alloy_primitives::B256;
use raiko2_primitives::Proof;
use serde::{Deserialize, Serialize};
use sp1_sdk::{ExecutionReport, SP1ProofWithPublicValues, SP1VerifyingKey};
use tracing::error;

use crate::sp1_config::ExecutionMode;

/// SP1 proof response.
#[derive(Clone, Serialize, Deserialize)]
pub struct Sp1Response {
    /// Hex-encoded serialized proof
    pub proof: Option<String>,
    /// Canonical 32-byte SP1 verifier digest as hex.
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
        let Sp1Response {
            proof,
            vkey_hash,
            input,
            sp1_proof,
            vkey,
            extra_data,
        } = value;
        let quote = match sp1_proof.as_ref() {
            Some(proof) => match serde_json::to_string(&proof.proof) {
                Ok(serialized) => Some(serialized),
                Err(err) => {
                    error!(error = %err, "failed to serialize sp1 proof");
                    None
                }
            },
            None => None,
        };
        let uuid = match vkey.as_ref() {
            Some(vk) => match serde_json::to_string(vk) {
                Ok(serialized) => Some(serialized),
                Err(err) => {
                    error!(error = %err, "failed to serialize sp1 verifying key");
                    vkey_hash
                }
            },
            None => vkey_hash,
        };

        Self {
            proof,
            quote,
            input: Some(input),
            uuid,
            kzg_proof: None,
            extra_data,
        }
    }
}
