#![allow(missing_docs)]

use std::{fs, path::PathBuf};

use raiko2_primitives_shasta::GuestInput;
use raiko2_prover::remote_prover::adapter::build_shasta_packet;
use serde_json::Value;

#[test]
fn shared_shasta_fixture_adapts_into_stable_remote_prover_packet() {
    let raw = fs::read_to_string(shared_fixture_path()).expect("read shared shasta guest input");
    let guest_input: GuestInput = serde_json::from_str(&raw).expect("parse shared fixture");
    let packet = build_shasta_packet(&guest_input).expect("build remote prover packet");

    assert_eq!(packet.payload.chain_id, 167_000);
    assert_eq!(packet.payload.blocks.len(), 192);
    assert_eq!(
        packet.payload.proof_carry_data.transition_input.proposal_id,
        2_222
    );
    assert_eq!(packet.payload.blocks[0].block.header.number, 5_412_225);
    assert_eq!(packet.payload.blocks[191].block.header.number, 5_412_416);
    assert_eq!(packet.payload.blocks[0].witness.headers.len(), 256);
    assert_eq!(packet.payload.blocks[0].witness.state.len(), 99);
    assert!(packet.payload.blocks[0].witness.state_indices.is_empty());
    assert!(
        packet.payload.blocks[0]
            .witness
            .headers
            .last()
            .expect("first block witness should contain a parent header")
            .full_header()
            .is_some()
    );

    let actual = canonical_json(&serde_json::to_value(&packet).expect("serialize packet to value"));
    let expected_raw = fs::read_to_string(expected_packet_path())
        .expect("read checked-in remote prover packet fixture");
    let expected = canonical_json(
        &serde_json::from_str::<Value>(&expected_raw)
            .expect("parse checked-in remote prover packet"),
    );

    assert_eq!(
        actual, expected,
        "adapter output drifted from the fixed v1 golden fixture"
    );
}

fn shared_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_2222_l2_5412225_5412416.json")
}

fn expected_packet_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../tests/fixtures/remote_prover/shasta_request_v1_taiko_mainnet_proposal_2222_l2_5412225_5412416.json",
    )
}

fn canonical_json(value: &Value) -> String {
    let mut out = String::new();
    write_canonical_json(value, &mut out);
    out
}

fn write_canonical_json(value: &Value, out: &mut String) {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            out.push_str(&serde_json::to_string(value).expect("encode scalar"));
        }
        Value::Array(values) => {
            out.push('[');
            for (index, item) in values.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical_json(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            for (index, (key, item)) in entries.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).expect("encode key"));
                out.push(':');
                write_canonical_json(item, out);
            }
            out.push('}');
        }
    }
}
