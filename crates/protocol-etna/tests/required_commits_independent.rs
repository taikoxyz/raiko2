//! Adversarial checks of the range wrapper at the native certificate boundary.
use alloy_primitives::{Address, U256, hex, keccak256};
use etna_core::{
    comet::{
        codec::decode_canonical_block,
        native::{Commit, CommitSig},
        validators::Validators,
        verify::{BftError, header_hash, verify_native_commit},
    },
    domain::{DeploymentDomain, DomainError, chain_id},
    key::ConsensusKey,
    params::UINT48_MAX,
};
use k256::ecdsa::SigningKey;
use raiko2_protocol_etna::coverage::{
    CommitVerificationError, CoverageContext, CoverageError, ExpectedValidatorSets,
    verify_required_native_commits,
};
use tendermint::{
    Hash, Vote,
    block::{Header, Id, parts},
    chain,
    vote::{CanonicalVote, Type},
};
use tendermint_proto::{Protobuf, v0_38::types::CanonicalVote as RawVote};

fn domain() -> DeploymentDomain {
    DeploymentDomain {
        l1_chain_id: U256::from(7) + (U256::from(1) << 208),
        l2_chain_id: U256::from(17) + (U256::from(1) << 192),
        inbox: Address::repeat_byte(0xb1),
        generation: 9,
    }
}

struct Signers {
    keys: [SigningKey; 3],
    validators: Validators,
}

impl Signers {
    fn new(seeds: [u8; 3], powers: [u64; 3]) -> Self {
        let keys = seeds.map(|seed| SigningKey::from_slice(&[seed; 32]).unwrap());
        let members = std::array::from_fn::<_, 3, _>(|index| {
            (
                ConsensusKey::from_slice(
                    keys[index]
                        .verifying_key()
                        .to_encoded_point(true)
                        .as_bytes(),
                )
                .unwrap(),
                powers[index],
            )
        });
        let validators = Validators::from_members(&members).unwrap();
        Self { keys, validators }
    }
}

#[derive(Clone, Copy)]
enum Slot {
    Block,
    Nil,
    Absent,
}

fn signed(
    header: &Header,
    signature_domain: &chain::Id,
    signers: &Signers,
    slots: [Slot; 3],
) -> Commit {
    let block_id = Id {
        hash: header_hash(header),
        part_set_header: parts::Header::new(3, Hash::Sha256([0x2f; 32])).unwrap(),
    };
    let round = 2u8.into();
    let signatures = signers
        .validators
        .validators()
        .iter()
        .zip(slots)
        .enumerate()
        .map(|(index, (validator, slot))| {
            if matches!(slot, Slot::Absent) {
                return CommitSig::BlockIdFlagAbsent;
            }
            let is_block = matches!(slot, Slot::Block);
            let vote = Vote {
                vote_type: Type::Precommit,
                height: header.height,
                round,
                block_id: is_block.then_some(block_id),
                timestamp: Some(header.time),
                validator_address: validator.address,
                validator_index: u32::try_from(index).unwrap().try_into().unwrap(),
                signature: None,
                extension: Vec::new(),
                extension_signature: None,
            };
            let sign_bytes = Protobuf::<RawVote>::encode_length_delimited_vec(CanonicalVote::new(
                vote,
                signature_domain.clone(),
            ));
            let key = signers
                .keys
                .iter()
                .find(|key| {
                    key.verifying_key().to_encoded_point(true).as_bytes()
                        == validator.pub_key.as_bytes()
                })
                .unwrap();
            let (signature, recovery) = key
                .sign_prehash_recoverable(keccak256(sign_bytes).as_slice())
                .unwrap();
            let mut bytes = signature.to_bytes().to_vec();
            bytes.push(recovery.to_byte());
            let signature = Some(bytes.try_into().unwrap());
            if is_block {
                CommitSig::BlockIdFlagCommit {
                    validator_address: validator.address,
                    timestamp: header.time,
                    signature,
                }
            } else {
                CommitSig::BlockIdFlagNil {
                    validator_address: validator.address,
                    timestamp: header.time,
                    signature,
                }
            }
        })
        .collect();
    Commit {
        height: header.height,
        round,
        block_id,
        signatures,
    }
}

const fn sets<'a>(active: &'a Signers, next: &'a Signers) -> ExpectedValidatorSets<'a> {
    ExpectedValidatorSets {
        active: &active.validators,
        next: &next.validators,
    }
}

fn header(height: u64, schedule: ExpectedValidatorSets<'_>, domain: DeploymentDomain) -> Header {
    let bytes = hex::decode(include_str!("fixtures/first-block.hex").trim()).unwrap();
    let (_, block) = decode_canonical_block(&bytes).unwrap();
    let mut h = block.header;
    h.height = height.try_into().unwrap();
    h.chain_id = chain_id(&domain).unwrap().parse().unwrap();
    h.validators_hash = schedule.active.hash();
    h.next_validators_hash = schedule.next.hash();
    // The native QC binds these opaque bytes through the header hash.
    h.app_hash = vec![0xab, 0xcd, 0xef].try_into().unwrap();
    h
}

fn all_block(h: &Header, signers: &Signers) -> Commit {
    signed(h, &h.chain_id, signers, [Slot::Block; 3])
}

#[test]
fn recurring_committee_checks_both_outgoing_ends_and_preflights_the_entire_target_list() {
    let a = Signers::new([1, 2, 3], [9, 2, 1]);
    let b = Signers::new([4, 5, 6], [9, 2, 1]);
    let schedule = [
        sets(&a, &a),
        sets(&a, &b),
        sets(&b, &b),
        sets(&b, &a),
        sets(&a, &a),
    ];
    let domain = domain();
    let headers = std::array::from_fn::<_, 5, _>(|i| {
        header(41 + u64::try_from(i).unwrap(), schedule[i], domain)
    });
    let context = CoverageContext {
        parent_height: 40,
        last_height: 45,
        max_blocks: 5,
    };
    let [outgoing_a, outgoing_b, terminal] = [
        all_block(&headers[1], &a),
        all_block(&headers[3], &b),
        all_block(&headers[4], &a),
    ];
    for (qc, index) in [(&outgoing_a, 1), (&outgoing_b, 3), (&terminal, 4)] {
        verify_native_commit(
            qc,
            &headers[index],
            schedule[index].active,
            headers[index].chain_id.as_str(),
        )
        .unwrap();
    }
    let valid = [outgoing_a.clone(), outgoing_b.clone(), terminal.clone()];
    assert_eq!(
        verify_required_native_commits(&headers, &schedule, &context, &valid, &domain)
            .unwrap()
            .required_commit_heights(),
        &[42, 44, 45]
    );

    assert!(matches!(
        verify_required_native_commits(
            &headers,
            &schedule,
            &context,
            &[outgoing_b.clone(), terminal.clone()],
            &domain,
        ),
        Err(CommitVerificationError::CommitCount {
            expected: 3,
            actual: 2
        })
    ));

    // The first certificate will fail native verification. A duplicated later height
    // must be reported before any QC is checked.
    let mut invalid_first = outgoing_a;
    invalid_first.signatures.clear();
    assert!(matches!(
        verify_required_native_commits(
            &headers,
            &schedule,
            &context,
            &[invalid_first, outgoing_b.clone(), outgoing_b],
            &domain,
        ),
        Err(CommitVerificationError::CommitHeight {
            expected: 45,
            actual: 44
        })
    ));

    // These signatures certify a different header hash at the correct target height.
    let mut other_header = headers[3].clone();
    other_header.app_hash = vec![0x42].try_into().unwrap();
    let mut wrong_header = valid.clone();
    wrong_header[1] = all_block(&other_header, &b);
    verify_native_commit(
        &wrong_header[1],
        &other_header,
        &b.validators,
        other_header.chain_id.as_str(),
    )
    .unwrap();
    assert!(matches!(
        verify_required_native_commits(&headers, &schedule, &context, &wrong_header, &domain),
        Err(CommitVerificationError::Certificate { height: 44, .. })
    ));

    // The BlockID and domain are unchanged, but the wrong committee supplies all signatures.
    let mut wrong_signers = valid.clone();
    wrong_signers[1] = all_block(&headers[3], &a);
    assert!(matches!(
        verify_required_native_commits(&headers, &schedule, &context, &wrong_signers, &domain),
        Err(CommitVerificationError::Certificate { height: 44, .. })
    ));

    // The block with no required QC still needs both schedule commitments.
    let mut incomplete = headers.clone();
    incomplete[2].next_validators_hash = a.validators.hash();
    assert!(matches!(
        verify_required_native_commits(&incomplete, &schedule, &context, &[], &domain),
        Err(CommitVerificationError::Schedule(CoverageError::NextHash {
            height: 43
        }))
    ));
}

#[test]
fn checks_the_full_deployment_domain_in_both_header_and_vote_signature() {
    let a = Signers::new([1, 2, 3], [9, 2, 1]);
    let expected = [sets(&a, &a)];
    let domain = domain();
    let header = header(UINT48_MAX, expected[0], domain);
    let context = CoverageContext {
        parent_height: UINT48_MAX - 1,
        last_height: UINT48_MAX,
        max_blocks: 1,
    };
    let canonical = all_block(&header, &a);
    let headers = [header];
    verify_required_native_commits(
        &headers,
        &expected,
        &context,
        std::slice::from_ref(&canonical),
        &domain,
    )
    .unwrap();
    for alternate in [
        DeploymentDomain {
            l1_chain_id: U256::from(7),
            ..domain
        },
        DeploymentDomain {
            l2_chain_id: U256::from(17),
            ..domain
        },
        DeploymentDomain {
            inbox: Address::repeat_byte(0xb0),
            ..domain
        },
        DeploymentDomain {
            generation: domain.generation + 1,
            ..domain
        },
    ] {
        assert!(matches!(
            verify_required_native_commits(
                &headers,
                &expected,
                &context,
                std::slice::from_ref(&canonical),
                &alternate,
            ),
            Err(CommitVerificationError::Certificate {
                height: UINT48_MAX,
                ..
            })
        ));
        let alternate_id = chain_id(&alternate).unwrap().parse().unwrap();
        let wrong_votes = signed(&headers[0], &alternate_id, &a, [Slot::Block; 3]);
        assert_eq!(canonical.block_id, wrong_votes.block_id);
        assert!(matches!(
            verify_required_native_commits(&headers, &expected, &context, &[wrong_votes], &domain),
            Err(CommitVerificationError::Certificate {
                height: UINT48_MAX,
                source: BftError::Certificate(message),
            }) if message.contains("signature verification failed")
        ));
    }
    assert!(matches!(
        verify_required_native_commits(
            &headers,
            &expected,
            &context,
            &[canonical],
            &DeploymentDomain {
                generation: UINT48_MAX + 1,
                ..domain
            },
        ),
        Err(CommitVerificationError::Domain(DomainError::Generation(_)))
    ));
}

#[test]
fn preserves_the_pinned_version_gate_and_strict_weighted_slot_policy() {
    let domain = domain();
    let p = CoverageContext {
        parent_height: 9,
        last_height: 10,
        max_blocks: 1,
    };
    let exact = Signers::new([1, 2, 3], [4, 1, 1]);
    let over = Signers::new([1, 2, 3], [5, 1, 1]);
    for (committee, fails) in [(&exact, true), (&over, false)] {
        let expected = [sets(committee, committee)];
        let h = header(10, expected[0], domain);
        let qc = signed(
            &h,
            &h.chain_id,
            committee,
            [Slot::Block, Slot::Nil, Slot::Absent],
        );
        let direct = verify_native_commit(&qc, &h, &committee.validators, h.chain_id.as_str());
        let wrapped = verify_required_native_commits(
            std::slice::from_ref(&h),
            &expected,
            &p,
            std::slice::from_ref(&qc),
            &domain,
        );
        if fails {
            assert!(
                matches!(direct, Err(BftError::Certificate(message)) if message.contains("4/6"))
            );
            assert!(matches!(
                wrapped,
                Err(CommitVerificationError::Certificate { height: 10, source: BftError::Certificate(message) })
                if message.contains("4/6")
            ));
            continue;
        }
        direct.unwrap();
        wrapped.unwrap();
        // Even though the block supporter alone is sufficient, all present slots
        // must be authenticated and must occupy their canonical indices.
        let mut bad_nil = qc.clone();
        let CommitSig::BlockIdFlagNil { signature, .. } = &mut bad_nil.signatures[1] else {
            panic!("fixture has no nil vote");
        };
        *signature = None;
        let mut wrong_slots = qc;
        wrong_slots.signatures.swap(0, 2);
        for bad in [bad_nil, wrong_slots] {
            assert!(matches!(
                verify_required_native_commits(
                    std::slice::from_ref(&h),
                    &expected,
                    &p,
                    &[bad],
                    &domain,
                ),
                Err(CommitVerificationError::Certificate { height: 10, .. })
            ));
        }

        // Re-sign the changed version so this failure cannot come from a stale header hash.
        let mut wrong_app = h;
        wrong_app.version.app += 1;
        let wrong_app_qc = all_block(&wrong_app, committee);
        assert!(matches!(
            verify_required_native_commits(&[wrong_app], &expected, &p, &[wrong_app_qc], &domain),
            Err(CommitVerificationError::Certificate { height: 10, source: BftError::Certificate(message) })
            if message.contains("APP_VERSION")
        ));
    }
}

// Append to tests/required_commits_independent.rs: reuses its input-only test fixture helpers.
#[test]
fn independent_fixed_total_exact_two_thirds_rejects_and_one_more_unit_accepts() {
    let signers = Signers::new([1, 2, 3], [4, 1, 1]);
    let domain = domain();
    let expected = [sets(&signers, &signers)];
    let h = header(10, expected[0], domain);
    let context = CoverageContext {
        parent_height: 9,
        last_height: 10,
        max_blocks: 1,
    };
    let exact = signed(
        &h,
        &h.chain_id,
        &signers,
        [Slot::Block, Slot::Absent, Slot::Absent],
    );
    assert!(
        verify_required_native_commits(
            std::slice::from_ref(&h),
            &expected,
            &context,
            std::slice::from_ref(&exact),
            &domain
        )
        .is_err()
    );
    let plus_one = signed(
        &h,
        &h.chain_id,
        &signers,
        [Slot::Block, Slot::Block, Slot::Absent],
    );
    verify_required_native_commits(
        std::slice::from_ref(&h),
        &expected,
        &context,
        &[plus_one],
        &domain,
    )
    .unwrap();
    let mut duplicate = exact;
    duplicate.signatures[1] = duplicate.signatures[0].clone();
    assert!(
        verify_required_native_commits(&[h], &expected, &context, &[duplicate], &domain).is_err()
    );
}

#[test]
fn independent_signatures_bind_both_full_block_id_part_set_fields() {
    let signers = Signers::new([1, 2, 3], [4, 1, 1]);
    let domain = domain();
    let expected = [sets(&signers, &signers)];
    let h = header(10, expected[0], domain);
    let context = CoverageContext {
        parent_height: 9,
        last_height: 10,
        max_blocks: 1,
    };
    let qc = all_block(&h, &signers);
    for part_set in [
        parts::Header::new(4, qc.block_id.part_set_header.hash).unwrap(),
        parts::Header::new(3, Hash::Sha256([0x30; 32])).unwrap(),
    ] {
        let mut changed = qc.clone();
        changed.block_id.part_set_header = part_set;
        assert!(
            verify_required_native_commits(
                std::slice::from_ref(&h),
                &expected,
                &context,
                &[changed],
                &domain
            )
            .is_err()
        );
    }
}
