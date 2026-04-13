use raiko2_primitives::{RaikoError, RaikoResult};
use raiko2_primitives_shasta::GuestInput;

use crate::gaiko2::protocol::{
    GAIKO2_SHASTA_REQUEST_SCHEMA, Gaiko2ReplayBlock, Gaiko2ShastaPayload, Gaiko2ShastaRequest,
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

    Ok(Gaiko2ShastaRequest {
        schema: GAIKO2_SHASTA_REQUEST_SCHEMA.to_string(),
        payload: Gaiko2ShastaPayload {
            chain_id,
            blocks: input
                .witnesses
                .iter()
                .cloned()
                .map(Gaiko2ReplayBlock::from)
                .collect(),
            proof_carry_data: input.proof_carry_data.clone(),
        },
    })
}
