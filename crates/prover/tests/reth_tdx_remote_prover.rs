#![allow(missing_docs)]

use alloy_primitives::{Address, B256, hex};
use httpmock::Method::POST;
use httpmock::MockServer;
use raiko2_pipeline::NativeBackend;
use raiko2_primitives::{Proof, ProverConfig};
use raiko2_primitives_shasta::GuestInput;
use raiko2_protocol_shasta::shasta::{
    Checkpoint, ProofCarryData, ShastaTransitionInput, TransitionInputData,
};
use raiko2_prover::{
    Prover,
    reth_tdx::{
        RethTdxConfig, RethTdxProver,
        protocol::{
            RETH_TDX_PROOF_RESPONSE_SCHEMA, RETH_TDX_SHASTA_AGGREGATE_REQUEST_SCHEMA,
            RETH_TDX_SHASTA_REQUEST_SCHEMA,
        },
    },
};
use serde_json::json;

fn fixture_guest_input() -> GuestInput {
    let mut input = GuestInput::default();
    input.proof_carry_data = ProofCarryData {
        chain_id: 167_013,
        verifier: Address::from([0x11; 20]),
        transition_input: TransitionInputData {
            proposal_id: 42,
            proposal_hash: B256::from([0xaa; 32]),
            parent_proposal_hash: B256::from([0xbb; 32]),
            parent_block_hash: B256::from([0xcc; 32]),
            actual_prover: Address::from([0x22; 20]),
            transition: ShastaTransitionInput {
                proposer: Address::from([0x33; 20]),
                timestamp: 1_700_000_000,
            },
            checkpoint: Checkpoint::default(),
        },
    };
    input
}

fn carry_for_response() -> ProofCarryData {
    fixture_guest_input().proof_carry_data
}

#[tokio::test]
async fn reth_tdx_prover_posts_shasta_packet_and_maps_success_response() {
    let server = MockServer::start();
    let signing_hash = format!("0x{}", hex::encode([0x44; 32]));
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/prove/shasta")
            .body_contains(RETH_TDX_SHASTA_REQUEST_SCHEMA);
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "schema": RETH_TDX_PROOF_RESPONSE_SCHEMA,
                "status": "ok",
                "result": {
                    "proof": "0xproof",
                    "quote": "0xquote",
                    "input": signing_hash,
                    "instance_address": "0xinstance",
                    "proof_carry_data_vec": [carry_for_response()],
                }
            }));
    });

    let prover = RethTdxProver::new(&RethTdxConfig {
        base_url: server.base_url(),
        timeout_ms: 5_000,
    })
    .expect("build reth-tdx prover");

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
        extra["reth_tdx"]["schema"].as_str(),
        Some(RETH_TDX_PROOF_RESPONSE_SCHEMA)
    );
    assert_eq!(
        extra["reth_tdx"]["instance_address"].as_str(),
        Some("0xinstance")
    );
    // Carry data echoed by reth-tdx must be reflected back in extra_data so
    // raiko2 can compute the on-chain commitment hash.
    let carry = &extra["shasta"]["proof_carry_data"];
    assert_eq!(carry["chain_id"].as_u64(), Some(167_013));
    assert_eq!(carry["transition_input"]["proposal_id"].as_u64(), Some(42));
}

#[tokio::test]
async fn reth_tdx_prover_rejects_missing_proof_carry_data_vec() {
    let server = MockServer::start();
    let signing_hash = format!("0x{}", hex::encode([0x55; 32]));
    let mock = server.mock(|when, then| {
        when.method(POST).path("/prove/shasta");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "schema": RETH_TDX_PROOF_RESPONSE_SCHEMA,
                "status": "ok",
                "result": {
                    "proof": "0xproof",
                    "quote": "0xquote",
                    "input": signing_hash,
                }
            }));
    });

    let prover = RethTdxProver::new(&RethTdxConfig {
        base_url: server.base_url(),
        timeout_ms: 5_000,
    })
    .expect("build reth-tdx prover");

    let err = prover
        .prove(
            fixture_guest_input(),
            &ProverConfig::default(),
            &NativeBackend,
        )
        .await
        .expect_err("missing carry data should fail");

    mock.assert();
    assert!(err.to_string().contains("proof_carry_data_vec"));
}

#[tokio::test]
async fn reth_tdx_prover_rejects_unsupported_response_schema() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST).path("/prove/shasta");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "schema": "reth-tdx-proof-vX",
                "status": "ok",
                "result": {
                    "proof": "0xproof",
                    "quote": "0xquote",
                    "input": format!("0x{}", hex::encode([0u8; 32])),
                    "proof_carry_data_vec": [carry_for_response()],
                }
            }));
    });

    let prover = RethTdxProver::new(&RethTdxConfig {
        base_url: server.base_url(),
        timeout_ms: 5_000,
    })
    .expect("build reth-tdx prover");

    let err = prover
        .prove(
            fixture_guest_input(),
            &ProverConfig::default(),
            &NativeBackend,
        )
        .await
        .expect_err("unsupported schema should fail");

    mock.assert();
    assert!(
        err.to_string()
            .contains("unsupported reth-tdx response schema")
    );
}

#[tokio::test]
async fn reth_tdx_prover_surfaces_remote_error_envelope() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST).path("/prove/shasta");
        then.status(500)
            .header("content-type", "application/json")
            .json_body(json!({
                "schema": RETH_TDX_PROOF_RESPONSE_SCHEMA,
                "status": "error",
                "error": {
                    "code": "TDX_QUOTE_FAILED",
                    "message": "tdxs unreachable"
                }
            }));
    });

    let prover = RethTdxProver::new(&RethTdxConfig {
        base_url: server.base_url(),
        timeout_ms: 5_000,
    })
    .expect("build reth-tdx prover");

    let err = prover
        .prove(
            fixture_guest_input(),
            &ProverConfig::default(),
            &NativeBackend,
        )
        .await
        .expect_err("remote error envelope should fail");

    mock.assert();
    assert!(err.to_string().contains("TDX_QUOTE_FAILED"));
    assert!(err.to_string().contains("tdxs unreachable"));
}

#[tokio::test]
async fn reth_tdx_prover_aggregates_and_maps_success_response() {
    let server = MockServer::start();
    let signing_hash = format!("0x{}", hex::encode([0x77; 32]));
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/prove/shasta-aggregate")
            .body_contains(RETH_TDX_SHASTA_AGGREGATE_REQUEST_SCHEMA);
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "schema": RETH_TDX_PROOF_RESPONSE_SCHEMA,
                "status": "ok",
                "result": {
                    "proof": "0xagg",
                    "quote": "0xaggquote",
                    "input": signing_hash,
                    "proof_carry_data_vec": [carry_for_response()],
                }
            }));
    });

    let prover = RethTdxProver::new(&RethTdxConfig {
        base_url: server.base_url(),
        timeout_ms: 5_000,
    })
    .expect("build reth-tdx prover");

    let sub_proof = Proof {
        proof: Some("0xsub".to_string()),
        input: Some(B256::from([0x88; 32])),
        quote: Some("0xsubquote".to_string()),
        extra_data: Some(serde_json::json!({
            "shasta": {
                "proof_carry_data": carry_for_response(),
            }
        })),
        ..Default::default()
    };
    let aggregate_input = raiko2_primitives::AggregationGuestInput {
        proofs: vec![sub_proof],
    };

    let proof = Prover::<NativeBackend>::aggregate(
        &prover,
        aggregate_input,
        &ProverConfig::default(),
        &NativeBackend,
    )
    .await
    .expect("aggregate");

    mock.assert();
    assert_eq!(proof.proof.as_deref(), Some("0xagg"));
    assert_eq!(proof.quote.as_deref(), Some("0xaggquote"));
    assert_eq!(proof.input, Some(B256::from([0x77; 32])));
}
