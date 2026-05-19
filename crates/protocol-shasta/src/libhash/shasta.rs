use crate::shasta::{
    Checkpoint, Commitment, CoreState, ProofCarryData, Proposal, TransitionInputData,
};
use alloy_primitives::{Address, B256, U256, keccak256};
use alloy_sol_types::SolValue;

use super::encode::{VERIFY_PROOF_B256, address_to_b256, u48_to_b256, u64_to_b256};
use super::values::{hash_five_values, hash_four_values, hash_three_values, hash_values_impl};

/// Hash a checkpoint using the same logic as the Solidity implementation
#[must_use]
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
    let transition_hash = hash_shasta_transition_input(&carry.transition_input);
    hash_four_values(
        VERIFY_PROOF_B256,
        U256::from(carry.chain_id).into(),
        address_to_b256(carry.verifier),
        transition_hash,
    )
}

#[must_use]
pub fn hash_shasta_transition_input(transition_input: &TransitionInputData) -> B256 {
    // IMPORTANT (soundness): Aggregation checks rely on fields beyond `Transition`.
    // This hash must bind all continuity-critical fields; otherwise a caller can tamper with
    // carry-data (e.g. parent hashes / end checkpoint) without invalidating the sub-proof input.
    let mut values: Vec<B256> = Vec::with_capacity(12);

    // Proposal linkage
    values.push(u64_to_b256(transition_input.proposal_id));
    values.push(transition_input.proposal_hash);
    values.push(transition_input.parent_proposal_hash);
    values.push(transition_input.parent_block_hash);

    // Prover identity (L1-level)
    values.push(address_to_b256(transition_input.actual_prover));

    // Transition fields (as in Solidity Transition struct)
    values.push(address_to_b256(transition_input.transition.proposer));
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

/// Optimized hashing for commitment data, matching Solidity's hashCommitment implementation.
/// Flattens all fields following the same memory layout as the Solidity buffer,
/// including static field ordering, offsets, and transition element packing.
#[must_use]
pub fn hash_commitment(commitment: &Commitment) -> B256 {
    let transitions_len = commitment.transitions.len();
    let total_words = 9 + transitions_len * 3;

    let mut buffer: Vec<B256> = Vec::with_capacity(total_words);

    // [0] offset to commitment (0x20)
    buffer.push(U256::from(0x20u64).into());

    // Commitment static section
    // [1] firstProposalId
    buffer.push(U256::from(commitment.firstProposalId).into());
    // [2] firstProposalParentBlockHash
    buffer.push(commitment.firstProposalParentBlockHash);
    // [3] lastProposalHash
    buffer.push(commitment.lastProposalHash);
    // [4] actualProver as address (160 bits zero-extended to 256)
    buffer.push(address_to_b256(commitment.actualProver));
    // [5] endBlockNumber
    buffer.push(U256::from(commitment.endBlockNumber).into());
    // [6] endStateRoot
    buffer.push(commitment.endStateRoot);
    // [7] offset to transitions (0xe0)
    buffer.push(U256::from(0xe0u64).into());

    // [8] transitions array length
    buffer.push(U256::from(transitions_len as u64).into());

    // Each transition: [proposer, timestamp, blockHash]
    for transition in &commitment.transitions {
        buffer.push(address_to_b256(transition.proposer));
        buffer.push(U256::from(transition.timestamp).into());
        buffer.push(transition.blockHash);
    }

    hash_values_impl(&buffer)
}

#[must_use]
pub fn hash_proposal(proposal: &Proposal) -> B256 {
    keccak256(proposal.abi_encode().as_slice())
}

#[must_use]
pub fn hash_core_state(core_state: &CoreState) -> B256 {
    hash_five_values(
        U256::from(core_state.nextProposalId).into(),
        U256::from(core_state.lastProposalBlockId).into(),
        U256::from(core_state.lastFinalizedProposalId).into(),
        U256::from(core_state.lastFinalizedTimestamp).into(),
        U256::from(core_state.lastCheckpointTimestamp).into(),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use tracing::{Level, Subscriber, dispatcher::Dispatch};
    use tracing_subscriber::{
        layer::{Context, Layer},
        prelude::*,
        registry::LookupSpan,
    };

    use super::hash_shasta_subproof_input;
    use crate::shasta::ProofCarryData;

    #[derive(Clone)]
    struct EventCounter {
        info_events: Arc<AtomicUsize>,
    }

    impl<S> Layer<S> for EventCounter
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
    {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            if *event.metadata().level() == Level::INFO
                && event.metadata().target() == "raiko2_protocol_shasta::libhash::shasta"
            {
                self.info_events.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    #[test]
    fn hash_shasta_subproof_input_does_not_emit_info_logs() {
        let info_events = Arc::new(AtomicUsize::new(0));
        let subscriber = tracing_subscriber::registry().with(EventCounter {
            info_events: info_events.clone(),
        });
        let dispatch = Dispatch::new(subscriber);

        let mut carry = ProofCarryData::default();
        carry.chain_id = 167_000;

        tracing::dispatcher::with_default(&dispatch, || {
            let _ = hash_shasta_subproof_input(&carry);
        });

        assert_eq!(info_events.load(Ordering::Relaxed), 0);
    }
}

#[must_use]
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
