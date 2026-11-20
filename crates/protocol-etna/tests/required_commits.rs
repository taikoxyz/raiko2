//! Signed native certificates for the required heights in a landing range.

use alloy_primitives::{Address, U256, hex, keccak256};
use etna_core::{
    comet::{
        codec::decode_canonical_block,
        native::{Commit, CommitSig, RawCommit, RawCommitSig},
        validators::Validators,
        verify::{BftError, header_hash, verify_native_commit},
    },
    domain::{DeploymentDomain, DomainError, chain_id},
    key::ConsensusKey,
    types::Member,
};
use k256::ecdsa::SigningKey;
use raiko2_protocol_etna::coverage::{
    CommitRequirements, CommitVerificationError, CoverageContext, CoverageError,
    ExpectedValidatorSets, verify_required_native_commits,
};
use tendermint::{
    Hash, Vote,
    block::{Header, Id, parts},
    vote::{CanonicalVote, Type},
};
use tendermint_proto::{Protobuf, v0_38::types::CanonicalVote as RawVote};

const PARENT_HEIGHT: u64 = 40;

struct Committee {
    validators: Validators,
    keys: Vec<SigningKey>,
}

fn committee(signers: &[(u8, u64)]) -> Committee {
    let keys: Vec<_> = signers
        .iter()
        .map(|&(seed, _)| SigningKey::from_slice(&[seed; 32]).unwrap())
        .collect();
    let members: Vec<_> = keys
        .iter()
        .zip(signers)
        .map(|(key, &(_, power))| Member {
            pubkey: ConsensusKey::from_slice(key.verifying_key().to_encoded_point(true).as_bytes())
                .unwrap(),
            eff_stake: U256::ZERO,
            power,
        })
        .collect();
    Committee {
        validators: Validators::from_members(&members).unwrap(),
        keys,
    }
}

fn domain() -> DeploymentDomain {
    DeploymentDomain {
        l1_chain_id: U256::from(1),
        l2_chain_id: U256::from(167_001),
        inbox: Address::repeat_byte(0x11),
        generation: 3,
    }
}

struct LandingRange<'a> {
    headers: Vec<Header>,
    expected: Vec<ExpectedValidatorSets<'a>>,
    context: CoverageContext,
    domain: DeploymentDomain,
}

impl LandingRange<'_> {
    fn verify(&self, commits: &[Commit]) -> Result<CommitRequirements, CommitVerificationError> {
        verify_required_native_commits(
            &self.headers,
            &self.expected,
            &self.context,
            commits,
            &self.domain,
        )
    }
}

fn landing_range<'a>(
    schedule: &[&'a Validators],
    terminal_next: &'a Validators,
) -> LandingRange<'a> {
    let block_bytes = hex::decode(include_str!("fixtures/first-block.hex").trim_end()).unwrap();
    let (_, block) = decode_canonical_block(&block_bytes).unwrap();
    let domain = domain();
    let mut template = block.header;
    template.chain_id = chain_id(&domain).unwrap().parse().unwrap();
    let expected: Vec<_> = schedule
        .iter()
        .copied()
        .zip(schedule.iter().skip(1).copied().chain([terminal_next]))
        .map(|(active, next)| ExpectedValidatorSets { active, next })
        .collect();
    let headers = expected
        .iter()
        .enumerate()
        .map(|(offset, sets)| {
            let mut header = template.clone();
            header.height = (PARENT_HEIGHT + offset as u64 + 1).try_into().unwrap();
            header.validators_hash = sets.active.hash();
            header.next_validators_hash = sets.next.hash();
            header
        })
        .collect();

    LandingRange {
        headers,
        expected,
        context: CoverageContext {
            parent_height: PARENT_HEIGHT,
            last_height: PARENT_HEIGHT + schedule.len() as u64,
            max_blocks: schedule.len(),
        },
        domain,
    }
}

fn signed_commit(header: &Header, committee: &Committee) -> Commit {
    signed_slots(header, committee, [Slot::Block; 2])
}

#[derive(Clone, Copy)]
enum Slot {
    Block,
    Nil,
    Absent,
}

fn signed_slots(header: &Header, committee: &Committee, slots: [Slot; 2]) -> Commit {
    let id = Id {
        hash: header_hash(header),
        part_set_header: parts::Header::new(1, Hash::Sha256([0x77; 32])).unwrap(),
    };
    let mut raw = RawCommit {
        height: header.height.into(),
        round: 2,
        block_id: Some(id.into()),
        signatures: vec![],
    };
    for (index, validator) in committee.validators.validators().iter().enumerate() {
        let (flag, block_id) = match slots[index] {
            Slot::Block => (2, Some(id)),
            Slot::Nil => (3, None),
            Slot::Absent => {
                raw.signatures.push(CommitSig::BlockIdFlagAbsent.into());
                continue;
            }
        };
        let vote = Vote {
            vote_type: Type::Precommit,
            height: header.height,
            round: 2u8.into(),
            block_id,
            timestamp: Some(header.time),
            validator_address: validator.address,
            validator_index: u32::try_from(index).unwrap().try_into().unwrap(),
            signature: None,
            extension: vec![],
            extension_signature: None,
        };
        let bytes = Protobuf::<RawVote>::encode_length_delimited_vec(CanonicalVote::new(
            vote,
            header.chain_id.clone(),
        ));
        let key = committee
            .keys
            .iter()
            .find(|key| {
                key.verifying_key().to_encoded_point(true).as_bytes()
                    == validator.pub_key.as_bytes()
            })
            .unwrap();
        let (signature, recovery) = key
            .sign_prehash_recoverable(keccak256(bytes).as_slice())
            .unwrap();
        let mut signature = signature.to_bytes().to_vec();
        signature.push(recovery.to_byte());
        raw.signatures.push(RawCommitSig {
            block_id_flag: flag,
            validator_address: validator.address.as_bytes().to_vec(),
            timestamp: Some(header.time.into()),
            signature,
        });
    }
    raw.try_into().unwrap()
}

#[test]
fn stable_range_accepts_only_its_signed_terminal_commit() {
    let a = committee(&[(1, 7), (2, 3)]);
    let range = landing_range(&[&a.validators; 3], &a.validators);
    let header = &range.headers[2];
    let commit = signed_commit(header, &a);
    verify_native_commit(&commit, header, &a.validators, header.chain_id.as_str())
        .expect("the terminal fixture has a valid native certificate");

    let requirements = range
        .verify(&[commit])
        .expect("the stable range only needs its signed terminal commit");
    assert_eq!(requirements.required_commit_heights(), &[43]);
}

fn recurring_range<'a>(a: &'a Committee, b: &'a Committee) -> LandingRange<'a> {
    let a = &a.validators;
    let b = &b.validators;
    // The required heights are 42, 44 and 46. The terminal header also binds
    // the successor set B at height 47, with no certificate needed at 47.
    landing_range(&[a, a, b, b, a, a], b)
}

fn recurring_commits(range: &LandingRange<'_>, a: &Committee, b: &Committee) -> [Commit; 3] {
    [
        signed_commit(&range.headers[1], a),
        signed_commit(&range.headers[3], b),
        signed_commit(&range.headers[5], a),
    ]
}

#[test]
fn recurring_active_set_requires_both_outgoing_commits_and_the_terminal() {
    let a = committee(&[(1, 7), (2, 3)]);
    let b = committee(&[(3, 8), (4, 2)]);
    let range = recurring_range(&a, &b);
    let commits = recurring_commits(&range, &a, &b);
    assert_eq!(
        range.verify(&commits).unwrap().required_commit_heights(),
        &[42, 44, 46],
    );
    assert!(matches!(
        range.verify(&commits[1..]),
        Err(CommitVerificationError::CommitCount {
            expected: 3,
            actual: 2,
        })
    ));
    let extra = [
        commits[0].clone(),
        signed_commit(&range.headers[2], &b),
        commits[1].clone(),
        commits[2].clone(),
    ];
    assert!(matches!(
        range.verify(&extra),
        Err(CommitVerificationError::CommitCount {
            expected: 3,
            actual: 4,
        })
    ));
}

#[test]
fn every_commit_target_is_checked_before_any_certificate() {
    let a = committee(&[(1, 7), (2, 3)]);
    let b = committee(&[(3, 8), (4, 2)]);
    let range = recurring_range(&a, &b);
    let [mut first, second, terminal] = recurring_commits(&range, &a, &b);
    first.signatures.clear();
    assert!(matches!(
        range.verify(&[first.clone(), second.clone(), terminal.clone()]),
        Err(CommitVerificationError::Certificate { height: 42, .. })
    ));
    let duplicate = [first.clone(), second.clone(), second.clone()];
    assert!(matches!(
        range.verify(&duplicate),
        Err(CommitVerificationError::CommitHeight {
            expected: 46,
            actual: 44,
        })
    ));
    let reordered = [first, terminal.clone(), second.clone()];
    assert!(matches!(
        range.verify(&reordered),
        Err(CommitVerificationError::CommitHeight {
            expected: 44,
            actual: 46,
        })
    ));
    let interior_substitution = [signed_commit(&range.headers[0], &a), second, terminal];
    assert!(matches!(
        range.verify(&interior_substitution),
        Err(CommitVerificationError::CommitHeight {
            expected: 42,
            actual: 41,
        })
    ));
}

#[test]
fn rejects_valid_signatures_over_a_different_target_header() {
    let a = committee(&[(1, 7), (2, 3)]);
    let range = landing_range(&[&a.validators; 3], &a.validators);
    let mut other = range.headers[2].clone();
    other.app_hash = vec![0x99; 7].try_into().unwrap();
    let commit = signed_commit(&other, &a);
    verify_native_commit(&commit, &other, &a.validators, other.chain_id.as_str()).unwrap();
    assert!(matches!(
        range.verify(&[commit]),
        Err(CommitVerificationError::Certificate { height: 43, .. })
    ));
}

#[test]
fn caller_domain_rejects_headers_and_commits_signed_for_other_deployments() {
    let a = committee(&[(1, 7), (2, 3)]);
    let mut range = landing_range(&[&a.validators; 3], &a.validators);
    let domain = range.domain;
    let upper_bits = U256::from(1) << 200;
    for foreign in [
        DeploymentDomain {
            generation: domain.generation + 1,
            ..domain
        },
        DeploymentDomain {
            l1_chain_id: domain.l1_chain_id + upper_bits,
            ..domain
        },
        DeploymentDomain {
            l2_chain_id: domain.l2_chain_id + upper_bits,
            ..domain
        },
        DeploymentDomain {
            inbox: Address::repeat_byte(0x22),
            ..domain
        },
    ] {
        let header = &mut range.headers[2];
        header.chain_id = chain_id(&foreign).unwrap().parse().unwrap();
        let commit = signed_commit(header, &a);
        verify_native_commit(&commit, header, &a.validators, header.chain_id.as_str()).unwrap();
        assert!(
            matches!(
                range.verify(&[commit]),
                Err(CommitVerificationError::Certificate { height: 43, .. })
            ),
            "a certificate in the foreign signing domain {foreign:?} must fail"
        );
    }
}

#[test]
fn reports_unrepresentable_caller_generation() {
    let a = committee(&[(1, 7), (2, 3)]);
    let mut range = landing_range(&[&a.validators; 3], &a.validators);
    let commit = signed_commit(&range.headers[2], &a);
    range.domain.generation = 1 << 48;
    assert!(matches!(
        range.verify(&[commit]),
        Err(CommitVerificationError::Domain(DomainError::Generation(generation)))
            if generation == 1 << 48
    ));
}

#[test]
fn native_commit_must_use_the_authorized_outgoing_set() {
    let a = committee(&[(1, 7), (2, 3)]);
    let b = committee(&[(3, 8), (4, 2)]);
    let c = committee(&[(5, 7), (6, 3)]);
    // Headers and schedule agree on C -> B -> A. At 42, the certificate
    // binds that header's hash but carries signatures from A instead of C.
    let range = landing_range(
        &[
            &c.validators,
            &c.validators,
            &b.validators,
            &b.validators,
            &a.validators,
            &a.validators,
        ],
        &b.validators,
    );
    let mut commits = recurring_commits(&range, &a, &b);
    assert!(matches!(
        range.verify(&commits),
        Err(CommitVerificationError::Certificate { height: 42, .. })
    ));
    commits[0] = signed_commit(&range.headers[1], &c);
    assert_eq!(
        range.verify(&commits).unwrap().required_commit_heights(),
        &[42, 44, 46],
    );
}

#[test]
fn only_block_votes_count_towards_strictly_more_than_two_thirds_power() {
    for other_vote in [Slot::Nil, Slot::Absent] {
        let majority = committee(&[(1, 7), (2, 3)]);
        let range = landing_range(&[&majority.validators], &majority.validators);
        let commit = signed_slots(&range.headers[0], &majority, [Slot::Block, other_vote]);
        assert_eq!(
            range.verify(&[commit]).unwrap().required_commit_heights(),
            &[41],
        );

        let exact = committee(&[(1, 2), (2, 1)]);
        let range = landing_range(&[&exact.validators], &exact.validators);
        let commit = signed_slots(&range.headers[0], &exact, [Slot::Block, other_vote]);
        assert!(matches!(
            range.verify(&[commit]),
            Err(CommitVerificationError::Certificate { height: 41, .. })
        ));
    }
}

#[test]
fn rejects_malformed_nil_signature_even_with_enough_block_voting_power() {
    let a = committee(&[(1, 7), (2, 3)]);
    let range = landing_range(&[&a.validators], &a.validators);
    let commit = signed_slots(&range.headers[0], &a, [Slot::Block, Slot::Nil]);
    let mut raw = RawCommit::from(commit);
    // Retain the native signature width and invalidate the recovery byte.
    raw.signatures[1].signature[64] = u8::MAX;
    let malformed = raw.try_into().unwrap();
    assert!(matches!(
        range.verify(&[malformed]),
        Err(CommitVerificationError::Certificate { height: 41, .. })
    ));
}

#[test]
fn rejects_a_mutated_block_vote_at_the_second_outgoing_height() {
    let a = committee(&[(1, 7), (2, 3)]);
    let b = committee(&[(3, 8), (4, 2)]);
    let range = recurring_range(&a, &b);
    let [first, second, terminal] = recurring_commits(&range, &a, &b);
    let mut raw = RawCommit::from(second);
    raw.signatures[0].signature[12] ^= 1;
    assert!(matches!(
        range.verify(&[first, raw.try_into().unwrap(), terminal]),
        Err(CommitVerificationError::Certificate { height: 44, .. })
    ));
}

#[test]
fn checks_active_and_successor_hashes_on_the_complete_range_first() {
    let a = committee(&[(1, 7), (2, 3)]);
    let b = committee(&[(3, 8), (4, 2)]);
    let mut range = recurring_range(&a, &b);
    range.headers[0].validators_hash = b.validators.hash();
    assert!(matches!(
        range.verify(&[]),
        Err(CommitVerificationError::Schedule(
            CoverageError::ActiveHash { height: 41 }
        ))
    ));
    range.headers[0].validators_hash = a.validators.hash();
    range.headers[5].next_validators_hash = a.validators.hash();
    assert!(matches!(
        range.verify(&[]),
        Err(CommitVerificationError::Schedule(CoverageError::NextHash {
            height: 46
        }))
    ));
    range.headers[5].next_validators_hash = b.validators.hash();
    range.expected[0].next = &b.validators;
    range.headers[0].next_validators_hash = b.validators.hash();
    assert!(matches!(
        range.verify(&[]),
        Err(CommitVerificationError::Schedule(
            CoverageError::ScheduleLink { height: 42 }
        ))
    ));
}

#[test]
fn rejects_a_truncated_range_with_the_original_endpoints() {
    let a = committee(&[(1, 7), (2, 3)]);
    let mut range = landing_range(&[&a.validators; 3], &a.validators);
    let truncated_terminal = signed_commit(&range.headers[1], &a);
    range.headers.pop();
    range.expected.pop();
    assert!(matches!(
        range.verify(&[truncated_terminal]),
        Err(CommitVerificationError::Schedule(
            CoverageError::HeaderCount {
                expected: 3,
                actual: 2,
            }
        ))
    ));
}

#[test]
fn pinned_checker_accepts_opaque_app_hash_and_rejects_changed_app_version() {
    let a = committee(&[(1, 7), (2, 3)]);
    let mut range = landing_range(&[&a.validators], &a.validators);
    range.headers[0].app_hash = vec![0xe1; 7].try_into().unwrap();
    range.headers[0].version.app = 1;
    let commit = signed_commit(&range.headers[0], &a);
    assert_eq!(
        range.verify(&[commit]).unwrap().required_commit_heights(),
        &[41],
    );

    // Sign the mutated header so the inherited version policy, rather than
    // a different header hash or invalid signature, causes the rejection.
    range.headers[0].version.app = 2;
    let commit = signed_commit(&range.headers[0], &a);
    assert!(matches!(
        range.verify(&[commit]),
        Err(CommitVerificationError::Certificate {
            height: 41,
            source: BftError::Certificate(reason),
        }) if reason.contains("APP_VERSION")
    ));
}
