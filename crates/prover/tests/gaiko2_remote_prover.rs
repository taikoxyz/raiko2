#![allow(missing_docs)]

use alloy_consensus::Header;
use alloy_primitives::{B256, hex};
use httpmock::Method::POST;
use httpmock::MockServer;
use raiko2_pipeline::NativeBackend;
use raiko2_primitives::{ProverConfig, WitnessHeader};
use raiko2_primitives_shasta::GuestInput;
use raiko2_prover::{
    Prover,
    gaiko2::{
        Gaiko2Config, Gaiko2Prover,
        protocol::{GAIKO2_PROOF_RESPONSE_SCHEMA, GAIKO2_SHASTA_REQUEST_SCHEMA, Gaiko2ProofError},
    },
};
use serde_json::json;

fn fixture_guest_input() -> GuestInput {
    let mut input = GuestInput::default();
    let mut witness = raiko2_primitives::StatelessInput::default();
    witness.block.header.number = 42;
    witness.block.header.parent_hash = B256::from([0x11; 32]);
    witness.block.header.state_root = B256::from([0x22; 32]);
    witness.chain_spec.chain_id = 167_013;

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
    input.proof_carry_data.transition_input.parent_block_hash = B256::from([0x33; 32]);
    input
}

#[tokio::test]
async fn gaiko2_prover_posts_shasta_packet_and_maps_success_response() {
    let server = MockServer::start();
    let expected_input = format!("0x{}", hex::encode([0x44; 32]));
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/prove/shasta")
            .body_contains(GAIKO2_SHASTA_REQUEST_SCHEMA);
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "schema": GAIKO2_PROOF_RESPONSE_SCHEMA,
                "status": "ok",
                "result": {
                    "proof": "0xproof",
                    "quote": "0xquote",
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
        .prove(
            fixture_guest_input(),
            &ProverConfig::default(),
            &NativeBackend,
        )
        .await
        .expect("remote prove");

    mock.assert();
    assert_eq!(proof.proof.as_deref(), Some("0xproof"));
    assert_eq!(proof.quote.as_deref(), Some("0xquote"));
    assert_eq!(proof.input, Some(B256::from([0x44; 32])));
    let extra = proof.extra_data.expect("extra_data");
    assert_eq!(
        extra["gaiko2"]["schema"].as_str(),
        Some(GAIKO2_PROOF_RESPONSE_SCHEMA)
    );
    assert_eq!(extra["gaiko2"]["public_key"].as_str(), Some("0xpub"));
    assert_eq!(extra["gaiko2"]["instance_address"].as_str(), Some("0xaddr"));
}

#[tokio::test]
async fn gaiko2_prover_surfaces_remote_error_envelope() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST).path("/prove/shasta");
        then.status(400)
            .header("content-type", "application/json")
            .json_body(json!({
                "schema": GAIKO2_PROOF_RESPONSE_SCHEMA,
                "status": "error",
                "error": Gaiko2ProofError {
                    code: "INVALID_SCHEMA".to_string(),
                    message: "unsupported schema".to_string(),
                }
            }));
    });

    let prover = Gaiko2Prover::new(&Gaiko2Config {
        base_url: server.base_url(),
        timeout_ms: 5_000,
    })
    .expect("build gaiko2 prover");

    let err = prover
        .prove(
            fixture_guest_input(),
            &ProverConfig::default(),
            &NativeBackend,
        )
        .await
        .expect_err("remote proof should fail");

    mock.assert();
    assert!(err.to_string().contains("INVALID_SCHEMA"));
    assert!(err.to_string().contains("unsupported schema"));
}
