//! Historical stock bytes exercise native decoding/body commitments, not envelope validity.

use alloy_primitives::hex;
use etna_core::comet::{
    codec::{decode_canonical_block, decode_canonical_commit},
    native::{RawBlock, RawCommit},
    verify::{BftError, CertifiedBlockRef, verify_body_at_origin},
};
use prost::Message;
use tendermint::Hash;

fn fixture(text: &str) -> Vec<u8> {
    hex::decode(text.trim_end()).unwrap()
}

#[test]
fn stock_native_body_yields_exact_committed_envelope_and_identity() {
    let raw = fixture(include_str!("fixtures/first-block.hex"));
    let (_, block) = decode_canonical_block(&raw).unwrap();
    let commit =
        decode_canonical_commit(&fixture(include_str!("fixtures/first-commit.hex"))).unwrap();
    let pair = CertifiedBlockRef {
        block: &block,
        raw_block: &raw,
        commit: &commit,
    };
    assert_eq!(
        verify_body_at_origin(pair, 1, 0).unwrap(),
        fixture(include_str!("fixtures/envelope.hex"))
    );
    assert_eq!(
        commit.block_id.hash.as_bytes(),
        fixture("6fb9068b78286ef93a23be01bf3747a2af428779bc93c46f2821ba8e0f255965")
    );
    assert_eq!(commit.block_id.part_set_header.total, 1);
    assert_eq!(
        commit.block_id.part_set_header.hash.as_bytes(),
        fixture("16810adf18cf791ba38db0966edb676b4479700c7926fea032f57c57d2b83754")
    );
    assert!(verify_body_at_origin(pair, 2, 0).is_err());
    assert!(verify_body_at_origin(pair, 1, 1).is_err());
}

#[test]
fn both_native_codecs_reject_noncanonical_extra_fields() {
    let mut raw = fixture(include_str!("fixtures/first-block.hex"));
    raw.extend([0xf8, 0x07, 1]);
    assert!(decode_canonical_block(&raw).is_err());
    let mut commit = fixture(include_str!("fixtures/first-commit.hex"));
    commit.extend([0xf8, 0x07, 1]);
    assert!(decode_canonical_commit(&commit).is_err());
}

#[test]
fn body_rejects_changed_transaction_and_wrong_original_part_set() {
    let raw = fixture(include_str!("fixtures/first-block.hex"));
    let mut proto = RawBlock::decode(raw.as_slice()).unwrap();
    proto.data.as_mut().unwrap().txs[0][0] ^= 1;
    let changed_raw = proto.encode_to_vec();
    let (_, changed_block) = decode_canonical_block(&changed_raw).unwrap();
    let commit =
        decode_canonical_commit(&fixture(include_str!("fixtures/first-commit.hex"))).unwrap();
    assert!(matches!(
        verify_body_at_origin(
            CertifiedBlockRef {
                block: &changed_block,
                raw_block: &changed_raw,
                commit: &commit
            },
            1,
            0
        ),
        Err(BftError::Commitment("data"))
    ));
    let (_, block) = decode_canonical_block(&raw).unwrap();
    let mut wrong_parts = commit;
    wrong_parts.block_id.part_set_header.hash = Hash::Sha256([0x77; 32]);
    assert!(
        verify_body_at_origin(
            CertifiedBlockRef {
                block: &block,
                raw_block: &raw,
                commit: &wrong_parts
            },
            1,
            0
        )
        .is_err()
    );
    // The canonical certificate codec still accepts this structural mutation: it is the
    // original-body part commitment, rather than protobuf decoding, that must reject it.
    assert!(decode_canonical_commit(&RawCommit::from(wrong_parts).encode_to_vec()).is_ok());
}
