use raiko2_primitives::{Proof, ShastaCheckpoint};
use raiko2_prover::sp1_config::Sp1ConfigOverrides;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(in super::super) enum BatchProofType {
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
    pub(in super::super) const fn as_str(self) -> &'static str {
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

    pub(in super::super) const fn is_public_batch_request_type(self) -> bool {
        matches!(
            self,
            Self::Native | Self::Sp1 | Self::Risc0 | Self::Sgx | Self::SgxGeth | Self::ZkAny
        )
    }

    pub(in super::super) const fn is_concrete_public_proof_type(self) -> bool {
        matches!(self, Self::Sp1 | Self::Risc0 | Self::Sgx | Self::SgxGeth)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BatchShastaRequest {
    pub(in super::super) proposals: Vec<ShastaProposal>,
    #[serde(default)]
    pub(in super::super) aggregate: bool,
    pub(in super::super) proof_type: BatchProofType,
    #[serde(default)]
    pub(in super::super) network: Option<String>,
    #[serde(default)]
    pub(in super::super) l1_network: Option<String>,
    #[serde(default)]
    pub(in super::super) graffiti: Option<String>,
    #[serde(default)]
    pub(in super::super) prover: Option<String>,
    #[serde(default)]
    pub(in super::super) blob_proof_type: Option<String>,
    #[serde(flatten)]
    pub(in super::super) prover_args: PublicProverArgs,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AggregateProofRequest {
    #[serde(default)]
    pub(in super::super) aggregation_ids: Vec<u64>,
    pub(in super::super) proofs: Vec<Proof>,
    pub(in super::super) proof_type: BatchProofType,
    #[serde(default)]
    pub(in super::super) network: Option<String>,
    #[serde(default)]
    pub(in super::super) l1_network: Option<String>,
    #[serde(default)]
    pub(in super::super) graffiti: Option<String>,
    #[serde(default)]
    pub(in super::super) prover: Option<String>,
    #[serde(default)]
    pub(in super::super) blob_proof_type: Option<String>,
    #[serde(flatten)]
    pub(in super::super) prover_args: PublicProverArgs,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(in super::super) struct ShastaProposal {
    pub(in super::super) proposal_id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in super::super) checkpoint: Option<ShastaCheckpoint>,
    pub(in super::super) l1_inclusion_block_number: u64,
    pub(in super::super) l2_block_numbers: Vec<u64>,
    pub(in super::super) last_anchor_block_number: u64,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(in super::super) struct PublicProverArgs {
    pub(in super::super) native: Option<Value>,
    pub(in super::super) sgx: Option<Value>,
    pub(in super::super) sgxgeth: Option<Value>,
    pub(in super::super) sp1: Option<Sp1ConfigOverrides>,
    pub(in super::super) risc0: Option<Value>,
}

impl PublicProverArgs {
    pub(in super::super) const fn is_empty(&self) -> bool {
        self.native.is_none()
            && self.sgx.is_none()
            && self.sgxgeth.is_none()
            && self.sp1.is_none()
            && self.risc0.is_none()
    }
}
