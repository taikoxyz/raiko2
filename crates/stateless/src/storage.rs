//! Batch authenticated storage reads through the existing sparse witness implementation.

use crate::{StatelessTrie, sparse::SparseState};
use alloy_primitives::{Address, B256, U256};
use raiko2_primitives::{ExecutionWitness, StatelessValidationError, WitnessStateNode};

/// Reads all requested words with one sparse-state hydration and cached per-account storage tries.
///
/// `state_root` must come from the composing verifier's authenticated full header, never cached
/// witness metadata. Missing proof nodes fail; only an authenticated absence returns zero.
///
/// # Errors
/// Returns an error if any requested account or slot cannot be resolved against `state_root`.
pub fn read_storage_batch_with_witness_resources(
    state_root: B256,
    requests: &[(Address, U256)],
    witness: &ExecutionWitness,
    shared_state_nodes: &[WitnessStateNode],
) -> Result<Vec<U256>, StatelessValidationError> {
    let (mut trie, _) = SparseState::new_with_state_pool(witness, shared_state_nodes, state_root)?;
    requests
        .iter()
        .map(|&(address, slot)| {
            trie.storage_from_witness(address, slot).map_err(|error| {
                StatelessValidationError::StatelessExecutionFailed(error.to_string())
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::TrieAccount;
    use alloy_primitives::{KECCAK256_EMPTY, keccak256};
    use alloy_trie::EMPTY_ROOT_HASH;
    use raiko2_primitives::WitnessHeader;
    use risc0_ethereum_trie::Trie;

    #[test]
    fn batch_proves_present_and_absent_slots_ignoring_cached_metadata() {
        let address = Address::with_last_byte(1);
        let mut storage = Trie::default();
        storage.insert(
            keccak256(B256::from(U256::from(2))),
            alloy_rlp::encode(U256::from(9)),
        );
        let mut state = Trie::default();
        state.insert(
            keccak256(address),
            alloy_rlp::encode(TrieAccount {
                nonce: 1,
                balance: U256::ZERO,
                storage_root: storage.hash_slow(),
                code_hash: KECCAK256_EMPTY,
            }),
        );
        let root = state.hash_slow();
        let witness = ExecutionWitness {
            state: state
                .rlp_nodes()
                .into_iter()
                .chain(storage.rlp_nodes())
                .map(WitnessStateNode::from_bytes)
                .collect(),
            headers: vec![WitnessHeader::from_header(alloy_consensus::Header {
                state_root: B256::repeat_byte(0xff),
                ..Default::default()
            })],
            ..Default::default()
        };
        assert_eq!(
            read_storage_batch_with_witness_resources(
                root,
                &[(address, U256::from(2)), (address, U256::from(3))],
                &witness,
                &[]
            )
            .unwrap(),
            vec![U256::from(9), U256::ZERO]
        );
        let absent = read_storage_batch_with_witness_resources(
            EMPTY_ROOT_HASH,
            &[(address, U256::from(2))],
            &ExecutionWitness::default(),
            &[],
        )
        .unwrap();
        assert_eq!(absent, vec![U256::ZERO]);
        let incomplete = ExecutionWitness {
            state: state
                .rlp_nodes()
                .into_iter()
                .map(WitnessStateNode::from_bytes)
                .collect(),
            ..Default::default()
        };
        assert!(
            read_storage_batch_with_witness_resources(
                root,
                &[(address, U256::from(2))],
                &incomplete,
                &[]
            )
            .is_err()
        );
        assert!(
            read_storage_batch_with_witness_resources(
                B256::repeat_byte(0xff),
                &[(address, U256::from(2))],
                &witness,
                &[]
            )
            .is_err()
        );
    }
}
