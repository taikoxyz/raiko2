//! Protocol instance types for raiko2.
//!
//! This module provides types for constructing and verifying protocol instances
//! for the Shasta hardfork. Legacy fork support (Hekla, Ontake, Pacaya) has been
//! removed in V2.

use crate::{GuestInput, input::ShastaRawAggregationGuestInput};
use alloy_primitives::{Address, B256, Uint, keccak256};
use alloy_sol_types::SolValue;
use anyhow::{Context, Result, ensure};
use raiko2_protocol_shasta::TaikoProverData;
use raiko2_protocol_shasta::libhash::{hash_commitment, hash_public_input, hash_two_values};
use raiko2_protocol_shasta::shasta::{Commitment, ProofCarryData, Transition};
use reth_ethereum_primitives::Block;
use serde::{Deserialize, Serialize};
use tracing::debug;

#[must_use]
pub fn words_to_bytes_le(words: &[u32; 8]) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    for i in 0..8 {
        let word_bytes = words[i].to_le_bytes();
        bytes[i * 4..(i + 1) * 4].copy_from_slice(&word_bytes);
    }
    bytes
}

#[must_use]
pub fn words_to_bytes_be(words: &[u32; 8]) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    for i in 0..8 {
        let word_bytes = words[i].to_be_bytes();
        bytes[i * 4..(i + 1) * 4].copy_from_slice(&word_bytes);
    }
    bytes
}

#[allow(dead_code)]
pub(crate) fn aggregation_output_combine(public_inputs: &[B256]) -> Vec<u8> {
    let mut output = Vec::with_capacity(public_inputs.len() * 32);
    for public_input in public_inputs {
        output.extend_from_slice(&public_input.0);
    }
    output
}

#[allow(dead_code)]
pub(crate) fn aggregation_output(program: B256, public_inputs: &[B256]) -> Vec<u8> {
    let mut inputs = Vec::with_capacity(public_inputs.len() + 1);
    inputs.push(program);
    inputs.extend_from_slice(public_inputs);
    aggregation_output_combine(&inputs)
}

#[allow(dead_code)]
pub(crate) fn validate_shasta_aggregate_proof_carry_data(
    aggregation_input: &ShastaRawAggregationGuestInput,
) -> bool {
    // The carry vector is meant to be a per-proof sidecar; treat mismatched sizes as invalid.
    if aggregation_input.proofs.len() != aggregation_input.proof_carry_data_vec.len() {
        return false;
    }
    validate_shasta_proof_carry_data_vec(&aggregation_input.proof_carry_data_vec)
}

pub(crate) fn validate_shasta_proof_carry_data_vec(
    proof_carry_data_vec: &[ProofCarryData],
) -> bool {
    let Some(first) = proof_carry_data_vec.first() else {
        return false;
    };

    let expected_actual_prover = first.transition_input.actual_prover;
    if !proof_carry_data_vec
        .iter()
        .all(|item| item.transition_input.actual_prover == expected_actual_prover)
    {
        return false;
    }

    for w in proof_carry_data_vec.windows(2) {
        let prev = &w[0];
        let next = &w[1];
        // Ensure proposal ids are sequential
        if prev.transition_input.proposal_id + 1 != next.transition_input.proposal_id {
            return false;
        }

        // Ensure proposal hashes chain correctly
        if prev.transition_input.proposal_hash != next.transition_input.parent_proposal_hash {
            return false;
        }

        if prev.chain_id != next.chain_id {
            return false;
        }

        if prev.verifier != next.verifier {
            return false;
        }

        // Continuity: prev checkpoint block hash must match next parent block hash.
        if prev.transition_input.checkpoint.blockHash != next.transition_input.parent_block_hash {
            return false;
        }
    }

    true
}

#[must_use]
pub fn build_shasta_commitment_from_proof_carry_data_vec(
    proof_carry_data_vec: &[ProofCarryData],
) -> Option<Commitment> {
    if !validate_shasta_proof_carry_data_vec(proof_carry_data_vec) {
        return None;
    }
    let first = proof_carry_data_vec.first()?;
    let last = proof_carry_data_vec.last()?;

    let transitions: Vec<Transition> = proof_carry_data_vec
        .iter()
        .map(|item| Transition {
            proposer: item.transition_input.transition.proposer,
            timestamp: Uint::from(item.transition_input.transition.timestamp),
            blockHash: item.transition_input.checkpoint.blockHash,
        })
        .collect();

    Some(Commitment {
        firstProposalId: Uint::from(first.transition_input.proposal_id),
        // This field is the parent block hash in the latest Shasta contract; we store it as bytes32.
        firstProposalParentBlockHash: first.transition_input.parent_block_hash,
        lastProposalHash: last.transition_input.proposal_hash,
        actualProver: first.transition_input.actual_prover,
        endBlockNumber: last.transition_input.checkpoint.blockNumber,
        endStateRoot: last.transition_input.checkpoint.stateRoot,
        transitions,
    })
}

#[must_use]
pub fn shasta_zk_aggregation_public_input_from_proof_carry_data_vec(
    sub_image_id: B256,
    proof_carry_data_vec: &[ProofCarryData],
    prover_address: Address,
) -> Option<B256> {
    let commitment = build_shasta_commitment_from_proof_carry_data_vec(proof_carry_data_vec)?;
    let first = proof_carry_data_vec.first()?;
    let aggregation_hash =
        shasta_aggregation_output(&commitment, first.chain_id, first.verifier, prover_address);
    Some(shasta_zk_aggregation_output(sub_image_id, aggregation_hash))
}

#[must_use]
pub fn shasta_aggregation_output(
    prove_input: &Commitment,
    chain_id: u64,
    verifier_address: Address,
    sgx_instance: Address,
) -> B256 {
    let prove_input_hash = hash_commitment(prove_input);
    hash_public_input(prove_input_hash, chain_id, verifier_address, sgx_instance)
}

#[must_use]
pub fn shasta_zk_aggregation_output(sub_image_id: B256, sub_input_hash: B256) -> B256 {
    hash_two_values(sub_image_id, sub_input_hash)
}

/// Transition data for Shasta.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ShastaTransition {
    pub parent_hash: B256,
    pub block_hash: B256,
    pub state_root: B256,
}

/// Proposal metadata for Shasta.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ShastaProposalMetadata {
    pub info_hash: B256,
    pub proposer: Address,
    pub proposal_id: u64,
    pub proposed_at: u64,
}

/// Protocol instance for Shasta.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProtocolInstance {
    pub transition: ShastaTransition,
    pub proposal_metadata: ShastaProposalMetadata,
    pub prover: Address,
    pub chain_id: u64,
    pub verifier_address: Address,
}

impl ProtocolInstance {
    /// Calculate the instance hash for the protocol instance.
    #[must_use]
    pub fn instance_hash(&self) -> B256 {
        let data = (
            self.transition.parent_hash,
            self.transition.block_hash,
            self.transition.state_root,
            self.proposal_metadata.info_hash,
            self.proposal_metadata.proposer,
            self.proposal_metadata.proposal_id,
            self.prover,
            self.chain_id,
        )
            .abi_encode();
        keccak256(data)
    }
}

/// Calculate the txs hash for Shasta.
#[allow(dead_code)]
pub(crate) fn calculate_txs_hash(tx_list_hash: B256, blob_hashes: &[B256]) -> B256 {
    debug!(
        "calculate_txs_hash from tx_list_hash: {:?}, blob_hashes: {:?}",
        tx_list_hash, blob_hashes
    );

    let abi_encode_data: Vec<u8> = (tx_list_hash, blob_hashes.iter().collect::<Vec<_>>())
        .abi_encode()
        .split_off(32);
    debug!("abi_encode_data: {:?}", hex::encode(&abi_encode_data));
    keccak256(abi_encode_data)
}

/// Create a protocol instance from proposal input and executed blocks.
#[allow(dead_code)]
pub(crate) fn new_protocol_instance(
    proposal_input: &GuestInput,
    blocks: &[Block],
    prover_data: &TaikoProverData,
    chain_id: u64,
    verifier_address: Address,
) -> Result<ProtocolInstance> {
    ensure!(!blocks.is_empty(), "blocks cannot be empty");

    let first_block = blocks.first().context("blocks cannot be empty")?;
    let last_block = blocks.last().context("blocks cannot be empty")?;

    let transition = ShastaTransition {
        parent_hash: first_block.header.parent_hash,
        block_hash: last_block.header.hash_slow(),
        state_root: last_block.header.state_root,
    };

    // Calculate proposal metadata
    let tx_list_hash = keccak256(&proposal_input.taiko.data_sources[0].tx_data_from_calldata);

    // TODO: Get blob hashes from proposal_event.
    let blob_hashes: Vec<B256> = vec![];
    let txs_hash = calculate_txs_hash(tx_list_hash, &blob_hashes);

    let proposal_metadata = ShastaProposalMetadata {
        info_hash: txs_hash, // Simplified for now
        proposer: Address::default(),
        proposal_id: proposal_input.taiko.proposal_id,
        proposed_at: 0,
    };

    Ok(ProtocolInstance {
        transition,
        proposal_metadata,
        prover: prover_data.actual_prover,
        chain_id,
        verifier_address,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_instance_hash() {
        let instance = ProtocolInstance::default();
        let hash = instance.instance_hash();
        assert_ne!(hash, B256::default());
    }

    #[test]
    fn test_calculate_txs_hash() {
        let tx_list_hash = B256::default();
        let blob_hashes = vec![B256::default()];
        let hash = calculate_txs_hash(tx_list_hash, &blob_hashes);
        assert_ne!(hash, B256::default());
    }
}
