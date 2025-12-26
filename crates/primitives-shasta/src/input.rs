//! Shasta input types for guest programs.

use alloy_primitives::{Address, B256};
use raiko2_primitives::{RawProof, StatelessInput};
use raiko2_protocol_shasta::TaikoManifest;
use raiko2_protocol_shasta::shasta::ProofCarryData;
use serde::{Deserialize, Serialize};

/// Shasta guest program input.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GuestInput {
    /// The witnesses for each block.
    pub witnesses: Vec<StatelessInput>,
    /// The Taiko manifest.
    pub taiko: TaikoManifest,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ShastaRawAggregationGuestInput {
    /// All block proofs to prove
    pub proofs: Vec<RawProof>,
    pub proof_carry_data_vec: Vec<ProofCarryData>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ShastaZkAggregationGuestInput {
    /// Verifier image id for the SP1 proofs being aggregated
    pub image_id: [u32; 8],
    /// Public inputs associated with each underlying proof
    pub block_inputs: Vec<B256>,
    /// Proof carry data associated with each underlying proof
    pub proof_carry_data_vec: Vec<ProofCarryData>,
    /// Address representing the prover/aggregator (zero for zk provers today)
    pub prover_address: Address,
}
