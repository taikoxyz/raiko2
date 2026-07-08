#![allow(missing_docs)]

use std::{fs, path::PathBuf};

use raiko2_primitives_shasta::GuestInput;
use raiko2_prover::remote_prover::{
    adapter::build_shasta_packet, protocol::RAIKO2_SHASTA_REQUEST_SCHEMA,
};

#[test]
fn shared_shasta_fixture_adapts_into_stable_remote_prover_v1_packet() {
    let raw = fs::read_to_string(shared_fixture_path()).expect("read shared shasta guest input");
    let guest_input: GuestInput = serde_json::from_str(&raw).expect("parse shared fixture");
    let packet = build_shasta_packet(&guest_input).expect("build remote prover packet");
    let payload = &packet.payload.guest_input;

    assert_eq!(packet.schema, RAIKO2_SHASTA_REQUEST_SCHEMA);
    assert_eq!(payload.proof_carry_data.chain_id, 167_000);
    assert_eq!(payload.witnesses.len(), 192);
    assert_eq!(payload.proof_carry_data.transition_input.proposal_id, 2_222);
    assert_eq!(payload.witnesses[0].block.header.number, 5_412_225);
    assert_eq!(payload.witnesses[191].block.header.number, 5_412_416);
    assert_eq!(payload.witnesses[0].witness.headers.len(), 256);
    assert_eq!(payload.witnesses[0].witness.state.len(), 99);
    assert!(payload.witnesses[0].witness.state_indices.is_empty());
    assert!(
        payload.witnesses[0]
            .witness
            .headers
            .iter()
            .all(|header| header.full_header().is_some()),
        "gaiko2 replay witnesses must carry full ancestor headers"
    );
}

fn shared_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_2222_l2_5412225_5412416.json")
}
