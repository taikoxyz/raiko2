#![allow(missing_docs)]

use alloy_consensus::Header;
use alloy_primitives::B256;
use raiko2_primitives::{StatelessInput, WitnessHeader};
use raiko2_primitives_shasta::GuestInput;
use raiko2_prover::remote_prover::{
    adapter::build_shasta_packet,
    protocol::{RAIKO2_SHASTA_REQUEST_SCHEMA, Raiko2ReplayBlock},
};

fn fixture_guest_input() -> GuestInput {
    let mut witness = StatelessInput::default();
    witness.block.header.number = 42;
    witness.block.header.parent_hash = B256::from([0x11; 32]);
    witness.block.header.state_root = B256::from([0x22; 32]);
    witness.chain_spec.chain_id = 167_013;

    let mut input = GuestInput::default();
    input.witnesses.push(witness);
    input
        .proposal_ancestor_headers
        .push(WitnessHeader::from_header(Header {
            number: 41,
            parent_hash: B256::from([0x33; 32]),
            state_root: B256::from([0x44; 32]),
            timestamp: 1,
            gas_limit: 30_000_000,
            ..Default::default()
        }));
    input.proof_carry_data.chain_id = 167_013;
    input.proof_carry_data.transition_input.proposal_id = 7;
    input.proof_carry_data.transition_input.parent_block_hash = B256::from([0x33; 32]);
    input
}

#[test]
fn adapter_projects_guest_input_into_execution_packet() {
    let input = fixture_guest_input();
    let packet = build_shasta_packet(&input).expect("build packet");

    assert_eq!(packet.schema, RAIKO2_SHASTA_REQUEST_SCHEMA);
    assert_eq!(
        packet.payload.guest_input.witnesses.len(),
        input.witnesses.len()
    );
    assert_eq!(
        packet.payload.guest_input.proposal_ancestor_headers.len(),
        1
    );
    assert_eq!(
        packet
            .payload
            .guest_input
            .proof_carry_data
            .transition_input
            .checkpoint,
        input.proof_carry_data.transition_input.checkpoint
    );
    assert_eq!(
        packet.payload.guest_input.witnesses[0].block.header.number,
        42
    );
    assert_eq!(
        packet.payload.guest_input.witnesses[0]
            .block
            .header
            .parent_hash,
        B256::from([0x11; 32])
    );
    assert_eq!(
        packet.payload.guest_input.witnesses[0]
            .witness
            .headers
            .len(),
        1
    );
}

#[test]
fn adapter_keeps_only_replay_parent_header_full() {
    let mut input = fixture_guest_input();
    input.proposal_ancestor_headers.insert(
        0,
        WitnessHeader::from_compact_header(&Header {
            number: 40,
            parent_hash: B256::from([0x55; 32]),
            timestamp: 0,
            gas_limit: 30_000_000,
            ..Default::default()
        }),
    );

    let mut next_witness = StatelessInput::default();
    next_witness.block.header.number = 43;
    next_witness.chain_spec.chain_id = 167_013;
    input.witnesses.push(next_witness);

    let packet = build_shasta_packet(&input).expect("build packet");
    let first_headers = &packet.payload.guest_input.witnesses[0].witness.headers;
    let second_headers = &packet.payload.guest_input.witnesses[1].witness.headers;

    assert_eq!(first_headers.len(), 2);
    assert!(first_headers[0].full_header().is_none());
    assert!(first_headers[1].full_header().is_some());
    assert_eq!(second_headers.len(), 3);
    assert!(second_headers[0].full_header().is_none());
    assert!(second_headers[1].full_header().is_none());
    assert!(second_headers[2].full_header().is_some());
}

#[test]
fn adapter_rejects_guest_input_without_witnesses() {
    let err = build_shasta_packet(&GuestInput::default()).expect_err("reject empty witness list");
    assert!(
        err.to_string()
            .contains("cannot build remote prover shasta packet without witnesses")
    );
}

#[test]
fn adapter_rejects_unset_proof_carry_chain_id() {
    let mut input = fixture_guest_input();
    input.proof_carry_data.chain_id = 0;

    let err = build_shasta_packet(&input).expect_err("reject unset carry chain id");
    assert!(err.to_string().contains("proof_carry_data.chain_id"));
}

#[test]
fn replay_block_wraps_stateless_input_fields() {
    let mut input = StatelessInput::default();
    input.block.header.number = 9;
    input.chain_spec.chain_id = 1234;

    let replay = Raiko2ReplayBlock::from(input);

    assert_eq!(replay.block.header.number, 9);
    assert_eq!(replay.chain_spec.chain_id, 1234);
}
