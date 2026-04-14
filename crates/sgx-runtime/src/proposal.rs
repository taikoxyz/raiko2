//! Proposal request validation and proof execution.

use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
use raiko2_prover::gaiko2::protocol::{
    GAIKO2_SHASTA_REQUEST_SCHEMA, Gaiko2ProofResponse, Gaiko2ShastaRequest,
};

use crate::{
    protocol::{RequestFailure, proof_result_from_input_hash},
    tee::TeeProvider,
};

pub(crate) fn prove_request<P: TeeProvider>(
    provider: &P,
    instance_id: u32,
    request: &Gaiko2ShastaRequest,
) -> Result<Gaiko2ProofResponse, RequestFailure> {
    validate_request(request)?;
    let input_hash = hash_shasta_subproof_input(&request.payload.proof_carry_data);
    let result = proof_result_from_input_hash(provider, instance_id, input_hash)
        .map_err(|err| RequestFailure::prover_error(err.to_string()))?;
    Ok(Gaiko2ProofResponse::success(result))
}

fn validate_request(request: &Gaiko2ShastaRequest) -> Result<(), RequestFailure> {
    if request.schema != GAIKO2_SHASTA_REQUEST_SCHEMA {
        return Err(RequestFailure::invalid_request(format!(
            "unsupported schema {:?}",
            request.schema
        )));
    }
    if request.payload.blocks.is_empty() {
        return Err(RequestFailure::invalid_request(
            "request must include at least one replay block",
        ));
    }

    let carry = &request.payload.proof_carry_data;
    if carry.chain_id != request.payload.chain_id {
        return Err(RequestFailure::invalid_request(format!(
            "chain_id mismatch: payload={} proof_carry_data={}",
            request.payload.chain_id, carry.chain_id
        )));
    }

    for window in request.payload.blocks.windows(2) {
        let prev = &window[0].block.header;
        let next = &window[1].block.header;
        if next.number != prev.number + 1 {
            return Err(RequestFailure::invalid_request(format!(
                "block numbers must be contiguous: got {} after {}",
                next.number, prev.number
            )));
        }
    }

    let first = &request.payload.blocks[0].block.header;
    if first.parent_hash != carry.transition_input.parent_block_hash {
        return Err(RequestFailure::invalid_request(format!(
            "first block parent hash mismatch: block={:#x} checkpoint={:#x}",
            first.parent_hash, carry.transition_input.parent_block_hash
        )));
    }

    let last = &request.payload.blocks[request.payload.blocks.len() - 1]
        .block
        .header;
    let expected_number: u64 = carry.transition_input.checkpoint.blockNumber.to();
    if last.number != expected_number {
        return Err(RequestFailure::invalid_request(format!(
            "checkpoint block number mismatch: block={} checkpoint={}",
            last.number, expected_number
        )));
    }
    if last.state_root != carry.transition_input.checkpoint.stateRoot {
        return Err(RequestFailure::invalid_request(format!(
            "checkpoint state root mismatch: block={:#x} checkpoint={:#x}",
            last.state_root, carry.transition_input.checkpoint.stateRoot
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, B256};
    use raiko2_primitives::{ChainSpec, ExecutionWitness, StatelessInput};
    use raiko2_protocol_shasta::{
        libhash::hash_shasta_subproof_input,
        shasta::{Checkpoint, ProofCarryData},
    };
    use raiko2_prover::gaiko2::protocol::{
        GAIKO2_PROOF_RESPONSE_SCHEMA, GAIKO2_SHASTA_REQUEST_SCHEMA, Gaiko2ProofStatus,
        Gaiko2ReplayBlock, Gaiko2ShastaPayload, Gaiko2ShastaRequest,
    };
    use reth_ethereum_primitives::Block;
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

    fn u48(value: u64) -> alloy_primitives::Uint<48, 1> {
        alloy_primitives::Uint::from_limbs([value])
    }

    fn request_fixture() -> Gaiko2ShastaRequest {
        let mut carry = ProofCarryData {
            chain_id: 167_013,
            ..ProofCarryData::default()
        };
        carry.transition_input.parent_block_hash = B256::from([0x11; 32]);
        carry.transition_input.checkpoint = Checkpoint {
            blockNumber: u48(42),
            blockHash: B256::from([0x22; 32]),
            stateRoot: B256::from([0x33; 32]),
        };

        let mut stateless = StatelessInput {
            block: Block::default(),
            chain_spec: ChainSpec::default(),
            witness: ExecutionWitness::default(),
            accounts: Default::default(),
        };
        stateless.block.header.number = 42;
        stateless.block.header.parent_hash = B256::from([0x11; 32]);
        stateless.block.header.state_root = B256::from([0x33; 32]);
        stateless.chain_spec.chain_id = 167_013;

        Gaiko2ShastaRequest {
            schema: GAIKO2_SHASTA_REQUEST_SCHEMA.to_string(),
            payload: Gaiko2ShastaPayload {
                chain_id: 167_013,
                blocks: vec![Gaiko2ReplayBlock::from(stateless)],
                proof_carry_data: carry,
            },
        }
    }

    #[test]
    fn prove_request_returns_gaiko2_envelope_with_expected_input_hash() {
        let provider = FakeProvider {
            secret_key: SecretKey::from_slice(&[8u8; 32]).expect("secret key"),
            quote: vec![0xAA, 0xBB],
        };
        let request = request_fixture();
        let expected_input = hash_shasta_subproof_input(&request.payload.proof_carry_data);

        let response = prove_request(&provider, 9, &request).expect("prove request");

        assert_eq!(response.schema, GAIKO2_PROOF_RESPONSE_SCHEMA);
        assert_eq!(response.status, Gaiko2ProofStatus::Ok);
        let result = response.result.expect("result");
        assert_eq!(result.input, format!("{expected_input:#x}"));
        assert_eq!(result.quote.as_deref(), Some("0xaabb"));
        assert!(response.error.is_none());
    }

    #[test]
    fn prove_request_rejects_chain_id_mismatch() {
        let provider = FakeProvider {
            secret_key: SecretKey::from_slice(&[9u8; 32]).expect("secret key"),
            quote: vec![],
        };
        let mut request = request_fixture();
        request.payload.chain_id = 1;

        let err = prove_request(&provider, 9, &request).expect_err("chain id mismatch");
        assert!(err.to_string().contains("chain_id mismatch"));
    }
}
