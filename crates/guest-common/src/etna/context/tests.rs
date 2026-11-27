use super::*;
use tendermint::Time;

fn origin() -> OriginRecord {
    OriginRecord {
        start_height: 0,
        l1_number: 18,
        l1_timestamp: 100,
        fallback_keys_hash: B256::ZERO,
    }
}
fn block() -> Header {
    Header {
        number: 1,
        timestamp: 100,
        gas_limit: 45_000_000,
        mix_hash: expected_mix_hash(U256::ZERO, 1),
        ..Default::default()
    }
}
fn anchor() -> AnchorContext {
    AnchorContext {
        number: 18,
        timestamp: 100,
        generation: U256::from(2),
        checkpoint: 0,
        rules: Rules {
            active: 0,
            next: 0,
            time: 0,
            stop: 0,
        },
    }
}

#[test]
fn packed_context_words_reject_high_bits_and_enforce_bounds() {
    assert_eq!(
        decode_max_anchor_age(U256::from(1152) << 48).unwrap(),
        1152,
        "FI delay is outside the retained PoC relation"
    );
    assert_eq!(
        decode_max_anchor_age(U256::from(1) | (U256::from(1152) << 48)).unwrap(),
        1152
    );
    assert!(decode_max_anchor_age(U256::from(1) | (U256::from(1151) << 48)).is_err());
    assert!(decode_max_anchor_age(U256::from(1) << 96).is_err());
    assert!(Rules::decode(U256::from(1) << 192).is_err());
    let rules = Rules::decode(
        U256::from(3) | (U256::from(4) << 48) | (U256::from(5) << 96) | (U256::from(6) << 144),
    )
    .unwrap();
    assert_eq!(
        (rules.active, rules.next, rules.time, rules.stop),
        (3, 4, 5, 6)
    );
}

#[test]
fn timestamp_origin_age_and_backpressure_are_exact_at_boundaries() {
    let parent = Header {
        timestamp: 99,
        ..Default::default()
    };
    let base = block();
    let current = Time::from_unix_timestamp(100, 0).unwrap();
    assert!(verify_header_context(
        &base,
        &parent,
        current,
        &anchor(),
        &origin(),
        2,
        128,
        1152,
        45_000_000
    )
    .is_ok());
    let mut wrong_origin = origin();
    wrong_origin.l1_number = 19;
    assert!(verify_header_context(
        &base,
        &parent,
        current,
        &anchor(),
        &wrong_origin,
        2,
        128,
        1152,
        45_000_000
    )
    .is_err());
    assert!(verify_header_context(
        &base,
        &parent,
        Time::from_unix_timestamp(100, 1).unwrap(),
        &anchor(),
        &origin(),
        2,
        128,
        1152,
        45_000_000
    )
    .is_err());
    let mut later = base.clone();
    later.number = 129;
    later.mix_hash = expected_mix_hash(parent.difficulty, 129);
    assert!(verify_header_context(
        &later,
        &parent,
        current,
        &anchor(),
        &origin(),
        2,
        128,
        1152,
        45_000_000
    )
    .is_err());
    later.number = 128;
    later.mix_hash = expected_mix_hash(parent.difficulty, 128);
    assert!(verify_header_context(
        &later,
        &parent,
        current,
        &anchor(),
        &origin(),
        2,
        128,
        1152,
        45_000_000
    )
    .is_ok());
    let mut future_checkpoint = anchor();
    future_checkpoint.checkpoint = 129;
    assert!(verify_header_context(
        &later,
        &parent,
        current,
        &future_checkpoint,
        &origin(),
        2,
        128,
        1152,
        45_000_000
    )
    .is_err());
    let mut aged = block();
    aged.number = 2;
    aged.timestamp = 1252;
    aged.mix_hash = expected_mix_hash(parent.difficulty, 2);
    assert!(verify_header_context(
        &aged,
        &parent,
        Time::from_unix_timestamp(1252, 0).unwrap(),
        &anchor(),
        &origin(),
        2,
        128,
        1152,
        45_000_000
    )
    .is_ok());
    aged.timestamp += 1;
    assert!(verify_header_context(
        &aged,
        &parent,
        Time::from_unix_timestamp(1253, 0).unwrap(),
        &anchor(),
        &origin(),
        2,
        128,
        1152,
        45_000_000
    )
    .is_err());
    assert!(verify_header_context(
        &base,
        &Header {
            timestamp: u64::MAX,
            ..parent
        },
        current,
        &anchor(),
        &origin(),
        2,
        128,
        1152,
        45_000_000
    )
    .is_err());
}

#[test]
fn rules_stop_generation_gas_and_mix_checks_do_not_depend_on_execution_success() {
    let parent = Header {
        timestamp: 99,
        ..Default::default()
    };
    let base = block();
    let current = Time::from_unix_timestamp(100, 0).unwrap();
    let mut proof = anchor();
    proof.rules.next = 1;
    proof.rules.time = 101;
    assert!(verify_header_context(
        &base,
        &parent,
        current,
        &proof,
        &origin(),
        2,
        128,
        1152,
        45_000_000
    )
    .is_ok());
    proof.rules.time = 100;
    assert!(verify_header_context(
        &base,
        &parent,
        current,
        &proof,
        &origin(),
        2,
        128,
        1152,
        45_000_000
    )
    .is_err());
    proof = anchor();
    proof.rules.stop = 100;
    assert!(verify_header_context(
        &base,
        &parent,
        current,
        &proof,
        &origin(),
        2,
        128,
        1152,
        45_000_000
    )
    .is_ok());
    proof.rules.stop = 99;
    assert!(verify_header_context(
        &base,
        &parent,
        current,
        &proof,
        &origin(),
        2,
        128,
        1152,
        45_000_000
    )
    .is_err());
    proof = anchor();
    proof.generation = U256::from(1);
    assert!(verify_header_context(
        &base,
        &parent,
        current,
        &proof,
        &origin(),
        2,
        128,
        1152,
        45_000_000
    )
    .is_err());
    for mutation in ["gas", "mix", "time"] {
        let mut changed = base.clone();
        match mutation {
            "gas" => changed.gas_limit -= 1,
            "mix" => changed.mix_hash = B256::ZERO,
            "time" => changed.timestamp += 1,
            _ => unreachable!(),
        }
        assert!(verify_header_context(
            &changed,
            &parent,
            current,
            &anchor(),
            &origin(),
            2,
            128,
            1152,
            45_000_000
        )
        .is_err());
    }
}

#[test]
fn implementation_words_require_exact_addresses_and_no_high_bits() {
    let expected = Address::with_last_byte(7);
    assert!(
        verify_implementation_word(U256::from_be_bytes(expected.into_word().0), expected).is_ok()
    );
    assert!(verify_implementation_word(U256::ZERO, expected).is_err());
    assert!(verify_implementation_word(
        U256::from_be_bytes(expected.into_word().0) | (U256::from(1) << 160),
        expected
    )
    .is_err());
}
