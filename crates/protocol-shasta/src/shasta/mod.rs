//! Shasta protocol implementation.
//!
//! This module contains all Shasta-specific protocol types and codecs.

pub mod blob_coder;
pub mod constants;
pub mod manifest;

use alloy_primitives::{Address, B256, ChainId};
use alloy_sol_types::sol;
use serde::{Deserialize, Serialize};

sol! {
    #[derive(Debug, Default, Deserialize, Serialize)]
    /// @notice Represents a frame of data that is stored in multiple blobs. Note the size is
    /// encoded as a bytes32 at the offset location.
    struct BlobSlice {
        /// @notice The blobs containing the proposal's content.
        bytes32[] blobHashes;
        /// @notice The byte offset of the proposal's content in the containing blobs.
        uint24 offset;
        /// @notice The timestamp when the frame was created.
        uint48 timestamp;
    }

    #[derive(Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
    struct Checkpoint {
        uint48 blockNumber;
        bytes32 blockHash;
        bytes32 stateRoot;
    }

    /// @notice Contains derivation data for a proposal that is not needed during proving.
    /// @dev This data is hashed and stored in the Proposal struct to reduce calldata size.
    #[derive(Debug, Default, Deserialize, Serialize)]

    /// @notice Represents a source of derivation data within a Derivation
    struct DerivationSource {
        /// @notice Whether this source is from a forced inclusion.
        bool isForcedInclusion;
        /// @notice Blobs that contain the source's manifest data.
        BlobSlice blobSlice;
    }

    #[derive(Debug, Default, Deserialize, Serialize)]
    /// @notice Contains derivation data for a proposal that is not needed during proving.
    /// @dev This data is hashed and stored in the Proposal struct to reduce calldata size.
    struct Derivation {
        /// @notice The L1 block number when the proposal was accepted.
        uint48 originBlockNumber;
        /// @notice The hash of the origin block.
        bytes32 originBlockHash;
        /// @notice The percentage of base fee paid to coinbase.
        uint8 basefeeSharingPctg;
        /// @notice Array of derivation sources, where each can be regular or forced inclusion.
        DerivationSource[] sources;
    }

    #[derive(Debug, Default, Deserialize, Serialize)]
    /// @notice Represents a proposal for L2 blocks.
    struct Proposal {
        /// @notice Unique identifier for the proposal.
        uint48 id;
        /// @notice The L1 block timestamp when the proposal was accepted.
        uint48 timestamp;
        /// @notice The timestamp of the last slot where the current preconfer can propose.
        uint48 endOfSubmissionWindowTimestamp;
        /// @notice Address of the proposer.
        address proposer;
        /// @notice Hash of the parent proposal (zero for genesis).
        bytes32 parentProposalHash;
        /// @notice The L1 block number when the proposal was accepted.
        uint48 originBlockNumber;
        /// @notice The hash of the origin block.
        bytes32 originBlockHash;
        /// @notice The percentage of base fee paid to coinbase.
        uint8 basefeeSharingPctg;
        /// @notice Array of derivation sources, where each can be regular or forced inclusion.
        DerivationSource[] sources;
    }

    #[derive(Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
    /// @notice Transition data for a proposal used in prove
    struct Transition {
        /// @notice Address of the proposer.
        address proposer;
        /// @notice Timestamp of the proposal.
        uint48 timestamp;
        /// @notice end block hash for the proposal.
        bytes32 blockHash;
    }

    #[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
    /// @notice Commitment data that the prover commits to when submitting a proof.
    struct Commitment {
        /// @notice The ID of the first proposal being proven.
        uint48 firstProposalId;
        /// @notice The block hash of the parent of the first proposal, this is used
        /// to verify block continuity in the proof.
        bytes32 firstProposalParentBlockHash;
        /// @notice The hash of the last proposal being proven.
        bytes32 lastProposalHash;
        /// @notice The actual prover who generated the proof.
        address actualProver;
        /// @notice The block number for the end L2 block in this proposal.
        uint48 endBlockNumber;
        /// @notice The state root for the end L2 block in this proposal.
        bytes32 endStateRoot;
        /// @notice Array of transitions for each proposal in the proof range.
        Transition[] transitions;
    }

    #[derive(Debug, Default, Deserialize, Serialize)]
    /// @notice Represents the core state of the inbox.
    struct CoreState {
        /// @notice The next proposal ID to be assigned.
        uint48 nextProposalId;
        /// @notice The last L1 block ID where a proposal was made.
        uint48 lastProposalBlockId;
        /// @notice The ID of the last finalized proposal.
        uint48 lastFinalizedProposalId;
        /// @notice The timestamp when the last proposal was finalized.
        uint48 lastFinalizedTimestamp;
        /// @notice The timestamp when the last checkpoint was saved.
        /// @dev In genesis block, this is set to 0 to allow the first checkpoint to be saved.
        uint48 lastCheckpointTimestamp;
        /// @notice The block hash of the last finalized proposal.
        bytes32 lastFinalizedBlockHash;
    }

    #[derive(Debug, Default, Deserialize, Serialize)]
    event Proposed(
        uint48 indexed id,
        address indexed proposer,
        bytes32 parentProposalHash,
        uint48 endOfSubmissionWindowTimestamp,
        uint8 basefeeSharingPctg,
        DerivationSource[] sources
    );
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
#[allow(non_snake_case)]
// In Shasta, each sub proposal signs this structure to prove the proposal's transition.
// We keep ABI-compatible field names.
pub struct ShastaTransitionInput {
    pub proposer: Address,
    pub timestamp: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TransitionInputData {
    pub proposal_id: u64,
    pub proposal_hash: B256,
    pub parent_proposal_hash: B256,
    pub parent_block_hash: B256,
    pub actual_prover: Address,
    pub transition: ShastaTransitionInput,
    pub checkpoint: Checkpoint,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ProofCarryData {
    pub chain_id: ChainId,
    pub verifier: Address,
    pub transition_input: TransitionInputData,
}

/// Decoded Shasta event data containing the proposal and related information
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ShastaEventData {
    pub proposal: Proposal,
}

impl ShastaEventData {
    /// Decode a Shasta Proposed event into ShastaEventData.
    pub fn from_proposal_event(proposal: &Proposed) -> Result<Self, alloy_sol_types::Error> {
        Ok(Self {
            proposal: Proposal {
                id: proposal.id,
                endOfSubmissionWindowTimestamp: proposal.endOfSubmissionWindowTimestamp,
                proposer: proposal.proposer,
                parentProposalHash: proposal.parentProposalHash,
                basefeeSharingPctg: proposal.basefeeSharingPctg,
                sources: proposal.sources.clone(),
                ..Default::default()
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Proposed, ShastaEventData};

    #[test]
    fn shasta_event_data_from_proposed() {
        let proposed = Proposed::default();
        let event = ShastaEventData::from_proposal_event(&proposed).unwrap();
        assert_eq!(event.proposal.id, proposed.id);
    }
}
