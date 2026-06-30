use raiko2_primitives::{Proof, ShastaCheckpoint};
use raiko2_prover::sp1_config::Sp1ConfigOverrides;
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
            Self::ZkAny => "zk_any",
        }
    }

    pub(super) const fn is_public_batch_request_type(self) -> bool {
        matches!(
            self,
            Self::Native | Self::Sp1 | Self::Risc0 | Self::Sgx | Self::SgxGeth | Self::ZkAny
        )
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
pub(crate) struct ApiData<T> {
    pub(crate) status: &'static str,
    pub(crate) data: T,
}

#[derive(Serialize)]
pub(crate) struct V4ApiErrorBody {
    pub(crate) status: &'static str,
    pub(crate) error: &'static str,
    pub(crate) message: String,
}

pub(crate) mod v4 {
    use alloy_primitives::Address;
    use raiko2_primitives::{Proof, ShastaCheckpoint};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub(crate) enum ProofType {
        Risc0,
        Sp1,
    }

    impl ProofType {
        pub(crate) const fn as_str(self) -> &'static str {
            match self {
                Self::Risc0 => "risc0",
                Self::Sp1 => "sp1",
            }
        }
    }

    #[derive(Debug, Clone, Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    pub(crate) struct ProposalRequest {
        pub(crate) proof_type: ProofType,
        pub(crate) proposal_id: u64,
        pub(crate) last_anchor_block_number: u64,
        pub(crate) l1_inclusion_block_number: u64,
        pub(crate) l2_block_number_start: u64,
        pub(crate) l2_block_number_end: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub(crate) checkpoint: Option<ShastaCheckpoint>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub(crate) prover: Option<Address>,
    }

    #[derive(Debug, Clone, Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    pub(crate) struct AggregationRequest {
        pub(crate) proof_type: ProofType,
        pub(crate) proposal_id_start: u64,
        pub(crate) proposal_id_end: u64,
    }

    #[derive(Debug, Clone, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub(crate) struct ProverStatusQuery {
        pub(crate) proof_type: ProofType,
    }

    #[derive(Debug, Clone, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub(crate) struct ProverClearRequest {
        pub(crate) proof_type: ProofType,
    }

    #[derive(Debug, Serialize)]
    pub(crate) struct ProofTaskData {
        pub(crate) task_id: String,
        pub(crate) route: String,
        pub(crate) prover_type: Option<String>,
        pub(crate) status: String,
        pub(crate) proof: Option<Proof>,
    }

    #[derive(Debug, Serialize)]
    pub(crate) struct AggregationTaskData {
        pub(crate) task_id: String,
        pub(crate) route: String,
        pub(crate) prover_type: Option<String>,
        pub(crate) status: String,
        pub(crate) proof: Option<Proof>,
        pub(crate) proposal_id_start: u64,
        pub(crate) proposal_id_end: u64,
    }
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
    pub(crate) submitted_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) quoted_mcycles_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) evaluated_mcycles_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_price_multiplier: Option<u32>,
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

#[derive(Serialize)]
pub(crate) struct ClearProverStatus {
    pub(crate) status: &'static str,
    pub(crate) cancelled: usize,
    pub(crate) skipped: ProverSkippedStatusCounts,
    pub(crate) failed: usize,
}

#[derive(Serialize)]
pub(crate) struct ProverStatus {
    pub(crate) clean: bool,
    pub(crate) tasks: ProverTaskStatusCounts,
    pub(crate) network: ProverNetworkStatus,
    pub(crate) skipped: ProverSkippedStatusCounts,
}

#[derive(Default, Serialize)]
pub(crate) struct ProverSkippedStatusCounts {
    pub(crate) invalid_metadata: usize,
    pub(crate) unavailable_pipeline: usize,
    pub(crate) remote_progress: usize,
}

impl ProverSkippedStatusCounts {
    pub(crate) const fn is_clean(&self) -> bool {
        self.invalid_metadata == 0 && self.unavailable_pipeline == 0 && self.remote_progress == 0
    }
}

#[derive(Default, Serialize)]
pub(crate) struct ProverTaskStatusCounts {
    pub(crate) pending: usize,
    pub(crate) ready: usize,
    pub(crate) retrying: usize,
    pub(crate) running: usize,
    pub(crate) orphaned: usize,
}

impl ProverTaskStatusCounts {
    pub(crate) const fn is_clean(&self) -> bool {
        self.pending == 0
            && self.ready == 0
            && self.retrying == 0
            && self.running == 0
            && self.orphaned == 0
    }
}

#[derive(Default, Serialize)]
pub(crate) struct ProverNetworkStatus {
    pub(crate) sp1: ProverNetworkBackendStatus,
    pub(crate) risc0: ProverNetworkBackendStatus,
}

impl ProverNetworkStatus {
    pub(crate) const fn is_clean(&self) -> bool {
        self.sp1.inflight_orders == 0 && self.risc0.inflight_orders == 0
    }
}

#[derive(Default, Serialize)]
pub(crate) struct ProverNetworkBackendStatus {
    pub(crate) inflight_orders: usize,
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
    use super::{
        ApiData, BatchProofType, BatchShastaRequest, ClearProverStatus, ProverNetworkBackendStatus,
        ProverNetworkStatus, ProverSkippedStatusCounts, ProverStatus, ProverTaskStatusCounts,
        PruneStatus, v4,
    };

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

    #[test]
    fn prover_api_shapes_match_issue_93() {
        let status = serde_json::to_value(ApiData {
            status: "ok",
            data: ProverStatus {
                clean: false,
                tasks: ProverTaskStatusCounts {
                    pending: 0,
                    ready: 2,
                    retrying: 1,
                    running: 6,
                    orphaned: 5,
                },
                network: ProverNetworkStatus {
                    sp1: ProverNetworkBackendStatus { inflight_orders: 3 },
                    risc0: ProverNetworkBackendStatus { inflight_orders: 0 },
                },
                skipped: ProverSkippedStatusCounts::default(),
            },
        })
        .expect("serialize status");
        assert_eq!(status["status"], "ok");
        assert_eq!(status["data"]["tasks"]["ready"], 2);
        assert_eq!(status["data"]["tasks"]["orphaned"], 5);
        assert_eq!(status["data"]["network"]["sp1"]["inflight_orders"], 3);

        let clear = serde_json::to_value(ClearProverStatus {
            status: "ok",
            cancelled: 2,
            skipped: ProverSkippedStatusCounts {
                invalid_metadata: 1,
                unavailable_pipeline: 3,
                remote_progress: 0,
            },
            failed: 4,
        })
        .expect("serialize clear");
        assert_eq!(
            clear,
            serde_json::json!({
                "status": "ok",
                "cancelled": 2,
                "skipped": {
                    "invalid_metadata": 1,
                    "remote_progress": 0,
                    "unavailable_pipeline": 3
                },
                "failed": 4
            })
        );

        let prune = serde_json::to_value(PruneStatus { status: "ok" }).expect("serialize prune");
        assert_eq!(prune, serde_json::json!({ "status": "ok" }));
    }

    #[test]
    fn v4_proposal_request_rejects_unknown_fields() {
        let err = serde_json::from_value::<v4::ProposalRequest>(serde_json::json!({
            "proof_type": "risc0",
            "proposal_id": 1,
            "last_anchor_block_number": 10,
            "l1_inclusion_block_number": 11,
            "l2_block_number_start": 20,
            "l2_block_number_end": 21,
            "network": "taiko_dev"
        }))
        .expect_err("unknown v4 proposal fields must be rejected");
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn v4_aggregation_request_accepts_only_explicit_proof_types() {
        let req = serde_json::from_value::<v4::AggregationRequest>(serde_json::json!({
            "proof_type": "sp1",
            "proposal_id_start": 10,
            "proposal_id_end": 12
        }))
        .expect("deserialize v4 aggregation request");
        assert!(matches!(req.proof_type, v4::ProofType::Sp1));

        let err = serde_json::from_value::<v4::AggregationRequest>(serde_json::json!({
            "proof_type": "zk_any",
            "proposal_id_start": 10,
            "proposal_id_end": 12
        }))
        .expect_err("zk_any is not a v4 proof type");
        assert!(err.to_string().contains("unknown variant"));
    }
}
