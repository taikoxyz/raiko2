use raiko2_primitives::{Proof, ShastaCheckpoint};
use raiko2_prover::sp1::Sp1ConfigOverrides;
use raiko2_runtime::RunnerStatus as RuntimeRunnerStatus;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::server::state::ProofStatus;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum BatchProofType {
    Native,
    Sp1,
    Risc0,
    #[serde(rename = "boundless", alias = "BOUNDLESS")]
    Boundless,
    Sgx,
    #[serde(rename = "sgxgeth", alias = "SGXGETH", alias = "sgx_geth")]
    SgxGeth,
    Tdx,
    ZkAny,
}

impl BatchProofType {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Sp1 => "sp1",
            Self::Risc0 => "risc0",
            Self::Boundless => "boundless",
            Self::Sgx => "sgx",
            Self::SgxGeth => "sgxgeth",
            Self::Tdx => "tdx",
            Self::ZkAny => "zk_any",
        }
    }

    pub(super) const fn is_public_batch_request_type(self) -> bool {
        match self {
            Self::Native | Self::Sp1 | Self::Risc0 | Self::Sgx | Self::SgxGeth | Self::ZkAny => {
                true
            }
            // TDX is only a valid public request type when raiko2 is built with --features tdx.
            // Without the feature, no TDX pipeline is registered and the request would produce
            // an opaque 404; returning false here lets the handler emit a clear 400 instead.
            Self::Tdx => cfg!(feature = "tdx"),
            Self::Boundless => false,
        }
    }

    pub(super) const fn is_concrete_public_proof_type(self) -> bool {
        matches!(self, Self::Sp1 | Self::Risc0 | Self::Sgx | Self::SgxGeth)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BatchShastaRequest {
    pub(super) proposals: Vec<ShastaProposal>,
    #[serde(default)]
    pub(super) aggregate: bool,
    pub(super) proof_type: BatchProofType,
    #[serde(default)]
    pub(super) network: Option<String>,
    #[serde(default)]
    pub(super) l1_network: Option<String>,
    #[serde(default)]
    pub(super) graffiti: Option<String>,
    #[serde(default)]
    pub(super) prover: Option<String>,
    #[serde(default)]
    pub(super) blob_proof_type: Option<String>,
    #[serde(flatten)]
    pub(super) prover_args: PublicProverArgs,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AggregateProofRequest {
    #[serde(default)]
    pub(super) aggregation_ids: Vec<u64>,
    pub(super) proofs: Vec<Proof>,
    pub(super) proof_type: BatchProofType,
    #[serde(default)]
    pub(super) network: Option<String>,
    #[serde(default)]
    pub(super) l1_network: Option<String>,
    #[serde(default)]
    pub(super) graffiti: Option<String>,
    #[serde(default)]
    pub(super) prover: Option<String>,
    #[serde(default)]
    pub(super) blob_proof_type: Option<String>,
    #[serde(flatten)]
    pub(super) prover_args: PublicProverArgs,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ShastaProposal {
    pub(super) proposal_id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) checkpoint: Option<ShastaCheckpoint>,
    pub(super) l1_inclusion_block_number: u64,
    pub(super) l2_block_numbers: Vec<u64>,
    pub(super) last_anchor_block_number: u64,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct PublicProverArgs {
    pub(super) native: Option<Value>,
    pub(super) sgx: Option<Value>,
    pub(super) sgxgeth: Option<Value>,
    pub(super) sp1: Option<Sp1ConfigOverrides>,
    pub(super) risc0: Option<Value>,
}

impl PublicProverArgs {
    pub(super) const fn is_empty(&self) -> bool {
        self.native.is_none()
            && self.sgx.is_none()
            && self.sgxgeth.is_none()
            && self.sp1.is_none()
            && self.risc0.is_none()
    }
}

#[derive(Serialize)]
pub(crate) struct ApiOk<T> {
    pub(crate) status: &'static str,
    pub(crate) proof_type: String,
    pub(crate) data: T,
}

#[derive(Serialize)]
pub(crate) struct LegacyProofEnvelope {
    pub(crate) status: &'static str,
    pub(crate) proof_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) batch_id: Option<u64>,
    pub(crate) data: LegacyProofData,
}

#[derive(Serialize)]
#[serde(untagged)]
pub(crate) enum LegacyProofData {
    Status { status: LegacyTaskStatus },
    Proof { proof: Proof },
}

#[derive(Serialize)]
pub(crate) struct LegacyProofError {
    pub(crate) status: &'static str,
    pub(crate) error: &'static str,
    pub(crate) message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LegacyTaskStatus {
    Registered,
    WorkInProgress,
    Cancelled,
    AnyhowError(String),
    ZkAnyNotDrawn,
}

#[derive(Serialize)]
pub(crate) struct TaskData {
    pub(crate) task_id: String,
    pub(crate) route: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) prover_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) execution_mode: Option<String>,
    pub(crate) status: ProofStatus,
    pub(crate) network: String,
    pub(crate) l1_network: String,
    pub(crate) runtime: RootRuntime,
    pub(crate) current_index: Option<usize>,
    pub(crate) proposals: Vec<ProposalStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) aggregate: Option<AggregateStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) proof: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) proof_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) proof_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct ProposalStatus {
    pub(crate) index: usize,
    pub(crate) proposal_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) checkpoint: Option<ShastaCheckpoint>,
    pub(crate) task_id: String,
    pub(crate) status: ProofStatus,
    pub(crate) l1_inclusion_block_number: u64,
    pub(crate) l2_block_numbers: Vec<u64>,
    pub(crate) last_anchor_block_number: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) proof: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) proof_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) proof_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) runtime: Option<TaskRuntime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) extra_data: Option<Value>,
}

#[derive(Serialize)]
pub(crate) struct AggregateStatus {
    pub(crate) task_id: String,
    pub(crate) status: ProofStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) proof: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) proof_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) proof_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) runtime: Option<TaskRuntime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) extra_data: Option<Value>,
}

#[derive(Serialize)]
pub(crate) struct RootRuntime {
    pub(crate) runner_status: RuntimeRunnerStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) active_stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_event: Option<String>,
    pub(crate) updated_at: i64,
    pub(crate) engine_state_present: bool,
}

#[derive(Serialize)]
pub(crate) struct TaskRuntime {
    pub(crate) updated_at: i64,
    pub(crate) engine_state_present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) provider_request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) remote_tx_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) image_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) deployment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) offchain: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) expires_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) quoted_mcycles_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) evaluated_mcycles_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sp1_network_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sp1_fulfillment_strategy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sp1_skip_simulation: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sp1_cycle_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sp1_timeout_secs: Option<u64>,
}

#[derive(Serialize)]
pub(crate) struct PruneStatus {
    pub(crate) status: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct CanonicalProposal {
    pub(super) proposal_id: u64,
    pub(super) checkpoint: Option<ShastaCheckpoint>,
    pub(super) l1_inclusion_block_number: u64,
    pub(super) l2_block_numbers: Vec<u64>,
    pub(super) l2_block_range: raiko2_primitives::L2BlockRange,
    pub(super) last_anchor_block_number: u64,
}

#[derive(Debug, Clone)]
pub(super) struct RootTaskState {
    pub(super) status: ProofStatus,
    pub(super) proof: Option<String>,
    pub(super) error: Option<String>,
    pub(super) current_index: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::{BatchProofType, BatchShastaRequest};

    #[test]
    fn shasta_batch_request_accepts_sgxgeth_json_variant() {
        let req: BatchShastaRequest = serde_json::from_value(serde_json::json!({
            "proposals": [{
                "proposal_id": 1,
                "l1_inclusion_block_number": 2,
                "l2_block_numbers": [3],
                "last_anchor_block_number": 1
            }],
            "aggregate": false,
            "proof_type": "sgxgeth",
            "network": "taiko_mainnet",
            "l1_network": "ethereum"
        }))
        .expect("deserialize request");

        assert!(matches!(req.proof_type, BatchProofType::SgxGeth));
    }

    #[test]
    fn native_is_accepted_for_internal_batch_requests() {
        assert!(BatchProofType::Native.is_public_batch_request_type());
    }
}
