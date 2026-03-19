#![allow(missing_docs)]

use raiko2_primitives::proof::{
    AggregationInput, ProofEnvelope, ProofPayload, PublicInputs, VerifierArtifact,
};
use raiko2_prover::boundless::aggregation::build_risc0_aggregation_input;
use serde_json::json;

mod fixtures;

#[test]
fn builds_risc0_aggregation_input() {
    let proof = ProofEnvelope {
        backend: "risc0".to_string(),
        public_inputs: PublicInputs {
            input_hash: None,
            instance_hash: None,
        },
        payload: ProofPayload {
            payload_kind: "risc0_journal".to_string(),
            bytes: vec![],
        },
        verifier_artifacts: vec![VerifierArtifact {
            kind: "receipt_json".to_string(),
            value: json!(fixtures::risc0_receipt_json()),
        }],
        carry_data: None,
        metadata: None,
    };

    let agg = AggregationInput {
        proofs: vec![proof],
        expected_image_id: None,
        metadata: None,
    };

    let bytes = build_risc0_aggregation_input(&agg).expect("build input");
    assert!(!bytes.is_empty());
}

#[test]
fn rejects_invalid_expected_image_id() {
    let proof = ProofEnvelope {
        backend: "risc0".to_string(),
        public_inputs: PublicInputs {
            input_hash: None,
            instance_hash: None,
        },
        payload: ProofPayload {
            payload_kind: "risc0_journal".to_string(),
            bytes: vec![],
        },
        verifier_artifacts: vec![VerifierArtifact {
            kind: "receipt_json".to_string(),
            value: json!(fixtures::risc0_receipt_json()),
        }],
        carry_data: None,
        metadata: None,
    };

    let agg = AggregationInput {
        proofs: vec![proof],
        expected_image_id: Some("0x1234".to_string()),
        metadata: None,
    };

    let err = build_risc0_aggregation_input(&agg).unwrap_err();
    assert!(err.to_string().contains("expected_image_id"));
}
