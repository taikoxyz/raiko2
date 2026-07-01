use raiko2_primitives::{Proof, RaikoError, RaikoResult, StatelessInput, WitnessHeader};
use raiko2_primitives_shasta::{GuestInput, roll_proposal_ancestor_headers_in_place};

use crate::remote_prover::protocol::{
    RAIKO2_SHASTA_AGGREGATE_REQUEST_SCHEMA, RAIKO2_SHASTA_REQUEST_SCHEMA, Raiko2AggregateProof,
    Raiko2ReplayBlock, Raiko2ShastaAggregatePayload, Raiko2ShastaAggregateRequest,
    Raiko2ShastaGuestInput, Raiko2ShastaPayload, Raiko2ShastaRequest,
};

/// # Errors
///
/// Returns an error when the guest input has no witnesses or the replay packet cannot be
/// assembled from the witness state.
pub fn build_shasta_packet(input: &GuestInput) -> RaikoResult<Raiko2ShastaRequest> {
    shasta_packet_chain_id(input)?;

    let shared_state_nodes = input.proposal_state_nodes();
    let proposal_ancestor_headers = input.initial_proposal_ancestor_headers();
    let mut ancestor_headers = proposal_ancestor_headers.clone();
    let mut witnesses = Vec::with_capacity(input.witnesses.len());

    for stateless_input in input.witnesses.iter().cloned() {
        let block_header = stateless_input.block.header.clone();
        witnesses.push(build_replay_block(
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

    Ok(Raiko2ShastaRequest {
        schema: RAIKO2_SHASTA_REQUEST_SCHEMA.to_string(),
        payload: Raiko2ShastaPayload {
            guest_input: Raiko2ShastaGuestInput {
                witnesses,
                taiko: input.taiko.clone(),
                proposal_ancestor_headers,
                proposal_state_nodes: input.proposal_state_nodes.clone(),
                proof_carry_data: input.proof_carry_data.clone(),
            },
        },
    })
}

fn shasta_packet_chain_id(input: &GuestInput) -> RaikoResult<u64> {
    if input.witnesses.is_empty() {
        return Err(RaikoError::InvalidRequestConfig(
            "cannot build remote prover shasta packet without witnesses".to_string(),
        ));
    }
    if input.proof_carry_data.chain_id == 0 {
        return Err(RaikoError::InvalidRequestConfig(
            "cannot build remote prover shasta packet with unset proof_carry_data.chain_id"
                .to_string(),
        ));
    }

    Ok(input.proof_carry_data.chain_id)
}

/// # Errors
///
/// Returns an error when the proof list is empty or a proof cannot be converted into the
/// remote aggregate proof envelope.
pub fn build_shasta_aggregate_request(
    proofs: &[Proof],
) -> RaikoResult<Raiko2ShastaAggregateRequest> {
    if proofs.is_empty() {
        return Err(RaikoError::InvalidRequestConfig(
            "cannot build remote prover shasta aggregate request without proofs".to_string(),
        ));
    }

    let proofs = proofs
        .iter()
        .map(Raiko2AggregateProof::from_proof)
        .collect::<RaikoResult<Vec<_>>>()?;

    Ok(Raiko2ShastaAggregateRequest {
        schema: RAIKO2_SHASTA_AGGREGATE_REQUEST_SCHEMA.to_string(),
        payload: Raiko2ShastaAggregatePayload { proofs },
    })
}

fn build_replay_block(
    mut stateless_input: StatelessInput,
    ancestor_headers: &[WitnessHeader],
    shared_state_nodes: &[raiko2_primitives::WitnessStateNode],
) -> RaikoResult<Raiko2ReplayBlock> {
    if stateless_input.witness.headers.is_empty() && !ancestor_headers.is_empty() {
        stateless_input.witness.headers = ancestor_headers.to_vec();
    }
    stateless_input.witness.headers =
        remote_prover_witness_headers(&stateless_input.witness.headers);

    if stateless_input.witness.state.is_empty() && !stateless_input.witness.state_indices.is_empty()
    {
        stateless_input.witness.state = stateless_input
            .witness
            .state_indices
            .iter()
            .map(|index| {
                shared_state_nodes.get(*index as usize).cloned().ok_or_else(|| {
                    RaikoError::InvalidRequestConfig(format!(
                        "remote prover shared witness state index {index} out of bounds for pool length {}",
                        shared_state_nodes.len()
                    ))
                })
            })
            .collect::<RaikoResult<_>>()?;
        stateless_input.witness.state_indices.clear();
    }

    if stateless_input.witness.headers.is_empty() {
        return Err(RaikoError::InvalidRequestConfig(
            "remote prover replay witness is missing the parent header".to_string(),
        ));
    }

    Ok(Raiko2ReplayBlock::from(stateless_input))
}

fn remote_prover_witness_headers(ancestor_headers: &[WitnessHeader]) -> Vec<WitnessHeader> {
    let mut headers = ancestor_headers.to_vec();
    let compact_len = headers.len().saturating_sub(1);
    for header in headers.iter_mut().take(compact_len) {
        header.compact_in_place();
    }
    headers
}
