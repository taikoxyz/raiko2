#![allow(missing_docs)]

use raiko2_primitives::proof::{ProofEnvelope, ProofPayload, PublicInputs, VerifierArtifact};
use serde_json::json;

#[test]
fn proof_envelope_roundtrip() {
    let envelope = ProofEnvelope {
        backend: "risc0".to_string(),
        public_inputs: PublicInputs {
            input_hash: Some("0x01".to_string()),
            instance_hash: None,
        },
        payload: ProofPayload {
            payload_kind: "risc0_seal".to_string(),
            bytes: vec![1, 2, 3],
        },
        verifier_artifacts: vec![VerifierArtifact {
            kind: "receipt_json".to_string(),
            value: json!("{...receipt...}"),
        }],
        carry_data: None,
        metadata: Some(json!({"fork": "shasta"})),
    };

    let encoded = serde_json::to_vec(&envelope).unwrap();
    let decoded: ProofEnvelope = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded.payload.bytes, vec![1, 2, 3]);
    assert_eq!(decoded.backend, "risc0");
    assert_eq!(decoded.payload.payload_kind, "risc0_seal");
}
