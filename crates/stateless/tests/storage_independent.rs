//! The unresolved-child case must not be confused with authenticated absence.
use alloy_consensus::{Header, TrieAccount};
use alloy_primitives::{Address, B256, KECCAK256_EMPTY, U256, keccak256};
use raiko2_primitives::{ExecutionWitness, WitnessHeader, WitnessStateNode};
use raiko2_stateless::read_storage_batch_with_witness_resources;
use risc0_ethereum_trie::Trie;

#[test]
fn independent_batch_combines_shared_nodes_and_rejects_unresolved_storage_child() {
    let account = Address::with_last_byte(1);
    let mut storage = Trie::default();
    for slot in [1, 2, 3] {
        storage.insert(
            keccak256(B256::from(U256::from(slot))),
            alloy_rlp::encode(U256::MAX - U256::from(slot)),
        );
    }
    let storage_root = storage.hash_slow();
    let mut state = Trie::default();
    state.insert(
        keccak256(account),
        alloy_rlp::encode(TrieAccount {
            nonce: 1,
            balance: U256::ZERO,
            storage_root,
            code_hash: KECCAK256_EMPTY,
        }),
    );
    let root = state.hash_slow();
    let mut witness = ExecutionWitness {
        state: state
            .rlp_nodes()
            .into_iter()
            .map(WitnessStateNode::from_bytes)
            .collect(),
        // Cached header roots are irrelevant to authority, including a valid-looking other root.
        headers: vec![WitnessHeader::from_header(Header {
            state_root: B256::repeat_byte(0x99),
            ..Default::default()
        })],
        ..Default::default()
    };
    let storage_nodes = storage.rlp_nodes();
    assert!(
        storage_nodes.len() > 1,
        "fixture must require hashed child nodes"
    );
    // Existing witnesses select either inline state or indexed shared state;
    // the shared mode must index both account nodes and storage nodes.
    let shared: Vec<_> = witness
        .state
        .iter()
        .cloned()
        .chain(
            storage_nodes
                .iter()
                .cloned()
                .map(WitnessStateNode::from_bytes),
        )
        .collect();
    let mut shared_witness = witness.clone();
    shared_witness.state.clear();
    shared_witness.state_indices = (0..shared.len())
        .map(|index| u32::try_from(index).unwrap())
        .collect();
    let requests = [(account, U256::from(1)), (account, U256::from(999))];
    assert_eq!(
        read_storage_batch_with_witness_resources(root, &requests, &shared_witness, &shared)
            .unwrap(),
        vec![U256::MAX - U256::ONE, U256::ZERO]
    );
    // Supply the authenticated account and actual storage root node, but no child opening.
    // This distinguishes unresolved material below a present root from a missing root.
    witness.state.extend(
        storage_nodes
            .into_iter()
            .filter(|node| keccak256(node) == storage_root)
            .map(WitnessStateNode::from_bytes),
    );
    assert!(
        read_storage_batch_with_witness_resources(root, &requests[..1], &witness, &[]).is_err()
    );
}
