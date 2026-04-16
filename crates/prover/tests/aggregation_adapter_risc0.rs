#![allow(missing_docs)]

use alloy_primitives::{Address, B256, Uint, address, b256};
use raiko2_primitives::proof::{
    AggregationInput, ProofEnvelope, ProofPayload, PublicInputs, VerifierArtifact,
};
use raiko2_primitives_shasta::{ShastaBoundlessAggregationGuestInput, encode_proof_carry_data};
use raiko2_protocol_shasta::{
    libhash::hash_shasta_subproof_input,
    shasta::{Checkpoint, ProofCarryData, ShastaTransitionInput, TransitionInputData},
};
use raiko2_prover::boundless::aggregation::build_risc0_aggregation_input;
use serde_json::json;

mod fixtures;

#[test]
fn builds_risc0_aggregation_input() {
    let carry = sample_carry();
    let proof = ProofEnvelope {
        backend: "risc0".to_string(),
        public_inputs: PublicInputs {
            input_hash: Some(alloy_primitives::hex::encode_prefixed(
                hash_shasta_subproof_input(&carry).as_slice(),
            )),
            instance_hash: None,
        },
        payload: ProofPayload {
            payload_kind: "risc0_seal".to_string(),
            bytes: vec![],
        },
        verifier_artifacts: vec![VerifierArtifact {
            kind: "receipt_json".to_string(),
            value: json!(fixtures::risc0_receipt_json()),
        }],
        carry_data: Some(encode_proof_carry_data(&carry).expect("encode carry data")),
        metadata: None,
    };

    let agg = AggregationInput {
        proofs: vec![proof],
        expected_image_id: Some(fixtures::risc0_receipt_image_id_hex()),
        metadata: None,
    };

    let bytes = build_risc0_aggregation_input(&agg).expect("build input");
    assert!(!bytes.is_empty());
    let decoded: ShastaBoundlessAggregationGuestInput =
        bincode::deserialize(&bytes).expect("decode boundless aggregation input");
    assert_eq!(decoded.proof_carry_data_vec.len(), 1);
    assert_eq!(decoded.receipts.len(), 1);
}

#[test]
fn rejects_invalid_expected_image_id() {
    let carry = sample_carry();
    let proof = ProofEnvelope {
        backend: "risc0".to_string(),
        public_inputs: PublicInputs {
            input_hash: Some(alloy_primitives::hex::encode_prefixed(
                hash_shasta_subproof_input(&carry).as_slice(),
            )),
            instance_hash: None,
        },
        payload: ProofPayload {
            payload_kind: "risc0_seal".to_string(),
            bytes: vec![],
        },
        verifier_artifacts: vec![VerifierArtifact {
            kind: "receipt_json".to_string(),
            value: json!(fixtures::risc0_receipt_json()),
        }],
        carry_data: Some(encode_proof_carry_data(&carry).expect("encode carry data")),
        metadata: None,
    };

    let agg = AggregationInput {
        proofs: vec![proof],
        expected_image_id: Some("0x1234".to_string()),
        metadata: None,
    };

    let err = build_risc0_aggregation_input(&agg).unwrap_err();
    assert!(err.to_string().contains("image id") || err.to_string().contains("expected_image_id"));
}

#[test]
fn rejects_mismatched_receipt_image_id() {
    let carry = sample_carry();
    let proof = ProofEnvelope {
        backend: "risc0".to_string(),
        public_inputs: PublicInputs {
            input_hash: Some(alloy_primitives::hex::encode_prefixed(
                hash_shasta_subproof_input(&carry).as_slice(),
            )),
            instance_hash: None,
        },
        payload: ProofPayload {
            payload_kind: "risc0_seal".to_string(),
            bytes: vec![],
        },
        verifier_artifacts: vec![VerifierArtifact {
            kind: "receipt_json".to_string(),
            value: json!(fixtures::risc0_receipt_json()),
        }],
        carry_data: Some(encode_proof_carry_data(&carry).expect("encode carry data")),
        metadata: None,
    };

    let agg = AggregationInput {
        proofs: vec![proof],
        expected_image_id: Some(format!("0x{:0>64}", "11")),
        metadata: None,
    };

    let err = build_risc0_aggregation_input(&agg).unwrap_err();
    assert!(err.to_string().contains("receipt image id"));
}

fn sample_carry() -> ProofCarryData {
    ProofCarryData {
        chain_id: 167_013,
        verifier: address!("00000000000000000000000000000000000000aa"),
        transition_input: TransitionInputData {
            proposal_id: 1,
            proposal_hash: b256!(
                "1111111111111111111111111111111111111111111111111111111111111111"
            ),
            parent_proposal_hash: B256::ZERO,
            parent_block_hash: b256!(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            ),
            actual_prover: address!("00000000000000000000000000000000000000bb"),
            transition: ShastaTransitionInput {
                proposer: Address::ZERO,
                timestamp: 123,
            },
            checkpoint: Checkpoint {
                blockNumber: Uint::from(10u64),
                blockHash: b256!(
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                ),
                stateRoot: b256!(
                    "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                ),
            },
        },
    }
}
