#![allow(missing_docs)]

use alloy_primitives::{Address, B256, Uint};
use httpmock::Method::POST;
use httpmock::MockServer;
use raiko2_pipeline::NativeBackend;
use raiko2_primitives::{AggregationGuestInput, Proof, ProofType, ProverConfig};
use raiko2_primitives_shasta::{
    instance::{build_shasta_commitment_from_proof_carry_data_vec, shasta_aggregation_output},
    proof_carry_from_proof,
};
use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
use raiko2_protocol_shasta::shasta::ProofCarryData;
use raiko2_prover::{
    Prover,
    gaiko2::{Gaiko2Config, Gaiko2Prover},
    remote_prover::{
        adapter::build_shasta_aggregate_request,
        protocol::{RAIKO2_PROOF_RESPONSE_SCHEMA, RAIKO2_SHASTA_AGGREGATE_REQUEST_SCHEMA},
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

fn expected_aggregate_input(proofs: &[Proof], instance_address: Address) -> B256 {
    let carries = proofs
        .iter()
        .map(|proof| {
            proof_carry_from_proof(proof)
                .expect("decode proof carry data")
                .expect("proof carry data")
        })
        .collect::<Vec<_>>();
    let commitment =
        build_shasta_commitment_from_proof_carry_data_vec(&carries).expect("commitment");
    let first = carries.first().expect("carry data");

    shasta_aggregation_output(
        &commitment,
        first.chain_id,
        first.verifier,
        instance_address,
    )
}

#[test]
fn shasta_aggregate_request_rejects_child_input_mismatch() {
    let mut proof = fixture_aggregate_proof();
    proof.input = Some(B256::from([0x99; 32]));

    let err = build_shasta_aggregate_request(&[proof]).expect_err("child input mismatch");

    assert!(err.to_string().contains("input hash"));
}

#[tokio::test]
async fn gaiko2_prover_posts_shasta_aggregate_packet_and_maps_success_response() {
    let server = MockServer::start();
    let proofs = vec![fixture_aggregate_proof()];
    let instance_address =
        Address::from_str("0x0000777735367b36bC9B61C50022d9D0700dB4Ec").expect("instance");
    let expected_input_hash = expected_aggregate_input(&proofs, instance_address);
    let expected_input = format!("{expected_input_hash:#x}");
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
                    "instance_address": format!("{instance_address:#x}"),
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
            AggregationGuestInput { proofs },
            &ProverConfig::default(),
            &NativeBackend,
        )
        .await
        .expect("remote aggregate");

    mock.assert();
    assert_eq!(proof.proof.as_deref(), Some("0xaggproof"));
    assert_eq!(proof.input, Some(expected_input_hash));
    let extra = proof.extra_data.expect("extra_data");
    assert_eq!(
        extra["sgxgeth"]["schema"].as_str(),
        Some(RAIKO2_PROOF_RESPONSE_SCHEMA)
    );
    assert!(extra.get("gaiko2").is_none());
}

#[tokio::test]
async fn gaiko2_prover_uses_sgx_namespace_for_sgx_aggregate_lane() {
    let server = MockServer::start();
    let proofs = vec![fixture_aggregate_proof()];
    let instance_address =
        Address::from_str("0x0000777735367b36bC9B61C50022d9D0700dB4Ec").expect("instance");
    let expected_input_hash = expected_aggregate_input(&proofs, instance_address);
    let expected_input = format!("{expected_input_hash:#x}");
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
                    "instance_address": format!("{instance_address:#x}"),
                    "input": expected_input,
                }
            }));
    });

    let prover = Gaiko2Prover::new_for_proof_type(
        &Gaiko2Config {
            base_url: server.base_url(),
            timeout_ms: 5_000,
        },
        ProofType::Sgx,
    )
    .expect("build sgx remote prover");

    let proof = prover
        .aggregate(
            AggregationGuestInput { proofs },
            &ProverConfig::default(),
            &NativeBackend,
        )
        .await
        .expect("remote aggregate");

    mock.assert();
    let extra = proof.extra_data.expect("extra_data");
    assert_eq!(
        extra["sgx"]["schema"].as_str(),
        Some(RAIKO2_PROOF_RESPONSE_SCHEMA)
    );
    assert!(extra.get("sgxgeth").is_none());
}

#[tokio::test]
async fn gaiko2_prover_rejects_aggregate_response_input_mismatch() {
    let server = MockServer::start();
    let proofs = vec![fixture_aggregate_proof()];
    let instance_address =
        Address::from_str("0x0000777735367b36bC9B61C50022d9D0700dB4Ec").expect("instance");
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
                    "instance_address": format!("{instance_address:#x}"),
                    "input": format!("0x{}", hex::encode([0x77; 32])),
                }
            }));
    });

    let prover = Gaiko2Prover::new(&Gaiko2Config {
        base_url: server.base_url(),
        timeout_ms: 5_000,
    })
    .expect("build gaiko2 prover");

    let err = prover
        .aggregate(
            AggregationGuestInput { proofs },
            &ProverConfig::default(),
            &NativeBackend,
        )
        .await
        .expect_err("mismatched aggregate response input should fail");

    mock.assert();
    assert!(err.to_string().contains("aggregate input hash"));
}
