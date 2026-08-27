#![allow(missing_docs)]

use alloy_primitives::{Address, B256};
use raiko2_prover::remote_prover::protocol::{
    RAIKO2_PROOF_RESPONSE_SCHEMA, RAIKO2_SHASTA_AGGREGATE_REQUEST_SCHEMA,
    RAIKO2_SHASTA_REQUEST_SCHEMA, Raiko2ProofResponse, Raiko2ProofResult,
    Raiko2ProposalAggregatePayload, Raiko2ProposalAggregateRequest, Raiko2ProposalGuestInput,
    Raiko2ProposalPayload, Raiko2ProposalRequest, Raiko2ReplayBlock,
};

#[test]
fn shasta_packet_roundtrip_preserves_guest_input_payload() {
    let mut replay_block = Raiko2ReplayBlock::default();
    replay_block.block.header.number = 42;
    replay_block.block.header.parent_hash = B256::from([0x11; 32]);
    replay_block.chain_spec.chain_id = 167_013;

    let mut proof_carry_data = raiko2_protocol_shasta::shasta::ProofCarryData {
        chain_id: 167_013,
        ..Default::default()
    };
    proof_carry_data.transition_input.proposal_id = 7;

    let packet = Raiko2ProposalRequest {
        schema: RAIKO2_SHASTA_REQUEST_SCHEMA.to_string(),
        payload: Raiko2ProposalPayload {
            guest_input: Raiko2ProposalGuestInput {
                witnesses: vec![replay_block],
                proof_carry_data,
                ..Default::default()
            },
        },
    };

    let json = serde_json::to_string(&packet).expect("serialize request");
    let decoded: Raiko2ProposalRequest = serde_json::from_str(&json).expect("deserialize request");

    assert_eq!(decoded.schema, RAIKO2_SHASTA_REQUEST_SCHEMA);
    assert_eq!(decoded.payload.guest_input.witnesses.len(), 1);
    assert_eq!(
        decoded.payload.guest_input.proof_carry_data.chain_id,
        167_013
    );
    assert_eq!(
        decoded
            .payload
            .guest_input
            .proof_carry_data
            .transition_input
            .proposal_id,
        7
    );
}

#[test]
fn proof_response_roundtrip_preserves_success_payload() {
    let response = Raiko2ProofResponse::success(Raiko2ProofResult {
        proof: Some("0xproof".to_string()),
        quote: Some("0xquote".to_string()),
        public_key: Some("0xpub".to_string()),
        instance_address: Some("0xaddr".to_string()),
        input: "0xinput".to_string(),
    });

    let json = serde_json::to_string(&response).expect("serialize response");
    let decoded: Raiko2ProofResponse = serde_json::from_str(&json).expect("deserialize response");

    assert_eq!(decoded.schema, RAIKO2_PROOF_RESPONSE_SCHEMA);
    assert_eq!(decoded.status.as_str(), "ok");
    assert_eq!(
        decoded.result.as_ref().expect("result payload").input,
        "0xinput"
    );
    assert!(decoded.error.is_none());
}

#[test]
fn shasta_aggregate_packet_roundtrip_preserves_schema_and_payload() {
    let mut proof_carry_data = raiko2_protocol_shasta::shasta::ProofCarryData {
        chain_id: 167_013,
        ..Default::default()
    };
    proof_carry_data.transition_input.proposal_id = 7;

    let packet = Raiko2ProposalAggregateRequest {
        schema: RAIKO2_SHASTA_AGGREGATE_REQUEST_SCHEMA.to_string(),
        payload: Raiko2ProposalAggregatePayload {
            proofs: vec![
                raiko2_prover::remote_prover::protocol::Raiko2AggregateProof {
                    input: format!("0x{}", hex::encode([0x11; 32])),
                    proof: "0xproof".to_string(),
                    proof_carry_data,
                },
            ],
        },
    };

    let json = serde_json::to_string(&packet).expect("serialize aggregate request");
    let decoded: Raiko2ProposalAggregateRequest =
        serde_json::from_str(&json).expect("deserialize aggregate request");

    assert_eq!(decoded.schema, RAIKO2_SHASTA_AGGREGATE_REQUEST_SCHEMA);
    assert_eq!(decoded.payload.proofs.len(), 1);
    assert_eq!(decoded.payload.proofs[0].proof, "0xproof");
    assert_eq!(decoded.payload.proofs[0].proof_carry_data.chain_id, 167_013);
}

#[test]
fn proof_response_roundtrip_preserves_error_payload() {
    let response =
        Raiko2ProofResponse::error(raiko2_prover::remote_prover::protocol::Raiko2ProofError {
            code: "INVALID_REQUEST".to_string(),
            message: "bad request".to_string(),
        });

    let json = serde_json::to_string(&response).expect("serialize error response");
    let decoded: Raiko2ProofResponse =
        serde_json::from_str(&json).expect("deserialize error response");

    assert_eq!(decoded.schema, RAIKO2_PROOF_RESPONSE_SCHEMA);
    assert_eq!(decoded.status.as_str(), "error");
    assert!(decoded.result.is_none());
    let err = decoded.error.expect("error payload");
    assert_eq!(err.code, "INVALID_REQUEST");
    assert_eq!(err.message, "bad request");
}

#[test]
fn aggregate_proof_from_proof_rejects_input_hash_mismatch() {
    let mut proof_carry_data = raiko2_protocol_shasta::shasta::ProofCarryData {
        chain_id: 167_013,
        verifier: Address::from([0xf9; 20]),
        ..Default::default()
    };
    proof_carry_data.transition_input.proposal_id = 7;

    let proof = raiko2_primitives::Proof {
        proof: Some("0xproof".to_string()),
        input: Some(B256::from([0x99; 32])),
        extra_data: Some(serde_json::json!({
            "shasta": {
                "proof_carry_data": proof_carry_data
            }
        })),
        ..Default::default()
    };

    let err = raiko2_prover::remote_prover::protocol::Raiko2AggregateProof::from_proof(&proof)
        .expect_err("mismatch should fail");
    assert!(err.to_string().contains("input hash"));
}

#[test]
fn aggregate_proof_from_proof_rejects_oversized_timestamp_without_panicking() {
    let mut proof_carry_data = raiko2_protocol_shasta::shasta::ProofCarryData::default();
    proof_carry_data.transition_input.transition.timestamp = 1_u64 << 48;

    let proof = raiko2_primitives::Proof {
        proof: Some("0xproof".to_string()),
        extra_data: Some(serde_json::json!({
            "shasta": {
                "proof_carry_data": proof_carry_data
            }
        })),
        ..Default::default()
    };

    let result = std::panic::catch_unwind(|| {
        raiko2_prover::remote_prover::protocol::Raiko2AggregateProof::from_proof(&proof)
    });

    assert!(result.is_ok(), "invalid carry data must not panic");
    let err = result
        .expect("checked above")
        .expect_err("oversized timestamp must be rejected");
    assert!(err.to_string().contains("invalid shasta proof carry data"));
}
