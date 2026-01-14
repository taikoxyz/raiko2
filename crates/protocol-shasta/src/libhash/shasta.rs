use crate::shasta::{
    Checkpoint, Commitment, CoreState, ProofCarryData, Proposal, TransitionInputData,
};
use alloy_primitives::{Address, B256, U256};

use super::encode::{VERIFY_PROOF_B256, address_to_b256, u48_to_b256, u64_to_b256};
use super::values::{hash_five_values, hash_four_values, hash_three_values, hash_values_impl};

/// Hash a checkpoint using the same logic as the Solidity implementation
pub fn hash_checkpoint(checkpoint: &Checkpoint) -> B256 {
    hash_three_values(
        U256::from(checkpoint.blockNumber).into(),
        checkpoint.blockHash,
        checkpoint.stateRoot,
    )
}

/// Domain-separated hash for a Shasta sub-proof public input.
///
/// This binds `chain_id` and `verifier` to the signed message to avoid cross-chain / cross-verifier
/// replay of otherwise identical transition inputs.
pub fn hash_shasta_subproof_input(carry: &ProofCarryData) -> B256 {
    tracing::info!("hash_shasta_subproof_input: {carry:?}");
    let transition_hash = hash_shasta_transition_input(&carry.transition_input);
    hash_four_values(
        VERIFY_PROOF_B256,
        U256::from(carry.chain_id).into(),
        address_to_b256(carry.verifier),
        transition_hash,
    )
}

pub fn hash_shasta_transition_input(transition_input: &TransitionInputData) -> B256 {
    // IMPORTANT (soundness): Aggregation checks rely on fields beyond `Transition`.
    // This hash must bind all continuity-critical fields; otherwise a caller can tamper with
    // carry-data (e.g. parent hashes / end checkpoint) without invalidating the sub-proof input.
    let mut values: Vec<B256> = Vec::with_capacity(13);

    // Proposal linkage
    values.push(u64_to_b256(transition_input.proposal_id));
    values.push(transition_input.proposal_hash);
    values.push(transition_input.parent_proposal_hash);
    values.push(transition_input.parent_checkpoint_hash);

    // Prover identity (L1-level)
    values.push(address_to_b256(transition_input.actual_prover));

    // Transition fields (as in Solidity Transition struct)
    values.push(address_to_b256(transition_input.transition.proposer));
    values.push(address_to_b256(
        transition_input.transition.designatedProver,
    ));
    values.push(u48_to_b256(transition_input.transition.timestamp));
    values.push(hash_checkpoint(&transition_input.checkpoint));

    // End checkpoint fields used by `Commitment` (bind to prevent tampering)
    values.push(u48_to_b256(
        transition_input.checkpoint.blockNumber.to::<u64>(),
    ));
    values.push(transition_input.checkpoint.blockHash);
    values.push(transition_input.checkpoint.stateRoot);

    hash_values_impl(&values)
}

pub fn hash_commitment(prove_input: &Commitment) -> B256 {
    // Flatten all the fields into a Vec<B256>, as in Solidity's buffer.
    let transition_count = prove_input.transitions.len();
    let mut buffer: Vec<B256> = Vec::with_capacity(9 + transition_count * 4);

    // Top-level head
    buffer.push(U256::from(0x20u64).into());

    // Commitment static fields
    buffer.push(U256::from(prove_input.firstProposalId).into());
    buffer.push(prove_input.firstProposalParentBlockHash);
    buffer.push(prove_input.lastProposalHash);
    buffer.push(address_to_b256(prove_input.actualProver));
    buffer.push(U256::from(prove_input.endBlockNumber).into());
    buffer.push(prove_input.endStateRoot);
    buffer.push(U256::from(0xe0u64).into());

    buffer.push(U256::from(transition_count as u64).into());
    // Flatten each Transition as in Solidity: [proposer, designatedProver, timestamp, checkpointHash]
    for transition in &prove_input.transitions {
        buffer.push(address_to_b256(transition.proposer));
        buffer.push(address_to_b256(transition.designatedProver));
        buffer.push(u48_to_b256(transition.timestamp.to::<u64>()));
        buffer.push(transition.checkpointHash);
    }

    hash_values_impl(&buffer)
}

pub fn hash_proposal(proposal: &Proposal) -> B256 {
    // Pack the fields as in Solidity, using proper bit shifts and concatenation.
    let packed: U256 = (U256::from(proposal.id) << 208)
        | (U256::from(proposal.timestamp) << 160)
        | (U256::from(proposal.endOfSubmissionWindowTimestamp) << 112);

    // Encode proposer address to B256 by zero-padding its 20 bytes to 32 bytes (uint256(uint160))
    let proposer_b256 = address_to_b256(proposal.proposer);
    hash_three_values(packed.into(), proposer_b256, proposal.derivationHash)
}

pub fn hash_core_state(core_state: &CoreState) -> B256 {
    hash_five_values(
        U256::from(core_state.nextProposalId).into(),
        U256::from(core_state.lastProposalBlockId).into(),
        U256::from(core_state.lastFinalizedProposalId).into(),
        U256::from(core_state.lastCheckpointTimestamp).into(),
        core_state.lastFinalizedTransitionHash,
    )
}

pub fn hash_public_input(
    prove_input_hash: B256,
    chain_id: u64,
    verifier_address: Address,
    sgx_instance: Address,
) -> B256 {
    hash_five_values(
        VERIFY_PROOF_B256,
        U256::from(chain_id).into(),
        address_to_b256(verifier_address),
        prove_input_hash,
        address_to_b256(sgx_instance),
    )
}
