use crate::{GuestInput, RaikoError, RaikoResult};

use super::util::{KzgCommitmentBytes, get_kzg_settings, verify_blob_kzg_proof_with_settings};

/// Verify blob usage in proposal mode.
///
/// Iterates through each data source and verifies each blob with its commitment and proof.
/// This function uses a static KZG settings loaded from the embedded trusted setup bytes.
pub fn verify_proposal_mode_blob_usage(guest_input: &GuestInput) -> RaikoResult<()> {
    let kzg_settings = get_kzg_settings()?;

    for data_source in &guest_input.taiko.data_sources {
        if data_source.tx_data_from_blob.is_empty() {
            continue;
        }

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

        for (idx, blob_data) in data_source.tx_data_from_blob.iter().enumerate() {
            if commitments[idx].len() != 48 {
                return Err(RaikoError::InvalidBlobOption(format!(
                    "Commitment at index {} has invalid length (expected 48 bytes, got {})",
                    idx,
                    commitments[idx].len()
                )));
            }
            let mut commitment_array: KzgCommitmentBytes = [0u8; 48];
            commitment_array.copy_from_slice(&commitments[idx]);

            if proofs[idx].len() != 48 {
                return Err(RaikoError::InvalidBlobOption(format!(
                    "Proof at index {} has invalid length (expected 48 bytes, got {})",
                    idx,
                    proofs[idx].len()
                )));
            }
            let mut proof_array: KzgCommitmentBytes = [0u8; 48];
            proof_array.copy_from_slice(&proofs[idx]);

            verify_blob_kzg_proof_with_settings(
                blob_data,
                &commitment_array,
                &proof_array,
                kzg_settings,
            )
            .map_err(|e| {
                RaikoError::InvalidBlobOption(format!(
                    "Blob verification failed at index {}: {}",
                    idx, e
                ))
            })?;
        }
    }

    Ok(())
}
