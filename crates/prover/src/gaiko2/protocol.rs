use alloy_consensus::TrieAccount;
use alloy_primitives::map::AddressMap;
use raiko2_primitives::{ChainSpec, ExecutionWitness, StatelessInput};
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
