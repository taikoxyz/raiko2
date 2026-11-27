//! Independent tester regressions for every public statement commitment.
use alloy_primitives::{Address, B256, U256, b256};
use raiko2_protocol_etna::{input::LandStatementInput, statement::LandInput};

fn statement() -> LandStatementInput {
    LandStatementInput {
        l1_chain_id: U256::from(1),
        l2_chain_id: U256::from(167_001),
        inbox: Address::repeat_byte(0x11),
        generation: U256::from(3),
        parent_hash: B256::repeat_byte(4),
        parent_comet_hash: B256::repeat_byte(5),
        config_hash: B256::repeat_byte(6),
        input: LandInput {
            parent_height: U256::from(7),
            last_height: U256::from(10),
            last_block_hash: B256::repeat_byte(8),
            last_comet_hash: B256::repeat_byte(9),
            last_state_root: B256::repeat_byte(10),
            l1_reference_block_number: U256::from(11),
            forced_inclusion_head: U256::from(12),
        },
        l1_reference_block_hash: B256::repeat_byte(13),
        blob_hashes: vec![B256::repeat_byte(14), B256::repeat_byte(15)],
        forced_inclusion_head_before: U256::from(16),
        cut_leaves: vec![B256::repeat_byte(17), B256::repeat_byte(18)],
    }
}

#[test]
fn independent_every_public_statement_field_is_bound() {
    let original = statement();
    let digest = original.as_statement().try_digest().unwrap();
    for index in 0..18 {
        let mut changed = statement();
        match index {
            0 => changed.l1_chain_id += U256::ONE,
            1 => changed.l2_chain_id += U256::ONE,
            2 => changed.inbox = Address::ZERO,
            3 => changed.generation += U256::ONE,
            4 => changed.input.parent_height += U256::ONE,
            5 => changed.parent_hash = B256::ZERO,
            6 => changed.parent_comet_hash = B256::ZERO,
            7 => changed.config_hash = B256::ZERO,
            8 => changed.input.last_height += U256::ONE,
            9 => changed.input.last_block_hash = B256::ZERO,
            10 => changed.input.last_comet_hash = B256::ZERO,
            11 => changed.input.last_state_root = B256::ZERO,
            12 => changed.input.l1_reference_block_number += U256::ONE,
            13 => changed.l1_reference_block_hash = B256::ZERO,
            14 => changed.blob_hashes.swap(0, 1),
            15 => changed.forced_inclusion_head_before += U256::ONE,
            16 => changed.input.forced_inclusion_head += U256::ONE,
            17 => changed.cut_leaves.swap(0, 1),
            _ => unreachable!(),
        }
        // Shape changes intentionally use the raw commitment: shape validity is
        // a separate invariant and cannot mask an omitted commitment word here.
        assert_ne!(changed.as_statement().digest(), digest, "field {index}");
    }
}

#[test]
fn independent_owned_conversion_preserves_every_word_and_list_order() {
    let mut original = statement();
    original.l1_chain_id |= U256::ONE << 255;
    original.l2_chain_id |= U256::ONE << 254;
    original.input.forced_inclusion_head |= U256::ONE << 253;
    original.forced_inclusion_head_before |= U256::ONE << 252;
    let borrowed = original.as_statement();
    let converted = LandStatementInput::from(&borrowed);
    assert_eq!(converted.as_statement().preimage(), borrowed.preimage());
    assert_eq!(converted.blob_hashes, original.blob_hashes);
    assert_eq!(converted.cut_leaves, original.cut_leaves);
    let encoded = serde_json::to_vec(&converted).unwrap();
    let decoded: LandStatementInput = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded.as_statement().preimage(), borrowed.preimage());
}

#[test]
fn independent_empty_and_nonempty_blob_lists_remain_distinct() {
    let original = statement();
    let mut empty = statement();
    empty.blob_hashes.clear();
    assert_ne!(
        empty.as_statement().digest(),
        original.as_statement().digest()
    );
    // Empty ordered lists use keccak256 of empty bytes, not ABI array framing.
    assert_eq!(
        &empty.as_statement().preimage()[15 * 32..16 * 32],
        b256!("c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470").as_slice(),
    );
}
