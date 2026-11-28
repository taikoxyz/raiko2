//! Boundary cases derived from COMM-4.3 and STOR-B, independent of fixture authors.
use alloy_primitives::{B256, U256, hex};
use etna_core::{
    committee::MAX_TOTAL_POWER, fallback::FallbackKeys, key::ConsensusKey, params::UINT48_MAX,
    staking::StakingWords,
};
use raiko2_protocol_etna::consensus::{authenticate_origin, derive_signers};

fn fallback() -> FallbackKeys {
    FallbackKeys::new(vec![
        ConsensusKey::from_slice(&hex!(
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
        ))
        .unwrap(),
    ])
    .unwrap()
}

fn snapshot(keys: &FallbackKeys, power: u64) -> StakingWords {
    let key = keys.keys()[0];
    let mut words = StakingWords::header_only(U256::from(41), U256::from(100));
    // Seat 127 is authoritative even when all lower records are empty.
    words.records[127] = [
        U256::from_be_slice(&key.as_bytes()[1..]),
        U256::from(key.as_bytes()[0]) | (U256::from(power) << 8),
    ];
    words
}

#[test]
fn independent_threshold_equality_and_maximum_disable_switch() {
    let keys = fallback();
    let mut words = snapshot(&keys, 9);
    for (minimum, expected_power) in [(99, 9), (100, 9), (101, 1)] {
        let actual = derive_signers(41, &words, U256::from(minimum), keys.hash(), &keys).unwrap();
        assert_eq!(actual.total_voting_power(), expected_power);
    }
    words.total_stake = U256::MAX;
    assert_eq!(
        derive_signers(41, &words, U256::MAX, keys.hash(), &keys)
            .unwrap()
            .total_voting_power(),
        1
    );
}

#[test]
fn independent_invalid_candidate_is_fallback_but_invalid_fallback_opening_is_error() {
    let keys = fallback();
    let valid = snapshot(&keys, 9);
    // A usable candidate still needs the deciding anchor's real fallback preimage.
    assert!(derive_signers(41, &valid, U256::ZERO, B256::ZERO, &keys).is_err());
    for power in [0, MAX_TOTAL_POWER + 1, 9] {
        let mut invalid = snapshot(&keys, power);
        if invalid == valid {
            invalid.records[0] = invalid.records[127]; // duplicate key, not an empty seat
        }
        assert_eq!(
            derive_signers(41, &invalid, U256::ZERO, keys.hash(), &keys)
                .unwrap()
                .total_voting_power(),
            1
        );
    }
}

#[test]
fn independent_high_snapshot_bits_are_never_masked() {
    let keys = fallback();
    for bit in [48, 63, 64, 127, 128, 255] {
        let mut invalid = snapshot(&keys, 9);
        invalid.snapshot_height |= U256::ONE << bit;
        assert_eq!(
            derive_signers(41, &invalid, U256::ZERO, keys.hash(), &keys)
                .unwrap()
                .total_voting_power(),
            1,
            "bit {bit}"
        );
    }
}

#[test]
fn independent_origin_checks_all_three_widths_and_actual_fourth_word() {
    let keys = fallback();
    let words = [
        U256::from(7),
        U256::from(UINT48_MAX),
        U256::from(UINT48_MAX),
        U256::from_be_bytes(keys.hash().0),
    ];
    assert!(authenticate_origin(&words, 7, &keys).is_ok());
    for index in 0..3 {
        for bit in [48, 64, 255] {
            let mut bad = words;
            bad[index] |= U256::ONE << bit;
            assert!(
                authenticate_origin(&bad, 7, &keys).is_err(),
                "word {index}, bit {bit}"
            );
        }
    }
    let mut legacy_three_word = words;
    legacy_three_word[3] = U256::ZERO;
    assert!(authenticate_origin(&legacy_three_word, 7, &keys).is_err());
}
