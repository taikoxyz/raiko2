//! Shasta protocol implementation.
//!
//! This module contains all Shasta-specific protocol types and codecs.

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
        /// @notice Hash of the Derivation struct containing additional proposal data.
        bytes32 derivationHash;
    }

    #[derive(Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
    /// @notice Transition data for a proposal used in prove
    struct Transition {
        /// @notice Address of the proposer.
        address proposer;
        /// @notice Address of the designated prover.
        address designatedProver;
        /// @notice Timestamp of the proposal.
        uint48 timestamp;
        /// @notice checkpoint hash for the proposal.
        bytes32 checkpointHash;
    }

    #[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
    /// @notice Commitment data that the prover commits to when submitting a proof.
    struct Commitment {
        /// @notice The ID of the first proposal being proven.
        uint48 firstProposalId;
        /// @notice The checkpoint hash of the parent of the first proposal, this is used
        /// to verify checkpoint continuity in the proof.
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
        /// @notice The last block ID where a proposal was made.
        uint48 lastProposalBlockId;
        /// @notice The ID of the last finalized proposal.
        uint48 lastFinalizedProposalId;
        /// @notice The timestamp when the last checkpoint was saved.
        /// @dev In genesis block, this is set to 0 to allow the first checkpoint to be saved.
        uint48 lastCheckpointTimestamp;
        /// @notice The hash of the last finalized transition.
        bytes32 lastFinalizedTransitionHash;
    }

    #[derive(Debug, Default, Deserialize, Serialize)]
    struct ProposedEventPayload {
        /// @notice The proposal that was created.
        Proposal proposal;
        /// @notice The derivation data for the proposal.
        Derivation derivation;
    }

    #[derive(Debug, Default, Deserialize, Serialize)]
    struct BlobReference {
        uint16 blobStartIndex;
        uint16 numBlobs;
        uint24 offset;
    }

    #[derive(Debug, Default, Deserialize, Serialize)]
    event Proposed(bytes data);

    #[derive(Debug, Default, Deserialize, Serialize)]
    event Proved(bytes data);
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
#[allow(non_snake_case)]
// In Shasta, each sub proposal signs this structure to prove the proposal's transition.
// We keep ABI-compatible field names.
pub struct ShastaTransitionInput {
    pub proposer: Address,
    pub designatedProver: Address,
    pub timestamp: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TransitionInputData {
    pub proposal_id: u64,
    pub proposal_hash: B256,
    pub parent_proposal_hash: B256,
    pub parent_checkpoint_hash: B256,
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
    pub derivation: Derivation,
}

mod decode;

#[cfg(test)]
mod tests {
    use super::ShastaEventData;
    use alloy_primitives::hex;

    #[test]
    fn test_decode_known_hex() {
        // This is a real example: decode the provided hex-encoded payload and check fields.
        let data = hex::decode("0000000009143c44cdddb6a900fa2b585dd299e03d12fa4293bc0000693e56cc000000000000d1374b45317e657e07505c83fc4702e8f6e043ff3e7beb2eaa0974783a4222ae0000000038aeb5f96a8745b06f7a00e5741f503d6d45c0d5ec1377960abe86e45299d6410cdf4b000100000101b1a43b3e87672be8a5102ac0d99dc4215491d8a07a7fa402d34d7f1ac9696d0000000000693e56cc36cf931b08528aa49160c33ecda1505b2c292a4947c416d9dc26646ebe9c0d35").unwrap();

        // Decode using manual decoding function
        let result = ShastaEventData::decode_event_data(&data);

        assert!(
            result.is_ok(),
            "Failed to manually decode Shasta event data: {:?}",
            result.err()
        );

        let event_data = result.unwrap();

        // Spot-check some expected field invariants:

        // Proposal
        println!("proposal.id: {}", event_data.proposal.id);
        println!("proposal.proposer: {:?}", event_data.proposal.proposer);
        println!("proposal.timestamp: {}", event_data.proposal.timestamp);
        println!(
            "proposal.parentProposalHash: 0x{}",
            hex::encode(event_data.proposal.parentProposalHash)
        );
        println!(
            "proposal.derivationHash: 0x{}",
            hex::encode(event_data.proposal.derivationHash)
        );

        // Derivation
        println!(
            "derivation.originBlockNumber: {}",
            event_data.derivation.originBlockNumber
        );
        println!(
            "derivation.originBlockHash: 0x{}",
            hex::encode(event_data.derivation.originBlockHash)
        );
        println!(
            "derivation.basefeeSharingPctg: {}",
            event_data.derivation.basefeeSharingPctg
        );
        println!(
            "derivation.sources.length: {}",
            event_data.derivation.sources.len()
        );

        // Derivation Source
        let s = &event_data.derivation.sources[0];
        println!("isForcedInclusion: {}", s.isForcedInclusion);
        println!("blobHashes.length: {}", s.blobSlice.blobHashes.len());
        println!(
            "blobHashes[0]: 0x{}",
            hex::encode(s.blobSlice.blobHashes[0])
        );
        println!("offset: {}", s.blobSlice.offset);
        println!("timestamp: {}", s.blobSlice.timestamp);
    }
}
