//! Protocol instance types for raiko2.
//!
//! This module provides types for constructing and verifying protocol instances
//! for the Shasta hardfork. Legacy fork support (Hekla, Ontake, Pacaya) has been
//! removed in V2.

#[cfg(test)]
use crate::input::ShastaRawAggregationGuestInput;
use alloy_primitives::{Address, B256, Uint, keccak256};
use alloy_sol_types::SolValue;
use raiko2_protocol_shasta::libhash::{hash_commitment, hash_public_input, hash_two_values};
use raiko2_protocol_shasta::shasta::{Commitment, ProofCarryData, Transition};
use serde::{Deserialize, Serialize};

pub const SHASTA_PROPOSAL_ID_MAX: u64 = (1_u64 << 48) - 1;

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

#[cfg(test)]
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

    if !proof_carry_data_vec
        .iter()
        .all(|item| item.transition_input.proposal_id <= SHASTA_PROPOSAL_ID_MAX)
    {
        return false;
    }

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
        if prev
            .transition_input
            .proposal_id
            .checked_add(1)
            .is_none_or(|expected| expected != next.transition_input.proposal_id)
        {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ShastaRawAggregationGuestInput;
    use alloy_primitives::{Address, address, b256};
    use raiko2_primitives::RawProof;
    use raiko2_protocol_shasta::shasta::{
        Checkpoint, ProofCarryData, ShastaTransitionInput, TransitionInputData,
    };

    #[test]
    fn test_instance_hash() {
        let instance = ProtocolInstance::default();
        let hash = instance.instance_hash();
        assert_ne!(hash, B256::default());
    }

    fn sample_checkpoint(block_number: u64, block_hash: B256, state_root: B256) -> Checkpoint {
        Checkpoint {
            blockNumber: Uint::from(block_number),
            blockHash: block_hash,
            stateRoot: state_root,
        }
    }

    fn sample_carry(
        proposal_id: u64,
        proposal_hash: B256,
        parent_proposal_hash: B256,
        parent_block_hash: B256,
        checkpoint: Checkpoint,
    ) -> ProofCarryData {
        ProofCarryData {
            chain_id: 167_000,
            verifier: address!("00000000000000000000000000000000000000aa"),
            transition_input: TransitionInputData {
                proposal_id,
                proposal_hash,
                parent_proposal_hash,
                parent_block_hash,
                actual_prover: address!("00000000000000000000000000000000000000bb"),
                transition: ShastaTransitionInput {
                    proposer: address!("00000000000000000000000000000000000000cc"),
                    timestamp: 100 + proposal_id,
                },
                checkpoint,
            },
        }
    }

    fn sample_carry_sequence() -> Vec<ProofCarryData> {
        let first_hash = b256!("1111111111111111111111111111111111111111111111111111111111111111");
        let second_hash = b256!("2222222222222222222222222222222222222222222222222222222222222222");
        let parent_block_hash =
            b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let checkpoint0 = sample_checkpoint(
            10,
            b256!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            b256!("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"),
        );
        let checkpoint1 = sample_checkpoint(
            11,
            b256!("dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"),
            b256!("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"),
        );

        vec![
            sample_carry(
                1,
                first_hash,
                B256::ZERO,
                parent_block_hash,
                checkpoint0.clone(),
            ),
            sample_carry(
                2,
                second_hash,
                first_hash,
                checkpoint0.blockHash,
                checkpoint1,
            ),
        ]
    }

    fn sample_raw_proofs() -> Vec<RawProof> {
        vec![
            RawProof {
                proof: vec![0u8],
                input: B256::ZERO,
            },
            RawProof {
                proof: vec![1u8],
                input: B256::ZERO,
            },
        ]
    }

    #[test]
    fn validate_shasta_aggregate_proof_carry_data_accepts_contiguous_sequence() {
        let input = ShastaRawAggregationGuestInput {
            proofs: sample_raw_proofs(),
            proof_carry_data_vec: sample_carry_sequence(),
        };

        assert!(validate_shasta_aggregate_proof_carry_data(&input));
    }

    #[test]
    fn validate_shasta_aggregate_proof_carry_data_rejects_mismatched_lengths() {
        let input = ShastaRawAggregationGuestInput {
            proofs: sample_raw_proofs(),
            proof_carry_data_vec: vec![],
        };

        assert!(!validate_shasta_aggregate_proof_carry_data(&input));
    }

    #[test]
    fn validate_shasta_proof_carry_data_vec_rejects_mismatched_actual_prover() {
        let mut proof_carry_data_vec = sample_carry_sequence();
        proof_carry_data_vec[1].transition_input.actual_prover = Address::from([0x22; 20]);

        assert!(!validate_shasta_proof_carry_data_vec(&proof_carry_data_vec));
    }

    #[test]
    fn validate_shasta_proof_carry_data_vec_rejects_non_sequential_ids() {
        let mut proof_carry_data_vec = sample_carry_sequence();
        proof_carry_data_vec[1].transition_input.proposal_id += 1;

        assert!(!validate_shasta_proof_carry_data_vec(&proof_carry_data_vec));
    }

    #[test]
    fn validate_shasta_proof_carry_data_vec_rejects_wrapping_ids() {
        let mut proof_carry_data_vec = sample_carry_sequence();
        proof_carry_data_vec[0].transition_input.proposal_id = u64::MAX;
        proof_carry_data_vec[1].transition_input.proposal_id = 0;

        assert!(!validate_shasta_proof_carry_data_vec(&proof_carry_data_vec));
    }

    #[test]
    fn validate_shasta_proof_carry_data_vec_rejects_ids_outside_uint48() {
        let mut proof_carry_data_vec = sample_carry_sequence();
        proof_carry_data_vec[0].transition_input.proposal_id = SHASTA_PROPOSAL_ID_MAX;
        proof_carry_data_vec[1].transition_input.proposal_id = SHASTA_PROPOSAL_ID_MAX + 1;

        assert!(!validate_shasta_proof_carry_data_vec(&proof_carry_data_vec));
    }

    #[test]
    fn validate_shasta_proof_carry_data_vec_rejects_broken_proposal_hash_chain() {
        let mut proof_carry_data_vec = sample_carry_sequence();
        proof_carry_data_vec[1]
            .transition_input
            .parent_proposal_hash =
            b256!("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");

        assert!(!validate_shasta_proof_carry_data_vec(&proof_carry_data_vec));
    }

    #[test]
    fn validate_shasta_proof_carry_data_vec_rejects_chain_id_mismatch() {
        let mut proof_carry_data_vec = sample_carry_sequence();
        proof_carry_data_vec[1].chain_id += 1;

        assert!(!validate_shasta_proof_carry_data_vec(&proof_carry_data_vec));
    }

    #[test]
    fn validate_shasta_proof_carry_data_vec_rejects_verifier_mismatch() {
        let mut proof_carry_data_vec = sample_carry_sequence();
        proof_carry_data_vec[1].verifier = Address::from([0x33; 20]);

        assert!(!validate_shasta_proof_carry_data_vec(&proof_carry_data_vec));
    }

    #[test]
    fn validate_shasta_proof_carry_data_vec_rejects_broken_parent_block_hash_chain() {
        let mut proof_carry_data_vec = sample_carry_sequence();
        proof_carry_data_vec[1].transition_input.parent_block_hash =
            b256!("9999999999999999999999999999999999999999999999999999999999999999");

        assert!(!validate_shasta_proof_carry_data_vec(&proof_carry_data_vec));
    }

    #[test]
    fn build_shasta_commitment_uses_first_and_last_entries() {
        let proof_carry_data_vec = sample_carry_sequence();
        let first = proof_carry_data_vec.first().expect("first entry");
        let last = proof_carry_data_vec.last().expect("last entry");

        let commitment = build_shasta_commitment_from_proof_carry_data_vec(&proof_carry_data_vec)
            .expect("commitment should build");

        assert_eq!(
            commitment.firstProposalId.to::<u64>(),
            first.transition_input.proposal_id
        );
        assert_eq!(
            commitment.firstProposalParentBlockHash,
            first.transition_input.parent_block_hash
        );
        assert_eq!(
            commitment.lastProposalHash,
            last.transition_input.proposal_hash
        );
        assert_eq!(
            commitment.actualProver,
            first.transition_input.actual_prover
        );
        assert_eq!(
            commitment.endBlockNumber.to::<u64>(),
            last.transition_input.checkpoint.blockNumber.to::<u64>()
        );
        assert_eq!(
            commitment.endStateRoot,
            last.transition_input.checkpoint.stateRoot
        );
        assert_eq!(commitment.transitions.len(), proof_carry_data_vec.len());
        assert_eq!(
            commitment.transitions[0].blockHash,
            first.transition_input.checkpoint.blockHash
        );
        assert_eq!(
            commitment.transitions[1].blockHash,
            last.transition_input.checkpoint.blockHash
        );
    }
}
