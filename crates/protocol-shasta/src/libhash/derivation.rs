use crate::shasta::{BlobSlice, Derivation, DerivationSource};
use alloy_primitives::{B256, U256, keccak256};

use super::encode::EMPTY_BYTES_HASH;
use super::values::{hash_three_values, hash_two_values};

/// Hash a derivation source (isForcedInclusion flag + blobSlice)
#[must_use]
pub fn hash_derivation_source(source: &DerivationSource) -> B256 {
    hash_two_values(
        if source.isForcedInclusion {
            B256::from([1u8; 32])
        } else {
            B256::from([0u8; 32])
        },
        hash_blob_slice(&source.blobSlice),
    )
}

/// Hash a blob slice using the same logic as the Solidity implementation
fn hash_blob_slice(blob_slice: &BlobSlice) -> B256 {
    // Hash the blob hashes array first
    let blob_hashes = &blob_slice.blobHashes;
    let blob_hashes_len = blob_hashes.len();
    let blob_hashes_hash = match blob_hashes_len {
        0 => EMPTY_BYTES_HASH,
        1 => hash_two_values(U256::from(blob_hashes_len).into(), blob_hashes[0]),
        2 => hash_three_values(
            U256::from(blob_hashes_len).into(),
            blob_hashes[0],
            blob_hashes[1],
        ),
        _ => {
            // For larger arrays, use memory-optimized approach
            let buffer_size = 32 + (blob_hashes_len * 32);
            let mut buffer = Vec::with_capacity(buffer_size);

            // Write array length at start of buffer
            buffer.extend_from_slice(&U256::from(blob_hashes_len).to_be_bytes::<32>());

            // Write each blob hash directly to buffer
            for blob_hash in blob_hashes {
                buffer.extend_from_slice(blob_hash.as_slice());
            }

            keccak256(&buffer)
        }
    };

    // Hash the three values: blob_hashes_hash, offset, timestamp
    hash_three_values(
        blob_hashes_hash,
        U256::from(blob_slice.offset).into(),
        U256::from(blob_slice.timestamp).into(),
    )
}

#[must_use]
pub fn hash_derivation(derivation: &Derivation) -> B256 {
    let sources_length = derivation.sources.len();

    // Calculate total words needed for the buffer
    // Base words: 6 (offset to tuple head, originBlockNumber, originBlockHash, basefeeSharingPctg, offset to sources, sources length)
    let mut total_words = 6 + sources_length;

    // Each source contributes: element head (2) + blobSlice head (3) + blobHashes length (1) + blobHashes entries
    for source in &derivation.sources {
        total_words += 6 + source.blobSlice.blobHashes.len();
    }

    // Allocate buffer: each word is 32 bytes (B256), initialize with zeros
    let mut buffer = vec![0u8; total_words * 32];

    // Helper function to write a word at a specific index
    let write_word = |buf: &mut [u8], index: usize, value: B256| {
        let pos = index * 32;
        buf[pos..pos + 32].copy_from_slice(value.as_slice());
    };

    // Set base words
    // [0] offset to tuple head (0x20)
    write_word(&mut buffer, 0, U256::from(0x20u64).into());
    // [1] originBlockNumber
    write_word(
        &mut buffer,
        1,
        U256::from(derivation.originBlockNumber).into(),
    );
    // [2] originBlockHash
    write_word(&mut buffer, 2, derivation.originBlockHash);
    // [3] basefeeSharingPctg
    write_word(
        &mut buffer,
        3,
        U256::from(derivation.basefeeSharingPctg).into(),
    );
    // [4] offset to sources (0x80)
    write_word(&mut buffer, 4, U256::from(0x80u64).into());
    // [5] sources length
    write_word(&mut buffer, 5, U256::from(sources_length).into());

    let offsets_base = 6;
    let mut data_cursor = offsets_base + sources_length;

    // Process each source
    for (i, source) in derivation.sources.iter().enumerate() {
        // Set offset for this source: (dataCursor - offsetsBase) << 5
        let offset = ((data_cursor - offsets_base) << 5) as u64;
        let offset_index = offsets_base + i;
        write_word(&mut buffer, offset_index, U256::from(offset).into());

        // DerivationSource head
        // [dataCursor] isForcedInclusion (1 or 0)
        let is_forced_inclusion_value = u64::from(source.isForcedInclusion);
        write_word(
            &mut buffer,
            data_cursor,
            U256::from(is_forced_inclusion_value).into(),
        );
        // [dataCursor + 1] offset to blobSlice (0x40)
        write_word(&mut buffer, data_cursor + 1, U256::from(0x40u64).into());

        // BlobSlice head
        let blob_slice_base = data_cursor + 2;
        // [blobSliceBase] offset to blobHashes (0x60)
        write_word(&mut buffer, blob_slice_base, U256::from(0x60u64).into());
        // [blobSliceBase + 1] offset
        write_word(
            &mut buffer,
            blob_slice_base + 1,
            U256::from(source.blobSlice.offset).into(),
        );
        // [blobSliceBase + 2] timestamp
        write_word(
            &mut buffer,
            blob_slice_base + 2,
            U256::from(source.blobSlice.timestamp).into(),
        );

        // Blob hashes array
        let blob_hashes_base = blob_slice_base + 3;
        let blob_hashes_length = source.blobSlice.blobHashes.len();
        // [blobHashesBase] blobHashes length
        write_word(
            &mut buffer,
            blob_hashes_base,
            U256::from(blob_hashes_length).into(),
        );

        // [blobHashesBase + 1 + j] each blobHash
        for (j, blob_hash) in source.blobSlice.blobHashes.iter().enumerate() {
            write_word(&mut buffer, blob_hashes_base + 1 + j, *blob_hash);
        }

        data_cursor = blob_hashes_base + 1 + blob_hashes_length;
    }

    // Hash the entire buffer
    keccak256(&buffer)
}
