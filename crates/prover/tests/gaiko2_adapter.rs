#![allow(missing_docs)]

use alloy_primitives::B256;
use raiko2_primitives::StatelessInput;
use raiko2_primitives_shasta::GuestInput;
use raiko2_prover::gaiko2::{
    adapter::build_shasta_packet,
    protocol::{GAIKO2_SHASTA_REQUEST_SCHEMA, Gaiko2ReplayBlock},
};

fn fixture_guest_input() -> GuestInput {
    let mut witness = StatelessInput::default();
    witness.block.header.number = 42;
    witness.block.header.parent_hash = B256::from([0x11; 32]);
    witness.block.header.state_root = B256::from([0x22; 32]);
    witness.chain_spec.chain_id = 167_013;

    let mut input = GuestInput::default();
    input.witnesses.push(witness);
    input.proof_carry_data.chain_id = 167_013;
    input.proof_carry_data.transition_input.proposal_id = 7;
    input.proof_carry_data.transition_input.parent_block_hash = B256::from([0x33; 32]);
    input
}

#[test]
fn adapter_projects_guest_input_into_execution_packet() {
    let input = fixture_guest_input();
    let packet = build_shasta_packet(&input).expect("build packet");

    assert_eq!(packet.schema, GAIKO2_SHASTA_REQUEST_SCHEMA);
    assert_eq!(packet.payload.blocks.len(), input.witnesses.len());
    assert_eq!(packet.payload.chain_id, input.proof_carry_data.chain_id);
    assert_eq!(
        packet.payload.proof_carry_data.transition_input.checkpoint,
        input.proof_carry_data.transition_input.checkpoint
    );
    assert_eq!(packet.payload.blocks[0].block.header.number, 42);
    assert_eq!(
        packet.payload.blocks[0].block.header.parent_hash,
        B256::from([0x11; 32])
    );
}

#[test]
fn adapter_rejects_guest_input_without_witnesses() {
    let err = build_shasta_packet(&GuestInput::default()).expect_err("reject empty witness list");
    assert!(
        err.to_string()
            .contains("cannot build gaiko2 shasta packet without witnesses")
    );
}

#[test]
fn replay_block_wraps_stateless_input_fields() {
    let mut input = StatelessInput::default();
    input.block.header.number = 9;
    input.chain_spec.chain_id = 1234;

    let replay = Gaiko2ReplayBlock::from(input);

    assert_eq!(replay.block.header.number, 9);
    assert_eq!(replay.chain_spec.chain_id, 1234);
}
