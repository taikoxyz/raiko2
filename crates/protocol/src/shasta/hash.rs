//! Shasta protocol hashing functions.
//!
//! Rust implementation of taiko-mono/packages/protocol/contracts/layer1/shasta/libs/LibHashing.sol

use alloy_primitives::{keccak256, Address, B256, U256};

use super::types::{
    Checkpoint, CoreState, Derivation, DerivationSource, Proposal, Transition, TransitionMetadata,
};

/// Empty bytes hash (keccak256 of empty bytes).
pub const EMPTY_BYTES_HASH: B256 = B256::new([
    0xc5, 0xd2, 0x46, 0x01, 0x86, 0xf7, 0x23, 0x3c, 0x92, 0x7e, 0x7d, 0xb2, 0xdc, 0xc7, 0x03, 0xc0,
    0xe5, 0x00, 0xb6, 0x53, 0xca, 0x82, 0x27, 0x3b, 0x7b, 0xfa, 0xd8, 0x04, 0x5d, 0x85, 0xa4, 0x70,
]);

/// VERIFY_PROOF constant as B256.
pub const VERIFY_PROOF_B256: B256 = B256::new([
    0x56, 0x45, 0x52, 0x49, 0x46, 0x59, 0x5f, 0x50, 0x52, 0x4f, 0x4f, 0x46, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
]);

/// Convert an Address to B256 by zero-padding (equivalent to bytes32(uint256(uint160(address)))).
#[inline]
pub fn address_to_b256(address: Address) -> B256 {
    B256::left_padding_from(address.as_slice())
}

/// Hash multiple B256 values (equivalent to EfficientHashLib.hash).
fn hash_values(values: &[B256]) -> B256 {
    let mut data = Vec::with_capacity(values.len() * 32);
    for v in values {
        data.extend_from_slice(v.as_slice());
    }
    keccak256(&data)
}

/// Returns `keccak256(abi.encode(value0, value1))`.
#[inline]
pub fn hash_two_values(value0: B256, value1: B256) -> B256 {
    hash_values(&[value0, value1])
}

/// Returns `keccak256(abi.encode(value0, value1, value2))`.
#[inline]
pub fn hash_three_values(value0: B256, value1: B256, value2: B256) -> B256 {
    hash_values(&[value0, value1, value2])
}

/// Returns `keccak256(abi.encode(value0, value1, value2, value3))`.
#[inline]
pub fn hash_four_values(value0: B256, value1: B256, value2: B256, value3: B256) -> B256 {
    hash_values(&[value0, value1, value2, value3])
}

/// Returns `keccak256(abi.encode(value0, value1, value2, value3, value4))`.
#[inline]
pub fn hash_five_values(
    value0: B256,
    value1: B256,
    value2: B256,
    value3: B256,
    value4: B256,
) -> B256 {
    hash_values(&[value0, value1, value2, value3, value4])
}

/// Returns `keccak256(abi.encode(value0, value1, value2, value3, value4, value5))`.
#[inline]
pub fn hash_six_values(
    value0: B256,
    value1: B256,
    value2: B256,
    value3: B256,
    value4: B256,
    value5: B256,
) -> B256 {
    hash_values(&[value0, value1, value2, value3, value4, value5])
}

/// Hash a checkpoint.
pub fn hash_checkpoint(checkpoint: &Checkpoint) -> B256 {
    hash_three_values(
        U256::from(checkpoint.block_number).into(),
        checkpoint.block_hash,
        checkpoint.state_root,
    )
}

/// Hash a transition with metadata.
pub fn hash_transition_with_metadata(
    transition: &Transition,
    metadata: &TransitionMetadata,
) -> B256 {
    let designated_prover_b256 = address_to_b256(metadata.designated_prover);
    let prover_b256 = address_to_b256(metadata.actual_prover);
    hash_five_values(
        transition.proposal_hash,
        transition.parent_transition_hash,
        hash_checkpoint(&transition.checkpoint),
        designated_prover_b256,
        prover_b256,
    )
}

/// Hash an array of transition hashes with length prefix.
pub fn hash_transitions_hash_array(transitions: &[B256]) -> B256 {
    if transitions.is_empty() {
        return EMPTY_BYTES_HASH;
    }

    if transitions.len() == 1 {
        return hash_two_values(U256::from(transitions.len()).into(), transitions[0]);
    }

    if transitions.len() == 2 {
        return hash_three_values(
            U256::from(transitions.len()).into(),
            transitions[0],
            transitions[1],
        );
    }

    // For larger arrays, use memory-optimized approach
    let array_length = transitions.len();
    let buffer_size = 32 + (array_length * 32);
    let mut buffer = Vec::with_capacity(buffer_size);

    buffer.extend_from_slice(&U256::from(array_length).to_be_bytes::<32>());
    for transition in transitions {
        buffer.extend_from_slice(transition.as_slice());
    }

    keccak256(&buffer)
}

/// Hash a proposal.
pub fn hash_proposal(proposal: &Proposal) -> B256 {
    // Pack the fields as in Solidity.
    let packed: U256 = (U256::from(proposal.id) << 208)
        | (U256::from(proposal.timestamp) << 160)
        | (U256::from(proposal.end_of_submission_window_timestamp) << 112);

    let proposer_b256 = address_to_b256(proposal.proposer);

    hash_four_values(
        packed.into(),
        proposer_b256,
        proposal.core_state_hash,
        proposal.derivation_hash,
    )
}

/// Hash a core state.
pub fn hash_core_state(core_state: &CoreState) -> B256 {
    hash_six_values(
        U256::from(core_state.next_proposal_id).into(),
        U256::from(core_state.last_proposal_block_id).into(),
        U256::from(core_state.last_finalized_proposal_id).into(),
        U256::from(core_state.last_checkpoint_timestamp).into(),
        core_state.last_finalized_transition_hash,
        core_state.bond_instructions_hash,
    )
}

/// Hash a blob slice.
pub fn hash_blob_slice(blob_slice: &super::types::BlobSlice) -> B256 {
    // Hash the blob hashes array first
    let blob_hashes_hash = if blob_slice.blob_hashes.is_empty() {
        EMPTY_BYTES_HASH
    } else if blob_slice.blob_hashes.len() == 1 {
        hash_two_values(
            U256::from(blob_slice.blob_hashes.len()).into(),
            blob_slice.blob_hashes[0],
        )
    } else if blob_slice.blob_hashes.len() == 2 {
        hash_three_values(
            U256::from(blob_slice.blob_hashes.len()).into(),
            blob_slice.blob_hashes[0],
            blob_slice.blob_hashes[1],
        )
    } else {
        let array_length = blob_slice.blob_hashes.len();
        let buffer_size = 32 + (array_length * 32);
        let mut buffer = Vec::with_capacity(buffer_size);

        buffer.extend_from_slice(&U256::from(array_length).to_be_bytes::<32>());
        for blob_hash in &blob_slice.blob_hashes {
            buffer.extend_from_slice(blob_hash.as_slice());
        }

        keccak256(&buffer)
    };

    hash_three_values(
        blob_hashes_hash,
        U256::from(blob_slice.offset).into(),
        U256::from(blob_slice.timestamp).into(),
    )
}

/// Hash a derivation source.
pub fn hash_derivation_source(source: &DerivationSource) -> B256 {
    hash_two_values(
        if source.is_forced_inclusion {
            B256::from([1u8; 32])
        } else {
            B256::ZERO
        },
        hash_blob_slice(&source.blob_slice),
    )
}

/// Pack derivation fields for hashing.
fn pack_derivation_fields(derivation: &Derivation) -> B256 {
    let mut packed = [0u8; 32];
    let origin_block_number_bytes = derivation.origin_block_number.to_be_bytes();
    packed[0..6].copy_from_slice(&origin_block_number_bytes[2..8]); // Take last 6 bytes
    packed[7] = derivation.basefee_sharing_pctg;
    B256::from(packed)
}

/// Hash a derivation.
pub fn hash_derivation(derivation: &Derivation) -> B256 {
    let packed_fields = pack_derivation_fields(derivation);

    let sources_hash = if derivation.sources.is_empty() {
        EMPTY_BYTES_HASH
    } else if derivation.sources.len() == 1 {
        hash_two_values(
            U256::from(derivation.sources.len()).into(),
            hash_derivation_source(&derivation.sources[0]),
        )
    } else if derivation.sources.len() == 2 {
        hash_three_values(
            U256::from(derivation.sources.len()).into(),
            hash_derivation_source(&derivation.sources[0]),
            hash_derivation_source(&derivation.sources[1]),
        )
    } else {
        let array_length = derivation.sources.len();
        let buffer_size = 32 + (array_length * 32);
        let mut buffer = Vec::with_capacity(buffer_size);

        buffer.extend_from_slice(&U256::from(array_length).to_be_bytes::<32>());
        for source in &derivation.sources {
            let source_hash = hash_derivation_source(source);
            buffer.extend_from_slice(source_hash.as_slice());
        }

        keccak256(&buffer)
    };

    hash_three_values(packed_fields, derivation.origin_block_hash, sources_hash)
}

/// Hash the public input for proof verification.
pub fn hash_public_input(
    aggregated_proving_hash: B256,
    chain_id: u64,
    verifier_address: Address,
    sgx_instance: Address,
) -> B256 {
    hash_five_values(
        VERIFY_PROOF_B256,
        U256::from(chain_id).into(),
        address_to_b256(verifier_address),
        aggregated_proving_hash,
        address_to_b256(sgx_instance),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{address, b256};

    #[test]
    fn test_hash_two_values() {
        let a = B256::ZERO;
        let b = B256::from([1u8; 32]);
        let result = hash_two_values(a, b);
        assert_ne!(result, B256::ZERO);
    }

    #[test]
    fn test_address_to_b256() {
        let addr = address!("3C44CdDdB6a900fa2b585dd299e03d12FA4293BC");
        let b256 = address_to_b256(addr);
        // First 12 bytes should be zero
        assert_eq!(&b256.0[..12], &[0u8; 12]);
        // Last 20 bytes should be the address
        assert_eq!(&b256.0[12..], addr.as_slice());
    }

    #[test]
    fn test_hash_checkpoint() {
        let checkpoint = Checkpoint {
            block_number: 100,
            block_hash: b256!("0000000000000000000000000000000000000000000000000000000000000001"),
            state_root: b256!("0000000000000000000000000000000000000000000000000000000000000002"),
        };
        let result = hash_checkpoint(&checkpoint);
        assert_ne!(result, B256::ZERO);
    }
}
