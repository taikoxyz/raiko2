use raiko2_primitives::{Proof, RaikoError, RaikoResult, StatelessInput};
use raiko2_primitives_shasta::{GuestInput, roll_proposal_ancestor_headers_in_place};

use crate::gaiko2::protocol::{
    GAIKO2_SHASTA_REQUEST_SCHEMA, Gaiko2AggregateProof, Gaiko2ReplayBlock,
    Gaiko2ShastaAggregatePayload, Gaiko2ShastaAggregateRequest, Gaiko2ShastaPayload,
    Gaiko2ShastaRequest,
};

pub fn build_shasta_packet(input: &GuestInput) -> RaikoResult<Gaiko2ShastaRequest> {
    let first_witness = input.witnesses.first().ok_or_else(|| {
        RaikoError::InvalidRequestConfig(
            "cannot build gaiko2 shasta packet without witnesses".to_string(),
        )
    })?;
    let chain_id = if input.proof_carry_data.chain_id != 0 {
        input.proof_carry_data.chain_id
    } else {
        first_witness.chain_spec.chain_id
    };

    let shared_state_nodes = input.proposal_state_nodes();
    let mut ancestor_headers = input.initial_proposal_ancestor_headers();
    let mut blocks = Vec::with_capacity(input.witnesses.len());

    for stateless_input in input.witnesses.iter().cloned() {
        let block_header = stateless_input.block.header.clone();
        blocks.push(build_replay_block(
            stateless_input,
            &ancestor_headers,
            shared_state_nodes,
        )?);
        roll_proposal_ancestor_headers_in_place(
            &mut ancestor_headers,
            &block_header,
            block_header.hash_slow(),
        );
    }

    Ok(Gaiko2ShastaRequest {
        schema: GAIKO2_SHASTA_REQUEST_SCHEMA.to_string(),
        payload: Gaiko2ShastaPayload {
            chain_id,
            blocks,
            proof_carry_data: input.proof_carry_data.clone(),
        },
    })
}

pub fn build_shasta_aggregate_request(
    proofs: &[Proof],
) -> RaikoResult<Gaiko2ShastaAggregateRequest> {
    if proofs.is_empty() {
        return Err(RaikoError::InvalidRequestConfig(
            "cannot build gaiko2 shasta aggregate request without proofs".to_string(),
        ));
    }

    let proofs = proofs
        .iter()
        .map(Gaiko2AggregateProof::from_proof)
        .collect::<RaikoResult<Vec<_>>>()?;

    Ok(Gaiko2ShastaAggregateRequest {
        schema: GAIKO2_SHASTA_REQUEST_SCHEMA.to_string(),
        payload: Gaiko2ShastaAggregatePayload { proofs },
    })
}

fn build_replay_block(
    mut stateless_input: StatelessInput,
    ancestor_headers: &[raiko2_primitives::WitnessHeader],
    shared_state_nodes: &[raiko2_primitives::WitnessStateNode],
) -> RaikoResult<Gaiko2ReplayBlock> {
    if stateless_input.witness.headers.is_empty() && !ancestor_headers.is_empty() {
        stateless_input.witness.headers = ancestor_headers.to_vec();
    }

    if stateless_input.witness.state.is_empty() && !stateless_input.witness.state_indices.is_empty()
    {
        stateless_input.witness.state = stateless_input
            .witness
            .state_indices
            .iter()
            .map(|index| {
                shared_state_nodes.get(*index as usize).cloned().ok_or_else(|| {
                    RaikoError::InvalidRequestConfig(format!(
                        "gaiko2 shared witness state index {index} out of bounds for pool length {}",
                        shared_state_nodes.len()
                    ))
                })
            })
            .collect::<RaikoResult<_>>()?;
        stateless_input.witness.state_indices.clear();
    }

    if stateless_input.witness.headers.is_empty() {
        return Err(RaikoError::InvalidRequestConfig(
            "gaiko2 replay witness is missing the parent header".to_string(),
        ));
    }

    Ok(Gaiko2ReplayBlock::from(stateless_input))
}
