//! Input types for raiko2 guest programs.

use alloy_consensus::TrieAccount;
use alloy_primitives::{B256, map::AddressMap};
use reth_ethereum_primitives::Block;
use reth_stateless::ExecutionWitness;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use crate::proof::Proof;

// Re-export Taiko-specific types from protocol crate
// This maintains backward compatibility while moving the canonical definitions to protocol
pub use raiko2_protocol::{BlobProofType, TaikoManifest, TaikoProverData};

/// Guest program input.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GuestInput {
    /// The witnesses for each block.
    pub witnesses: Vec<StatelessInput>,
    /// The Taiko manifest.
    pub taiko: TaikoManifest,
}

/// Stateless input for a single block.
#[serde_as]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StatelessInput {
    /// The block being executed in the stateless validation function.
    #[serde_as(
        as = "reth_primitives_traits::serde_bincode_compat::Block<reth_ethereum_primitives::TransactionSigned, alloy_consensus::Header>"
    )]
    pub block: Block,
    /// `ExecutionWitness` for the stateless validation function.
    pub witness: ExecutionWitness,
    /// The accounts being accessed in the stateless validation function.
    pub accounts: AddressMap<TrieAccount>,
}

/// External aggregation input.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AggregationGuestInput {
    /// All block proofs to prove.
    pub proofs: Vec<Proof>,
}

/// The raw proof data necessary to verify a proof.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct RawProof {
    /// The actual proof.
    pub proof: Vec<u8>,
    /// The resulting hash.
    pub input: B256,
}

/// External aggregation input with raw proofs.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct RawAggregationGuestInput {
    /// All block proofs to prove.
    pub proofs: Vec<RawProof>,
}

/// ZK aggregation guest input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZkAggregationGuestInput {
    pub image_id: [u32; 8],
    pub block_inputs: Vec<B256>,
}
