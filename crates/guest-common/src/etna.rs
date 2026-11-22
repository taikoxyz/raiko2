//! Etna blob payload derivation.

use alloy_eips::eip4844::{Blob, BYTES_PER_BLOB};
use anyhow::{ensure, Context, Result};
use flate2::{Decompress, FlushDecompress, Status};
use raiko2_protocol_etna::{
    payload::verify_decompressed_execution_payload, projection::ExecutionProjection,
};
use taiko_client_protocol::shasta::BlobCoder;

const MAX_BLOBS: usize = 6;
const PAYLOAD_BYTES_PER_BLOB: usize = BYTES_PER_BLOB * 127 / 128 - 4;
const PAYLOAD_VERSION: u8 = 0x06;
const MAX_DECOMPRESSED_BYTES: usize = 64 * 1024 * 1024;
const INFLATE_CHUNK_BYTES: usize = 8 * 1024;

fn decode_blob_payload(blobs: &[Vec<u8>]) -> Result<Vec<u8>> {
    ensure!(
        (1..=MAX_BLOBS).contains(&blobs.len()),
        "Etna payload requires between 1 and {MAX_BLOBS} blobs"
    );

    let mut payload = Vec::with_capacity(blobs.len() * PAYLOAD_BYTES_PER_BLOB);
    for (index, raw) in blobs.iter().enumerate() {
        let blob: &Blob = raw.as_slice().try_into().with_context(|| {
            format!("Etna blob {index} has invalid length, expected {BYTES_PER_BLOB} bytes")
        })?;
        // BlobCoder checks the reserved bits after the first field element.
        ensure!(
            blob[0] & 0xc0 == 0,
            "Etna blob {index} has invalid first field element"
        );
        let chunk = BlobCoder::decode_blob(blob)
            .with_context(|| format!("invalid Etna blob encoding at index {index}"))?;
        ensure!(!chunk.is_empty(), "Etna blob {index} has an empty chunk");
        ensure!(
            index + 1 == blobs.len() || chunk.len() == PAYLOAD_BYTES_PER_BLOB,
            "nonfinal Etna blob {index} is not full"
        );
        payload.extend_from_slice(&chunk);
    }
    Ok(payload)
}

fn decode_execution_payload(bytes: &[u8]) -> Result<Vec<u8>> {
    let (&version, compressed) = bytes
        .split_first()
        .context("missing Etna payload version")?;
    ensure!(
        version == PAYLOAD_VERSION,
        "unsupported Etna payload version {version}"
    );

    let mut decoder = Decompress::new(true);
    let mut output = Vec::new();
    let mut chunk = [0; INFLATE_CHUNK_BYTES];
    let mut input_offset = 0;
    loop {
        let output_before = decoder.total_out();
        let status = decoder
            .decompress(
                &compressed[input_offset..],
                &mut chunk,
                FlushDecompress::None,
            )
            .context("invalid Etna zlib stream")?;
        let consumed =
            usize::try_from(decoder.total_in()).context("Etna zlib input offset overflow")?;
        let produced = usize::try_from(decoder.total_out() - output_before)
            .context("Etna zlib output size overflow")?;
        ensure!(
            produced <= MAX_DECOMPRESSED_BYTES - output.len(),
            "Etna payload exceeds the decompressed byte limit"
        );
        output.extend_from_slice(&chunk[..produced]);

        if status == Status::StreamEnd {
            ensure!(
                consumed == compressed.len(),
                "trailing bytes after the Etna zlib stream"
            );
            return Ok(output);
        }
        ensure!(
            consumed != input_offset || produced != 0,
            "incomplete Etna zlib stream"
        );
        input_offset = consumed;
    }
}

/// Verifies that the ordered Etna blobs derive exactly the projected execution list.
///
/// The caller authenticates these same blobs using KZG and binds their ordered
/// versioned hashes to the statement. It establishes the complete projected
/// range and its context, and verifies execution of those matched blocks.
///
/// # Errors
///
/// Returns an error if blob packing, V6 decompression or execution-list
/// equality fails.
pub fn verify_blob_execution_payload(
    blobs: &[Vec<u8>],
    projected: &[ExecutionProjection<'_>],
) -> Result<()> {
    let payload = decode_blob_payload(blobs)?;
    let decoded = decode_execution_payload(&payload)?;
    verify_decompressed_execution_payload(&decoded, projected)?;
    Ok(())
}

#[cfg(test)]
mod tests;
