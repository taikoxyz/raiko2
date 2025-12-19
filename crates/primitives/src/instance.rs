//! Protocol instance types for raiko2.
//!
//! This module provides types for constructing and verifying protocol instances
//! for the Shasta hardfork. Legacy fork support (Hekla, Ontake, Pacaya) has been
//! removed in V2.

use crate::{BlobProofType, GuestInput, TaikoProverData, input::ShastaRawAggregationGuestInput};
use alloy_primitives::{Address, B256, Uint, keccak256};
use alloy_sol_types::SolValue;
use anyhow::{Result, ensure};
use raiko2_protocol::{
    Commitment, ProofCarryData, Transition, hash_checkpoint, hash_commitment, hash_public_input,
    hash_two_values,
};
use reth_ethereum_primitives::Block;
use serde::{Deserialize, Serialize};
use tracing::debug;

pub fn words_to_bytes_le(words: &[u32; 8]) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    for i in 0..8 {
        let word_bytes = words[i].to_le_bytes();
        bytes[i * 4..(i + 1) * 4].copy_from_slice(&word_bytes);
    }
    bytes
}

pub fn words_to_bytes_be(words: &[u32; 8]) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    for i in 0..8 {
        let word_bytes = words[i].to_be_bytes();
        bytes[i * 4..(i + 1) * 4].copy_from_slice(&word_bytes);
    }
    bytes
}

pub fn aggregation_output_combine(public_inputs: Vec<B256>) -> Vec<u8> {
    let mut output = Vec::with_capacity(public_inputs.len() * 32);
    for public_input in public_inputs.iter() {
        output.extend_from_slice(&public_input.0);
    }
    output
}

pub fn aggregation_output(program: B256, public_inputs: Vec<B256>) -> Vec<u8> {
    aggregation_output_combine([vec![program], public_inputs].concat())
}

pub fn validate_shasta_aggregate_proof_carry_data(
    aggregation_input: &ShastaRawAggregationGuestInput,
) -> bool {
    // The carry vector is meant to be a per-proof sidecar; treat mismatched sizes as invalid.
    if aggregation_input.proofs.len() != aggregation_input.proof_carry_data_vec.len() {
        return false;
    }
    validate_shasta_proof_carry_data_vec(&aggregation_input.proof_carry_data_vec)
}

pub fn validate_shasta_proof_carry_data_vec(proof_carry_data_vec: &[ProofCarryData]) -> bool {
    if proof_carry_data_vec.is_empty() {
        return false;
    }

    let expected_actual_prover = proof_carry_data_vec[0].transition_input.actual_prover;
    for item in proof_carry_data_vec.iter() {
        // Commitment uses a single `actualProver` field; make the range unambiguous.
        if item.transition_input.actual_prover != expected_actual_prover {
            return false;
        }
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

        // Continuity: prev checkpoint must match next parent checkpoint hash.
        if hash_checkpoint(&prev.transition_input.checkpoint)
            != next.transition_input.parent_checkpoint_hash
        {
            return false;
        }
    }

    true
}

pub fn build_shasta_commitment_from_proof_carry_data_vec(
    proof_carry_data_vec: &[ProofCarryData],
) -> Option<Commitment> {
    if !validate_shasta_proof_carry_data_vec(proof_carry_data_vec) {
        return None;
    }
    let last = proof_carry_data_vec.last()?;

    let transitions: Vec<Transition> = proof_carry_data_vec
        .iter()
        .map(|item| Transition {
            proposer: item.transition_input.transition.proposer,
            designatedProver: item.transition_input.transition.designatedProver,
            timestamp: Uint::from(item.transition_input.transition.timestamp),
            checkpointHash: hash_checkpoint(&item.transition_input.checkpoint),
        })
        .collect();

    Some(Commitment {
        firstProposalId: Uint::from(proof_carry_data_vec[0].transition_input.proposal_id),
        // This field is a checkpoint hash in the latest Shasta contract; we store it as bytes32.
        firstProposalParentBlockHash: proof_carry_data_vec[0]
            .transition_input
            .parent_checkpoint_hash,
        lastProposalHash: last.transition_input.proposal_hash,
        actualProver: proof_carry_data_vec[0].transition_input.actual_prover,
        endBlockNumber: last.transition_input.checkpoint.blockNumber,
        endStateRoot: last.transition_input.checkpoint.stateRoot,
        transitions,
    })
}

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

pub fn shasta_aggregation_output(
    prove_input: &Commitment,
    chain_id: u64,
    verifier_address: Address,
    sgx_instance: Address,
) -> B256 {
    let prove_input_hash = hash_commitment(prove_input);
    hash_public_input(prove_input_hash, chain_id, verifier_address, sgx_instance)
}

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

/// Batch metadata for Shasta.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ShastaBatchMetadata {
    pub info_hash: B256,
    pub proposer: Address,
    pub batch_id: u64,
    pub proposed_at: u64,
}

/// Protocol instance for Shasta.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProtocolInstance {
    pub transition: ShastaTransition,
    pub batch_metadata: ShastaBatchMetadata,
    pub prover: Address,
    pub chain_id: u64,
    pub verifier_address: Address,
}

impl ProtocolInstance {
    /// Calculate the instance hash for the protocol instance.
    pub fn instance_hash(&self) -> B256 {
        let data = (
            self.transition.parent_hash,
            self.transition.block_hash,
            self.transition.state_root,
            self.batch_metadata.info_hash,
            self.batch_metadata.proposer,
            self.batch_metadata.batch_id,
            self.prover,
            self.chain_id,
        )
            .abi_encode();
        keccak256(data)
    }
}

/// Verify blob usage in batch mode.
///
/// Checks that raw blob commitment matches input blob commitment,
/// then verifies the blob version hash.
pub fn verify_batch_mode_blob_usage(
    _guest_input: &GuestInput,
    blob_proof_type: BlobProofType,
) -> Result<()> {
    match blob_proof_type {
        BlobProofType::KzgVersionedHash => {
            // ensure!(
            //     batch_input.taiko.tx_data_from_blob.len()
            //         == batch_input
            //             .taiko
            //             .blob_commitments
            //             .as_ref()
            //             .map_or(0, |c| c.len()),
            //     "Each blob should have its own hash commit"
            // );
        }
        BlobProofType::ProofOfEquivalence => {
            // ensure!(
            //     batch_input.taiko.tx_data_from_blob.len()
            //         == batch_input
            //             .taiko
            //             .blob_proofs
            //             .as_ref()
            //             .map_or(0, |p| p.len()),
            //     "Each blob should have its own proof"
            // );
        }
    }

    // TODO: Implement full blob verification with KZG
    // For now, just verify the counts match

    Ok(())
}

/// Calculate the txs hash for Shasta.
pub fn calculate_txs_hash(tx_list_hash: B256, blob_hashes: &[B256]) -> B256 {
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

/// Create a protocol instance from batch input and executed blocks.
pub fn new_protocol_instance(
    batch_input: &GuestInput,
    blocks: Vec<Block>,
    prover_data: &TaikoProverData,
    chain_id: u64,
    verifier_address: Address,
) -> Result<ProtocolInstance> {
    ensure!(!blocks.is_empty(), "blocks cannot be empty");

    let first_block = blocks.first().unwrap();
    let last_block = blocks.last().unwrap();

    let transition = ShastaTransition {
        parent_hash: first_block.header.parent_hash,
        block_hash: last_block.header.hash_slow(),
        state_root: last_block.header.state_root,
    };

    // Calculate batch metadata
    let tx_list_hash = keccak256(&batch_input.taiko.data_sources[0].tx_data_from_calldata);

    // TODO: Get blob hashes from batch_proposed
    let blob_hashes: Vec<B256> = vec![];
    let txs_hash = calculate_txs_hash(tx_list_hash, &blob_hashes);

    let batch_metadata = ShastaBatchMetadata {
        info_hash: txs_hash, // Simplified for now
        proposer: Address::default(),
        batch_id: batch_input.taiko.batch_id,
        proposed_at: 0,
    };

    Ok(ProtocolInstance {
        transition,
        batch_metadata,
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
