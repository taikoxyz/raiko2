//! Byte equality for the decompressed Etna execution payload.

use alloy_rlp::Header;

use crate::projection::ExecutionProjection;

const MAX_DECOMPRESSED_BYTES: usize = 64 * 1024 * 1024;

/// The decompressed blob payload failed the Etna execution-list checks.
#[derive(Debug, thiserror::Error)]
pub enum PayloadError {
    /// A landing payload must cover at least one execution item.
    #[error("empty Etna execution range")]
    EmptyRange,
    /// The decompressed bytes or canonical execution list exceed the byte limit.
    #[error("Etna execution list exceeds the decompressed byte limit")]
    DecompressedSize,
    /// The decompressed bytes differ from the canonical projected execution list.
    #[error("Etna payload does not match the projected execution items")]
    PayloadMismatch,
}

/// Checks the exact canonical RLP list of original projected execution items.
///
/// The caller establishes that `projected` is the complete authenticated range
/// in execution order. The caller also authenticates the ordered blobs using
/// KZG, checks their packing and the versioned compressed framing, and
/// enforces the output limit during inflation. Pass the complete output of
/// one strictly verified zlib stream as `decoded`.
///
/// # Errors
///
/// Returns an error if the range is empty, the input or expected encoding
/// exceeds 64 MiB including the list prefix, or the bytes differ from the
/// projected execution list.
pub fn verify_decompressed_execution_payload(
    decoded: &[u8],
    projected: &[ExecutionProjection<'_>],
) -> Result<(), PayloadError> {
    if projected.is_empty() {
        return Err(PayloadError::EmptyRange);
    }
    if decoded.len() > MAX_DECOMPRESSED_BYTES {
        return Err(PayloadError::DecompressedSize);
    }

    let payload_length = projected
        .iter()
        .try_fold(0usize, |length, projection| {
            length
                .checked_add(projection.execution_item_bytes().len())
                .filter(|&length| length <= MAX_DECOMPRESSED_BYTES)
        })
        .ok_or(PayloadError::DecompressedSize)?;
    let header = Header {
        list: true,
        payload_length,
    };
    let prefix_length = header.length();
    let expected_length = payload_length
        .checked_add(prefix_length)
        .filter(|&length| length <= MAX_DECOMPRESSED_BYTES)
        .ok_or(PayloadError::DecompressedSize)?;
    if decoded.len() != expected_length {
        return Err(PayloadError::PayloadMismatch);
    }

    let mut prefix = [0u8; 1 + core::mem::size_of::<usize>()];
    header.encode(&mut prefix.as_mut_slice());
    let mut items = decoded
        .strip_prefix(&prefix[..prefix_length])
        .ok_or(PayloadError::PayloadMismatch)?;
    for projection in projected {
        items = items
            .strip_prefix(projection.execution_item_bytes())
            .ok_or(PayloadError::PayloadMismatch)?;
    }
    Ok(())
}
