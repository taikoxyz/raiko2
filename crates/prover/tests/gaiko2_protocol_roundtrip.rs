#![allow(missing_docs)]

use alloy_primitives::{Address, B256};
use raiko2_prover::gaiko2::protocol::{
    GAIKO2_PROOF_RESPONSE_SCHEMA, GAIKO2_SHASTA_REQUEST_SCHEMA, Gaiko2ProofError,
    Gaiko2ProofResponse, Gaiko2ProofResult, Gaiko2ReplayBlock, Gaiko2ShastaAggregatePayload,
    Gaiko2ShastaAggregateRequest, Gaiko2ShastaPayload, Gaiko2ShastaRequest,
};

#[test]
fn shasta_packet_roundtrip_preserves_schema_and_payload() {
    let mut replay_block = Gaiko2ReplayBlock::default();
    replay_block.block.header.number = 42;
    replay_block.block.header.parent_hash = B256::from([0x11; 32]);
    replay_block.block.header.state_root = B256::from([0x22; 32]);
    replay_block.chain_spec.chain_id = 167_013;
    replay_block.accounts.insert(
        Address::from([0x33; 20]),
        alloy_consensus::TrieAccount::default(),
    );

    let mut proof_carry_data = raiko2_protocol_shasta::shasta::ProofCarryData::default();
    proof_carry_data.chain_id = 167_013;
    proof_carry_data.transition_input.proposal_id = 7;
    proof_carry_data.transition_input.parent_block_hash = B256::from([0x44; 32]);

    let packet = Gaiko2ShastaRequest {
        schema: GAIKO2_SHASTA_REQUEST_SCHEMA.to_string(),
        payload: Gaiko2ShastaPayload {
            chain_id: 167_013,
            blocks: vec![replay_block],
            proof_carry_data,
        },
    };

    let json = serde_json::to_string(&packet).expect("serialize request");
    let decoded: Gaiko2ShastaRequest = serde_json::from_str(&json).expect("deserialize request");

    assert_eq!(decoded.schema, GAIKO2_SHASTA_REQUEST_SCHEMA);
    assert_eq!(decoded.payload.chain_id, 167_013);
    assert_eq!(decoded.payload.blocks.len(), 1);
    assert_eq!(decoded.payload.blocks[0].block.header.number, 42);
    assert_eq!(
        decoded
            .payload
            .proof_carry_data
            .transition_input
            .proposal_id,
        7
    );
}

#[test]
fn proof_response_roundtrip_preserves_success_payload() {
    let response = Gaiko2ProofResponse::success(Gaiko2ProofResult {
        proof: Some("0xproof".to_string()),
        quote: Some("0xquote".to_string()),
        public_key: Some("0xpub".to_string()),
        instance_address: Some("0xaddr".to_string()),
        input: "0xinput".to_string(),
    });

    let json = serde_json::to_string(&response).expect("serialize response");
    let decoded: Gaiko2ProofResponse = serde_json::from_str(&json).expect("deserialize response");

    assert_eq!(decoded.schema, GAIKO2_PROOF_RESPONSE_SCHEMA);
    assert_eq!(decoded.status.as_str(), "ok");
    assert_eq!(
        decoded.result.as_ref().expect("result payload").input,
        "0xinput"
    );
    assert!(decoded.error.is_none());
}

#[test]
fn shasta_aggregate_packet_roundtrip_preserves_schema_and_payload() {
    let mut proof_carry_data = raiko2_protocol_shasta::shasta::ProofCarryData::default();
    proof_carry_data.chain_id = 167_013;
    proof_carry_data.transition_input.proposal_id = 7;

    let packet = Gaiko2ShastaAggregateRequest {
        schema: GAIKO2_SHASTA_REQUEST_SCHEMA.to_string(),
        payload: Gaiko2ShastaAggregatePayload {
            proofs: vec![raiko2_prover::gaiko2::protocol::Gaiko2AggregateProof {
                input: format!("0x{}", hex::encode([0x11; 32])),
                proof: "0xproof".to_string(),
                proof_carry_data,
            }],
        },
    };

    let json = serde_json::to_string(&packet).expect("serialize request");
    let decoded: Gaiko2ShastaAggregateRequest =
        serde_json::from_str(&json).expect("deserialize request");

    assert_eq!(decoded.schema, GAIKO2_SHASTA_REQUEST_SCHEMA);
    assert_eq!(decoded.payload.proofs.len(), 1);
    assert_eq!(
        decoded.payload.proofs[0].input,
        format!("0x{}", hex::encode([0x11; 32]))
    );
    assert_eq!(decoded.payload.proofs[0].proof_carry_data.chain_id, 167_013);
}

#[test]
fn proof_response_roundtrip_preserves_error_payload() {
    let response = Gaiko2ProofResponse::error(Gaiko2ProofError {
        code: "INVALID_SCHEMA".to_string(),
        message: "unsupported schema".to_string(),
    });

    let json = serde_json::to_string(&response).expect("serialize response");
    let decoded: Gaiko2ProofResponse = serde_json::from_str(&json).expect("deserialize response");

    assert_eq!(decoded.schema, GAIKO2_PROOF_RESPONSE_SCHEMA);
    assert_eq!(decoded.status.as_str(), "error");
    assert!(decoded.result.is_none());
    assert_eq!(
        decoded.error.as_ref().expect("error payload").code,
        "INVALID_SCHEMA"
    );
}
