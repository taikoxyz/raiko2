//! Native prover implementation (no zk proof).

use alloy_primitives::{B256, Bytes};
use raiko2_pipeline::ProverBackend;
use raiko2_primitives::{Proof, ProverConfig, RaikoError, RaikoResult};
use raiko2_primitives_shasta::{
    GuestInput, ShastaZkAggregationGuestInput, encode_proof_carry_data,
    instance::{
        ProtocolInstance, ShastaProposalMetadata, ShastaTransition,
        shasta_zk_aggregation_public_input_from_proof_carry_data_vec, words_to_bytes_be,
        words_to_bytes_le,
    },
};
use raiko2_protocol_shasta::shasta::ProofCarryData;

use crate::{GuestInputCodec, parse_proof_carry_data, parse_shasta_aggregation_input};

/// Native prover for local execution (returns public input only).
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeProver;

impl GuestInputCodec<GuestInput> for NativeProver {
    fn encode(&self, input: &GuestInput, _config: &ProverConfig) -> RaikoResult<Bytes> {
        let bytes = bincode::serialize(input)
            .map_err(|e| RaikoError::Guest(format!("Failed to serialize input: {}", e)))?;
        Ok(Bytes::from(bytes))
    }
}

#[async_trait::async_trait]
impl<B> crate::Prover<B> for NativeProver
where
    B: ProverBackend,
{
    type GuestInput = GuestInput;

    fn encode(&self, input: &Self::GuestInput, config: &ProverConfig) -> RaikoResult<Bytes> {
        GuestInputCodec::encode(self, input, config)
    }

    async fn prove_encoded(
        &self,
        input: Bytes,
        config: &ProverConfig,
        _backend: &B,
    ) -> RaikoResult<Proof> {
        let input: GuestInput = bincode::deserialize(input.as_ref())
            .map_err(|e| RaikoError::Guest(format!("Failed to deserialize input: {}", e)))?;
        if input.witnesses.is_empty() {
            return Err(RaikoError::Guest(
                "GuestInput must contain at least one witness".to_string(),
            ));
        }

        let proof_carry_data: ProofCarryData = parse_proof_carry_data(config);

        let first = input.witnesses.first().expect("checked");
        let last = input.witnesses.last().expect("checked");

        let transition = ShastaTransition {
            parent_hash: first.block.header.parent_hash,
            block_hash: last.block.header.hash_slow(),
            state_root: last.block.header.state_root,
        };

        let proposal_metadata = ShastaProposalMetadata {
            info_hash: Default::default(),
            proposer: proof_carry_data.transition_input.transition.proposer,
            proposal_id: proof_carry_data.transition_input.proposal_id,
            proposed_at: proof_carry_data.transition_input.transition.timestamp,
        };

        let instance = ProtocolInstance {
            transition,
            proposal_metadata,
            prover: proof_carry_data.transition_input.actual_prover,
            chain_id: proof_carry_data.chain_id,
            verifier_address: proof_carry_data.verifier,
        };

        let extra_data = encode_proof_carry_data(&proof_carry_data)?;

        Ok(Proof {
            input: Some(instance.instance_hash()),
            extra_data: Some(extra_data),
            ..Default::default()
        })
    }

    async fn aggregate(
        &self,
        _input: raiko2_primitives::AggregationGuestInput,
        config: &ProverConfig,
        _backend: &B,
    ) -> RaikoResult<Proof> {
        let aggregation_input: ShastaZkAggregationGuestInput =
            parse_shasta_aggregation_input(config)?;

        let endianness = config
            .get("native_image_id_endianness")
            .and_then(|v| v.as_str())
            .unwrap_or("be");
        let image_id_bytes = match endianness {
            "be" => words_to_bytes_be(&aggregation_input.image_id),
            "le" => words_to_bytes_le(&aggregation_input.image_id),
            other => {
                return Err(RaikoError::InvalidRequestConfig(format!(
                    "Unsupported native_image_id_endianness: {other}"
                )));
            }
        };
        let sub_image_id = B256::from(image_id_bytes);

        let public_input = shasta_zk_aggregation_public_input_from_proof_carry_data_vec(
            sub_image_id,
            &aggregation_input.proof_carry_data_vec,
            aggregation_input.prover_address,
        )
        .ok_or_else(|| {
            RaikoError::InvalidRequestConfig("Invalid proof_carry_data_vec".to_string())
        })?;

        Ok(Proof {
            input: Some(public_input),
            ..Default::default()
        })
    }
}
