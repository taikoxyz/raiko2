use alloy_primitives::{Address, B256, Uint};
use alloy_sol_types::SolValue;

use super::{
    BlobSlice, Derivation, DerivationSource, Proposal, ProposedEventPayload, ShastaEventData,
};

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
        })
    }

    fn unpack_uint24(data: &[u8], pos: usize) -> Result<(u32, usize), alloy_sol_types::Error> {
        // Ensure we have enough data for a 3-byte value
        if pos + 3 > data.len() {
            return Err(alloy_sol_types::Error::custom(
                "Not enough data to read 3-byte uint24".to_string(),
            ));
        }

        let value = u32::from_be_bytes([0, data[pos], data[pos + 1], data[pos + 2]]);
        // New position is old position + 3 bytes
        let new_pos = pos + 3;
        Ok((value, new_pos))
    }

    /// Unpacks a uint48 value from the data buffer at the given position
    /// Matches the Solidity mload behavior by reading a full 32-byte word and extracting 6 bytes
    fn unpack_uint48(data: &[u8], pos: usize) -> Result<(u64, usize), alloy_sol_types::Error> {
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
    pub(crate) fn decode_event_data(data: &[u8]) -> Result<Self, alloy_sol_types::Error> {
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
        let (parent_proposal_hash, new_ptr) = Self::unpack_hash(data, ptr)?;
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
                    offset: Uint::from(offset),
                    timestamp: Uint::from(blob_timestamp),
                },
            });
        }

        let (derivation_hash, _new_ptr) = Self::unpack_hash(data, ptr)?;

        Ok(Self {
            proposal: Proposal {
                id: Uint::from(proposal_id),
                timestamp: Uint::from(timestamp),
                endOfSubmissionWindowTimestamp: Uint::from(end_of_submission_window_timestamp),
                proposer,
                parentProposalHash: parent_proposal_hash,
                derivationHash: derivation_hash,
            },
            derivation: Derivation {
                originBlockNumber: Uint::from(origin_block_number),
                originBlockHash: origin_block_hash,
                basefeeSharingPctg: basefee_sharing_pctg,
                sources,
            },
        })
    }
}
