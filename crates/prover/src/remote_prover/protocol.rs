use alloy_consensus::TrieAccount;
use alloy_primitives::map::AddressMap;
use raiko2_primitives::{
    ChainSpec, ExecutionWitness, Proof, StatelessInput, WitnessHeader, WitnessStateNode,
};
use raiko2_primitives_shasta::{
    GuestInput, instance::build_shasta_commitment_from_proof_carry_data_vec, proof_carry_from_proof,
};
use raiko2_protocol_shasta::TaikoManifest;
use raiko2_protocol_shasta::{libhash::hash_shasta_subproof_input, shasta::ProofCarryData};
use reth_ethereum_primitives::Block;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

pub const RAIKO2_SHASTA_REQUEST_SCHEMA: &str = "raiko2-shasta-request-v1";
pub const RAIKO2_SHASTA_AGGREGATE_REQUEST_SCHEMA: &str = "raiko2-shasta-aggregate-request-v1";
pub const RAIKO2_PROOF_RESPONSE_SCHEMA: &str = "raiko2-proof-v1";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Raiko2ShastaRequest {
    pub schema: String,
    pub payload: Raiko2ShastaPayload,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Raiko2ShastaPayload {
    pub guest_input: Raiko2ShastaGuestInput,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Raiko2ShastaGuestInput {
    pub witnesses: Vec<Raiko2ReplayBlock>,
    pub taiko: TaikoManifest,
    pub proposal_ancestor_headers: Vec<WitnessHeader>,
    pub proposal_state_nodes: Vec<WitnessStateNode>,
    pub proof_carry_data: ProofCarryData,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Raiko2ShastaAggregateRequest {
    pub schema: String,
    pub payload: Raiko2ShastaAggregatePayload,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Raiko2ShastaAggregatePayload {
    pub proofs: Vec<Raiko2AggregateProof>,
}

#[serde_as]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Raiko2ReplayBlock {
    #[serde_as(as = "raiko2_primitives::EthereumBlock<'_>")]
    pub block: Block,
    pub chain_spec: ChainSpec,
    pub witness: ExecutionWitness,
    pub accounts: AddressMap<TrieAccount>,
}

impl From<StatelessInput> for Raiko2ReplayBlock {
    fn from(value: StatelessInput) -> Self {
        Self {
            block: value.block,
            chain_spec: value.chain_spec,
            witness: value.witness,
            accounts: value.accounts,
        }
    }
}

impl From<Raiko2ReplayBlock> for StatelessInput {
    fn from(value: Raiko2ReplayBlock) -> Self {
        Self {
            block: value.block,
            chain_spec: value.chain_spec,
            witness: value.witness,
            accounts: value.accounts,
        }
    }
}

impl From<Raiko2ShastaGuestInput> for GuestInput {
    fn from(value: Raiko2ShastaGuestInput) -> Self {
        Self {
            witnesses: value.witnesses.into_iter().map(Into::into).collect(),
            taiko: value.taiko,
            proposal_ancestor_headers: value.proposal_ancestor_headers,
            proposal_state_nodes: value.proposal_state_nodes,
            proof_carry_data: value.proof_carry_data,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Raiko2AggregateProof {
    pub input: String,
    pub proof: String,
    pub proof_carry_data: ProofCarryData,
}

impl Raiko2AggregateProof {
    /// # Errors
    ///
    /// Returns an error when the proof is missing remote aggregate fields, missing shasta carry
    /// data, or carries an input hash that does not match the shasta carry data.
    pub fn from_proof(proof: &Proof) -> Result<Self, raiko2_primitives::RaikoError> {
        let proof_hex = proof.proof.clone().ok_or_else(|| {
            raiko2_primitives::RaikoError::InvalidRequestConfig(
                "remote aggregation proof missing proof bytes".to_string(),
            )
        })?;
        let proof_carry_data = proof_carry_from_proof(proof)?.ok_or_else(|| {
            raiko2_primitives::RaikoError::InvalidRequestConfig(
                "remote aggregation proof missing shasta carry data".to_string(),
            )
        })?;
        build_shasta_commitment_from_proof_carry_data_vec(std::slice::from_ref(&proof_carry_data))
            .ok_or_else(|| {
                raiko2_primitives::RaikoError::InvalidRequestConfig(
                    "invalid shasta proof carry data".to_string(),
                )
            })?;
        let expected_input = hash_shasta_subproof_input(&proof_carry_data);
        if let Some(input) = proof.input
            && input != expected_input
        {
            return Err(raiko2_primitives::RaikoError::InvalidRequestConfig(
                "remote aggregation proof input hash does not match shasta carry data".to_string(),
            ));
        }

        Ok(Self {
            input: expected_input.to_string(),
            proof: proof_hex,
            proof_carry_data,
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Raiko2ProofResponse {
    pub schema: String,
    pub status: Raiko2ProofStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Raiko2ProofResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Raiko2ProofError>,
}

impl Raiko2ProofResponse {
    #[must_use]
    pub fn success(result: Raiko2ProofResult) -> Self {
        Self {
            schema: RAIKO2_PROOF_RESPONSE_SCHEMA.to_string(),
            status: Raiko2ProofStatus::Ok,
            result: Some(result),
            error: None,
        }
    }

    #[must_use]
    pub fn error(error: Raiko2ProofError) -> Self {
        Self {
            schema: RAIKO2_PROOF_RESPONSE_SCHEMA.to_string(),
            status: Raiko2ProofStatus::Error,
            result: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Raiko2ProofStatus {
    #[default]
    Ok,
    Error,
}

impl Raiko2ProofStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Raiko2ProofResult {
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
pub struct Raiko2ProofError {
    pub code: String,
    pub message: String,
}
