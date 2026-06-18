//! Proposal request validation and proof execution.

use raiko2_guest_common::prove_shasta_proposal_for_proof_type;
use raiko2_primitives::ProofType;
use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
use raiko2_prover::remote_prover::protocol::{
    RAIKO2_SHASTA_REQUEST_SCHEMA, Raiko2ProofResponse, Raiko2ShastaRequest,
};

use crate::{
    protocol::{RequestFailure, proof_result_from_input_hash},
    tee::TeeProvider,
};

pub(crate) fn prove_request<P: TeeProvider>(
    provider: &P,
    instance_id: u32,
    request: &Raiko2ShastaRequest,
) -> Result<Raiko2ProofResponse, RequestFailure> {
    validate_schema(request)?;
    let guest_input = request.payload.guest_input.as_ref().ok_or_else(|| {
        RequestFailure::invalid_request("raiko2-sgx request must include GuestInput")
    })?;
    validate_request(request, guest_input)?;
    let input_hash =
        prove_shasta_proposal_for_proof_type(guest_input, ProofType::Sgx).map_err(|err| {
            RequestFailure::invalid_request(format!("invalid raiko2-sgx GuestInput: {err:#}"))
        })?;
    let expected_input = hash_shasta_subproof_input(&request.payload.proof_carry_data);
    if input_hash != expected_input {
        return Err(RequestFailure::invalid_request(format!(
            "GuestInput output hash mismatch: got {input_hash:#x} expected {expected_input:#x}"
        )));
    }
    let result = proof_result_from_input_hash(provider, instance_id, input_hash)
        .map_err(|err| RequestFailure::prover_error(err.to_string()))?;
    Ok(Raiko2ProofResponse::success(result))
}

fn validate_schema(request: &Raiko2ShastaRequest) -> Result<(), RequestFailure> {
    if request.schema != RAIKO2_SHASTA_REQUEST_SCHEMA {
        return Err(RequestFailure::invalid_request(format!(
            "unsupported schema {:?}",
            request.schema
        )));
    }
    Ok(())
}

fn validate_request(
    request: &Raiko2ShastaRequest,
    guest_input: &raiko2_primitives_shasta::GuestInput,
) -> Result<(), RequestFailure> {
    let carry = &request.payload.proof_carry_data;
    if carry.chain_id != request.payload.chain_id {
        return Err(RequestFailure::invalid_request(format!(
            "chain_id mismatch: payload={} proof_carry_data={}",
            request.payload.chain_id, carry.chain_id
        )));
    }
    if guest_input.proof_carry_data != *carry {
        return Err(RequestFailure::invalid_request(
            "GuestInput proof_carry_data mismatch",
        ));
    }
    if let Some(first_witness) = guest_input.witnesses.first()
        && first_witness.chain_spec.chain_id != request.payload.chain_id
    {
        return Err(RequestFailure::invalid_request(format!(
            "GuestInput chain_id mismatch: payload={} witness={}",
            request.payload.chain_id, first_witness.chain_spec.chain_id
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use alloy_primitives::Address;
    use raiko2_primitives_shasta::GuestInput;
    use raiko2_protocol_shasta::shasta::ProofCarryData;
    use raiko2_prover::remote_prover::protocol::{
        RAIKO2_SHASTA_REQUEST_SCHEMA, Raiko2ShastaPayload, Raiko2ShastaRequest,
    };
    use secp256k1::SecretKey;

    use super::prove_request;
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

    fn request_fixture() -> Raiko2ShastaRequest {
        let mut carry = ProofCarryData {
            chain_id: 167_013,
            ..ProofCarryData::default()
        };
        carry.transition_input.proposal_id = 42;

        Raiko2ShastaRequest {
            schema: RAIKO2_SHASTA_REQUEST_SCHEMA.to_string(),
            payload: Raiko2ShastaPayload {
                chain_id: 167_013,
                blocks: Vec::new(),
                proof_carry_data: carry,
                guest_input: None,
            },
        }
    }

    #[test]
    fn prove_request_rejects_missing_guest_input() {
        let provider = FakeProvider {
            secret_key: SecretKey::from_slice(&[8u8; 32]).expect("secret key"),
            quote: vec![0xAA, 0xBB],
        };
        let request = request_fixture();

        let err = prove_request(&provider, 9, &request).expect_err("missing guest input");

        assert!(err.to_string().contains("GuestInput"));
    }

    #[test]
    fn prove_request_rejects_chain_id_mismatch() {
        let provider = FakeProvider {
            secret_key: SecretKey::from_slice(&[9u8; 32]).expect("secret key"),
            quote: vec![],
        };
        let mut request = request_fixture();
        request.payload.guest_input = Some(GuestInput {
            proof_carry_data: request.payload.proof_carry_data.clone(),
            ..GuestInput::default()
        });
        request.payload.chain_id = 1;

        let err = prove_request(&provider, 9, &request).expect_err("chain id mismatch");
        assert!(err.to_string().contains("chain_id mismatch"));
    }

    #[test]
    fn prove_request_rejects_unverified_replay_packet() {
        let provider = FakeProvider {
            secret_key: SecretKey::from_slice(&[6u8; 32]).expect("secret key"),
            quote: vec![],
        };
        let request = request_fixture();

        let err = prove_request(&provider, 9, &request).expect_err("unverified replay packet");

        assert!(
            err.to_string().contains("GuestInput")
                || err.to_string().contains("stateless")
                || err.to_string().contains("witness")
        );
    }

    #[test]
    fn prove_request_rejects_empty_guest_input_without_legacy_replay_fallback() {
        let provider = FakeProvider {
            secret_key: SecretKey::from_slice(&[5u8; 32]).expect("secret key"),
            quote: vec![],
        };
        let mut request = request_fixture();
        request.payload.guest_input = Some(GuestInput {
            proof_carry_data: request.payload.proof_carry_data.clone(),
            ..GuestInput::default()
        });

        let err = prove_request(&provider, 9, &request).expect_err("invalid guest input");

        assert!(err.to_string().contains("GuestInput"));
    }
}
