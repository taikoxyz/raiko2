//! Etna blob payload derivation.

use alloy_consensus::Header;
use alloy_eips::eip4844::{Blob, BYTES_PER_BLOB};
use alloy_primitives::B256;
use anyhow::{ensure, Context, Result};
use flate2::{Decompress, FlushDecompress, Status};
use raiko2_primitives::blob::util::{
    commitment_to_version_hash, verify_blob_proof_of_equivalence, KzgCommitmentBytes,
};
use raiko2_protocol::InputDataSource;
use raiko2_protocol_etna::{
    payload::verify_decompressed_execution_payload,
    projection::ExecutionProjection,
    range::{verify_range_statement_projection, RangeBlock, RangeProjectionContext},
    statement::LandStatement,
};
use reth_ethereum_primitives::TransactionSigned;
use taiko_client_protocol::shasta::BlobCoder;

const MAX_BLOBS: usize = 6;
const PAYLOAD_BYTES_PER_BLOB: usize = BYTES_PER_BLOB * 127 / 128 - 4;
const PAYLOAD_VERSION: u8 = 0x06;
const MAX_DECOMPRESSED_BYTES: usize = 64 * 1024 * 1024;
const INFLATE_CHUNK_BYTES: usize = 8 * 1024;

fn validate_blob_count(count: usize) -> Result<()> {
    ensure!(
        (1..=MAX_BLOBS).contains(&count),
        "Etna payload requires between 1 and {MAX_BLOBS} blobs"
    );
    Ok(())
}

fn decode_blob_payload(blobs: &[Vec<u8>]) -> Result<Vec<u8>> {
    validate_blob_count(blobs.len())?;

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

/// Authenticates each ordered Etna blob against its expected versioned hash.
///
/// The composing verifier obtains `expected_hashes` from the authenticated
/// proposal context and uses the same bytes for Etna payload derivation.
///
/// # Errors
///
/// Returns an error for invalid source shape, an unexpected versioned hash
/// or a malformed or invalid proof of equivalence.
pub fn verify_blob_authentication(
    source: &InputDataSource,
    expected_hashes: &[B256],
) -> Result<()> {
    let count = expected_hashes.len();
    validate_blob_count(count)?;
    ensure!(
        source.tx_data_from_blob.len() == count
            && source.blob_commitments.len() == count
            && source.blob_proofs.len() == count,
        "Etna blob, commitment, proof and expected hash counts differ"
    );
    ensure!(
        source.tx_data_from_calldata.is_empty(),
        "Etna blob source contains inline calldata"
    );
    ensure!(
        !source.is_forced_inclusion,
        "Etna blob source sets the legacy forced-inclusion flag"
    );

    for (index, (blob, expected_hash)) in source
        .tx_data_from_blob
        .iter()
        .zip(expected_hashes)
        .enumerate()
    {
        let commitment: &KzgCommitmentBytes = source.blob_commitments[index]
            .as_slice()
            .try_into()
            .with_context(|| format!("Etna blob {index} commitment must contain 48 bytes"))?;
        let proof: &KzgCommitmentBytes = source.blob_proofs[index]
            .as_slice()
            .try_into()
            .with_context(|| format!("Etna blob {index} proof must contain 48 bytes"))?;
        ensure!(
            commitment_to_version_hash(commitment) == *expected_hash,
            "Etna blob {index} versioned hash mismatch"
        );
        verify_blob_proof_of_equivalence(blob, commitment, proof)
            .with_context(|| format!("Etna blob {index} proof of equivalence failed"))?;
    }
    Ok(())
}

/// Authenticates the ordered source and derives the exact projected execution list.
///
/// The caller obtains `expected_hashes` from the authenticated proposal context
/// and establishes the complete range and its execution validity.
///
/// # Errors
///
/// Returns an error if ordered blob authentication, payload derivation or
/// execution-list equality fails.
pub fn verify_authenticated_blob_execution_payload(
    source: &InputDataSource,
    expected_hashes: &[B256],
    projected: &[ExecutionProjection<'_>],
) -> Result<()> {
    verify_blob_authentication(source, expected_hashes)?;
    verify_blob_execution_payload(&source.tx_data_from_blob, projected)
}

/// Binds the ordered blob payload, projected range and phase-one landing statement.
///
/// The statement supplies the same ordered blob hashes for authentication
/// and digest construction. The caller authenticates the deployment, proof
/// origin and L1 context, verifies anchor state, timing, rules, committee
/// authority and certificates, and checks strict execution of the matched
/// blocks before applying backend proof framing. It must restrict the proof
/// to an authenticated phase-one deployment with FI disabled and no pending
/// queue entries. This function rejects FI witnesses and cursor consumption;
/// it does not authenticate that deployment profile or the queue state.
///
/// # Errors
///
/// Returns an error if FI witnesses or cursor consumption are present, or if
/// range or statement projection, blob authentication, payload derivation or
/// execution-list equality fails.
pub fn verify_blob_range_statement_projection<'a>(
    source: &InputDataSource,
    blocks: &[RangeBlock<'a, TransactionSigned>],
    context: &RangeProjectionContext,
    statement: &LandStatement<'_>,
    post_block_cursors: &[u64],
    parent_execution_header: &Header,
) -> Result<(Vec<ExecutionProjection<'a>>, B256)> {
    validate_blob_count(statement.blob_hashes.len())?;
    let (projected, digest) = verify_range_statement_projection(
        blocks,
        context,
        statement,
        post_block_cursors,
        parent_execution_header,
    )?;
    // The checked cursor sequence is nondecreasing and ends at this terminal
    // cursor, so equal endpoints require every intermediate cursor to stay put.
    ensure!(
        statement.input.forced_inclusion_head == statement.forced_inclusion_head_before,
        "phase-one Etna range consumes forced inclusion entries"
    );
    for (block, projection) in blocks.iter().zip(&projected) {
        ensure!(
            projection.forced_witness().is_none(),
            "phase-one Etna range carries an FI witness at height {}",
            block.execution_header.number
        );
    }
    verify_authenticated_blob_execution_payload(source, statement.blob_hashes, &projected)?;
    Ok((projected, digest))
}

#[cfg(test)]
mod tests;
