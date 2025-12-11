//! Input types for raiko2 guest programs.

use alloy_consensus::{Header, TrieAccount};
use alloy_primitives::{Address, B256, map::AddressMap};
use anyhow::{Error, anyhow};
use core::str::FromStr;
use raiko2_protocol::{Checkpoint, ShastaEventData};
use reth_ethereum_primitives::Block;
use reth_stateless::ExecutionWitness;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use crate::{chain_spec::ChainSpec, proof::Proof};

/// Blob proof type for Taiko.
#[derive(Clone, Debug, Serialize, Deserialize, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BlobProofType {
    /// Guest runs through the entire computation from blob to Kzg commitment
    /// then to version hash.
    #[default]
    KzgVersionedHash,
    /// Simplified Proof of Equivalence with fiat input in non-aligned field.
    ProofOfEquivalence,
}

impl FromStr for BlobProofType {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "proof_of_equivalence" => Ok(BlobProofType::ProofOfEquivalence),
            "kzg_versioned_hash" => Ok(BlobProofType::KzgVersionedHash),
            _ => Err(anyhow!("invalid blob proof type")),
        }
    }
}

/// Taiko prover data.
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct TaikoProverData {
    pub actual_prover: Address,
    pub designated_prover: Option<Address>,
    pub graffiti: B256,
    pub parent_transition_hash: Option<B256>,
    pub checkpoint: Option<Checkpoint>,
    pub last_anchor_block_number: Option<u64>,
}

/// Taiko batch input for guest programs.
#[serde_as]
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct TaikoManifest {
    pub batch_id: u64,
    pub l1_header: Header,
    pub batch_proposed: ShastaEventData,
    pub chain_spec: ChainSpec,
    pub prover_data: TaikoProverData,
    pub data_sources: Vec<InputDataSource>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct InputDataSource {
    pub tx_data_from_calldata: Vec<u8>,
    pub tx_data_from_blob: Vec<Vec<u8>>,
    pub blob_commitments: Option<Vec<Vec<u8>>>,
    pub blob_proofs: Option<Vec<Vec<u8>>>,
    pub blob_proof_type: BlobProofType,
    pub is_forced_inclusion: bool,
}

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
