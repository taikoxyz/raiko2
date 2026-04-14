use alloy_consensus::TrieAccount;
use alloy_primitives::map::AddressMap;
use raiko2_primitives::{ChainSpec, ExecutionWitness, Proof, StatelessInput};
use raiko2_primitives_shasta::proof_carry_from_proof;
use raiko2_protocol_shasta::shasta::ProofCarryData;
use reth_ethereum_primitives::Block;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

pub const GAIKO2_SHASTA_REQUEST_SCHEMA: &str = "v1";
pub const GAIKO2_PROOF_RESPONSE_SCHEMA: &str = "gaiko2-proof-v1";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Gaiko2ShastaRequest {
    pub schema: String,
    pub payload: Gaiko2ShastaPayload,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Gaiko2ShastaPayload {
    pub chain_id: u64,
    pub blocks: Vec<Gaiko2ReplayBlock>,
    pub proof_carry_data: ProofCarryData,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Gaiko2ShastaAggregateRequest {
    pub schema: String,
    pub payload: Gaiko2ShastaAggregatePayload,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Gaiko2ShastaAggregatePayload {
    pub proofs: Vec<Gaiko2AggregateProof>,
}

#[serde_as]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Gaiko2ReplayBlock {
    #[serde_as(as = "raiko2_primitives::EthereumBlock<'_>")]
    pub block: Block,
    pub chain_spec: ChainSpec,
    pub witness: ExecutionWitness,
    pub accounts: AddressMap<TrieAccount>,
}

impl From<StatelessInput> for Gaiko2ReplayBlock {
    fn from(value: StatelessInput) -> Self {
        Self {
            block: value.block,
            chain_spec: value.chain_spec,
            witness: value.witness,
            accounts: value.accounts,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Gaiko2AggregateProof {
    pub input: String,
    pub proof: String,
    pub proof_carry_data: ProofCarryData,
}

impl Gaiko2AggregateProof {
    pub fn from_proof(proof: &Proof) -> Result<Self, raiko2_primitives::RaikoError> {
        let input = proof.input.ok_or_else(|| {
            raiko2_primitives::RaikoError::InvalidRequestConfig(
                "gaiko2 aggregation proof missing input".to_string(),
            )
        })?;
        let proof_hex = proof.proof.clone().ok_or_else(|| {
            raiko2_primitives::RaikoError::InvalidRequestConfig(
                "gaiko2 aggregation proof missing proof bytes".to_string(),
            )
        })?;
        let proof_carry_data = proof_carry_from_proof(proof)?.ok_or_else(|| {
            raiko2_primitives::RaikoError::InvalidRequestConfig(
                "gaiko2 aggregation proof missing shasta carry data".to_string(),
            )
        })?;

        Ok(Self {
            input: input.to_string(),
            proof: proof_hex,
            proof_carry_data,
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Gaiko2ProofResponse {
    pub schema: String,
    pub status: Gaiko2ProofStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Gaiko2ProofResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Gaiko2ProofError>,
}

impl Gaiko2ProofResponse {
    #[must_use]
    pub fn success(result: Gaiko2ProofResult) -> Self {
        Self {
            schema: GAIKO2_PROOF_RESPONSE_SCHEMA.to_string(),
            status: Gaiko2ProofStatus::Ok,
            result: Some(result),
            error: None,
        }
    }

    #[must_use]
    pub fn error(error: Gaiko2ProofError) -> Self {
        Self {
            schema: GAIKO2_PROOF_RESPONSE_SCHEMA.to_string(),
            status: Gaiko2ProofStatus::Error,
            result: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Gaiko2ProofStatus {
    #[default]
    Ok,
    Error,
}

impl Gaiko2ProofStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Gaiko2ProofResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proof: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_address: Option<String>,
    pub input: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Gaiko2ProofError {
    pub code: String,
    pub message: String,
}
