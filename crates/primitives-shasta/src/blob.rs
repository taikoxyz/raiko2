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

        if expected_blob_hashes.is_empty() {
            if !data_source.tx_data_from_calldata.is_empty()
                || !data_source.tx_data_from_blob.is_empty()
                || !data_source.blob_commitments.is_empty()
                || !data_source.blob_proofs.is_empty()
            {
                return Err(RaikoError::InvalidBlobOption(format!(
                    "inline payloads are not accepted for ZK proposal source {source_idx}"
                )));
            }
            continue;
        }

        if data_source.tx_data_from_blob.is_empty() {
            return Err(RaikoError::InvalidBlobOption(format!(
                "blob-backed source {source_idx} is missing blob data"
            )));
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

#[cfg(test)]
mod tests {
    use super::verify_proposal_mode_blob_usage;
    use crate::GuestInput;
    use alloy_primitives::{B256, Uint};
    use raiko2_protocol::InputDataSource;
    use raiko2_protocol_shasta::{
        TaikoManifest,
        shasta::{BlobSlice, DerivationSource, ShastaEventData},
    };

    fn guest_input(source: DerivationSource, data_source: InputDataSource) -> GuestInput {
        GuestInput {
            taiko: TaikoManifest {
                proposal_event: ShastaEventData {
                    proposal: raiko2_protocol_shasta::shasta::Proposal {
                        sources: vec![source],
                        ..Default::default()
                    },
                },
                data_sources: vec![data_source],
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn source(blob_hashes: Vec<B256>) -> DerivationSource {
        DerivationSource {
            blobSlice: BlobSlice {
                blobHashes: blob_hashes,
                offset: Uint::ZERO,
                timestamp: Uint::ZERO,
            },
            ..Default::default()
        }
    }

    #[test]
    fn rejects_inline_calldata_payload() {
        let input = guest_input(
            source(Vec::new()),
            InputDataSource {
                tx_data_from_calldata: vec![1, 2, 3],
                ..Default::default()
            },
        );

        let err = verify_proposal_mode_blob_usage(&input).expect_err("inline payload rejected");

        assert!(err.to_string().contains("inline payloads are not accepted"));
    }

    #[test]
    fn rejects_blob_commitments_for_empty_hash_source() {
        let input = guest_input(
            source(Vec::new()),
            InputDataSource {
                blob_commitments: vec![vec![0xCC; 48]],
                ..Default::default()
            },
        );

        let err = verify_proposal_mode_blob_usage(&input)
            .expect_err("blob commitment without an on-chain hash must be rejected");

        assert!(err.to_string().contains("inline payloads are not accepted"));
    }

    #[test]
    fn rejects_blob_proofs_for_empty_hash_source() {
        let input = guest_input(
            source(Vec::new()),
            InputDataSource {
                blob_proofs: vec![vec![0xDD; 48]],
                ..Default::default()
            },
        );

        let err = verify_proposal_mode_blob_usage(&input)
            .expect_err("blob proof without an on-chain hash must be rejected");

        assert!(err.to_string().contains("inline payloads are not accepted"));
    }

    #[test]
    fn allows_empty_default_source() {
        let input = guest_input(source(Vec::new()), InputDataSource::default());

        verify_proposal_mode_blob_usage(&input).expect("empty default source accepted");
    }

    #[test]
    fn rejects_blob_backed_source_without_blob_data() {
        let input = guest_input(
            source(vec![B256::repeat_byte(0x11)]),
            InputDataSource::default(),
        );

        let err = verify_proposal_mode_blob_usage(&input).expect_err("missing blob data rejected");

        assert!(err.to_string().contains("missing blob data"));
    }
}
