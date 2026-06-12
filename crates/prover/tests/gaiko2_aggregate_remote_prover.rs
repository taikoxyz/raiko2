#![allow(missing_docs)]

use alloy_primitives::{Address, B256, Uint};
use httpmock::Method::POST;
use httpmock::MockServer;
use raiko2_pipeline::NativeBackend;
use raiko2_primitives::{AggregationGuestInput, Proof, ProverConfig};
use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
use raiko2_protocol_shasta::shasta::ProofCarryData;
use raiko2_prover::{
    Prover,
    gaiko2::{Gaiko2Config, Gaiko2Prover},
    remote_prover::protocol::{
        RAIKO2_PROOF_RESPONSE_SCHEMA, RAIKO2_SHASTA_AGGREGATE_REQUEST_SCHEMA,
    },
};
use serde_json::json;
use std::str::FromStr;

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
        input: Some(hash_shasta_subproof_input(&carry)),
        extra_data: Some(
            raiko2_primitives_shasta::encode_proof_carry_data(&carry).expect("encode carry data"),
        ),
        ..Proof::default()
    }
}

#[tokio::test]
async fn gaiko2_prover_posts_shasta_aggregate_packet_and_maps_success_response() {
    let server = MockServer::start();
    let expected_input = format!("0x{}", hex::encode([0x77; 32]));
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/prove/shasta-aggregate")
            .body_contains(RAIKO2_SHASTA_AGGREGATE_REQUEST_SCHEMA)
            .body_contains("proof_carry_data");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "schema": RAIKO2_PROOF_RESPONSE_SCHEMA,
                "status": "ok",
                "result": {
                    "proof": "0xaggproof",
                    "public_key": "0xpub",
                    "instance_address": "0xaddr",
                    "input": expected_input,
                }
            }));
    });

    let prover = Gaiko2Prover::new(&Gaiko2Config {
        base_url: server.base_url(),
        timeout_ms: 5_000,
    })
    .expect("build gaiko2 prover");

    let proof = prover
        .aggregate(
            AggregationGuestInput {
                proofs: vec![fixture_aggregate_proof()],
                tdx_direct: None,
            },
            &ProverConfig::default(),
            &NativeBackend,
        )
        .await
        .expect("remote aggregate");

    mock.assert();
    assert_eq!(proof.proof.as_deref(), Some("0xaggproof"));
    assert_eq!(proof.input, Some(B256::from([0x77; 32])));
    let extra = proof.extra_data.expect("extra_data");
    assert_eq!(
        extra["gaiko2"]["schema"].as_str(),
        Some(RAIKO2_PROOF_RESPONSE_SCHEMA)
    );
}
