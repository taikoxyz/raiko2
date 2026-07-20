use alloy_primitives::Address;
use raiko2_primitives::ShastaCheckpoint;
use serde::{Deserialize, Serialize};

use super::ProverSkippedStatusCounts;

/// Error payload returned by v4 endpoints.
#[derive(Serialize)]
pub(crate) struct ApiErrorBody {
    pub(crate) status: &'static str,
    pub(crate) error: &'static str,
    pub(crate) message: String,
}

/// Success envelope for v4 asynchronous proof submission responses.
#[derive(Serialize)]
pub(crate) struct TaskResponse<T> {
    pub(crate) status: &'static str,
    pub(crate) proof_type: String,
    pub(crate) proposal_id_start: u64,
    pub(crate) proposal_id_end: u64,
    pub(crate) data: T,
}

/// Concrete proof backends accepted by v4 endpoints.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProofType {
    Risc0,
    Sp1,
    Sgx,
    #[serde(rename = "sgxgeth")]
    SgxGeth,
}

impl ProofType {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Risc0 => "risc0",
            Self::Sp1 => "sp1",
            Self::Sgx => "sgx",
            Self::SgxGeth => "sgxgeth",
        }
    }
}

/// Request body for v4 proposal-side proof admission.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProofRequest {
    pub(crate) proof_type: ProofType,
    pub(crate) proposals: Vec<ProposalRequest>,
    #[serde(default)]
    pub(crate) aggregate: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) prover: Option<Address>,
}

/// Proposal inputs carried by the unified v4 proof request.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProposalRequest {
    pub(crate) proposal_id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) checkpoint: Option<ShastaCheckpoint>,
    pub(crate) l1_inclusion_block_number: u64,
    pub(crate) l2_block_number_start: u64,
    pub(crate) l2_block_number_end: u64,
    pub(crate) last_anchor_block_number: u64,
}

/// Query string accepted by the v4 prover-status endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProverStatusQuery {
    pub(crate) proof_type: ProofType,
}

/// Request body accepted by the v4 prover-clear endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProverClearRequest {
    pub(crate) proof_type: ProofType,
}

/// Request body accepted by the v4 proof-artifact invalidation endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InvalidateArtifactsRequest {
    pub(crate) proof_type: ProofType,
    #[serde(default)]
    pub(crate) proof_prefix: Option<String>,
    #[serde(default)]
    pub(crate) proposal_id_start: Option<u64>,
    #[serde(default)]
    pub(crate) proposal_id_end: Option<u64>,
    #[serde(default)]
    pub(crate) dry_run: bool,
}

/// Response data for a v4 proof task.
#[derive(Debug, Serialize)]
pub(crate) struct ProofTaskData {
    pub(crate) task_id: String,
    pub(crate) status: String,
    pub(crate) proof: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
}

/// Response data for a v4 prover-clear request.
#[derive(Serialize)]
pub(crate) struct ClearProverData {
    pub(crate) cancelled: usize,
    pub(crate) skipped: ProverSkippedStatusCounts,
    pub(crate) failed: usize,
}

/// Response data for a v4 proof-artifact invalidation request.
#[derive(Default, Serialize)]
pub(crate) struct InvalidateArtifactsData {
    pub(crate) dry_run: bool,
    pub(crate) artifacts: InvalidateArtifactCounts,
    pub(crate) tasks: InvalidateTaskCounts,
}

#[derive(Default, Serialize)]
pub(crate) struct InvalidateArtifactCounts {
    pub(crate) matched: usize,
    pub(crate) removed: usize,
    pub(crate) manifests_removed: usize,
    pub(crate) manifests_missing: usize,
    pub(crate) failed: usize,
}

#[derive(Default, Serialize)]
pub(crate) struct InvalidateTaskCounts {
    pub(crate) matched: usize,
    pub(crate) removed: usize,
    pub(crate) skipped_non_terminal: usize,
    pub(crate) invalid_metadata: usize,
    pub(crate) failed: usize,
}
