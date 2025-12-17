//! Proof types for raiko2.

use alloy_primitives::{B256, ChainId};
use raiko2_protocol::ProofCarryData;
use serde::{Deserialize, Serialize};

/// Prover error types.
#[derive(thiserror::Error, Debug)]
pub enum ProverError {
    #[error("ProverError::GuestError `{0}`")]
    GuestError(String),
    #[error("ProverError::FileIo `{0}`")]
    FileIo(#[from] std::io::Error),
    #[error("ProverError::Param `{0}`")]
    Param(#[from] serde_json::Error),
    #[error("Store error `{0}`")]
    StoreError(String),
}

impl From<String> for ProverError {
    fn from(e: String) -> Self {
        ProverError::GuestError(e)
    }
}

/// Result type for prover operations.
pub type ProverResult<T, E = ProverError> = core::result::Result<T, E>;

/// Prover configuration (JSON value for flexibility).
pub type ProverConfig = serde_json::Value;

/// Key for identifying a proof: (chain_id, block_number, block_hash, proof_type).
pub type ProofKey = (ChainId, u64, B256, u8);

/// The response body of a proof request.
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Proof {
    /// The proof either TEE or ZK.
    pub proof: Option<String>,
    /// The public input.
    pub input: Option<B256>,
    /// The TEE quote.
    pub quote: Option<String>,
    /// The assumption UUID.
    pub uuid: Option<String>,
    /// The kzg proof.
    pub kzg_proof: Option<String>,
    /// the extra data of Proof
    pub extra_data: Option<ProofCarryData>,
}

impl std::fmt::Display for Proof {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Proof {{ proof: {:?}, input: {:?}, uuid: {:?} }}",
            self.proof
                .as_ref()
                .map(|p| format!("{}...", &p[..std::cmp::min(20, p.len())])),
            self.input,
            self.uuid
        )
    }
}

/// Trait for storing proof IDs.
#[async_trait::async_trait]
pub trait IdWrite: Send {
    async fn store_id(&mut self, key: ProofKey, id: String) -> ProverResult<()>;
    async fn remove_id(&mut self, key: ProofKey) -> ProverResult<()>;
}

/// Trait for reading proof IDs.
#[async_trait::async_trait]
pub trait IdStore: IdWrite {
    async fn read_id(&mut self, key: ProofKey) -> ProverResult<String>;
}
