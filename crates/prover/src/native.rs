//! Native prover implementation (no zk proof).

use alloy_primitives::B256;
use raiko2_pipeline::ProverBackend;
use raiko2_primitives::{
    GuestInput, Proof, ProverConfig, RaikoError, RaikoResult, ShastaZkAggregationGuestInput,
    instance::{
        ProtocolInstance, ShastaProposalMetadata, ShastaTransition,
        shasta_zk_aggregation_public_input_from_proof_carry_data_vec, words_to_bytes_be,
        words_to_bytes_le,
    },
};
use raiko2_protocol::ProofCarryData;

/// Native prover for local execution (returns public input only).
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeProver;

#[async_trait::async_trait]
impl<B> crate::Prover<B> for NativeProver
where
    B: ProverBackend,
{
    async fn prove(
        &self,
        input: GuestInput,
        config: &ProverConfig,
        _backend: &B,
    ) -> RaikoResult<Proof> {
        if input.witnesses.is_empty() {
            return Err(RaikoError::Guest(
                "GuestInput must contain at least one witness".to_string(),
            ));
        }

        let proof_carry_data: ProofCarryData = serde_json::from_value(
            config
                .get("proof_carry_data")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        )
        .unwrap_or_default();

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

        Ok(Proof {
            input: Some(instance.instance_hash()),
            extra_data: Some(proof_carry_data),
            ..Default::default()
        })
    }

    async fn aggregate(
        &self,
        _input: raiko2_primitives::AggregationGuestInput,
        config: &ProverConfig,
        _backend: &B,
    ) -> RaikoResult<Proof> {
        let aggregation_input: ShastaZkAggregationGuestInput = serde_json::from_value(
            config
                .get("shasta_zk_aggregation_input")
                .cloned()
                .ok_or_else(|| {
                    RaikoError::InvalidRequestConfig(
                        "Missing 'shasta_zk_aggregation_input' in config".to_string(),
                    )
                })?,
        )
        .map_err(|e| {
            RaikoError::InvalidRequestConfig(format!("Failed to parse aggregation input: {}", e))
        })?;

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
