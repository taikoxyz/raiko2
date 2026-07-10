//! Net feature integration tests for Shasta helpers.

#![cfg(feature = "net")]
#![allow(missing_docs)]

use alloy_primitives::{Bytes, U256, b256};
use raiko2_protocol_shasta::shasta::constants::{
    TAIKO_DEVNET_CHAIN_ID, TAIKO_HOODI_CHAIN_ID, TAIKO_MAINNET_CHAIN_ID, TAIKO_MASAYA_CHAIN_ID,
    max_anchor_offset_for_chain, min_base_fee_for_chain, shasta_fork_condition_for_chain,
    shasta_fork_timestamp_for_chain, timestamp_max_offset_for_chain,
};
use raiko2_protocol_shasta::shasta::{
    AnchorV4Input, ShastaForkConfigError, calculate_shasta_difficulty, encode_extra_data,
    manifest::{BlockManifest, DerivationSourceManifest},
};

#[test]
fn extra_data_encoding_matches_shasta_header_layout() {
    let extra_data = encode_extra_data(7, 0x01_02_03_04_05_06);
    assert_eq!(extra_data, Bytes::from(vec![7, 1, 2, 3, 4, 5, 6]));
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
fn manifest_codec_uses_taiko_client_default_manifest() {
    let manifest = DerivationSourceManifest {
        blocks: vec![BlockManifest::default()],
    };
    let payload = manifest.encode_and_compress().expect("encode manifest");
    let decoded =
        DerivationSourceManifest::decompress_and_decode(&payload, 0).expect("decode manifest");
    assert_eq!(decoded.blocks.len(), 1);

    let decoded = DerivationSourceManifest::decompress_and_decode(&[0u8; 64], 0)
        .expect("invalid payload returns default manifest");
    assert_eq!(decoded.blocks.len(), 1);
    assert!(decoded.blocks[0].transactions.is_empty());
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
    assert_eq!(
        shasta_fork_timestamp_for_chain(TAIKO_MAINNET_CHAIN_ID).expect("mainnet timestamp"),
        1_775_135_700
    );
    assert!(shasta_fork_timestamp_for_chain(1).is_err());
    assert!(matches!(
        shasta_fork_condition_for_chain(1),
        Err(ShastaForkConfigError::UnsupportedChainId(1))
    ));
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
