//! Shasta proof helpers for carrying protocol data.

use crate::GuestInput;
use alloy_primitives::Address;
use raiko2_primitives::{Proof, RaikoResult};
use raiko2_protocol_shasta::libhash::hash_proposal;
use raiko2_protocol_shasta::shasta::ProofCarryData;
use raiko2_protocol_shasta::shasta::TransitionInputData;

const SHASTA_METADATA_KEY: &str = "shasta";
const PROOF_CARRY_DATA_KEY: &str = "proof_carry_data";

/// Encode `ProofCarryData` into JSON for storage in `Proof.extra_data`.
///
/// # Errors
///
/// Returns an error if `ProofCarryData` cannot be serialized to JSON.
pub fn encode_proof_carry_data(carry: &ProofCarryData) -> RaikoResult<serde_json::Value> {
    Ok(serde_json::json!({
        SHASTA_METADATA_KEY: {
            PROOF_CARRY_DATA_KEY: carry,
        }
    }))
}

/// Decode `ProofCarryData` from JSON stored in `Proof.extra_data`.
///
/// # Errors
///
/// Returns an error if the JSON payload cannot be deserialized into `ProofCarryData`.
pub fn decode_proof_carry_data(value: &serde_json::Value) -> RaikoResult<ProofCarryData> {
    if let Some(carry) = value
        .get(SHASTA_METADATA_KEY)
        .and_then(|v| v.get(PROOF_CARRY_DATA_KEY))
    {
        return Ok(serde_json::from_value(carry.clone())?);
    }
    if let Some(carry) = value.get(PROOF_CARRY_DATA_KEY) {
        return Ok(serde_json::from_value(carry.clone())?);
    }
    Ok(serde_json::from_value(value.clone())?)
}

/// Decode optional `ProofCarryData` from `Proof.extra_data`.
///
/// # Errors
///
/// Returns an error if the JSON payload cannot be deserialized into `ProofCarryData`.
pub fn decode_proof_carry_data_opt(
    value: Option<&serde_json::Value>,
) -> RaikoResult<Option<ProofCarryData>> {
    match value {
        Some(v) => Ok(Some(decode_proof_carry_data(v)?)),
        None => Ok(None),
    }
}

/// Decode `ProofCarryData` from a `Proof` if present.
///
/// # Errors
///
/// Returns an error if `Proof.extra_data` cannot be deserialized into `ProofCarryData`.
#[allow(dead_code)]
pub fn proof_carry_from_proof(proof: &Proof) -> RaikoResult<Option<ProofCarryData>> {
    decode_proof_carry_data_opt(proof.extra_data.as_ref())
}

/// Build the canonical `ProofCarryData` for a Shasta proposal guest input.
///
/// # Panics
///
/// Panics if the last witness block number does not fit in Shasta's `uint48` checkpoint field.
#[must_use]
pub fn build_proof_carry_data(input: &GuestInput) -> ProofCarryData {
    // The witness chain id is the canonical value the guest validates against.
    let chain_id = input
        .witnesses
        .first()
        .map(|witness| witness.chain_spec.chain_id)
        .filter(|&id| id != 0)
        .unwrap_or(input.taiko.chain_spec.chain_id);
    let first_witness = input.witnesses.first();
    let last_witness = input.witnesses.last();
    let proposal = &input.taiko.proposal_event.proposal;

    ProofCarryData {
        chain_id,
        verifier: Address::default(),
        transition_input: TransitionInputData {
            proposal_id: input.taiko.proposal_id,
            proposal_hash: hash_proposal(proposal),
            parent_proposal_hash: proposal.parentProposalHash,
            parent_block_hash: first_witness
                .map(|witness| witness.block.header.parent_hash)
                .unwrap_or_default(),
            actual_prover: input.taiko.prover_data.actual_prover,
            transition: raiko2_protocol_shasta::shasta::ShastaTransitionInput {
                proposer: proposal.proposer,
                timestamp: proposal.timestamp.to::<u64>(),
            },
            checkpoint: raiko2_protocol_shasta::shasta::Checkpoint {
                blockNumber: last_witness
                    .map(|witness| {
                        witness
                            .block
                            .header
                            .number
                            .try_into()
                            .expect("block number fits in uint48")
                    })
                    .unwrap_or_default(),
                blockHash: last_witness
                    .map(|witness| witness.block.header.hash_slow())
                    .unwrap_or_default(),
                stateRoot: last_witness
                    .map(|witness| witness.block.header.state_root)
                    .unwrap_or_default(),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::build_proof_carry_data;
    use crate::GuestInput;
    use alloy_primitives::{Address, B256};

    #[test]
    fn build_proof_carry_data_populates_transition_fields_from_input() {
        let mut input = GuestInput::default();
        input.taiko.proposal_id = 7;
        input.taiko.prover_data.actual_prover = Address::from([0x11; 20]);
        input.taiko.proposal_event.proposal.proposer = Address::from([0x22; 20]);
        input.taiko.proposal_event.proposal.timestamp =
            123u64.try_into().expect("timestamp fits in uint48");
        input.taiko.proposal_event.proposal.parentProposalHash = B256::from([0x33; 32]);

        let mut witness = raiko2_primitives::StatelessInput::default();
        witness.chain_spec.chain_id = 167_000;
        witness.block.header.number = 42;
        witness.block.header.parent_hash = B256::from([0x44; 32]);
        witness.block.header.state_root = B256::from([0x55; 32]);
        input.witnesses.push(witness.clone());

        let carry = build_proof_carry_data(&input);

        assert_eq!(carry.chain_id, 167_000);
        assert_eq!(carry.transition_input.proposal_id, 7);
        assert_eq!(
            carry.transition_input.actual_prover,
            input.taiko.prover_data.actual_prover
        );
        assert_eq!(
            carry.transition_input.parent_proposal_hash,
            input.taiko.proposal_event.proposal.parentProposalHash
        );
        assert_eq!(
            carry.transition_input.parent_block_hash,
            witness.block.header.parent_hash
        );
        assert_eq!(
            carry.transition_input.transition.proposer,
            input.taiko.proposal_event.proposal.proposer
        );
        assert_eq!(carry.transition_input.transition.timestamp, 123);
        assert_eq!(
            carry.transition_input.checkpoint.blockNumber.to::<u64>(),
            witness.block.header.number
        );
        assert_eq!(
            carry.transition_input.checkpoint.blockHash,
            witness.block.header.hash_slow()
        );
        assert_eq!(
            carry.transition_input.checkpoint.stateRoot,
            witness.block.header.state_root
        );
    }
}
