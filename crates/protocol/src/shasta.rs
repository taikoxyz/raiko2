//! Shasta protocol implementation.
//!
//! This module contains all Shasta-specific protocol types and codecs.

use alloy_primitives::{Address, B256, Uint};
use alloy_sol_types::{SolValue, sol};
use core::fmt::Debug;
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

    #[derive(Debug, Deserialize, Serialize)]
    enum BondType {
        NONE,
        PROVABILITY,
        LIVENESS
    }

    #[derive(Debug, Deserialize, Serialize)]
    struct BondInstruction {
        uint48 proposalId;
        BondType bondType;
        address payer;
        address receiver;
    }

    #[derive(Debug, Default, Deserialize, Serialize)]
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
        /// @notice The current hash of coreState
        bytes32 coreStateHash;
        /// @notice Hash of the Derivation struct containing additional proposal data.
        bytes32 derivationHash;
    }

    #[derive(Debug, Default, Deserialize, Serialize)]
    struct Transition {
        bytes32 proposalHash;
        bytes32 parentTransitionHash;
        Checkpoint checkpoint;
    }

    #[derive(Debug, Default, Deserialize, Serialize)]
    struct TransitionMetadata {
        /// @notice The designated prover for this transition.
        address designatedProver;
        /// @notice The actual prover who submitted the proof.
        address actualProver;
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
        /// @notice The hash of all bond instructions.
        bytes32 bondInstructionsHash;
    }

    #[derive(Debug, Default, Deserialize, Serialize)]
    struct ProposedEventPayload {
        Proposal proposal;
        Derivation derivation;
        CoreState coreState;
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

    #[derive(Debug, Default, Deserialize, Serialize)]
    event BondInstructed(BondInstruction[] instructions);
}

/// Decoded Shasta event data containing the proposal and related information
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ShastaEventData {
    pub proposal: Proposal,
    pub derivation: Derivation,
    pub core_state: CoreState,
}

impl ShastaEventData {
    /// Decode the bytes data from Shasta Proposed event into ShastaEventData
    pub fn from_event_data(data: &[u8]) -> Result<Self, alloy_sol_types::Error> {
        Self::decode_event_data(data)
    }

    fn _decode_event_data_with_abi(data: &[u8]) -> Result<Self, alloy_sol_types::Error> {
        let payload = ProposedEventPayload::abi_decode(data)?;
        Ok(Self {
            proposal: payload.proposal,
            derivation: payload.derivation,
            core_state: payload.coreState,
        })
    }

    fn unpack_uint24(
        data: &[u8],
        pos: usize,
    ) -> Result<(Uint<24, 1>, usize), alloy_sol_types::Error> {
        // Ensure we have enough data for a 3-byte value
        if pos + 3 > data.len() {
            return Err(alloy_sol_types::Error::custom(
                "Not enough data to read 3-byte uint24".to_string(),
            ));
        }

        let value = u32::from_be_bytes([0, data[pos], data[pos + 1], data[pos + 2]]);
        let value = Uint::<24, 1>::from_limbs([value as u64]);
        // New position is old position + 3 bytes
        let new_pos = pos + 3;
        Ok((value, new_pos))
    }

    /// Unpacks a uint48 value from the data buffer at the given position
    /// Matches the Solidity mload behavior by reading a full 32-byte word and extracting 6 bytes
    fn unpack_uint48(
        data: &[u8],
        pos: usize,
    ) -> Result<(Uint<48, 1>, usize), alloy_sol_types::Error> {
        // Ensure we have enough data for a full 32-byte word
        if pos + 6 > data.len() {
            return Err(alloy_sol_types::Error::custom(
                "Not enough data to read 32-byte word".to_string(),
            ));
        }

        let value = u64::from_be_bytes([
            0,
            0,
            data[pos],
            data[pos + 1],
            data[pos + 2],
            data[pos + 3],
            data[pos + 4],
            data[pos + 5],
        ]);
        let value = Uint::<48, 1>::from_limbs([value]);
        // New position is old position + 6 bytes
        let new_pos = pos + 6;
        Ok((value, new_pos))
    }

    fn unpack_address(data: &[u8], pos: usize) -> Result<(Address, usize), alloy_sol_types::Error> {
        if pos + 20 > data.len() {
            return Err(alloy_sol_types::Error::custom(
                "Not enough data to read 20-byte address".to_string(),
            ));
        }

        let address = Address::from_slice(&data[pos..pos + 20]);
        let new_pos = pos + 20;
        Ok((address, new_pos))
    }

    fn unpack_hash(data: &[u8], pos: usize) -> Result<(B256, usize), alloy_sol_types::Error> {
        if pos + 32 > data.len() {
            return Err(alloy_sol_types::Error::custom(
                "Not enough data to read 32-byte hash".to_string(),
            ));
        }

        let hash = B256::from_slice(&data[pos..pos + 32]);
        let new_pos = pos + 32;
        Ok((hash, new_pos))
    }

    /// Add helper function to unpack uint16
    fn unpack_uint16(data: &[u8], pos: usize) -> Result<(u16, usize), alloy_sol_types::Error> {
        if pos + 2 > data.len() {
            return Err(alloy_sol_types::Error::custom(
                "Not enough data to read 2-byte uint16".to_string(),
            ));
        }

        let value = u16::from_be_bytes([data[pos], data[pos + 1]]);
        let new_pos = pos + 2;
        Ok((value, new_pos))
    }

    /// Add helper function to unpack uint8
    fn unpack_uint8(data: &[u8], pos: usize) -> Result<(u8, usize), alloy_sol_types::Error> {
        if pos + 1 > data.len() {
            return Err(alloy_sol_types::Error::custom(
                "Not enough data to read 1-byte uint8".to_string(),
            ));
        }

        let value = data[pos];
        let new_pos = pos + 1;
        Ok((value, new_pos))
    }

    /// Manual decoding of Shasta event data following the Solidity implementation
    /// Reference: taiko-mono/packages/protocol/contracts/layer1/shasta/libs/LibProposedEventEncoder.sol
    fn decode_event_data(data: &[u8]) -> Result<Self, alloy_sol_types::Error> {
        let mut ptr = 0;

        // Decode Proposal
        let (proposal_id, new_ptr) = Self::unpack_uint48(data, ptr)?;
        ptr = new_ptr;
        let (proposer, new_ptr) = Self::unpack_address(data, ptr)?;
        ptr = new_ptr;
        let (timestamp, new_ptr) = Self::unpack_uint48(data, ptr)?;
        ptr = new_ptr;
        let (end_of_submission_window_timestamp, new_ptr) = Self::unpack_uint48(data, ptr)?;
        ptr = new_ptr;

        // Decode derivation fields
        let (origin_block_number, new_ptr) = Self::unpack_uint48(data, ptr)?;
        ptr = new_ptr;
        let (origin_block_hash, new_ptr) = Self::unpack_hash(data, ptr)?;
        ptr = new_ptr;
        let (basefee_sharing_pctg, new_ptr) = Self::unpack_uint8(data, ptr)?;
        ptr = new_ptr;

        // Decode sources array length
        let (sources_length, new_ptr) = Self::unpack_uint16(data, ptr)?;
        ptr = new_ptr;

        let mut sources = Vec::new();
        for _ in 0..sources_length {
            // Decode is_forced_inclusion flag
            let (is_forced_inclusion_u8, new_ptr) = Self::unpack_uint8(data, ptr)?;
            ptr = new_ptr;
            let is_forced_inclusion = is_forced_inclusion_u8 != 0;

            // Decode blob slice for this source
            let (blob_hashes_length, new_ptr) = Self::unpack_uint16(data, ptr)?;
            ptr = new_ptr;

            let mut blob_hashes = Vec::new();
            for _ in 0..blob_hashes_length {
                let (blob_hash, new_ptr) = Self::unpack_hash(data, ptr)?;
                ptr = new_ptr;
                blob_hashes.push(blob_hash);
            }

            let (offset, new_ptr) = Self::unpack_uint24(data, ptr)?;
            ptr = new_ptr;
            let (blob_timestamp, new_ptr) = Self::unpack_uint48(data, ptr)?;
            ptr = new_ptr;

            sources.push(DerivationSource {
                isForcedInclusion: is_forced_inclusion,
                blobSlice: BlobSlice {
                    blobHashes: blob_hashes,
                    offset,
                    timestamp: blob_timestamp,
                },
            });
        }

        let (core_state_hash, new_ptr) = Self::unpack_hash(data, ptr)?;
        ptr = new_ptr;
        let (derivation_hash, new_ptr) = Self::unpack_hash(data, ptr)?;
        ptr = new_ptr;

        // Decode core state
        let (next_proposal_id, new_ptr) = Self::unpack_uint48(data, ptr)?;
        ptr = new_ptr;
        let (last_proposal_block_id, new_ptr) = Self::unpack_uint48(data, ptr)?;
        ptr = new_ptr;
        let (last_finalized_proposal_id, new_ptr) = Self::unpack_uint48(data, ptr)?;
        ptr = new_ptr;
        let (last_checkpoint_timestamp, new_ptr) = Self::unpack_uint48(data, ptr)?;
        ptr = new_ptr;
        let (last_finalized_transition_hash, new_ptr) = Self::unpack_hash(data, ptr)?;
        ptr = new_ptr;
        let (bond_instructions_hash, _new_ptr) = Self::unpack_hash(data, ptr)?;

        Ok(Self {
            proposal: Proposal {
                id: proposal_id,
                timestamp,
                endOfSubmissionWindowTimestamp: end_of_submission_window_timestamp,
                proposer,
                coreStateHash: core_state_hash,
                derivationHash: derivation_hash,
            },
            derivation: Derivation {
                originBlockNumber: origin_block_number,
                originBlockHash: origin_block_hash,
                basefeeSharingPctg: basefee_sharing_pctg,
                sources,
            },
            core_state: CoreState {
                nextProposalId: next_proposal_id,
                lastProposalBlockId: last_proposal_block_id,
                lastFinalizedProposalId: last_finalized_proposal_id,
                lastCheckpointTimestamp: last_checkpoint_timestamp,
                lastFinalizedTransitionHash: last_finalized_transition_hash,
                bondInstructionsHash: bond_instructions_hash,
            },
        })
    }
}
