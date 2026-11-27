//! Validator schedule binding over a complete Etna landing range.

use alloy_primitives::hex;
use etna_core::{
    comet::{codec::decode_canonical_block, validators::Validators},
    key::ConsensusKey,
    params::UINT48_MAX,
};
use raiko2_protocol_etna::coverage::{
    CoverageContext, CoverageError, ExpectedValidatorSets, bind_committee_schedule,
};
use tendermint::{Hash, block::Header};

const KEY_A: [u8; 33] = hex!("0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798");
const KEY_B: [u8; 33] = hex!("02c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5");

fn member(key: &[u8; 33], power: u64) -> (ConsensusKey, u64) {
    (ConsensusKey::from_slice(key).unwrap(), power)
}

fn headers(parent: u64, expected: &[ExpectedValidatorSets<'_>]) -> Vec<Header> {
    let bytes = hex::decode(include_str!("fixtures/first-block.hex").trim_end()).unwrap();
    let (_, base) = decode_canonical_block(&bytes).unwrap();
    let mut height = parent;
    expected
        .iter()
        .map(|sets| {
            height += 1;
            Header {
                height: height.try_into().unwrap(),
                validators_hash: sets.active.hash(),
                next_validators_hash: sets.next.hash(),
                ..base.header.clone()
            }
        })
        .collect()
}

#[test]
fn returning_to_the_initial_set_preserves_both_outgoing_commit_heights() {
    let a = Validators::from_members(&[member(&KEY_A, 10)]).unwrap();
    let b = Validators::from_members(&[member(&KEY_B, 10)]).unwrap();
    let expected = [
        ExpectedValidatorSets {
            active: &a,
            next: &a,
        },
        ExpectedValidatorSets {
            active: &a,
            next: &b,
        },
        ExpectedValidatorSets {
            active: &b,
            next: &b,
        },
        ExpectedValidatorSets {
            active: &b,
            next: &a,
        },
        ExpectedValidatorSets {
            active: &a,
            next: &a,
        },
    ];
    let context = CoverageContext {
        parent_height: 10,
        last_height: 15,
        max_blocks: 5,
    };
    let headers = headers(context.parent_height, &expected);

    let requirements = bind_committee_schedule(&headers, &expected, &context).unwrap();

    assert_eq!(requirements.required_commit_heights(), &[12, 14, 15]);
}

const fn sets<'a>(active: &'a Validators, next: &'a Validators) -> ExpectedValidatorSets<'a> {
    ExpectedValidatorSets { active, next }
}

#[test]
fn singleton_at_maximum_height_requires_only_the_terminal_commit() {
    let a = Validators::from_members(&[member(&KEY_A, 10)]).unwrap();
    let b = Validators::from_members(&[member(&KEY_B, 10)]).unwrap();
    let context = CoverageContext {
        parent_height: UINT48_MAX - 1,
        last_height: UINT48_MAX,
        max_blocks: 1,
    };
    let expected = [sets(&a, &b)];

    let requirements = bind_committee_schedule(
        &headers(context.parent_height, &expected),
        &expected,
        &context,
    )
    .unwrap();

    assert_eq!(requirements.required_commit_heights(), &[UINT48_MAX]);
}

#[test]
fn power_only_change_requires_the_outgoing_commit_even_beyond_u32() {
    let a = Validators::from_members(&[member(&KEY_A, 10)]).unwrap();
    let updated = Validators::from_members(&[member(&KEY_A, 11)]).unwrap();
    let context = CoverageContext {
        parent_height: 4_294_967_300,
        last_height: 4_294_967_303,
        max_blocks: 3,
    };
    let expected = [
        sets(&a, &updated),
        sets(&updated, &updated),
        sets(&updated, &updated),
    ];

    let requirements = bind_committee_schedule(
        &headers(context.parent_height, &expected),
        &expected,
        &context,
    )
    .unwrap();

    assert_eq!(
        requirements.required_commit_heights(),
        &[4_294_967_301, 4_294_967_303]
    );
}

#[test]
fn reordering_does_not_require_extra_commits() {
    let first = member(&KEY_A, 10);
    let second = member(&KEY_B, 10);
    let a = Validators::from_members(&[first, second]).unwrap();
    let equivalent = Validators::from_members(&[second, first]).unwrap();
    let expected = [sets(&a, &equivalent), sets(&equivalent, &a), sets(&a, &a)];

    let requirements =
        bind_committee_schedule(&headers(0, &expected), &expected, &THREE_BLOCKS).unwrap();

    assert_eq!(requirements.required_commit_heights(), &[3]);
}

const THREE_BLOCKS: CoverageContext = CoverageContext {
    parent_height: 0,
    last_height: 3,
    max_blocks: 3,
};

#[test]
fn mismatched_current_and_next_hashes_reject_at_interior_non_commit_height() {
    let a = Validators::from_members(&[member(&KEY_A, 10)]).unwrap();
    let expected = [sets(&a, &a); 3];
    let original = headers(0, &expected);
    let mut bad_current = original.clone();
    bad_current[1].validators_hash = Hash::None;
    assert!(matches!(
        bind_committee_schedule(&bad_current, &expected, &THREE_BLOCKS),
        Err(CoverageError::ActiveHash { height: 2 })
    ));

    let mut bad_next = original;
    bad_next[1].next_validators_hash = Hash::None;
    assert!(matches!(
        bind_committee_schedule(&bad_next, &expected, &THREE_BLOCKS),
        Err(CoverageError::NextHash { height: 2 })
    ));
}

#[test]
fn each_header_matching_its_entry_still_requires_schedule_linkage() {
    let a = Validators::from_members(&[member(&KEY_A, 10)]).unwrap();
    let b = Validators::from_members(&[member(&KEY_B, 10)]).unwrap();
    let expected = [sets(&a, &a), sets(&b, &b), sets(&b, &b)];

    assert!(matches!(
        bind_committee_schedule(&headers(0, &expected), &expected, &THREE_BLOCKS),
        Err(CoverageError::ScheduleLink { height: 2 })
    ));
}

#[test]
fn both_input_counts_must_equal_the_range_length() {
    let a = Validators::from_members(&[member(&KEY_A, 10)]).unwrap();
    let expected = [sets(&a, &a); 4];
    let headers = headers(0, &expected);

    for count in [0, 2, 4] {
        assert!(matches!(
            bind_committee_schedule(&headers[..count], &expected[..3], &THREE_BLOCKS),
            Err(CoverageError::HeaderCount { expected: 3, actual }) if actual == count
        ));
        assert!(matches!(
            bind_committee_schedule(&headers[..3], &expected[..count], &THREE_BLOCKS),
            Err(CoverageError::ScheduleCount { expected: 3, actual }) if actual == count
        ));
    }
    assert!(matches!(
        bind_committee_schedule(&headers[..2], &expected[..2], &THREE_BLOCKS),
        Err(CoverageError::HeaderCount {
            expected: 3,
            actual: 2
        })
    ));
}

#[test]
fn invalid_endpoints_and_insufficient_budgets_reject() {
    for (parent_height, last_height) in [
        (0, 0),
        (2, 1),
        (UINT48_MAX, UINT48_MAX + 1),
        (0, u64::MAX),
        (u64::MAX, u64::MAX),
    ] {
        let context = CoverageContext {
            parent_height,
            last_height,
            max_blocks: 3,
        };
        assert!(matches!(
            bind_committee_schedule(&[], &[], &context),
            Err(CoverageError::InvalidRange)
        ));
    }

    let a = Validators::from_members(&[member(&KEY_A, 10)]).unwrap();
    let expected = [sets(&a, &a); 3];
    let headers = headers(0, &expected);
    for max_blocks in [0, 2] {
        let context = CoverageContext {
            max_blocks,
            ..THREE_BLOCKS
        };
        assert!(matches!(
            bind_committee_schedule(&headers[..max_blocks], &expected[..max_blocks], &context),
            Err(CoverageError::BlockLimit { actual: 3, max }) if max == max_blocks
        ));
    }
}

#[test]
fn header_heights_must_be_consecutive_and_in_order() {
    let a = Validators::from_members(&[member(&KEY_A, 10)]).unwrap();
    let expected = [sets(&a, &a); 3];
    let original = headers(0, &expected);
    for wrong_height in [1, 3] {
        let mut headers = original.clone();
        headers[1].height = wrong_height.try_into().unwrap();
        assert!(matches!(
            bind_committee_schedule(&headers, &expected, &THREE_BLOCKS),
            Err(CoverageError::HeaderHeight { expected: 2, actual }) if actual == wrong_height
        ));
    }

    let mut swapped = original;
    swapped.swap(0, 1);
    assert!(matches!(
        bind_committee_schedule(&swapped, &expected, &THREE_BLOCKS),
        Err(CoverageError::HeaderHeight {
            expected: 1,
            actual: 2
        })
    ));
}

#[test]
fn app_version_and_header_ancestry_are_outside_schedule_binding() {
    let a = Validators::from_members(&[member(&KEY_A, 10)]).unwrap();
    let expected = [sets(&a, &a); 3];
    let mut headers = headers(0, &expected);
    headers[1].version.app = u64::MAX;
    headers[1].last_block_id = None;

    let requirements = bind_committee_schedule(&headers, &expected, &THREE_BLOCKS).unwrap();

    assert_eq!(requirements.required_commit_heights(), &[3]);
}

#[test]
fn terminal_next_hash_must_match_even_without_a_successor_header() {
    let a = Validators::from_members(&[member(&KEY_A, 10)]).unwrap();
    let b = Validators::from_members(&[member(&KEY_B, 10)]).unwrap();
    let expected = [sets(&a, &a), sets(&a, &a), sets(&a, &b)];
    let mut headers = headers(0, &expected);
    headers[2].next_validators_hash = a.hash();

    assert!(matches!(
        bind_committee_schedule(&headers, &expected, &THREE_BLOCKS),
        Err(CoverageError::NextHash { height: 3 })
    ));
}
