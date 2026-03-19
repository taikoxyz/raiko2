//! Net feature integration tests for Shasta helpers.

#![cfg(feature = "net")]
#![allow(missing_docs)]

use alloy_eips::eip4844::builder::{SidecarBuilder, SidecarCoder};
use alloy_primitives::{Bytes, U256, b256};
use alloy_rpc_types_engine::PayloadId;
use raiko2_protocol_shasta::shasta::constants::{
    TAIKO_DEVNET_CHAIN_ID, TAIKO_HOODI_CHAIN_ID, TAIKO_MAINNET_CHAIN_ID, TAIKO_MASAYA_CHAIN_ID,
    max_anchor_offset_for_chain, min_base_fee_for_chain, shasta_fork_condition_for_chain,
    shasta_fork_timestamp_for_chain, timestamp_max_offset_for_chain,
};
use raiko2_protocol_shasta::shasta::{
    AnchorV4Input, BlobCoder, PAYLOAD_ID_VERSION_V2, calculate_shasta_difficulty,
    encode_extra_data, encode_tx_list, payload_id_to_bytes,
};

#[test]
fn payload_helpers_match_expected_encoding() {
    let extra_data = encode_extra_data(7, 0x01_02_03_04_05_06);
    assert_eq!(extra_data, Bytes::from(vec![7, 1, 2, 3, 4, 5, 6]));

    let payload_id = PayloadId::new([PAYLOAD_ID_VERSION_V2, 1, 2, 3, 4, 5, 6, 7]);
    assert_eq!(payload_id_to_bytes(payload_id), [2, 1, 2, 3, 4, 5, 6, 7]);
}

#[test]
fn shasta_difficulty_is_stable() {
    let difficulty = calculate_shasta_difficulty(
        b256!("1111111111111111111111111111111111111111111111111111111111111111"),
        42,
    );
    assert_eq!(
        difficulty,
        b256!("4c451d52528fa2c52d3f8fc59c2278c8ac1d0ba0341fa2b8e5dd37fb0a759d34")
    );
}

#[test]
fn blob_coder_round_trips_single_and_multi_blob_payloads() {
    let single_payload = (0..256u16)
        .flat_map(|value| value.to_be_bytes())
        .collect::<Vec<u8>>();
    let single_blobs = SidecarBuilder::<BlobCoder>::from_slice(&single_payload).take();
    let mut coder = BlobCoder;
    let single_decoded = coder.decode_all(&single_blobs).expect("single decode");
    assert_eq!(single_decoded.concat(), single_payload);

    let multi_payload = vec![0xAB; 131_000];
    let multi_blobs = SidecarBuilder::<BlobCoder>::from_slice(&multi_payload).take();
    assert!(multi_blobs.len() > 1, "payload should span multiple blobs");
    let mut coder = BlobCoder;
    let multi_decoded = coder.decode_all(&multi_blobs).expect("multi decode");
    assert_eq!(multi_decoded.concat(), multi_payload);
}

#[test]
fn shasta_chain_constants_are_chain_aware() {
    assert_eq!(
        shasta_fork_timestamp_for_chain(TAIKO_DEVNET_CHAIN_ID).expect("devnet timestamp"),
        0
    );
    assert_eq!(
        shasta_fork_timestamp_for_chain(TAIKO_MASAYA_CHAIN_ID).expect("masaya timestamp"),
        0
    );
    assert_eq!(max_anchor_offset_for_chain(TAIKO_MAINNET_CHAIN_ID), 512);
    assert_eq!(
        timestamp_max_offset_for_chain(TAIKO_MAINNET_CHAIN_ID),
        12 * 512
    );
    assert_eq!(min_base_fee_for_chain(TAIKO_MAINNET_CHAIN_ID), 10_000_000);
    assert_eq!(
        shasta_fork_timestamp_for_chain(TAIKO_HOODI_CHAIN_ID).expect("hoodi timestamp"),
        1_770_296_400
    );
    assert!(shasta_fork_timestamp_for_chain(TAIKO_MAINNET_CHAIN_ID).is_err());
    assert!(shasta_fork_timestamp_for_chain(1).is_err());
    assert!(shasta_fork_condition_for_chain(1).is_none());
}

#[test]
fn encode_tx_list_matches_rlp_list_encoding() {
    let txs = vec![
        Bytes::from(vec![0x01, 0x02, 0x03]),
        Bytes::from(vec![0xAA, 0xBB]),
    ];

    assert_eq!(
        encode_tx_list(&txs),
        Bytes::from(vec![0xc7, 0x83, 0x01, 0x02, 0x03, 0x82, 0xaa, 0xbb])
    );
}

#[test]
fn anchor_v4_input_reexport_is_constructible() {
    let input = AnchorV4Input {
        anchor_block_number: 42,
        anchor_block_hash: b256!(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ),
        anchor_state_root: b256!(
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        ),
        l2_height: 1337,
        base_fee: U256::from(10_000_000u64),
    };

    assert_eq!(input.anchor_block_number, 42);
    assert_eq!(input.l2_height, 1337);
    assert_eq!(input.base_fee, U256::from(10_000_000u64));
}
