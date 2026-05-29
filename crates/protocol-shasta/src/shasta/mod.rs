//! Shasta protocol implementation.
//!
//! This module contains all Shasta-specific protocol types and codecs.

#[cfg(feature = "net")]
pub mod anchor;
pub mod blob_coder;
pub mod constants;
pub mod derivation;
pub mod error;
pub mod manifest;
pub mod payload_helpers;

#[cfg(feature = "net")]
pub use anchor::{AnchorTxConstructor, AnchorTxConstructorError, AnchorV4Input};
pub use blob_coder::BlobCoder;
pub use derivation::{
    ParentBlockContext, ProposalMetadata, SourceDerivationError, ValidationContext,
    ValidationError, apply_inherited_metadata, decode_source_manifest_for_tx_list,
    manifest_is_default, prepare_source_manifest, prepare_source_manifest_with_max_blocks,
    validate_source_manifest,
};
pub use error::{ForkConfigResult, ProtocolError, Result, ShastaForkConfigError};
pub use payload_helpers::{
    PAYLOAD_ID_VERSION_V2, calculate_shasta_difficulty, encode_extra_data, encode_transactions,
    encode_tx_list, payload_id_to_bytes,
};

/// Byte length of Shasta block header extra data.
pub const SHASTA_EXTRA_DATA_LEN: usize = 7;

/// Decode the Shasta proposal id embedded in block header extra data.
#[must_use]
pub fn decode_proposal_id_from_extra_data(extra_data: &[u8]) -> Option<u64> {
    if extra_data.len() < SHASTA_EXTRA_DATA_LEN {
        return None;
    }
    let mut proposal_bytes = [0_u8; 8];
    proposal_bytes[2..8].copy_from_slice(&extra_data[1..SHASTA_EXTRA_DATA_LEN]);
    Some(u64::from_be_bytes(proposal_bytes))
}

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
#[derive(Debug, Clone, Default)]
pub struct ShastaEventData {
    pub proposal: Proposal,
}

mod proposal_bincode_compat {
    use super::{BlobSlice, DerivationSource, Proposal};
    use alloy_primitives::{Address, B256, Uint};
    use serde::{Deserialize, Serialize};

    type U48 = Uint<48, 1>;
    type U24 = Uint<24, 1>;

    #[derive(Serialize, Deserialize)]
    #[allow(non_snake_case)]
    struct BlobSliceBin {
        blobHashes: Vec<B256>,
        offset: u32,
        timestamp: u64,
    }

    #[derive(Serialize, Deserialize)]
    #[allow(non_snake_case)]
    struct DerivationSourceBin {
        isForcedInclusion: bool,
        blobSlice: BlobSliceBin,
    }

    #[derive(Serialize, Deserialize)]
    #[allow(non_snake_case)]
    pub(super) struct ProposalBin {
        id: u64,
        timestamp: u64,
        endOfSubmissionWindowTimestamp: u64,
        proposer: Address,
        parentProposalHash: B256,
        originBlockNumber: u64,
        originBlockHash: B256,
        basefeeSharingPctg: u8,
        sources: Vec<DerivationSourceBin>,
    }

    const fn u48_from_u64(n: u64) -> U48 {
        // Ensure it fits 48 bits.
        U48::from_limbs([n & 0xffff_ffff_ffff])
    }

    const fn u24_from_u32(n: u32) -> U24 {
        U24::from_limbs([(n as u64) & 0x00ff_ffff])
    }

    pub(super) fn to_bin(p: &Proposal) -> ProposalBin {
        ProposalBin {
            id: p.id.to(),
            timestamp: p.timestamp.to(),
            endOfSubmissionWindowTimestamp: p.endOfSubmissionWindowTimestamp.to(),
            proposer: p.proposer,
            parentProposalHash: p.parentProposalHash,
            originBlockNumber: p.originBlockNumber.to(),
            originBlockHash: p.originBlockHash,
            basefeeSharingPctg: p.basefeeSharingPctg,
            sources: p
                .sources
                .iter()
                .map(|src| DerivationSourceBin {
                    isForcedInclusion: src.isForcedInclusion,
                    blobSlice: BlobSliceBin {
                        blobHashes: src.blobSlice.blobHashes.clone(),
                        offset: src.blobSlice.offset.to(),
                        timestamp: src.blobSlice.timestamp.to(),
                    },
                })
                .collect(),
        }
    }

    pub(super) fn from_bin(bin: ProposalBin) -> Proposal {
        Proposal {
            id: u48_from_u64(bin.id),
            timestamp: u48_from_u64(bin.timestamp),
            endOfSubmissionWindowTimestamp: u48_from_u64(bin.endOfSubmissionWindowTimestamp),
            proposer: bin.proposer,
            parentProposalHash: bin.parentProposalHash,
            originBlockNumber: u48_from_u64(bin.originBlockNumber),
            originBlockHash: bin.originBlockHash,
            basefeeSharingPctg: bin.basefeeSharingPctg,
            sources: bin
                .sources
                .into_iter()
                .map(|src| DerivationSource {
                    isForcedInclusion: src.isForcedInclusion,
                    blobSlice: BlobSlice {
                        blobHashes: src.blobSlice.blobHashes,
                        offset: u24_from_u32(src.blobSlice.offset),
                        timestamp: u48_from_u64(src.blobSlice.timestamp),
                    },
                })
                .collect(),
        }
    }
}

impl ShastaEventData {
    /// Decode a Shasta Proposed event into `ShastaEventData`.
    ///
    /// # Errors
    ///
    /// Returns an error if the event cannot be decoded.
    pub fn from_proposal_event(
        proposal: &Proposed,
    ) -> std::result::Result<Self, alloy_sol_types::Error> {
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

impl Serialize for ShastaEventData {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Keep JSON schema stable (proposal-only, like `raiko`).
        // For bincode, use an explicit bincode-safe encoding for the nested `Proposal`.
        if serializer.is_human_readable() {
            #[derive(Serialize)]
            struct Hr<'a> {
                proposal: &'a Proposal,
            }
            Hr {
                proposal: &self.proposal,
            }
            .serialize(serializer)
        } else {
            #[derive(Serialize)]
            struct Bin {
                proposal: proposal_bincode_compat::ProposalBin,
            }
            Bin {
                proposal: proposal_bincode_compat::to_bin(&self.proposal),
            }
            .serialize(serializer)
        }
    }
}

impl<'de> Deserialize<'de> for ShastaEventData {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            // Accept both the old JSON shape:
            //   { proposal: {..}, derivation: {..} }
            // and the new shape:
            //   { proposal: {..all fields..} }
            #[derive(Deserialize)]
            struct Old {
                proposal: ProposalLegacy,
                derivation: Derivation,
            }

            #[derive(Deserialize)]
            struct New {
                proposal: Proposal,
            }

            #[derive(Deserialize)]
            #[serde(untagged)]
            enum Hr {
                Old(Old),
                New(New),
            }

            // Legacy proposal JSON shape (without derivation fields embedded).
            #[derive(Deserialize)]
            #[allow(non_snake_case)]
            struct ProposalLegacy {
                id: alloy_primitives::Uint<48, 1>,
                timestamp: alloy_primitives::Uint<48, 1>,
                endOfSubmissionWindowTimestamp: alloy_primitives::Uint<48, 1>,
                proposer: Address,
                parentProposalHash: B256,
            }

            let v = Hr::deserialize(deserializer)?;
            match v {
                Hr::New(New { proposal }) => Ok(Self { proposal }),
                Hr::Old(Old {
                    proposal: p,
                    derivation: d,
                }) => Ok(Self {
                    proposal: Proposal {
                        id: p.id,
                        timestamp: p.timestamp,
                        endOfSubmissionWindowTimestamp: p.endOfSubmissionWindowTimestamp,
                        proposer: p.proposer,
                        parentProposalHash: p.parentProposalHash,
                        originBlockNumber: d.originBlockNumber,
                        originBlockHash: d.originBlockHash,
                        basefeeSharingPctg: d.basefeeSharingPctg,
                        sources: d.sources,
                    },
                }),
            }
        } else {
            #[derive(Deserialize)]
            struct Bin {
                proposal: proposal_bincode_compat::ProposalBin,
            }
            let bin = Bin::deserialize(deserializer)?;
            Ok(Self {
                proposal: proposal_bincode_compat::from_bin(bin.proposal),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Proposed, ShastaEventData, decode_proposal_id_from_extra_data};

    #[test]
    fn shasta_event_data_from_proposed() -> Result<(), Box<dyn std::error::Error>> {
        let proposed = Proposed::default();
        let event = ShastaEventData::from_proposal_event(&proposed)?;
        assert_eq!(event.proposal.id, proposed.id);
        Ok(())
    }

    #[test]
    fn decodes_proposal_id_from_extra_data() {
        let proposal_id = 2_670_u64;
        let proposal_bytes = proposal_id.to_be_bytes();
        let extra_data = [
            0,
            proposal_bytes[2],
            proposal_bytes[3],
            proposal_bytes[4],
            proposal_bytes[5],
            proposal_bytes[6],
            proposal_bytes[7],
        ];

        assert_eq!(
            decode_proposal_id_from_extra_data(&extra_data),
            Some(proposal_id)
        );
    }

    #[test]
    fn returns_none_for_short_extra_data() {
        assert_eq!(decode_proposal_id_from_extra_data(&[1, 2, 3]), None);
    }
}
