use alloy_primitives::{Address, B256};
use serde::{Deserialize, Serialize};

/// Direct aggregate input for TDX remote provers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TdxDirectAggregateGuestInput {
    pub proposals: Vec<TdxDirectAggregateProposal>,
}

/// One Shasta proposal entry for TDX direct aggregation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TdxDirectAggregateProposal {
    pub chain_id: u64,
    pub verifier: Address,
    pub proposal_id: u64,
    pub proposal_hash: B256,
    pub parent_proposal_hash: B256,
    pub actual_prover: Address,
    pub transition: TdxDirectAggregateTransition,
    pub l2_block_numbers: Vec<u64>,
}

/// Shasta transition fields needed by the remote TDX signer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TdxDirectAggregateTransition {
    pub proposer: Address,
    pub timestamp: u64,
}
