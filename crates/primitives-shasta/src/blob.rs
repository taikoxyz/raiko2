//! Shasta-specific blob verification helpers.

use raiko2_primitives::blob::util::{
    KzgCommitmentBytes, commitment_to_version_hash, verify_blob_proof_of_equivalence,
};
use raiko2_primitives::{RaikoError, RaikoResult};

use crate::GuestInput;

fn read_kzg_bytes(value: &[u8], label: &str, idx: usize) -> RaikoResult<KzgCommitmentBytes> {
    if value.len() != 48 {
        return Err(RaikoError::InvalidBlobOption(format!(
            "{label} at index {idx} has invalid length (expected 48 bytes, got {})",
            value.len()
        )));
    }

    let mut bytes: KzgCommitmentBytes = [0u8; 48];
    bytes.copy_from_slice(value);
    Ok(bytes)
}

/// Verify blob usage in proposal mode.
///
/// Iterates through each data source and verifies each blob with its commitment and proof.
///
/// # Errors
///
/// Returns an error if blob counts mismatch, KZG inputs are invalid, or blob verification fails.
pub fn verify_proposal_mode_blob_usage(guest_input: &GuestInput) -> RaikoResult<()> {
    if !guest_input.taiko.proposal_event.proposal.sources.is_empty()
        && guest_input.taiko.data_sources.len()
            != guest_input.taiko.proposal_event.proposal.sources.len()
    {
        return Err(RaikoError::InvalidBlobOption(format!(
            "data source count ({}) does not match proposal source count ({})",
            guest_input.taiko.data_sources.len(),
            guest_input.taiko.proposal_event.proposal.sources.len()
        )));
    }

    for (source_idx, data_source) in guest_input.taiko.data_sources.iter().enumerate() {
        if data_source.tx_data_from_blob.is_empty() {
            continue;
        }

        let expected_blob_hashes = guest_input
            .taiko
            .proposal_event
            .proposal
            .sources
            .get(source_idx)
            .ok_or_else(|| {
                RaikoError::InvalidBlobOption(format!(
                    "missing proposal source for data source index {source_idx}"
                ))
            })?
            .blobSlice
            .blobHashes
            .as_slice();
        let commitments = &data_source.blob_commitments;
        let proofs = &data_source.blob_proofs;

        if data_source.tx_data_from_blob.len() != commitments.len() {
            return Err(RaikoError::InvalidBlobOption(format!(
                "blob count ({}) does not match commitment count ({})",
                data_source.tx_data_from_blob.len(),
                commitments.len()
            )));
        }
        if commitments.len() != proofs.len() {
            return Err(RaikoError::InvalidBlobOption(format!(
                "commitment count ({}) does not match proof count ({})",
                commitments.len(),
                proofs.len()
            )));
        }
        if expected_blob_hashes.len() != commitments.len() {
            return Err(RaikoError::InvalidBlobOption(format!(
                "expected blob hash count ({}) does not match commitment count ({}) for source {}",
                expected_blob_hashes.len(),
                commitments.len(),
                source_idx
            )));
        }

        for (idx, blob_data) in data_source.tx_data_from_blob.iter().enumerate() {
            let commitment_array = read_kzg_bytes(&commitments[idx], "Commitment", idx)?;

            let proof_bytes = proofs.get(idx).ok_or_else(|| {
                RaikoError::InvalidBlobOption(format!(
                    "missing proof for source {source_idx}, index {idx}"
                ))
            })?;
            let proof_array = read_kzg_bytes(proof_bytes, "Proof", idx)?;
            verify_blob_proof_of_equivalence(blob_data, &commitment_array, &proof_array).map_err(
                |e| {
                    RaikoError::InvalidBlobOption(format!(
                        "Blob proof-of-equivalence verification failed at index {idx}: {e}"
                    ))
                },
            )?;
            let versioned_hash = commitment_to_version_hash(&commitment_array);
            if versioned_hash != expected_blob_hashes[idx] {
                return Err(RaikoError::InvalidBlobOption(format!(
                    "blob versioned hash mismatch at source {}, index {}: expected {:?}, got {:?}",
                    source_idx, idx, expected_blob_hashes[idx], versioned_hash
                )));
            }
        }
    }

    Ok(())
}
