//! Input types for raiko2 guest programs.

use crate::{chain_spec::ChainSpec, proof::Proof};
use alloy_consensus::TrieAccount;
use alloy_primitives::{B256, map::AddressMap};
use reth_ethereum_primitives::Block;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

/// Stateless input for a single block.
#[serde_as]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StatelessInput {
    /// The block being executed in the stateless validation function.
    #[serde_as(as = "crate::serde_bincode::EthereumBlock<'_>")]
    pub block: Block,
    /// The network to generate the proof for
    pub chain_spec: ChainSpec,
    /// `ExecutionWitness` for the stateless validation function.
    pub witness: crate::ExecutionWitness,
    /// Host-provided account snapshots, retained only for wire-format compatibility.
    ///
    /// These are never verified against the pre-state root and are not consumed by consensus
    /// validation — they must not be used in guest logic. Execution reads accounts from the
    /// Merkle-verified witness trie instead.
    #[serde_as(as = "crate::serde_bincode::SortedAddressMap")]
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
