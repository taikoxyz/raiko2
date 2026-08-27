//! Proposal request validation and proof execution.

use raiko2_guest_common::prove_shasta_proposal;
use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
use raiko2_prover::remote_prover::protocol::{
    RAIKO2_SHASTA_REQUEST_SCHEMA, Raiko2ProofResponse, Raiko2ProposalRequest,
};

use crate::{
    protocol::{RequestFailure, proof_result_from_input_hash},
    tee::TeeProvider,
};

pub(crate) fn prove_request<P: TeeProvider>(
    provider: &P,
    instance_id: u32,
    request: &Raiko2ProposalRequest,
) -> Result<Raiko2ProofResponse, RequestFailure> {
    validate_schema(request)?;
    let guest_input: raiko2_primitives_shasta::GuestInput =
        request.payload.guest_input.clone().into();
    validate_request(&guest_input)?;
    let input_hash = prove_shasta_proposal(&guest_input).map_err(|err| {
        RequestFailure::invalid_request(format!("invalid raiko2-sgx GuestInput: {err}"))
    })?;
    let expected_input = hash_shasta_subproof_input(&guest_input.proof_carry_data);
    if input_hash != expected_input {
        return Err(RequestFailure::invalid_request(format!(
            "GuestInput output hash mismatch: got {input_hash:#x} expected {expected_input:#x}"
        )));
    }
    let result = proof_result_from_input_hash(provider, instance_id, input_hash)
        .map_err(|err| RequestFailure::prover_error(err.to_string()))?;
    Ok(Raiko2ProofResponse::success(result))
}

fn validate_schema(request: &Raiko2ProposalRequest) -> Result<(), RequestFailure> {
    if request.schema != RAIKO2_SHASTA_REQUEST_SCHEMA {
        return Err(RequestFailure::invalid_request(format!(
            "unsupported schema {:?}",
            request.schema
        )));
    }
    Ok(())
}

fn validate_request(
    guest_input: &raiko2_primitives_shasta::GuestInput,
) -> Result<(), RequestFailure> {
    let carry = &guest_input.proof_carry_data;
    let first_witness = guest_input.witnesses.first().ok_or_else(|| {
        RequestFailure::invalid_request("GuestInput must include at least one witness")
    })?;
    if first_witness.chain_spec.chain_id != carry.chain_id {
        return Err(RequestFailure::invalid_request(format!(
            "GuestInput chain_id mismatch: proof_carry_data={} witness={}",
            carry.chain_id, first_witness.chain_spec.chain_id
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use alloy_primitives::Address;
    use raiko2_protocol_shasta::shasta::ProofCarryData;
    use raiko2_prover::remote_prover::protocol::{
        RAIKO2_SHASTA_REQUEST_SCHEMA, Raiko2ProposalGuestInput, Raiko2ProposalPayload,
        Raiko2ProposalRequest, Raiko2ReplayBlock,
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

    fn request_fixture() -> Raiko2ProposalRequest {
        let mut carry = ProofCarryData {
            chain_id: 167_013,
            ..ProofCarryData::default()
        };
        carry.transition_input.proposal_id = 42;

        Raiko2ProposalRequest {
            schema: RAIKO2_SHASTA_REQUEST_SCHEMA.to_string(),
            payload: Raiko2ProposalPayload {
                guest_input: Raiko2ProposalGuestInput {
                    proof_carry_data: carry,
                    ..Default::default()
                },
            },
        }
    }

    #[test]
    fn prove_request_rejects_chain_id_mismatch() {
        let provider = FakeProvider {
            secret_key: SecretKey::from_slice(&[9u8; 32]).expect("secret key"),
            quote: vec![],
        };
        let mut request = request_fixture();
        request.payload.guest_input.proof_carry_data.chain_id = 1;
        request.payload.guest_input.witnesses = vec![Raiko2ReplayBlock {
            chain_spec: raiko2_primitives::ChainSpec {
                chain_id: 167_013,
                ..Default::default()
            },
            ..Default::default()
        }];

        let err = prove_request(&provider, 9, &request).expect_err("chain id mismatch");
        assert!(err.to_string().contains("chain_id mismatch"));
    }

    #[test]
    fn prove_request_rejects_empty_guest_input_without_legacy_replay_fallback() {
        let provider = FakeProvider {
            secret_key: SecretKey::from_slice(&[5u8; 32]).expect("secret key"),
            quote: vec![],
        };
        let request = request_fixture();

        let err = prove_request(&provider, 9, &request).expect_err("invalid guest input");

        assert!(
            err.to_string()
                .contains("GuestInput must include at least one witness")
        );
    }
}
