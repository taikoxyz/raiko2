//! Synthetic-context end-to-end fixtures with genuine EVM execution, MPT proofs and native QCs.
//! The tiny staking program is test-only and is not EtnaStaking or devnet acceptance evidence.
use super::*;
use crate::etna::{
    verify_blob_range_statement_consensus, ConsensusParameters, ConsensusWitness,
    L1CommitteeWitness, ParentConsensusWitness,
};
use etna_core::{
    comet::{
        native::{Commit, CommitSig, EvidenceList, RawBlock, RawCommit},
        validators::Validators,
        verify::{block_parts, commit_hash},
    },
    fallback::FallbackKeys,
    key::ConsensusKey,
    params::config_hash,
    staking::{key_to_seat_slot, record_slots, snapshot_height_slot, snapshot_total_stake_slot},
};
use k256::ecdsa::SigningKey;
use prost::Message;
use raiko2_protocol_etna::consensus::origin_slots;
use tendermint::{
    block::Id,
    vote::{CanonicalVote, Type},
    Vote,
};
use tendermint_proto::{
    v0_38::types::{CanonicalVote as RawVote, Data},
    Protobuf,
};

fn signing_key(seed: u8) -> SigningKey {
    SigningKey::from_slice(&[seed; 32]).unwrap()
}
fn consensus_key(seed: u8) -> ConsensusKey {
    ConsensusKey::from_slice(
        signing_key(seed)
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes(),
    )
    .unwrap()
}
fn keys() -> FallbackKeys {
    let mut keys = vec![consensus_key(1), consensus_key(2), consensus_key(3)];
    keys.sort();
    FallbackKeys::new(keys).unwrap()
}
fn parameters() -> ConsensusParameters {
    ConsensusParameters {
        staking: STAKING,
        max_uncheckpointed: 128,
        max_block_bytes: 2_097_152,
        max_commit_bytes: 524_288,
    }
}

fn signed_commit(header: &tendermint::block::Header, id: Id, set: &Validators) -> Commit {
    let signatures = set
        .validators()
        .iter()
        .enumerate()
        .map(|(index, validator)| {
            let vote = Vote {
                vote_type: Type::Precommit,
                height: header.height,
                round: 1u8.into(),
                block_id: Some(id),
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
            let key = (1..=4)
                .map(signing_key)
                .find(|key| {
                    key.verifying_key().to_encoded_point(true).as_bytes()
                        == validator.pub_key.as_bytes()
                })
                .unwrap();
            let (signature, recovery) = key
                .sign_prehash_recoverable(keccak256(bytes).as_slice())
                .unwrap();
            let mut bytes = signature.to_bytes().to_vec();
            bytes.push(recovery.to_byte());
            CommitSig::BlockIdFlagCommit {
                validator_address: validator.address,
                timestamp: header.time,
                signature: Some(bytes.try_into().unwrap()),
            }
        })
        .collect();
    Commit {
        height: header.height,
        round: 1u8.into(),
        block_id: id,
        signatures,
    }
}

#[derive(Clone, Copy)]
enum Candidate {
    Valid,
    Stale,
    Invalid,
    SameAsFallback,
    FullWidthHeight,
}

struct Fixture {
    configured_staking: Address,
    execution: ExecutingRange,
    fallback: FallbackKeys,
    l1: ExecutionWitness,
    originals: Vec<Vec<u8>>,
    commits: Vec<Vec<u8>>,
    all_commits: Vec<Commit>,
    sets: Vec<Validators>,
}

impl Fixture {
    fn new(count: usize, candidate: Candidate, minimum: U256, missing: bool) -> Self {
        Self::with_overrides(count, candidate, minimum, missing, None, U256::from(0x0701))
    }

    fn with_overrides(
        count: usize,
        candidate: Candidate,
        minimum: U256,
        missing: bool,
        inbox_override: Option<(U256, U256)>,
        seat_word: U256,
    ) -> Self {
        let fallback = keys();
        let params = parameters();
        let mut range = RangeFixture::new(1);
        let inbox = range.context.domain.inbox;
        let mut storage = Trie::default();
        let mut words = vec![
            (U256::from(268), U256::from(range.context.domain.generation)),
            (
                U256::from(269),
                U256::from_be_bytes(config_hash(167_001, STAKING, params.max_uncheckpointed).0),
            ),
            (U256::from(278), U256::from_be_bytes(fallback.hash().0)),
            (U256::from(279), minimum),
        ];
        words.extend(
            origin_slots(range.context.domain.generation)
                .into_iter()
                .zip([
                    U256::ZERO,
                    U256::from(18),
                    U256::from(100),
                    U256::from_be_bytes(fallback.hash().0),
                ])
                .map(|(slot, value)| (U256::from_be_bytes(slot.0), value)),
        );
        if let Some((slot, value)) = inbox_override {
            if let Some(word) = words.iter_mut().find(|word| word.0 == slot) {
                word.1 = value;
            } else {
                words.push((slot, value));
            }
        }
        for &(slot, value) in &words {
            if !value.is_zero() {
                storage.insert(keccak256(B256::from(slot)), alloy_rlp::encode(value));
            }
        }
        let mut state = Trie::default();
        state.insert(
            keccak256(inbox),
            alloy_rlp::encode(TrieAccount {
                nonce: 1,
                balance: U256::ZERO,
                storage_root: storage.hash_slow(),
                code_hash: keccak256([]),
            }),
        );
        let root = state.hash_slow();
        let l1 = ExecutionWitness {
            state: state
                .rlp_nodes()
                .into_iter()
                .chain(storage.rlp_nodes())
                .map(WitnessStateNode::from_bytes)
                .collect(),
            ..Default::default()
        };
        range.l1_headers = vec![raw_l1_header(18, B256::ZERO, root)];
        let anchor_box = list(&[&alloy_rlp::encode(range.l1_headers[0].as_slice()), &[0xc0]]);
        range.blocks[0].anchor_witness = Some(anchor_box);
        range.blocks[0].execution.parent_beacon_block_root = Some(root);
        while range.blocks.len() < count {
            let template = &range.blocks[0];
            let mut comet = template.comet.clone();
            comet.header.last_block_id = Some(Id::default());
            range.blocks.push(BlockFixture {
                comet,
                execution: template.execution.clone(),
                transactions: vec![],
                item: vec![],
                anchor_witness: None,
            });
        }
        range.context.max_blocks = count;
        if missing {
            for block in range.blocks.iter_mut().skip(1) {
                block.execution.extra_data = {
                    let mut extra = block.execution.extra_data.to_vec();
                    extra[13..].copy_from_slice(&(1u128 << 7).to_be_bytes());
                    extra.into()
                };
            }
        }
        let fallback_set = Validators::from_members(&fallback.validator_set()).unwrap();
        let mut staking_storage = vec![];
        let candidate_members = match candidate {
            Candidate::SameAsFallback => fallback.validator_set(),
            _ => vec![(consensus_key(4), 5)],
        };
        for (seat, (key, power)) in candidate_members.iter().enumerate() {
            let slots = record_slots(u8::try_from(seat).unwrap());
            staking_storage.push((
                U256::from_be_bytes(slots[0].0),
                U256::from_be_bytes(key.x().0),
            ));
            let mut packed = U256::from(key.prefix()) | (U256::from(*power) << 8);
            if matches!(candidate, Candidate::Invalid) {
                packed |= U256::from(1) << 80;
            }
            staking_storage.push((U256::from_be_bytes(slots[1].0), packed));
        }
        // Every fallback member maps to the same seat so flags can be changed without changing
        // the expected bitmap; live contracts would enforce their own seat assignment rules.
        for key in fallback.keys() {
            staking_storage.push((
                U256::from_be_bytes(key_to_seat_slot(key.address()).0),
                seat_word,
            ));
        }
        staking_storage.push((
            U256::from_be_bytes(key_to_seat_slot(consensus_key(4).address()).0),
            seat_word,
        ));
        let mut code = match candidate {
            Candidate::Stale => vec![0x60, 0],
            Candidate::FullWidthHeight => {
                let mut code = vec![0x43, 0x7f];
                code.extend_from_slice(&(U256::from(1) << 200usize).to_be_bytes::<32>());
                code.push(0x17); // OR preserves the real low height but adds an invalid high bit.
                code
            }
            _ => vec![0x43], // NUMBER is stored by actual block-start EVM execution.
        };
        code.push(0x7f);
        code.extend_from_slice(snapshot_height_slot().as_slice());
        code.extend_from_slice(&[0x55, 0x60, 100, 0x7f]);
        code.extend_from_slice(snapshot_total_stake_slot().as_slice());
        code.extend_from_slice(&[0x55, 0]);
        let execution =
            ExecutingRange::from_range(range, false, Some(code.into()), &staking_storage);
        let candidate_set = if minimum > U256::from(100)
            || matches!(
                candidate,
                Candidate::Stale | Candidate::Invalid | Candidate::FullWidthHeight
            ) {
            fallback_set.clone()
        } else {
            Validators::from_members(&candidate_members).unwrap()
        };
        let sets = (0..count + 1)
            .map(|index| {
                if index < 2 {
                    fallback_set.clone()
                } else {
                    candidate_set.clone()
                }
            })
            .collect::<Vec<_>>();
        let mut fixture = Self {
            configured_staking: STAKING,
            execution,
            fallback,
            l1,
            originals: vec![],
            commits: vec![],
            all_commits: vec![],
            sets,
        };
        fixture.seal_consensus(missing);
        fixture
    }

    fn seal_consensus(&mut self, missing: bool) {
        self.originals.clear();
        self.all_commits.clear();
        self.commits.clear();
        let range = &mut self.execution.authenticated.range;
        let mut previous: Option<Commit> = None;
        for (index, block) in range.blocks.iter_mut().enumerate() {
            block.comet.header.validators_hash = self.sets[index].hash();
            block.comet.header.next_validators_hash = self.sets[index + 1].hash();
            let mut last = previous.clone();
            if missing {
                if let Some(last) = &mut last {
                    last.signatures[0] = CommitSig::BlockIdFlagAbsent;
                }
            }
            block.comet.last_commit = last.clone();
            block.comet.header.last_block_id = last.as_ref().map(|last| last.block_id);
            block.comet.header.last_commit_hash =
                Some(last.as_ref().map_or_else(|| data_hash(&[]), commit_hash));
            block.comet.header.evidence_hash = Some(data_hash(&[]));
            let raw = RawBlock {
                header: Some(block.comet.header.clone().into()),
                data: Some(Data {
                    txs: block.comet.data.clone(),
                }),
                evidence: Some(EvidenceList { evidence: vec![] }),
                last_commit: last.map(RawCommit::from),
            };
            let id = Id {
                hash: header_hash(&block.comet.header),
                part_set_header: block_parts(&raw).unwrap(),
            };
            let commit = signed_commit(&block.comet.header, id, &self.sets[index]);
            previous = Some(commit.clone());
            self.all_commits.push(commit);
            self.originals.push(raw.encode_to_vec());
        }
        range.context.last_comet_hash =
            B256::from_slice(header_hash(&range.blocks.last().unwrap().comet.header).as_bytes());
        for index in 0..range.blocks.len() {
            if index + 1 == range.blocks.len()
                || self.sets[index].hash() != self.sets[index + 1].hash()
            {
                self.commits
                    .push(RawCommit::from(self.all_commits[index].clone()).encode_to_vec());
            }
        }
    }

    fn verify(&self) -> anyhow::Result<B256> {
        self.verify_from(0)
    }
    fn verify_from(&self, start: usize) -> anyhow::Result<B256> {
        let mut consensus_parameters = parameters();
        consensus_parameters.staking = self.configured_staking;
        let fixture = &self.execution.authenticated;
        let range = &fixture.range;
        let parent = if start == 0 {
            &range.parent
        } else {
            &range.blocks[start - 1].execution
        };
        let mut context = range.context;
        context.parent_height = parent.number;
        context.parent_el_hash = parent.hash_slow();
        context.parent_comet_hash = if start == 0 {
            B256::ZERO
        } else {
            B256::from_slice(header_hash(&range.blocks[start - 1].comet.header).as_bytes())
        };
        let blocks = range.inputs();
        let blocks = &blocks[start..];
        let leaves: Vec<_> = range.blocks[start..range.blocks.len() - 1]
            .iter()
            .map(|block| {
                cut_leaf(
                    U256::from(block.execution.number),
                    block.execution.hash_slow(),
                    B256::from_slice(header_hash(&block.comet.header).as_bytes()),
                    U256::ZERO,
                )
            })
            .collect();
        // Build fresh DA for a suffix; the execution, witness and Comet envelopes stay original.
        let items: Vec<_> = range.blocks[start..]
            .iter()
            .map(|block| block.item.as_slice())
            .collect();
        let (source, hashes) =
            prove_blobs(pack(&v6_payload(&list(&items), Compression::default())));
        let mut statement = range.statement(&hashes, &leaves);
        statement.parent_hash = context.parent_el_hash;
        statement.parent_comet_hash = context.parent_comet_hash;
        statement.input.parent_height = U256::from(parent.number);
        statement.config_hash = config_hash(
            167_001,
            consensus_parameters.staking,
            consensus_parameters.max_uncheckpointed,
        );
        let resources: Vec<_> = self.execution.witnesses[start..]
            .iter()
            .map(|witness| EtnaBlockExecutionWitness {
                witness,
                ancestor_headers: &witness.headers,
            })
            .collect();
        let anchor = [L1CommitteeWitness {
            anchor_number: 18,
            witness: &self.l1,
            fallback_keys: &self.fallback,
        }];
        let openings = range.openings(&range.l1_headers);
        let originals = &self.originals[start..];
        let commits = self
            .commits
            .iter()
            .filter(|raw| {
                etna_core::comet::codec::decode_canonical_commit(raw)
                    .unwrap()
                    .height
                    .value()
                    > parent.number
            })
            .cloned()
            .collect::<Vec<_>>();
        verify_blob_range_statement_consensus(
            &source,
            blocks,
            &context,
            &statement,
            &vec![0; blocks.len()],
            parent,
            range.l1_proof(&openings),
            &resources,
            &[],
            &config(),
            &consensus_parameters,
            ConsensusWitness {
                original_blocks: originals,
                required_commits: &commits,
                origin_fallback_keys: &self.fallback,
                reference_state: &self.l1,
                anchors: &anchor,
                parent: (start != 0).then(|| ParentConsensusWitness {
                    header: &range.blocks[start - 1].comet.header,
                    active: &self.sets[start - 1],
                    next: &self.sets[start],
                    anchor_header: &range.l1_headers[0],
                }),
            },
        )
    }
}

#[test]
fn native_consensus_verifies_origin_single_two_and_interior_boundary_change() {
    for count in [1, 2, 3, 4] {
        let fixture = Fixture::new(count, Candidate::Valid, U256::from(1), false);
        assert!(fixture.verify().is_ok(), "count {count}");
        if count >= 3 {
            assert_eq!(fixture.commits.len(), 2);
        }
        if count >= 2 {
            assert!(
                fixture.verify_from(1).is_ok(),
                "parent bootstrap count {count}"
            );
        }
    }
}

#[test]
fn native_consensus_checks_fallback_stale_invalid_and_source_only_changes() {
    for candidate in [
        Candidate::Stale,
        Candidate::Invalid,
        Candidate::FullWidthHeight,
        Candidate::SameAsFallback,
    ] {
        let fixture = Fixture::new(4, candidate, U256::from(1), false);
        assert!(fixture.verify().is_ok());
        assert_eq!(
            fixture.commits.len(),
            1,
            "same keys and powers add no intermediate QC"
        );
    }
    let fixture = Fixture::new(4, Candidate::Valid, U256::MAX, false);
    assert!(fixture.verify().is_ok());
    assert_eq!(fixture.commits.len(), 1);
}

#[test]
fn native_consensus_rejects_missing_extra_forged_and_exact_two_thirds_qcs() {
    for mutation in ["missing", "extra", "forged", "two_thirds"] {
        let mut fixture = Fixture::new(3, Candidate::Valid, U256::from(1), false);
        match mutation {
            "missing" => {
                fixture.commits.remove(0);
            }
            "extra" => fixture.commits.insert(
                0,
                RawCommit::from(fixture.all_commits[0].clone()).encode_to_vec(),
            ),
            "forged" | "two_thirds" => {
                let mut commit = fixture.all_commits[1].clone();
                if mutation == "two_thirds" {
                    commit.signatures[0] = CommitSig::BlockIdFlagAbsent;
                } else {
                    let CommitSig::BlockIdFlagCommit { signature, .. } = &mut commit.signatures[0]
                    else {
                        panic!()
                    };
                    *signature = Some(vec![0; 65].try_into().unwrap());
                }
                fixture.commits[0] = RawCommit::from(commit).encode_to_vec();
            }
            _ => unreachable!(),
        }
        assert!(fixture.verify().is_err(), "{mutation}");
    }
}

#[test]
fn native_consensus_authenticates_missing_flags_seats_and_every_inherited_anchor() {
    let fixture = Fixture::new(3, Candidate::Valid, U256::from(1), true);
    assert!(fixture.verify().is_ok());
    assert!(fixture.verify_from(1).is_ok());
    let mut corrupted = Fixture::new(3, Candidate::Valid, U256::from(1), true);
    let mut raw = RawBlock::decode(corrupted.originals[1].as_slice()).unwrap();
    raw.last_commit.as_mut().unwrap().signatures[0].block_id_flag = 2;
    corrupted.originals[1] = raw.encode_to_vec();
    assert!(
        corrupted.verify().is_err(),
        "LastCommit flag mutation must be committed by header"
    );
    let mut missing = Fixture::new(2, Candidate::Valid, U256::from(1), false);
    missing.l1.state.clear();
    assert!(
        missing.verify().is_err(),
        "inherited anchor cannot skip Inbox proofs"
    );
}

#[test]
fn native_consensus_rejects_missing_untouched_l2_proof_material_and_false_cached_roots() {
    let mut fixture = Fixture::new(3, Candidate::Valid, U256::from(1), false);
    let witness = &mut fixture.execution.witnesses[1];
    // Removing all state is sufficient to establish fail-closed behavior. The batch-reader test
    // separately removes only the untouched storage subtree while retaining the account proof.
    witness.state.clear();
    for header in &mut witness.headers {
        header.hash = B256::ZERO;
        header.number = 0;
    }
    assert!(fixture.verify().is_err());
    let mut fixture = Fixture::new(2, Candidate::Valid, U256::from(1), false);
    for witness in &mut fixture.execution.witnesses {
        for header in &mut witness.headers {
            header.hash = B256::ZERO;
            header.number = u64::MAX;
        }
    }
    assert!(
        fixture.verify().is_ok(),
        "cached metadata cannot substitute state-root authority"
    );
}

// Append to guest-common/src/etna/tests/execution/consensus.rs.
#[test]
fn independent_validly_signed_wrong_body_part_set_is_rejected_by_composer() {
    for change_total in [false, true] {
        let mut fixture = Fixture::new(3, Candidate::Valid, U256::from(1), false);
        let mut id = fixture.all_commits[1].block_id;
        id.part_set_header = if change_total {
            tendermint::block::parts::Header::new(
                id.part_set_header.total + 1,
                id.part_set_header.hash,
            )
            .unwrap()
        } else {
            tendermint::block::parts::Header::new(
                id.part_set_header.total,
                tendermint::Hash::Sha256([0xaa; 32]),
            )
            .unwrap()
        };
        let header = &fixture.execution.authenticated.range.blocks[1].comet.header;
        let wrong = signed_commit(header, id, &fixture.sets[1]);
        // This is a valid input QC for the claimed full BlockID: rejection must
        // bind the real body's part set, not merely notice a broken signature.
        etna_core::comet::verify::verify_native_commit(
            &wrong,
            header,
            &fixture.sets[1],
            header.chain_id.as_str(),
        )
        .unwrap();
        fixture.commits[0] = RawCommit::from(wrong).encode_to_vec();
        let error = fixture.verify().unwrap_err();
        assert!(format!("{error:#}").contains("part-set"));
    }
}

// Rebuild only the terminal Comet body/header/QC after modifying its embedded LastCommit.
// EL blocks, EVM execution, MPT resources and envelope data remain genuine and unchanged.
fn independent_reseal_terminal(fixture: &mut Fixture) {
    assert_eq!(fixture.originals.len(), 2);
    let range = &mut fixture.execution.authenticated.range;
    let block = &mut range.blocks[1];
    block.comet.header.last_commit_hash =
        Some(commit_hash(block.comet.last_commit.as_ref().unwrap()));
    let raw = RawBlock {
        header: Some(block.comet.header.clone().into()),
        data: Some(Data {
            txs: block.comet.data.clone(),
        }),
        evidence: Some(EvidenceList { evidence: vec![] }),
        last_commit: block.comet.last_commit.clone().map(RawCommit::from),
    };
    let id = Id {
        hash: header_hash(&block.comet.header),
        part_set_header: block_parts(&raw).unwrap(),
    };
    let terminal = signed_commit(&block.comet.header, id, &fixture.sets[1]);
    range.context.last_comet_hash = B256::from_slice(id.hash.as_bytes());
    fixture.originals[1] = raw.encode_to_vec();
    fixture.commits = vec![RawCommit::from(terminal).encode_to_vec()];
}

#[test]
fn independent_embedded_commit_flags_use_hash_authentication_without_signature_verification() {
    let mut fixture = Fixture::new(2, Candidate::Valid, U256::from(1), false);
    let last = fixture.execution.authenticated.range.blocks[1]
        .comet
        .last_commit
        .as_mut()
        .unwrap();
    let CommitSig::BlockIdFlagCommit { signature, .. } = &mut last.signatures[0] else {
        panic!()
    };
    *signature = Some(vec![0; 65].try_into().unwrap());
    independent_reseal_terminal(&mut fixture);
    assert!(
        fixture.verify().is_ok(),
        "LastCommit uses committed flags, while terminal QC is independently valid"
    );

    let mut changed_flag = Fixture::new(2, Candidate::Valid, U256::from(1), false);
    changed_flag.execution.authenticated.range.blocks[1]
        .comet
        .last_commit
        .as_mut()
        .unwrap()
        .signatures[0] = CommitSig::BlockIdFlagAbsent;
    independent_reseal_terminal(&mut changed_flag);
    let error = changed_flag.verify().unwrap_err();
    assert!(format!("{error:#}").contains("miss bitmap"), "{error:#}");
}

#[test]
fn native_consensus_rejects_authenticated_l1_storage_contradictions() {
    let origin = origin_slots(2);
    for (slot, value, expected) in [
        (U256::from(268), U256::from(1), "generation"),
        (U256::from(269), U256::ZERO, "config"),
        (U256::from(278), U256::ZERO, "fallback"),
        (U256::from(279), U256::MAX, "validator"),
        (U256::from_be_bytes(origin[0].0), U256::from(1), "origin"),
        (U256::from_be_bytes(origin[3].0), U256::ZERO, "fallback"),
    ] {
        let fixture = Fixture::with_overrides(
            3,
            Candidate::Valid,
            U256::from(1),
            false,
            Some((slot, value)),
            U256::from(0x0701),
        );
        let error = fixture.verify().expect_err(expected);
        assert!(
            format!("{error:#}").contains(expected),
            "{expected}: {error:#}"
        );
    }
}

#[test]
fn native_consensus_binds_live_key_to_seat_word_in_actual_parent_state() {
    for seat in [
        U256::ZERO,
        U256::from(0x0301),
        U256::from(0x0702),
        U256::from(0x8001),
        U256::from(0x10701),
    ] {
        let fixture = Fixture::with_overrides(3, Candidate::Valid, U256::from(1), true, None, seat);
        let error = fixture
            .verify()
            .expect_err("seat word must determine actual committed bitmap");
        assert!(format!("{error:#}").contains("miss bitmap"), "{error:#}");
    }
}

#[test]
fn native_consensus_rejects_unbound_parent_bootstrap_sets() {
    for active in [false, true] {
        let mut fixture = Fixture::new(3, Candidate::Valid, U256::from(1), false);
        let replacement = Validators::from_members(&[(consensus_key(4), 5)]).unwrap();
        if active {
            fixture.sets[0] = replacement;
        } else {
            fixture.sets[1] = replacement;
        }
        let error = fixture
            .verify_from(1)
            .expect_err("parent sets must open actual authenticated Comet commitments");
        let expected = if active {
            "parent active"
        } else {
            "parent next"
        };
        assert!(format!("{error:#}").contains(expected), "{error:#}");
    }
}

#[test]
fn native_consensus_rejects_resigned_incorrect_current_and_next_commitments() {
    for current in [false, true] {
        let mut fixture = Fixture::new(2, Candidate::Valid, U256::from(1), false);
        let replacement = if current {
            fixture.sets[2].hash()
        } else {
            fixture.sets[1].hash()
        };
        let header = &mut fixture.execution.authenticated.range.blocks[1].comet.header;
        if current {
            header.validators_hash = replacement;
        } else {
            header.next_validators_hash = replacement;
        }
        independent_reseal_terminal(&mut fixture);
        let error = fixture
            .verify()
            .expect_err("header commitments must match state-derived schedule");
        let expected = if current {
            "current validator"
        } else {
            "next validator"
        };
        assert!(format!("{error:#}").contains(expected), "{error:#}");
    }
}

#[test]
fn native_consensus_rejects_configured_staking_different_from_actual_evm_target() {
    use alethia_reth_chainspec::spec::TaikoExecutorSpec;
    use etna_core::l1_header::L1HeaderView;
    use raiko2_stateless::read_storage_batch_with_witness_resources;

    let alternate = Address::with_last_byte(0x77);
    let committed_config = config_hash(167_001, alternate, parameters().max_uncheckpointed);
    let mut fixture = Fixture::with_overrides(
        1,
        Candidate::Valid,
        U256::from(1),
        false,
        Some((U256::from(269), U256::from_be_bytes(committed_config.0))),
        U256::from(0x0701),
    );
    fixture.configured_staking = alternate;
    // All existing DA/SSZ/range/strict-EVM checks pass, executing the chain's actual target A.
    // The authenticated reference Inbox, requested configuration and caller parameters name B.
    // At the origin singleton no snapshot read is needed, so configHash alone cannot detect it.
    assert!(fixture.execution.verify().is_ok());
    assert_eq!(config().chain_spec().etna_staking_address(), Some(STAKING));
    let range = &fixture.execution.authenticated.range;
    let reference = L1HeaderView::from_raw(&range.l1_headers[0]).unwrap();
    let proven = read_storage_batch_with_witness_resources(
        reference.state_root(),
        &[(range.context.domain.inbox, U256::from(269))],
        &fixture.l1,
        &[],
    )
    .unwrap();
    assert_eq!(B256::from(proven[0]), committed_config);
    let error = fixture
        .verify()
        .expect_err("configured staking must equal EVM block-start target");
    assert!(format!("{error:#}").contains("staking target"), "{error:#}");
}

#[test]
fn native_consensus_verifies_parent_next_bootstrap_after_origin_first_two_heights() {
    let fixture = Fixture::new(4, Candidate::Valid, U256::from(1), false);
    for parent_height in [2, 3] {
        assert!(
            fixture.verify_from(parent_height).is_ok(),
            "parent next bootstrap at {parent_height}"
        );
    }
}
