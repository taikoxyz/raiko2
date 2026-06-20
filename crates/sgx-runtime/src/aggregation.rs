//! Aggregation request validation and proof execution.

use alloy_primitives::{Address, B256};
use raiko2_primitives_shasta::instance::{
    build_shasta_commitment_from_proof_carry_data_vec, shasta_aggregation_output,
};
use raiko2_protocol_shasta::{libhash::hash_shasta_subproof_input, shasta::ProofCarryData};
use raiko2_prover::remote_prover::protocol::{
    RAIKO2_SHASTA_AGGREGATE_REQUEST_SCHEMA, Raiko2ProofResponse, Raiko2ShastaAggregateRequest,
};
use secp256k1::{
    Message, Secp256k1,
    ecdsa::{RecoverableSignature, RecoveryId},
};

use crate::{
    bootstrap::public_key_to_address,
    protocol::{RequestFailure, load_signer_identity, proof_result_from_input_hash},
    tee::TeeProvider,
};

const SHASTA_TEE_PROOF_LEN: usize = 89;

pub(crate) fn aggregate_request<P: TeeProvider>(
    provider: &P,
    instance_id: u32,
    request: &Raiko2ShastaAggregateRequest,
) -> Result<Raiko2ProofResponse, RequestFailure> {
    let identity = load_signer_identity(provider)
        .map_err(|err| RequestFailure::prover_error(err.to_string()))?;
    let carries = validate_request(request, instance_id, identity.instance_address)?;
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
    expected_instance_id: u32,
    expected_instance_address: Address,
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
        if proof_bytes.len() != SHASTA_TEE_PROOF_LEN {
            return Err(RequestFailure::invalid_request(format!(
                "aggregate proof {index} length mismatch: got {} expected {SHASTA_TEE_PROOF_LEN}",
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
        validate_sgx_child_proof(
            index,
            &proof_bytes,
            expected_input,
            expected_instance_id,
            expected_instance_address,
        )?;

        carries.push(item.proof_carry_data.clone());
    }

    if !validate_shasta_proof_carry_data_vec(&carries) {
        return Err(RequestFailure::invalid_request(
            "invalid shasta proof carry data",
        ));
    }

    Ok(carries)
}

fn validate_sgx_child_proof(
    index: usize,
    proof_bytes: &[u8],
    input_hash: B256,
    expected_instance_id: u32,
    expected_instance_address: Address,
) -> Result<(), RequestFailure> {
    let instance_id = u32::from_be_bytes(
        proof_bytes[0..4]
            .try_into()
            .expect("SGX proof length was already checked"),
    );
    if instance_id != expected_instance_id {
        return Err(RequestFailure::invalid_request(format!(
            "aggregate proof {index} SGX instance id mismatch: got {instance_id} expected {expected_instance_id}"
        )));
    }

    let instance_address = Address::from_slice(&proof_bytes[4..24]);
    if instance_address != expected_instance_address {
        return Err(RequestFailure::invalid_request(format!(
            "aggregate proof {index} SGX instance address mismatch: got {instance_address:#x} expected {expected_instance_address:#x}"
        )));
    }

    let recovery_id = match proof_bytes[88] {
        27 => RecoveryId::Zero,
        28 => RecoveryId::One,
        value => {
            return Err(RequestFailure::invalid_request(format!(
                "aggregate proof {index} has invalid SGX signature recovery id {value}"
            )));
        }
    };
    let signature =
        RecoverableSignature::from_compact(&proof_bytes[24..88], recovery_id).map_err(|err| {
            RequestFailure::invalid_request(format!(
                "aggregate proof {index} has invalid SGX signature: {err}"
            ))
        })?;
    let message = Message::from_digest_slice(input_hash.as_slice()).map_err(|err| {
        RequestFailure::invalid_request(format!(
            "aggregate proof {index} has invalid input hash: {err}"
        ))
    })?;
    let recovered_key = Secp256k1::new()
        .recover_ecdsa(&message, &signature)
        .map_err(|err| {
            RequestFailure::invalid_request(format!(
                "aggregate proof {index} signature recovery failed: {err}"
            ))
        })?;
    let recovered_address = public_key_to_address(&recovered_key);
    if recovered_address != instance_address {
        return Err(RequestFailure::invalid_request(format!(
            "aggregate proof {index} SGX signature signer mismatch: recovered {recovered_address:#x} expected {instance_address:#x}"
        )));
    }

    Ok(())
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
    use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};
    use std::str::FromStr;

    use super::aggregate_request;
    use crate::bootstrap::public_key_to_address;
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
        let signing_key = SecretKey::from_slice(&[13u8; 32]).expect("signing key");
        let instance_address = instance_address_for_key(&signing_key);

        let proofs = [first, second]
            .into_iter()
            .map(|carry| {
                let input = hash_shasta_subproof_input(&carry);
                let proof = Proof {
                    proof: Some(sgx_proof_for_input(
                        7,
                        input,
                        &signing_key,
                        instance_address,
                    )),
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

    fn sgx_proof_for_input(
        instance_id: u32,
        input_hash: B256,
        signing_key: &SecretKey,
        embedded_instance_address: Address,
    ) -> String {
        let message = Message::from_digest_slice(input_hash.as_slice()).expect("input hash");
        let signature = Secp256k1::new().sign_ecdsa_recoverable(&message, signing_key);
        let (recovery_id, data) = signature.serialize_compact();

        let mut proof = Vec::with_capacity(89);
        proof.extend(instance_id.to_be_bytes());
        proof.extend(embedded_instance_address);
        proof.extend(data);
        proof.push(u8::try_from(i32::from(recovery_id) + 27).expect("recovery id"));
        format!("0x{}", hex::encode(proof))
    }

    fn instance_address_for_key(secret_key: &SecretKey) -> Address {
        let public_key = PublicKey::from_secret_key(&Secp256k1::new(), secret_key);
        public_key_to_address(&public_key)
    }

    #[test]
    fn aggregate_request_returns_raiko2_envelope() {
        let provider = FakeProvider {
            secret_key: SecretKey::from_slice(&[13u8; 32]).expect("secret key"),
            quote: vec![0x12, 0x34],
        };
        let request = aggregate_request_fixture();

        let response = aggregate_request(&provider, 7, &request).expect("aggregate request");

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

    #[test]
    fn aggregate_request_rejects_child_proof_signature_mismatch() {
        let signing_key = SecretKey::from_slice(&[13u8; 32]).expect("signing key");
        let different_key = SecretKey::from_slice(&[14u8; 32]).expect("different key");
        let provider = FakeProvider {
            secret_key: different_key,
            quote: vec![],
        };
        let mut request = aggregate_request_fixture();
        let first = request.payload.proofs.first_mut().expect("first proof");
        let input_hash = hash_shasta_subproof_input(&first.proof_carry_data);
        first.proof = sgx_proof_for_input(
            19,
            input_hash,
            &signing_key,
            instance_address_for_key(&different_key),
        );

        let err = aggregate_request(&provider, 19, &request).expect_err("signature mismatch");

        assert!(err.to_string().contains("signature"));
    }

    #[test]
    fn aggregate_request_rejects_child_proof_from_other_instance() {
        let provider = FakeProvider {
            secret_key: SecretKey::from_slice(&[12u8; 32]).expect("secret key"),
            quote: vec![],
        };
        let request = aggregate_request_fixture();

        let err = aggregate_request(&provider, 7, &request).expect_err("instance mismatch");

        assert!(err.to_string().contains("instance address"));
    }
}
