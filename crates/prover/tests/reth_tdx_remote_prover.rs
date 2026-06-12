#![allow(missing_docs)]

use alloy_primitives::{Address, B256, Uint, hex};
use httpmock::Method::POST;
use httpmock::MockServer;
use raiko2_pipeline::NativeBackend;
use raiko2_primitives::tdx::{
    TdxDirectAggregateGuestInput, TdxDirectAggregateProposal, TdxDirectAggregateTransition,
};
use raiko2_primitives::{AggregationGuestInput, Proof, ProverConfig};
use raiko2_primitives_shasta::{
    GuestInput,
    instance::{build_shasta_commitment_from_proof_carry_data_vec, shasta_aggregation_output},
};
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
    GuestInput {
        proof_carry_data: ProofCarryData {
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
        },
        ..Default::default()
    }
}

fn carry_for_response() -> ProofCarryData {
    fixture_guest_input().proof_carry_data
}

fn direct_carry_for_response() -> ProofCarryData {
    let mut carry = carry_for_response();
    carry.transition_input.checkpoint.blockNumber = Uint::from(43);
    carry
}

fn tdx_proof_hex() -> String {
    let mut proof = Vec::with_capacity(85);
    proof.extend([0x11; 20]);
    proof.extend([0x22; 65]);
    format!("0x{}", hex::encode(proof))
}

fn tdx_instance_address() -> Address {
    Address::from([0x11; 20])
}

fn expected_tdx_input_hex(carry_vec: &[ProofCarryData]) -> String {
    let commitment = build_shasta_commitment_from_proof_carry_data_vec(carry_vec)
        .expect("test carry data should build commitment");
    let first = carry_vec.first().expect("non-empty test carry vec");
    let input = shasta_aggregation_output(
        &commitment,
        first.chain_id,
        first.verifier,
        tdx_instance_address(),
    );
    format!("0x{}", hex::encode(input))
}

fn direct_aggregate_input() -> TdxDirectAggregateGuestInput {
    let carry = carry_for_response();
    TdxDirectAggregateGuestInput {
        proposals: vec![TdxDirectAggregateProposal {
            chain_id: carry.chain_id,
            verifier: carry.verifier,
            proposal_id: carry.transition_input.proposal_id,
            proposal_hash: carry.transition_input.proposal_hash,
            parent_proposal_hash: carry.transition_input.parent_proposal_hash,
            actual_prover: carry.transition_input.actual_prover,
            transition: TdxDirectAggregateTransition {
                proposer: carry.transition_input.transition.proposer,
                timestamp: carry.transition_input.transition.timestamp,
            },
            l2_block_numbers: vec![42, 43],
        }],
    }
}

fn sgx_style_proof_with_instance_id_hex() -> String {
    let mut proof = Vec::with_capacity(89);
    proof.extend([0x12; 4]);
    proof.extend([0x11; 20]);
    proof.extend([0x22; 65]);
    format!("0x{}", hex::encode(proof))
}

/// Carry data for a *different* proposal than [`fixture_guest_input`] requests —
/// simulates a misbehaving/compromised remote prover echoing a substituted proof.
fn tampered_carry() -> ProofCarryData {
    let mut carry = carry_for_response();
    carry.transition_input.proposal_id = 999; // requested was 42
    carry
}

#[tokio::test]
async fn reth_tdx_prover_posts_shasta_packet_and_maps_success_response() {
    let server = MockServer::start();
    let signing_hash = expected_tdx_input_hex(&[direct_carry_for_response()]);
    let proof_bytes = tdx_proof_hex();
    let expected_proof = proof_bytes.clone();
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
                    "proof": proof_bytes,
                    "quote": "0xquote",
                    "input": signing_hash,
                    "instance_address": "0xinstance",
                    "proof_carry_data_vec": [direct_carry_for_response()],
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
    assert_eq!(proof.proof.as_deref(), Some(expected_proof.as_str()));
    assert_eq!(
        hex::decode(expected_proof.trim_start_matches("0x"))
            .expect("valid proof hex")
            .len(),
        85
    );
    assert_eq!(proof.quote.as_deref(), Some("0xquote"));
    assert_eq!(
        proof.input,
        Some(
            signing_hash
                .parse()
                .expect("expected signing hash should parse")
        )
    );

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
    let signing_hash = expected_tdx_input_hex(&[carry_for_response()]);
    let proof_bytes = tdx_proof_hex();
    let mock = server.mock(|when, then| {
        when.method(POST).path("/prove/shasta");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "schema": RETH_TDX_PROOF_RESPONSE_SCHEMA,
                "status": "ok",
                "result": {
                    "proof": proof_bytes,
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
async fn reth_tdx_prover_rejects_input_hash_mismatched_with_carry_and_instance() {
    let server = MockServer::start();
    let proof_bytes = tdx_proof_hex();
    let mock = server.mock(|when, then| {
        when.method(POST).path("/prove/shasta");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "schema": RETH_TDX_PROOF_RESPONSE_SCHEMA,
                "status": "ok",
                "result": {
                    "proof": proof_bytes,
                    "quote": "0xquote",
                    "input": format!("0x{}", hex::encode([0x44; 32])),
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
        .expect_err("mismatched input hash should fail");

    mock.assert();
    assert!(
        err.to_string().contains("input hash does not match"),
        "got: {err}"
    );
}

#[tokio::test]
async fn reth_tdx_prover_rejects_sgx_style_proof_with_instance_id_prefix() {
    let server = MockServer::start();
    let signing_hash = expected_tdx_input_hex(&[carry_for_response()]);
    let sgx_style_proof = sgx_style_proof_with_instance_id_hex();
    let mock = server.mock(|when, then| {
        when.method(POST).path("/prove/shasta");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "schema": RETH_TDX_PROOF_RESPONSE_SCHEMA,
                "status": "ok",
                "result": {
                    "proof": sgx_style_proof,
                    "quote": "0xquote",
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

    let err = prover
        .prove(
            fixture_guest_input(),
            &ProverConfig::default(),
            &NativeBackend,
        )
        .await
        .expect_err("SGX-style 89-byte TDX proof should be rejected");

    mock.assert();
    assert!(
        err.to_string().contains("invalid reth-tdx proof length"),
        "got: {err}"
    );
    assert!(err.to_string().contains("expected 85"), "got: {err}");
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
    let signing_hash = expected_tdx_input_hex(&[carry_for_response()]);
    let proof_bytes = tdx_proof_hex();
    let expected_proof = proof_bytes.clone();
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
                    "proof": proof_bytes,
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
        tdx_direct: None,
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
    assert_eq!(proof.proof.as_deref(), Some(expected_proof.as_str()));
    assert_eq!(proof.quote.as_deref(), Some("0xaggquote"));
    assert_eq!(
        proof.input,
        Some(
            signing_hash
                .parse()
                .expect("expected signing hash should parse")
        )
    );

    // The aggregation proof must carry the full vec under
    // `extra_data.shasta.proof_carry_data_vec` so on-chain callers can compute
    // `hashCommitment(commitment)` from the original sub-proof inputs.
    let extra_data = proof
        .extra_data
        .as_ref()
        .expect("aggregation extra_data populated");
    let vec_field = extra_data
        .pointer("/shasta/proof_carry_data_vec")
        .expect("proof_carry_data_vec stored in extra_data");
    assert!(vec_field.is_array(), "expected an array, got {vec_field}");
    assert_eq!(vec_field.as_array().unwrap().len(), 1);
    assert!(
        extra_data.pointer("/reth_tdx/schema").is_some(),
        "reth_tdx metadata must remain alongside the carry vec"
    );
}

#[tokio::test]
async fn reth_tdx_prover_direct_aggregates_from_proposals_without_subproofs() {
    let server = MockServer::start();
    let signing_hash = expected_tdx_input_hex(&[direct_carry_for_response()]);
    let proof_bytes = tdx_proof_hex();
    let expected_proof = proof_bytes.clone();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/prove/shasta-direct-aggregate")
            .body_contains("reth-tdx-shasta-direct-aggregate-request-v1")
            .body_contains("\"l2_block_numbers\":[42,43]");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "schema": RETH_TDX_PROOF_RESPONSE_SCHEMA,
                "status": "ok",
                "result": {
                    "proof": proof_bytes,
                    "quote": "0xdirectquote",
                    "input": signing_hash,
                    "proof_carry_data_vec": [direct_carry_for_response()],
                }
            }));
    });

    let prover = RethTdxProver::new(&RethTdxConfig {
        base_url: server.base_url(),
        timeout_ms: 5_000,
    })
    .expect("build reth-tdx prover");

    let proof = Prover::<NativeBackend>::aggregate(
        &prover,
        AggregationGuestInput {
            proofs: Vec::new(),
            tdx_direct: Some(direct_aggregate_input()),
        },
        &ProverConfig::default(),
        &NativeBackend,
    )
    .await
    .expect("direct aggregate");

    mock.assert();
    assert_eq!(proof.proof.as_deref(), Some(expected_proof.as_str()));
    assert_eq!(proof.quote.as_deref(), Some("0xdirectquote"));
    assert_eq!(
        proof.input,
        Some(
            signing_hash
                .parse()
                .expect("expected signing hash should parse")
        )
    );
}

#[tokio::test]
async fn reth_tdx_prover_rejects_direct_aggregate_non_contiguous_proposals() {
    let server = MockServer::start();
    let mut input = direct_aggregate_input();
    let carry = carry_for_response();
    let mut next = input.proposals[0].clone();
    next.proposal_id = carry.transition_input.proposal_id + 2;
    next.l2_block_numbers = vec![44];
    input.proposals.push(next);

    let prover = RethTdxProver::new(&RethTdxConfig {
        base_url: server.base_url(),
        timeout_ms: 5_000,
    })
    .expect("build reth-tdx prover");

    let err = Prover::<NativeBackend>::aggregate(
        &prover,
        AggregationGuestInput {
            proofs: Vec::new(),
            tdx_direct: Some(input),
        },
        &ProverConfig::default(),
        &NativeBackend,
    )
    .await
    .expect_err("non-contiguous direct aggregate proposals should be rejected");

    assert!(
        err.to_string().contains("proposal_id must be contiguous"),
        "got: {err}"
    );
}

#[tokio::test]
async fn reth_tdx_prover_rejects_aggregate_missing_proof_carry_data_vec() {
    let server = MockServer::start();
    let signing_hash = expected_tdx_input_hex(&[carry_for_response()]);
    let proof_bytes = tdx_proof_hex();
    let _mock = server.mock(|when, then| {
        when.method(POST).path("/prove/shasta-aggregate");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "schema": RETH_TDX_PROOF_RESPONSE_SCHEMA,
                "status": "ok",
                "result": {
                    "proof": proof_bytes,
                    "quote": "0xaggquote",
                    "input": signing_hash,
                    // proof_carry_data_vec deliberately omitted.
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
        tdx_direct: None,
    };

    let err = Prover::<NativeBackend>::aggregate(
        &prover,
        aggregate_input,
        &ProverConfig::default(),
        &NativeBackend,
    )
    .await
    .expect_err("aggregation missing carry vec must error");
    assert!(err.to_string().contains("proof_carry_data_vec"));
}

#[tokio::test]
async fn reth_tdx_prover_rejects_carry_for_a_different_proposal() {
    // The remote echoes a syntactically valid proof whose carry data describes a
    // *different* proposal than we asked it to prove. raiko2 must refuse it
    // rather than package a silently-substituted proof.
    let server = MockServer::start();
    let signing_hash = expected_tdx_input_hex(&[tampered_carry()]);
    let proof_bytes = tdx_proof_hex();
    let mock = server.mock(|when, then| {
        when.method(POST).path("/prove/shasta");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "schema": RETH_TDX_PROOF_RESPONSE_SCHEMA,
                "status": "ok",
                "result": {
                    "proof": proof_bytes,
                    "quote": "0xquote",
                    "input": signing_hash,
                    "proof_carry_data_vec": [tampered_carry()],
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
        .expect_err("substituted carry should be rejected");

    mock.assert();
    assert!(
        err.to_string()
            .contains("does not match the requested proposal"),
        "got: {err}"
    );
    assert!(err.to_string().contains("proposal_id"), "got: {err}");
}

#[tokio::test]
async fn reth_tdx_prover_rejects_aggregate_carry_for_a_different_proposal() {
    let server = MockServer::start();
    let signing_hash = expected_tdx_input_hex(&[tampered_carry()]);
    let proof_bytes = tdx_proof_hex();
    let _mock = server.mock(|when, then| {
        when.method(POST).path("/prove/shasta-aggregate");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "schema": RETH_TDX_PROOF_RESPONSE_SCHEMA,
                "status": "ok",
                "result": {
                    "proof": proof_bytes,
                    "quote": "0xaggquote",
                    "input": signing_hash,
                    "proof_carry_data_vec": [tampered_carry()],
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
            "shasta": { "proof_carry_data": carry_for_response() }
        })),
        ..Default::default()
    };
    let aggregate_input = raiko2_primitives::AggregationGuestInput {
        proofs: vec![sub_proof],
        tdx_direct: None,
    };

    let err = Prover::<NativeBackend>::aggregate(
        &prover,
        aggregate_input,
        &ProverConfig::default(),
        &NativeBackend,
    )
    .await
    .expect_err("substituted aggregate carry should be rejected");
    assert!(
        err.to_string()
            .contains("does not match the requested proposal"),
        "got: {err}"
    );
}

#[tokio::test]
async fn reth_tdx_prover_rejects_invalid_input_hash() {
    let server = MockServer::start();
    let proof_bytes = tdx_proof_hex();
    let mock = server.mock(|when, then| {
        when.method(POST).path("/prove/shasta");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "schema": RETH_TDX_PROOF_RESPONSE_SCHEMA,
                "status": "ok",
                "result": {
                    "proof": proof_bytes,
                    "quote": "0xquote",
                    "input": "0xnot-a-valid-hash",
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
        .expect_err("invalid input hash should fail");

    mock.assert();
    assert!(
        err.to_string().contains("invalid reth-tdx input hash"),
        "got: {err}"
    );
}

#[tokio::test]
async fn reth_tdx_prover_rejects_non_json_body() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST).path("/prove/shasta");
        then.status(200)
            .header("content-type", "application/json")
            .body("this is definitely not json");
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
        .expect_err("non-json body should fail");

    mock.assert();
    assert!(
        err.to_string().contains("response decode failed"),
        "got: {err}"
    );
}
