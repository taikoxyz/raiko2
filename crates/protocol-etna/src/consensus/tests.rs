use super::*;
use alloy_primitives::hex;
use etna_core::{key::ConsensusKey, staking::StakingWords};

fn fallback() -> FallbackKeys {
    FallbackKeys::new(alloc::vec![
        ConsensusKey::from_slice(&hex!(
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
        ))
        .unwrap()
    ])
    .unwrap()
}

#[test]
fn origin_authentication_binds_all_words_and_fallback_list() {
    let keys = fallback();
    let words = [
        U256::from(7),
        U256::from(11),
        U256::from(13),
        U256::from_be_bytes(keys.hash().0),
    ];
    let origin = authenticate_origin(&words, 7, &keys).unwrap();
    assert_eq!(origin.start_height, 7);
    let mut altered = words;
    altered[0] += U256::from(1);
    assert!(authenticate_origin(&altered, 7, &keys).is_err());
    altered = words;
    altered[1] |= U256::from(1) << 200;
    assert!(authenticate_origin(&altered, 7, &keys).is_err());
    altered = words;
    altered[3] = U256::ZERO;
    assert!(authenticate_origin(&altered, 7, &keys).is_err());
}

#[test]
fn snapshot_full_height_and_invalid_records_use_fallback_without_rescaling() {
    let keys = fallback();
    let key = keys.keys()[0];
    let mut snapshot = StakingWords::header_only(U256::from(9), U256::from(100));
    snapshot.records[127] = [
        U256::from_be_slice(&key.as_bytes()[1..]),
        U256::from(key.as_bytes()[0]) | (U256::from(5) << 8),
    ];
    let candidate = derive_signers(9, &snapshot, U256::from(1), keys.hash(), &keys).unwrap();
    assert_eq!(candidate.total_voting_power(), 5);
    snapshot.snapshot_height |= U256::from(1) << 200;
    assert_eq!(
        derive_signers(9, &snapshot, U256::ZERO, keys.hash(), &keys)
            .unwrap()
            .total_voting_power(),
        1
    );
    snapshot.snapshot_height = U256::from(9);
    snapshot.records[127][1] |= U256::from(1) << 80;
    assert_eq!(
        derive_signers(9, &snapshot, U256::ZERO, keys.hash(), &keys)
            .unwrap()
            .total_voting_power(),
        1
    );
    assert!(derive_signers(9, &snapshot, U256::ZERO, B256::ZERO, &keys).is_err());
}

#[test]
fn origin_slots_use_generation_mapping_and_checked_wrapping_offsets() {
    let slots = origin_slots(3);
    let mut encoded = [0u8; 64];
    encoded[..32].copy_from_slice(&U256::from(3).to_be_bytes::<32>());
    encoded[32..].copy_from_slice(&U256::from(290).to_be_bytes::<32>());
    assert_eq!(slots[0], keccak256(encoded));
    assert_eq!(
        U256::from_be_bytes(slots[3].0),
        U256::from_be_bytes(slots[0].0).wrapping_add(U256::from(3))
    );
}
