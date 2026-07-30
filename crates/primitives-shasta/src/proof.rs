//! Shasta proof helpers for carrying protocol data.

use crate::GuestInput;
use crate::instance::SHASTA_PROPOSAL_ID_MAX;
use raiko2_primitives::{ChainSpec, Proof, ProofType, RaikoError, RaikoResult};
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
pub fn proof_carry_from_proof(proof: &Proof) -> RaikoResult<Option<ProofCarryData>> {
    decode_proof_carry_data_opt(proof.extra_data.as_ref())
}

/// Build `ProofCarryData` for a Shasta proposal guest input, resolving the verifier address
/// from the witness-embedded chain spec.
///
/// # Trust
///
/// The verifier address placed in the journal-bound carry data is resolved from
/// `GuestInput.witnesses[0].chain_spec`, which is untrusted request data. This is only
/// appropriate on host-local construction paths (preflight, dev tooling) where the host itself
/// just resolved that spec. Admission validation MUST NOT trust carry data built this way and
/// MUST rebuild it via [`build_proof_carry_data_with_chain_spec`] with a host-resolved spec,
/// as `raiko2-pipeline` does (covered by a regression test there). On-chain verifier contracts
/// substitute `address(this)` when recomputing the public input, so a forged verifier address
/// makes verification fail (a liveness issue) rather than breaking soundness.
///
/// # Errors
///
/// Returns an error if the input is missing witnesses, if the verifier address cannot be resolved,
/// if the witness block number does not fit the protocol checkpoint type, or if the embedded prover
/// checkpoint does not match the canonical witness checkpoint.
pub fn build_proof_carry_data_from_witness_spec(
    input: &GuestInput,
    proof_type: ProofType,
) -> RaikoResult<ProofCarryData> {
    let first_witness = input.witnesses.first().ok_or_else(|| {
        RaikoError::InvalidRequestConfig(
            "cannot build Shasta proof carry data without witnesses".to_string(),
        )
    })?;

    build_proof_carry_data_with_chain_spec(input, proof_type, &first_witness.chain_spec)
}

/// Build `ProofCarryData` for a Shasta proposal using a trusted chain spec.
///
/// The host uses this variant during request validation so verifier selection is keyed by the
/// host-resolved chain spec, not by the untrusted `GuestInput.chain_spec` copy.
///
/// # Errors
///
/// Returns an error if the input is missing witnesses, if the verifier address cannot be resolved,
/// if the witness block number does not fit the protocol checkpoint type, or if the embedded prover
/// checkpoint does not match the canonical witness checkpoint.
pub fn build_proof_carry_data_with_chain_spec(
    input: &GuestInput,
    proof_type: ProofType,
    chain_spec: &ChainSpec,
) -> RaikoResult<ProofCarryData> {
    let first_witness = input.witnesses.first().ok_or_else(|| {
        RaikoError::InvalidRequestConfig(
            "cannot build Shasta proof carry data without witnesses".to_string(),
        )
    })?;
    let last_witness = input.witnesses.last().ok_or_else(|| {
        RaikoError::InvalidRequestConfig(
            "cannot build Shasta proof carry data without witnesses".to_string(),
        )
    })?;
    if chain_spec.chain_id == 0 {
        return Err(RaikoError::InvalidRequestConfig(
            "trusted chain_spec.chain_id must be non-zero".to_string(),
        ));
    }
    let chain_id = chain_spec.chain_id;
    let verifier_proof_type = match proof_type {
        ProofType::Native => ProofType::Sgx,
        other => other,
    };
    let verifier = chain_spec
        .get_fork_verifier_address(
            first_witness.block.header.number,
            first_witness.block.header.timestamp,
            verifier_proof_type,
        )
        .map_err(|err| {
            RaikoError::InvalidRequestConfig(format!(
                "failed to resolve verifier address for proof type {verifier_proof_type}: {err}"
            ))
        })?;
    let proposal = &input.taiko.proposal_event.proposal;
    let proposal_event_id = proposal.id.to::<u64>();
    if input.taiko.proposal_id > SHASTA_PROPOSAL_ID_MAX {
        return Err(RaikoError::InvalidRequestConfig(format!(
            "proposal_id does not fit in uint48: {}",
            input.taiko.proposal_id
        )));
    }
    if input.taiko.proposal_id != proposal_event_id {
        return Err(RaikoError::InvalidRequestConfig(format!(
            "proposal_id mismatch: expected {proposal_event_id}, got {}",
            input.taiko.proposal_id
        )));
    }
    let checkpoint = raiko2_protocol_shasta::shasta::Checkpoint {
        blockNumber: last_witness.block.header.number.try_into().map_err(|_| {
            RaikoError::InvalidRequestConfig(
                "last witness block number does not fit in uint48".to_string(),
            )
        })?,
        blockHash: last_witness.block.header.hash_slow(),
        stateRoot: last_witness.block.header.state_root,
    };
    if let Some(expected_checkpoint) = input.taiko.prover_data.checkpoint.as_ref()
        && expected_checkpoint != &checkpoint
    {
        return Err(RaikoError::InvalidRequestConfig(format!(
            "prover checkpoint mismatch: expected {expected_checkpoint:?}, got {checkpoint:?}"
        )));
    }

    Ok(ProofCarryData {
        chain_id,
        verifier,
        transition_input: TransitionInputData {
            proposal_id: input.taiko.proposal_id,
            proposal_hash: hash_proposal(proposal),
            parent_proposal_hash: proposal.parentProposalHash,
            parent_block_hash: first_witness.block.header.parent_hash,
            actual_prover: input.taiko.prover_data.actual_prover,
            transition: raiko2_protocol_shasta::shasta::ShastaTransitionInput {
                proposer: proposal.proposer,
                timestamp: proposal.timestamp.to::<u64>(),
            },
            checkpoint,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::{build_proof_carry_data_from_witness_spec, build_proof_carry_data_with_chain_spec};
    use crate::GuestInput;
    use alloy_primitives::{Address, B256};
    use raiko2_primitives::{ProofType, SupportedChainSpecs};

    #[test]
    fn build_proof_carry_data_populates_transition_fields_from_input() {
        let mut input = GuestInput::default();
        input.taiko.proposal_id = 7;
        input.taiko.proposal_event.proposal.id = 7u64.try_into().expect("fits in uint48");
        input.taiko.prover_data.actual_prover = Address::from([0x11; 20]);
        input.taiko.proposal_event.proposal.proposer = Address::from([0x22; 20]);
        input.taiko.proposal_event.proposal.timestamp =
            123u64.try_into().expect("timestamp fits in uint48");
        input.taiko.proposal_event.proposal.parentProposalHash = B256::from([0x33; 32]);

        let mut witness = raiko2_primitives::StatelessInput {
            chain_spec: SupportedChainSpecs::default()
                .get_chain_spec_with_chain_id(167_000)
                .expect("supported taiko mainnet chain spec"),
            ..Default::default()
        };
        witness.block.header.number = 42;
        witness.block.header.timestamp = u64::MAX / 2;
        witness.block.header.parent_hash = B256::from([0x44; 32]);
        witness.block.header.state_root = B256::from([0x55; 32]);
        input.witnesses.push(witness.clone());

        let carry = build_proof_carry_data_from_witness_spec(&input, ProofType::Native)
            .expect("build carry data");

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

    #[test]
    fn build_proof_carry_data_rejects_proposal_id_mismatch() {
        let mut input = GuestInput::default();
        input.taiko.proposal_id = 7;
        input.taiko.proposal_event.proposal.id = 8u64.try_into().expect("fits in uint48");

        let mut witness = raiko2_primitives::StatelessInput {
            chain_spec: SupportedChainSpecs::default()
                .get_chain_spec_with_chain_id(167_000)
                .expect("supported taiko mainnet chain spec"),
            ..Default::default()
        };
        witness.block.header.timestamp = u64::MAX / 2;
        input.witnesses.push(witness);

        let err = build_proof_carry_data_from_witness_spec(&input, ProofType::Native)
            .expect_err("proposal id mismatch should fail");
        assert!(err.to_string().contains("proposal_id mismatch"));
    }

    #[test]
    fn build_proof_carry_data_with_chain_spec_uses_trusted_verifier() {
        let trusted_spec = SupportedChainSpecs::default()
            .get_chain_spec_with_chain_id(167_000)
            .expect("supported taiko mainnet chain spec");
        let mut tampered_spec = trusted_spec.clone();
        for verifier_map in tampered_spec.verifier_address_forks.values_mut() {
            verifier_map.insert(ProofType::Sgx, Some(Address::from([0x99; 20])));
        }

        let mut input = GuestInput::default();
        input.taiko.proposal_id = 7;
        input.taiko.proposal_event.proposal.id = 7u64.try_into().expect("fits in uint48");

        let mut witness = raiko2_primitives::StatelessInput {
            chain_spec: tampered_spec,
            ..Default::default()
        };
        witness.block.header.number = 42;
        witness.block.header.timestamp = u64::MAX / 2;
        witness.block.header.parent_hash = B256::from([0x44; 32]);
        witness.block.header.state_root = B256::from([0x55; 32]);
        input.witnesses.push(witness);

        let carry_from_witness =
            build_proof_carry_data_from_witness_spec(&input, ProofType::Native)
                .expect("build carry data");
        let carry_from_trusted =
            build_proof_carry_data_with_chain_spec(&input, ProofType::Native, &trusted_spec)
                .expect("build carry data from trusted spec");

        assert_eq!(carry_from_witness.verifier, Address::from([0x99; 20]));
        assert_ne!(carry_from_trusted.verifier, carry_from_witness.verifier);
        assert_eq!(carry_from_trusted.chain_id, trusted_spec.chain_id);
    }
}
