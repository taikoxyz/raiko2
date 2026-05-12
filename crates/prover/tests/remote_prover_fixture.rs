#![allow(missing_docs)]

use std::{fs, path::PathBuf, str::FromStr};

use alloy_primitives::{Address, B256, Uint};
use raiko2_primitives::Proof;
use raiko2_primitives_shasta::GuestInput;
use raiko2_protocol_shasta::shasta::ProofCarryData;
use raiko2_prover::remote_prover::adapter::{build_shasta_aggregate_request, build_shasta_packet};

#[test]
fn vendored_proposal_request_fixture_matches_adapter_output() {
    let raw =
        fs::read_to_string(shared_guest_input_path()).expect("read shared shasta guest input");
    let guest_input: GuestInput = serde_json::from_str(&raw).expect("parse shared fixture");
    let actual = serde_json::to_string_pretty(
        &build_shasta_packet(&guest_input).expect("build remote prover packet"),
    )
    .expect("serialize proposal request");

    let expected = fs::read_to_string(vendored_proposal_fixture_path())
        .expect("read vendored proposal fixture");
    assert_eq!(actual, expected);
}

#[test]
fn vendored_aggregate_request_fixture_matches_adapter_output() {
    let actual = serde_json::to_string_pretty(
        &build_shasta_aggregate_request(&[fixture_aggregate_proof()])
            .expect("build aggregate packet"),
    )
    .expect("serialize aggregate request");

    let expected = fs::read_to_string(vendored_aggregate_fixture_path())
        .expect("read vendored aggregate fixture");
    assert_eq!(actual, expected);
}

fn shared_guest_input_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_2222_l2_5412225_5412416.json")
}

fn vendored_proposal_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../tests/fixtures/remote_prover/shasta_request_v1_taiko_mainnet_proposal_2222_l2_5412225_5412416.json",
    )
}

fn vendored_aggregate_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../tests/fixtures/remote_prover/shasta_aggregate_request_v1_single_fixture_proof.json",
    )
}

fn fixture_aggregate_proof() -> Proof {
    let mut carry = ProofCarryData {
        chain_id: 167_013,
        verifier: Address::from_str("0x00f9f60C79e38c08b785eE4F1a849900693C6630")
            .expect("verifier"),
        ..ProofCarryData::default()
    };
    carry.transition_input.proposal_id = 7;
    carry.transition_input.parent_block_hash = B256::from([0x11; 32]);
    carry.transition_input.proposal_hash = B256::from([0x22; 32]);
    carry.transition_input.parent_proposal_hash = B256::from([0x33; 32]);
    carry.transition_input.actual_prover =
        Address::from_str("0x0000777735367b36bC9B61C50022d9D0700dB4Ec").expect("prover");
    carry.transition_input.transition.proposer =
        Address::from_str("0x4444444444444444444444444444444444444444").expect("proposer");
    carry.transition_input.transition.timestamp = 123;
    carry.transition_input.checkpoint.blockNumber = Uint::from(42u64);
    carry.transition_input.checkpoint.blockHash = B256::from([0x44; 32]);
    carry.transition_input.checkpoint.stateRoot = B256::from([0x55; 32]);

    Proof {
        proof: Some(format!("0x{}", "11".repeat(89))),
        input: Some(B256::from([0x66; 32])),
        extra_data: Some(
            raiko2_primitives_shasta::encode_proof_carry_data(&carry).expect("encode carry data"),
        ),
        ..Proof::default()
    }
}
