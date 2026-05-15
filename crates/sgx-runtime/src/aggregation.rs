//! Aggregation request validation and proof execution.

use alloy_primitives::B256;
use raiko2_primitives_shasta::instance::{
    build_shasta_commitment_from_proof_carry_data_vec, shasta_aggregation_output,
};
use raiko2_protocol_shasta::{libhash::hash_shasta_subproof_input, shasta::ProofCarryData};
use raiko2_prover::remote_prover::protocol::{
    RAIKO2_SHASTA_AGGREGATE_REQUEST_SCHEMA, Raiko2ProofResponse, Raiko2ShastaAggregateRequest,
};

use crate::{
    protocol::{RequestFailure, load_signer_identity, proof_result_from_input_hash},
    tee::TeeProvider,
};

const SHASTA_SGX_PROOF_LEN: usize = 89;

pub(crate) fn aggregate_request<P: TeeProvider>(
    provider: &P,
    instance_id: u32,
    request: &Raiko2ShastaAggregateRequest,
) -> Result<Raiko2ProofResponse, RequestFailure> {
    let carries = validate_request(request)?;
    let identity = load_signer_identity(provider)
        .map_err(|err| RequestFailure::prover_error(err.to_string()))?;
    let commitment = build_shasta_commitment_from_proof_carry_data_vec(&carries)
        .ok_or_else(|| RequestFailure::invalid_request("invalid shasta proof carry data"))?;
    let first = carries.first().ok_or_else(|| {
        RequestFailure::invalid_request("request must include at least one aggregate proof")
    })?;
    let input_hash = shasta_aggregation_output(
        &commitment,
        first.chain_id,
        first.verifier,
        identity.instance_address,
    );
    let result = proof_result_from_input_hash(provider, instance_id, input_hash)
        .map_err(|err| RequestFailure::prover_error(err.to_string()))?;
    Ok(Raiko2ProofResponse::success(result))
}

fn validate_request(
    request: &Raiko2ShastaAggregateRequest,
) -> Result<Vec<ProofCarryData>, RequestFailure> {
    if request.schema != RAIKO2_SHASTA_AGGREGATE_REQUEST_SCHEMA {
        return Err(RequestFailure::invalid_request(format!(
            "unsupported schema {:?}",
            request.schema
        )));
    }
    if request.payload.proofs.is_empty() {
        return Err(RequestFailure::invalid_request(
            "request must include at least one aggregate proof",
        ));
    }

    let mut carries = Vec::with_capacity(request.payload.proofs.len());

    for (index, item) in request.payload.proofs.iter().enumerate() {
        if item.input.trim().is_empty() {
            return Err(RequestFailure::invalid_request(format!(
                "aggregate proof {index} is missing input"
            )));
        }
        if item.proof.trim().is_empty() {
            return Err(RequestFailure::invalid_request(format!(
                "aggregate proof {index} is missing proof"
            )));
        }
        let proof_bytes = hex::decode(item.proof.trim_start_matches("0x")).map_err(|err| {
            RequestFailure::invalid_request(format!("decode aggregate proof {index}: {err}"))
        })?;
        if proof_bytes.len() != SHASTA_SGX_PROOF_LEN {
            return Err(RequestFailure::invalid_request(format!(
                "aggregate proof {index} length mismatch: got {} expected {SHASTA_SGX_PROOF_LEN}",
                proof_bytes.len()
            )));
        }

        let input_hash = parse_hash(&item.input).map_err(|err| {
            RequestFailure::invalid_request(format!("decode aggregate proof {index} input: {err}"))
        })?;
        let expected_input = hash_shasta_subproof_input(&item.proof_carry_data);
        if input_hash != expected_input {
            return Err(RequestFailure::invalid_request(format!(
                "aggregate proof {index} input mismatch: got {input_hash:#x} expected {expected_input:#x}"
            )));
        }

        carries.push(item.proof_carry_data.clone());
    }

    if !validate_shasta_proof_carry_data_vec(&carries) {
        return Err(RequestFailure::invalid_request(
            "invalid shasta proof carry data",
        ));
    }

    Ok(carries)
}

fn parse_hash(value: &str) -> Result<B256, String> {
    let decoded =
        hex::decode(value.trim().trim_start_matches("0x")).map_err(|err| err.to_string())?;
    if decoded.len() != 32 {
        return Err(format!("expected 32 bytes, got {}", decoded.len()));
    }
    Ok(B256::from_slice(&decoded))
}

fn validate_shasta_proof_carry_data_vec(proof_carry_data_vec: &[ProofCarryData]) -> bool {
    let Some(first) = proof_carry_data_vec.first() else {
        return false;
    };
    let expected_actual_prover = first.transition_input.actual_prover;
    if !proof_carry_data_vec
        .iter()
        .all(|item| item.transition_input.actual_prover == expected_actual_prover)
    {
        return false;
    }

    for window in proof_carry_data_vec.windows(2) {
        let prev = &window[0];
        let next = &window[1];
        if prev.transition_input.proposal_id + 1 != next.transition_input.proposal_id {
            return false;
        }
        if prev.transition_input.proposal_hash != next.transition_input.parent_proposal_hash {
            return false;
        }
        if prev.chain_id != next.chain_id {
            return false;
        }
        if prev.verifier != next.verifier {
            return false;
        }
        if prev.transition_input.checkpoint.blockHash != next.transition_input.parent_block_hash {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, B256, Uint};
    use raiko2_primitives::Proof;
    use raiko2_primitives_shasta::encode_proof_carry_data;
    use raiko2_protocol_shasta::{libhash::hash_shasta_subproof_input, shasta::ProofCarryData};
    use raiko2_prover::remote_prover::protocol::{
        RAIKO2_PROOF_RESPONSE_SCHEMA, RAIKO2_SHASTA_AGGREGATE_REQUEST_SCHEMA, Raiko2AggregateProof,
        Raiko2ProofStatus, Raiko2ShastaAggregatePayload, Raiko2ShastaAggregateRequest,
    };
    use secp256k1::SecretKey;
    use std::str::FromStr;

    use super::aggregate_request;
    use crate::tee::TeeProvider;

    #[derive(Clone)]
    struct FakeProvider {
        secret_key: SecretKey,
        quote: Vec<u8>,
    }

    impl TeeProvider for FakeProvider {
        fn save_private_key(&self, _key: &SecretKey) -> anyhow::Result<()> {
            unreachable!("unused in tests")
        }

        fn load_private_key(&self) -> anyhow::Result<SecretKey> {
            Ok(self.secret_key)
        }

        fn load_quote(&self, _instance_address: Address) -> anyhow::Result<Vec<u8>> {
            Ok(self.quote.clone())
        }
    }

    fn carry_fixture(
        proposal_id: u64,
        parent_proposal_hash: B256,
        proposal_hash: B256,
    ) -> ProofCarryData {
        let actual_prover =
            Address::from_str("0x0000777735367b36bC9B61C50022d9D0700dB4Ec").expect("prover");
        let verifier =
            Address::from_str("0x00f9f60C79e38c08b785eE4F1a849900693C6630").expect("verifier");
        let proposer =
            Address::from_str("0x4444444444444444444444444444444444444444").expect("proposer");
        let checkpoint_hash = B256::from([proposal_id as u8; 32]);
        ProofCarryData {
            chain_id: 167_013,
            verifier,
            transition_input: raiko2_protocol_shasta::shasta::TransitionInputData {
                proposal_id,
                proposal_hash,
                parent_proposal_hash,
                parent_block_hash: checkpoint_hash,
                actual_prover,
                transition: raiko2_protocol_shasta::shasta::ShastaTransitionInput {
                    proposer,
                    timestamp: 123 + proposal_id,
                },
                checkpoint: raiko2_protocol_shasta::shasta::Checkpoint {
                    blockNumber: Uint::from(40u64 + proposal_id),
                    blockHash: B256::from([proposal_id as u8 + 1; 32]),
                    stateRoot: B256::from([proposal_id as u8 + 2; 32]),
                },
            },
        }
    }

    fn aggregate_request_fixture() -> Raiko2ShastaAggregateRequest {
        let first = carry_fixture(7, B256::from([0xAA; 32]), B256::from([0xBB; 32]));
        let mut second = carry_fixture(
            8,
            first.transition_input.proposal_hash,
            B256::from([0xCC; 32]),
        );
        second.transition_input.parent_block_hash = first.transition_input.checkpoint.blockHash;

        let proofs = [first, second]
            .into_iter()
            .map(|carry| {
                let input = hash_shasta_subproof_input(&carry);
                let proof = Proof {
                    proof: Some(format!("0x{}", "11".repeat(89))),
                    input: Some(input),
                    extra_data: Some(encode_proof_carry_data(&carry).expect("carry data")),
                    ..Proof::default()
                };
                Raiko2AggregateProof::from_proof(&proof).expect("aggregate proof")
            })
            .collect();

        Raiko2ShastaAggregateRequest {
            schema: RAIKO2_SHASTA_AGGREGATE_REQUEST_SCHEMA.to_string(),
            payload: Raiko2ShastaAggregatePayload { proofs },
        }
    }

    #[test]
    fn aggregate_request_returns_raiko2_envelope() {
        let provider = FakeProvider {
            secret_key: SecretKey::from_slice(&[10u8; 32]).expect("secret key"),
            quote: vec![0x12, 0x34],
        };
        let request = aggregate_request_fixture();

        let response = aggregate_request(&provider, 19, &request).expect("aggregate request");

        assert_eq!(response.schema, RAIKO2_PROOF_RESPONSE_SCHEMA);
        assert_eq!(response.status, Raiko2ProofStatus::Ok);
        let result = response.result.expect("result");
        assert_eq!(result.quote.as_deref(), Some("0x1234"));
        assert!(result.proof.is_some());
        assert!(response.error.is_none());
    }

    #[test]
    fn aggregate_request_rejects_empty_proof_list() {
        let provider = FakeProvider {
            secret_key: SecretKey::from_slice(&[11u8; 32]).expect("secret key"),
            quote: vec![],
        };
        let request = Raiko2ShastaAggregateRequest {
            schema: RAIKO2_SHASTA_AGGREGATE_REQUEST_SCHEMA.to_string(),
            payload: Raiko2ShastaAggregatePayload { proofs: vec![] },
        };

        let err = aggregate_request(&provider, 19, &request).expect_err("empty proofs");
        assert!(err.to_string().contains("at least one aggregate proof"));
    }
}
