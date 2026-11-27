//! Independent end-of-run model for Etna committee coverage.

use alloy_primitives::hex;
use etna_core::{
    comet::{codec::decode_canonical_block, validators::Validators},
    key::ConsensusKey,
};
use raiko2_protocol_etna::coverage::{
    CoverageContext, ExpectedValidatorSets, bind_committee_schedule,
};
use tendermint::{Hash, Time, account, block};

const PUBLIC_KEYS: [[u8; 33]; 3] = [
    hex!("0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"),
    hex!("02c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"),
    hex!("02f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9"),
];
const LAST_UINT48: u64 = (1 << 48) - 1;

struct Committee {
    signing_set: Vec<([u8; 33], u64)>,
    validators: Validators,
}

impl Committee {
    fn new(members: [(usize, u64); 2]) -> Self {
        let signing_members = members.map(|(key, power)| (PUBLIC_KEYS[key], power));
        let native_members = members
            .map(|(key, power)| (ConsensusKey::from_slice(&PUBLIC_KEYS[key]).unwrap(), power));
        let mut signing_set = signing_members.to_vec();
        signing_set.sort_unstable();
        Self {
            signing_set,
            validators: Validators::from_members(&native_members).unwrap(),
        }
    }
}

fn committees() -> [Committee; 4] {
    [
        Committee::new([(0, 10), (1, 10)]),
        Committee::new([(1, 10), (0, 10)]),
        Committee::new([(0, 10), (2, 10)]),
        Committee::new([(0, 11), (1, 10)]),
    ]
}

fn header_template() -> block::Header {
    let wire = hex::decode(include_str!("fixtures/first-block.hex").trim_end()).unwrap();
    let (_, block) = decode_canonical_block(&wire).unwrap();
    block.header
}

fn context(parent_height: u64, length: usize) -> CoverageContext {
    CoverageContext {
        parent_height,
        last_height: parent_height + u64::try_from(length).unwrap(),
        max_blocks: length,
    }
}

fn headers(
    template: &block::Header,
    context: &CoverageContext,
    schedule: &[ExpectedValidatorSets<'_>],
) -> Vec<block::Header> {
    (context.parent_height + 1..=context.last_height)
        .zip(schedule)
        .map(|(height, sets)| block::Header {
            height: height.try_into().unwrap(),
            validators_hash: sets.active.hash(),
            next_validators_hash: sets.next.hash(),
            ..template.clone()
        })
        .collect()
}

fn required_run_endpoints(
    context: &CoverageContext,
    active: &[usize],
    committees: &[Committee],
) -> Vec<u64> {
    let heights = context.parent_height + 1..=context.last_height;
    let identities = active.iter().map(|index| &committees[*index].signing_set);
    let heights_by_identity: Vec<_> = heights.zip(identities).collect();
    heights_by_identity
        .chunk_by(|left, right| left.1 == right.1)
        .map(|run| run.last().unwrap().0)
        .collect()
}

#[test]
fn exhaustive_short_schedules_match_explicit_key_and_power_runs() {
    let active = committees();
    let next = committees();
    assert_eq!(active[0].signing_set, active[1].signing_set);
    assert_ne!(active[0].signing_set, active[2].signing_set);
    assert_ne!(active[0].signing_set, active[3].signing_set);
    let template = header_template();

    let mut schedules = 0;
    let mut hash_mutations = 0;
    let mut adjacency_mutations = 0;
    let mut permutations = active.len();
    for length in 1..=4 {
        let context = context((1 << 32) + 13, length);
        let mut digits = vec![0; length + 1];
        permutations *= active.len();
        for mut number in 0..permutations {
            for digit in &mut digits {
                *digit = number % active.len();
                number /= active.len();
            }

            let schedule: Vec<_> = digits
                .windows(2)
                .map(|pair| ExpectedValidatorSets {
                    active: &active[pair[0]].validators,
                    next: &next[pair[1]].validators,
                })
                .collect();
            let mut headers = headers(&template, &context, &schedule);
            let expected = required_run_endpoints(&context, &digits[..length], &active);
            let result = bind_committee_schedule(&headers, &schedule, &context)
                .unwrap_or_else(|error| panic!("schedule {digits:?}: {error}"));
            assert_eq!(result.required_commit_heights(), expected, "{digits:?}");
            schedules += 1;

            for index in 0..length {
                let original = headers[index].validators_hash;
                headers[index].validators_hash = Hash::None;
                assert!(
                    bind_committee_schedule(&headers, &schedule, &context).is_err(),
                    "active mutation at {index} in {digits:?}"
                );
                headers[index].validators_hash = original;

                let original = headers[index].next_validators_hash;
                headers[index].next_validators_hash = Hash::Sha256([0; 32]);
                assert!(
                    bind_committee_schedule(&headers, &schedule, &context).is_err(),
                    "next mutation at {index} in {digits:?}"
                );
                headers[index].next_validators_hash = original;
                hash_mutations += 2;
            }

            let mut inconsistent = schedule.clone();
            for index in 0..length - 1 {
                let wrong = if digits[index + 1] == 2 { 3 } else { 2 };
                inconsistent[index].next = &next[wrong].validators;
                headers[index].next_validators_hash = next[wrong].validators.hash();
                assert!(
                    bind_committee_schedule(&headers, &inconsistent, &context).is_err(),
                    "locally matching but disconnected lookahead at {index} in {digits:?}"
                );
                inconsistent[index].next = schedule[index].next;
                headers[index].next_validators_hash = schedule[index].next.hash();
                adjacency_mutations += 1;
            }
        }
    }
    assert_eq!(schedules, 1_360);
    assert_eq!(hash_mutations, 10_016);
    assert_eq!(adjacency_mutations, 3_648);
}

#[test]
fn full_landing_omits_origin_internal_cut_and_height_after_terminal() {
    let sets = committees();
    let a = &sets[0].validators;
    let equivalent_a = &sets[1].validators;
    let b = &sets[2].validators;
    let c = &sets[3].validators;
    let context = context(90, 5);
    let schedule = [
        ExpectedValidatorSets { active: a, next: a },
        ExpectedValidatorSets {
            active: a,
            next: equivalent_a,
        },
        ExpectedValidatorSets {
            active: equivalent_a,
            next: b,
        },
        ExpectedValidatorSets { active: b, next: b },
        ExpectedValidatorSets { active: b, next: c },
    ];
    let headers = headers(&header_template(), &context, &schedule);

    // A candidate child ends at 92. In this complete landing the A interval ends at 93.
    let result = bind_committee_schedule(&headers, &schedule, &context).unwrap();
    assert_eq!(result.required_commit_heights(), &[93, 95]);
    assert!(bind_committee_schedule(&headers[..2], &schedule[..2], &context).is_err());
}

#[test]
fn range_slice_lengths_and_caller_budget_are_checked_independently() {
    let sets = committees();
    let a = &sets[0].validators;
    let schedule = [ExpectedValidatorSets { active: a, next: a }; 4];
    let headers = headers(&header_template(), &context(7, 4), &schedule);
    let context = context(7, 3);

    for header_count in 0..=4 {
        for expected_count in 0..=4 {
            let result = bind_committee_schedule(
                &headers[..header_count],
                &schedule[..expected_count],
                &context,
            );
            assert_eq!(result.is_ok(), header_count == 3 && expected_count == 3);
        }
    }

    for max_blocks in [0, 1, 2, 3, 4, usize::MAX] {
        let context = CoverageContext {
            max_blocks,
            ..context
        };
        let result = bind_committee_schedule(&headers[..3], &schedule[..3], &context);
        assert_eq!(result.is_ok(), max_blocks >= 3, "budget {max_blocks}");
        assert!(bind_committee_schedule(&headers[..2], &schedule[..2], &context).is_err());
    }
}

#[test]
fn exact_height_sequence_and_uint48_endpoints_are_checked() {
    let sets = committees();
    let a = &sets[0].validators;
    let schedule = [ExpectedValidatorSets { active: a, next: a }; 3];
    let template = header_template();

    for (parent, length) in [(0, 1), (LAST_UINT48 - 1, 1), (LAST_UINT48 - 3, 3)] {
        let context = context(parent, length);
        let schedule = &schedule[..length];
        let mut headers = headers(&template, &context, schedule);
        let result = bind_committee_schedule(&headers, schedule, &context).unwrap();
        assert_eq!(result.required_commit_heights(), &[context.last_height]);
        for index in 0..length {
            let true_height = headers[index].height;
            for wrong in [0, true_height.value() - 1, true_height.value() + 1] {
                headers[index].height = wrong.try_into().unwrap();
                assert!(
                    bind_committee_schedule(&headers, schedule, &context).is_err(),
                    "height {wrong} at {index}, parent {parent}"
                );
            }
            headers[index].height = true_height;
        }
        if length > 1 {
            headers.swap(0, length - 1);
            assert!(bind_committee_schedule(&headers, schedule, &context).is_err());
        }
    }

    let singleton = &schedule[..1];
    let above_limit = context(LAST_UINT48, 1);
    let above_limit_headers = headers(&template, &above_limit, singleton);
    assert!(bind_committee_schedule(&above_limit_headers, singleton, &above_limit).is_err());
    let crossing_limit = context(LAST_UINT48 - 1, 2);
    let crossing_schedule = &schedule[..2];
    let crossing_headers = headers(&template, &crossing_limit, crossing_schedule);
    assert!(
        bind_committee_schedule(&crossing_headers, crossing_schedule, &crossing_limit).is_err()
    );
    for (parent_height, last_height) in [
        (0, 0),
        (8, 8),
        (8, 7),
        (u64::MAX, 0),
        (u64::MAX - 1, u64::MAX),
        (0, u64::MAX),
        (LAST_UINT48 + 1, LAST_UINT48 + 2),
    ] {
        let context = CoverageContext {
            parent_height,
            last_height,
            max_blocks: usize::MAX,
        };
        assert!(bind_committee_schedule(&[], &[], &context).is_err());
        assert!(bind_committee_schedule(&above_limit_headers, singleton, &context).is_err());
    }
}

#[test]
fn unrelated_header_fields_are_opaque_to_coverage() {
    let sets = committees();
    let a = &sets[0].validators;
    let schedule = [ExpectedValidatorSets { active: a, next: a }; 2];
    let context = context(51, schedule.len());
    let mut headers = headers(&header_template(), &context, &schedule);
    let header = &mut headers[1];
    header.version.app = u64::MAX;
    header.version.block = u64::MAX;
    header.chain_id = "fixture-opaque-domain".parse().unwrap();
    header.time = Time::from_unix_timestamp(17, 0).unwrap();
    header.last_block_id = Some(block::Id::default());
    header.last_commit_hash = Some(Hash::Sha256([1; 32]));
    header.data_hash = Some(Hash::Sha256([2; 32]));
    header.consensus_hash = Hash::None;
    header.app_hash = vec![3, 4].try_into().unwrap();
    header.last_results_hash = Some(Hash::Sha256([5; 32]));
    header.evidence_hash = Some(Hash::Sha256([6; 32]));
    header.proposer_address = account::Id::new([7; 20]);

    let result = bind_committee_schedule(&headers, &schedule, &context).unwrap();
    assert_eq!(result.required_commit_heights(), &[53]);
}
