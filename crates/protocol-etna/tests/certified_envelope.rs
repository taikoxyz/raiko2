//! Certified-envelope regressions using the independent stock Comet byte oracle.
use alloy_primitives::{Address, B256, U256, hex, keccak256};
use etna_core::{
    comet::{
        codec::decode_canonical_block,
        native::{Commit, RawBlock, RawCommit, RawCommitSig},
        validators::Validators,
        verify::{
            CertifiedBlockRef, block_parts, commit_hash, header_hash, verify_body_at_origin,
            verify_native_commit,
        },
    },
    domain::DeploymentDomain,
    key::ConsensusKey,
    types::Member,
};
use k256::ecdsa::SigningKey;
use prost::Message;
use raiko2_protocol_etna::{
    CertifiedEnvelopeError, CometBlock, CometBlockDecodeError, CometWireLimits, ConsensusContext,
    PreviousComet, verify_certified_envelope,
};
use tendermint::{
    Hash, Vote,
    block::{Header, Id},
    vote::{CanonicalVote, Type},
};
use tendermint_proto::{Protobuf, v0_38::types::CanonicalVote as RawVote};

const GO_BLOCK_HASH: &str = "6fb9068b78286ef93a23be01bf3747a2af428779bc93c46f2821ba8e0f255965";
const GO_PARTS_HASH: &str = "16810adf18cf791ba38db0966edb676b4479700c7926fea032f57c57d2b83754";
const ORIGIN: u64 = (1 << 32) + 17;
const PARENT_EL: B256 = B256::repeat_byte(0x20);

struct Wire {
    block: Vec<u8>,
    commit: Vec<u8>,
}

impl Wire {
    fn decode(&self) -> CometBlock<'_> {
        CometBlock::decode(&self.block, &self.commit, self.limits()).unwrap()
    }

    const fn limits(&self) -> CometWireLimits {
        CometWireLimits {
            block_bytes: self.block.len(),
            commit_bytes: self.commit.len(),
        }
    }
}

fn fixture(hex: &str) -> Vec<u8> {
    hex::decode(hex.trim_end()).unwrap()
}

fn stock_wire() -> Wire {
    Wire {
        block: fixture(include_str!("fixtures/first-block.hex")),
        commit: fixture(include_str!("fixtures/first-commit.hex")),
    }
}

fn envelope() -> Vec<u8> {
    fixture(include_str!("fixtures/envelope.hex"))
}

fn signing_key(seed: u8) -> SigningKey {
    SigningKey::from_slice(&[seed; 32]).unwrap()
}

fn committee(powers: [u64; 2]) -> Validators {
    let keys = [1, 2].map(signing_key);
    let members = core::array::from_fn::<_, 2, _>(|index| Member {
        pubkey: ConsensusKey::from_slice(
            keys[index]
                .verifying_key()
                .to_encoded_point(true)
                .as_bytes(),
        )
        .unwrap(),
        eff_stake: U256::ZERO,
        power: powers[index],
    });
    Validators::from_members(&members).unwrap()
}

fn context(
    height: u64,
    origin_height: u64,
    parent_el_hash: B256,
    validators: &Validators,
) -> ConsensusContext<'_> {
    ConsensusContext {
        domain: DeploymentDomain {
            l1_chain_id: U256::from(1),
            l2_chain_id: U256::from(167_001),
            inbox: Address::repeat_byte(0x11),
            generation: 0,
        },
        height,
        origin_height,
        parent_el_hash,
        validators,
    }
}

#[test]
fn stock_first_block_yields_the_exact_committed_transaction_and_identity() {
    let wire = stock_wire();
    let block = wire.decode();
    let validators = committee([10, 30]);
    let ctx = context(1, 0, B256::ZERO, &validators);
    let certified = verify_certified_envelope(&block, &ctx, None).unwrap();
    assert_eq!(certified.envelope_bytes(), envelope());
    assert_eq!(certified.header().height.value(), 1);
    assert_eq!(certified.block_id().hash.as_bytes(), fixture(GO_BLOCK_HASH));
    assert_eq!(certified.block_id().part_set_header.total, 1);
    assert_eq!(
        certified.block_id().part_set_header.hash.as_bytes(),
        fixture(GO_PARTS_HASH),
    );
}

#[test]
fn wire_bounds_and_both_canonical_codecs_are_enforced() {
    let wire = stock_wire();
    let limits = wire.limits();
    assert!(matches!(
        CometBlock::decode(
            &wire.block,
            &wire.commit,
            CometWireLimits {
                block_bytes: limits.block_bytes - 1,
                ..limits
            }
        ),
        Err(CometBlockDecodeError::BlockTooLarge { .. })
    ));
    assert!(matches!(
        CometBlock::decode(
            &wire.block,
            &wire.commit,
            CometWireLimits {
                commit_bytes: limits.commit_bytes - 1,
                ..limits
            }
        ),
        Err(CometBlockDecodeError::CommitTooLarge { .. })
    ));

    let mut noncanonical_block = wire.block.clone();
    noncanonical_block.extend([0xf8, 0x07, 1]);
    assert!(matches!(
        CometBlock::decode(
            &noncanonical_block,
            &wire.commit,
            CometWireLimits {
                block_bytes: noncanonical_block.len(),
                ..limits
            }
        ),
        Err(CometBlockDecodeError::Block(_))
    ));

    let mut noncanonical_commit = wire.commit.clone();
    noncanonical_commit.extend([0xf8, 0x07, 1]);
    assert!(matches!(
        CometBlock::decode(
            &wire.block,
            &noncanonical_commit,
            CometWireLimits {
                commit_bytes: noncanonical_commit.len(),
                ..limits
            }
        ),
        Err(CometBlockDecodeError::Commit(_))
    ));
}

#[test]
fn rejects_mismatched_domain_requested_height_origin_parent_and_committee() {
    let wire = stock_wire();
    let block = wire.decode();
    let validators = committee([10, 30]);
    let base = context(1, 0, B256::ZERO, &validators);
    for domain in [
        DeploymentDomain {
            generation: 1,
            ..base.domain
        },
        DeploymentDomain {
            l2_chain_id: base.domain.l2_chain_id + (U256::from(1) << 128),
            ..base.domain
        },
    ] {
        assert!(matches!(
            verify_certified_envelope(&block, &ConsensusContext { domain, ..base }, None),
            Err(CertifiedEnvelopeError::CurrentCertificate(_))
        ));
    }
    let substituted = committee([30, 10]);
    assert!(matches!(
        verify_certified_envelope(
            &block,
            &ConsensusContext {
                validators: &substituted,
                ..base
            },
            None
        ),
        Err(CertifiedEnvelopeError::CurrentCertificate(_))
    ));

    for ctx in [
        ConsensusContext { height: 2, ..base },
        ConsensusContext {
            origin_height: 1,
            ..base
        },
        ConsensusContext {
            origin_height: u64::MAX,
            ..base
        },
        ConsensusContext {
            parent_el_hash: PARENT_EL,
            ..base
        },
    ] {
        assert!(matches!(
            verify_certified_envelope(&block, &ctx, None),
            Err(CertifiedEnvelopeError::Body(_))
        ));
    }
}

#[test]
fn rejects_changed_transaction_and_qc_over_wrong_parts() {
    let raw = stock_wire();
    let mut changed = RawBlock::decode(raw.block.as_slice()).unwrap();
    changed.data.as_mut().unwrap().txs[0][0] ^= 1;
    let altered = Wire {
        block: changed.encode_to_vec(),
        commit: raw.commit,
    };
    let validators = committee([10, 30]);
    let ctx = context(1, 0, B256::ZERO, &validators);
    assert!(matches!(
        verify_certified_envelope(&altered.decode(), &ctx, None),
        Err(CertifiedEnvelopeError::Body(_))
    ));

    let raw = stock_wire();
    let (proto, block) = decode_canonical_block(&raw.block).unwrap();
    let mut id = block_id(&proto, &block.header);
    id.part_set_header.hash = Hash::Sha256([0x77; 32]);
    let commit = signed_commit(&block.header, id, &validators, false);
    verify_native_commit(
        &commit,
        &block.header,
        &validators,
        block.header.chain_id.as_str(),
    )
    .expect("valid signatures over the wrong part set");
    let wrong_parts = Wire {
        block: raw.block,
        commit: RawCommit::from(commit).encode_to_vec(),
    };
    assert!(matches!(
        verify_certified_envelope(&wrong_parts.decode(), &ctx, None),
        Err(CertifiedEnvelopeError::Body(_))
    ));
}

// Extend the stock wire template with deterministic linked blocks. The stock differential
// fixture's later-height predecessors are synthetic IDs, so they cannot form a full chain.
struct LinkedWire {
    previous: Wire,
    current: Wire,
    invalid_embedded: Wire,
}

fn block_id(raw: &RawBlock, header: &Header) -> Id {
    Id {
        hash: header_hash(header),
        part_set_header: block_parts(raw).unwrap(),
    }
}

fn signed_commit(header: &Header, id: Id, validators: &Validators, nil_light_slot: bool) -> Commit {
    let keys = [signing_key(1), signing_key(2)];
    let mut raw = RawCommit {
        height: header.height.into(),
        round: 0,
        block_id: Some(id.into()),
        signatures: vec![],
    };
    for (index, validator) in validators.validators().iter().enumerate() {
        let nil = nil_light_slot && index == 1;
        let vote = Vote {
            vote_type: Type::Precommit,
            height: header.height,
            round: 0u8.into(),
            block_id: (!nil).then_some(id),
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
        let key = keys
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
            block_id_flag: if nil { 3 } else { 2 },
            validator_address: validator.address.as_bytes().to_vec(),
            timestamp: Some(header.time.into()),
            signature,
        });
    }
    raw.try_into().unwrap()
}

fn certify(raw: &RawBlock, header: &Header, validators: &Validators) -> Wire {
    let commit = signed_commit(header, block_id(raw, header), validators, false);
    Wire {
        block: raw.encode_to_vec(),
        commit: RawCommit::from(commit).encode_to_vec(),
    }
}

fn linked_wire(previous_validators: &Validators, current_validators: &Validators) -> LinkedWire {
    let stock = stock_wire();
    let (mut raw, block) = decode_canonical_block(&stock.block).unwrap();
    let mut header = block.header;
    header.height = (ORIGIN + 1).try_into().unwrap();
    header.next_validators_hash = current_validators.hash();
    raw.header = Some(header.clone().into());
    let previous = certify(&raw, &header, previous_validators);

    // A valid variant for the same BlockID: the 30-power validator votes block, the
    // 10-power validator votes nil. The standalone predecessor QC has two block votes.
    let id = block_id(&raw, &header);
    let mut embedded = RawCommit::from(signed_commit(&header, id, previous_validators, true));
    header.height = (ORIGIN + 2).try_into().unwrap();
    header.app_hash = PARENT_EL.as_slice().to_vec().try_into().unwrap();
    header.last_block_id = Some(id);
    header.validators_hash = current_validators.hash();
    let mut successor = |commit: &RawCommit| {
        header.last_commit_hash = Some(commit_hash(&commit.clone().try_into().unwrap()));
        raw.header = Some(header.clone().into());
        raw.last_commit = Some(commit.clone());
        certify(&raw, &header, current_validators)
    };
    let current = successor(&embedded);

    // Corrupt the nil signature then independently recommit and sign the successor.
    // Current structure and current QC remain valid, so only the embedded QC rejects.
    embedded.signatures[1].signature.fill(0);
    let invalid_embedded = successor(&embedded);
    LinkedWire {
        previous,
        current,
        invalid_embedded,
    }
}

#[test]
fn successor_accepts_distinct_valid_embedded_commit_using_previous_committee() {
    let outgoing = committee([10, 30]);
    let incoming = committee([35, 5]);
    let wire = linked_wire(&outgoing, &incoming);
    let previous_block = wire.previous.decode();
    let first = context(ORIGIN + 1, ORIGIN, B256::ZERO, &outgoing);
    let previous = verify_certified_envelope(&previous_block, &first, None).unwrap();
    let previous = PreviousComet {
        header: previous.header(),
        block_id: previous.block_id(),
        validators: &outgoing,
    };
    assert!(matches!(
        verify_certified_envelope(&previous_block, &first, Some(previous)),
        Err(CertifiedEnvelopeError::PreviousPresence)
    ));
    let current = wire.current.decode();
    let ctx = context(ORIGIN + 2, ORIGIN, PARENT_EL, &incoming);
    let certified = verify_certified_envelope(&current, &ctx, Some(previous)).unwrap();
    assert_eq!(certified.envelope_bytes(), envelope());
    assert_eq!(certified.header().height.value(), ORIGIN + 2);
    assert!(matches!(
        verify_certified_envelope(
            &current,
            &ConsensusContext {
                origin_height: ORIGIN + 1,
                ..ctx
            },
            None
        ),
        Err(CertifiedEnvelopeError::Body(_))
    ));
    assert!(matches!(
        verify_certified_envelope(
            &current,
            &ctx,
            Some(PreviousComet {
                validators: &incoming,
                ..previous
            })
        ),
        Err(CertifiedEnvelopeError::EmbeddedCertificate(_))
    ));
}

#[test]
fn successor_requires_the_exact_authenticated_previous_block_id_and_header() {
    let outgoing = committee([10, 30]);
    let incoming = committee([35, 5]);
    let wire = linked_wire(&outgoing, &incoming);
    let previous_block = wire.previous.decode();
    let first = verify_certified_envelope(
        &previous_block,
        &context(ORIGIN + 1, ORIGIN, B256::ZERO, &outgoing),
        None,
    )
    .unwrap();
    let block = wire.current.decode();
    let ctx = context(ORIGIN + 2, ORIGIN, PARENT_EL, &incoming);
    assert!(matches!(
        verify_certified_envelope(&block, &ctx, None),
        Err(CertifiedEnvelopeError::PreviousPresence)
    ));

    let mut wrong_parts = *first.block_id();
    wrong_parts.part_set_header.total += 1;
    let wrong_previous = PreviousComet {
        header: first.header(),
        block_id: &wrong_parts,
        validators: &outgoing,
    };
    assert!(matches!(
        verify_certified_envelope(&block, &ctx, Some(wrong_previous)),
        Err(CertifiedEnvelopeError::PreviousIdentity)
    ));

    let mut wrong_header = first.header().clone();
    wrong_header.height = (ORIGIN + 2).try_into().unwrap();
    let wrong_previous = PreviousComet {
        header: &wrong_header,
        block_id: first.block_id(),
        validators: &outgoing,
    };
    assert!(matches!(
        verify_certified_envelope(&block, &ctx, Some(wrong_previous)),
        Err(CertifiedEnvelopeError::EmbeddedCertificate(_))
    ));
}

#[test]
fn invalid_embedded_nil_signature_is_rejected_even_with_valid_current_qc_and_body() {
    let outgoing = committee([10, 30]);
    let incoming = committee([35, 5]);
    let wire = linked_wire(&outgoing, &incoming);
    let previous_block = wire.previous.decode();
    let first = verify_certified_envelope(
        &previous_block,
        &context(ORIGIN + 1, ORIGIN, B256::ZERO, &outgoing),
        None,
    )
    .unwrap();
    let previous = PreviousComet {
        header: first.header(),
        block_id: first.block_id(),
        validators: &outgoing,
    };
    let invalid = wire.invalid_embedded.decode();

    let (_, block) = decode_canonical_block(&wire.invalid_embedded.block).unwrap();
    let commit: Commit = RawCommit::decode(wire.invalid_embedded.commit.as_slice())
        .unwrap()
        .try_into()
        .unwrap();
    verify_native_commit(
        &commit,
        &block.header,
        &incoming,
        block.header.chain_id.as_str(),
    )
    .expect("current QC certifies the altered last-commit hash");
    verify_body_at_origin(
        CertifiedBlockRef {
            block: &block,
            raw_block: &wire.invalid_embedded.block,
            commit: &commit,
        },
        ORIGIN + 2,
        PARENT_EL,
        ORIGIN,
    )
    .expect("the altered last commit is bound to the current body");
    assert_eq!(block.last_commit.unwrap().block_id, *first.block_id());

    let ctx = context(ORIGIN + 2, ORIGIN, PARENT_EL, &incoming);
    assert!(matches!(
        verify_certified_envelope(&invalid, &ctx, Some(previous)),
        Err(CertifiedEnvelopeError::EmbeddedCertificate(_))
    ));
}
