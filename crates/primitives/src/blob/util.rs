use crate::{RaikoError, RaikoResult};
use alloy_primitives::B256;
use kzg_rs::kzg_proof::{
    evaluate_polynomial_in_evaluation_form, safe_g1_affine_from_bytes, scalar_from_bytes_unchecked,
    verify_kzg_proof_impl,
};
use kzg_rs::{Blob as KzgBlob, Bytes48, KzgProof, KzgSettings};
use sha2::{Digest, Sha256};

#[cfg(feature = "kzg-host")]
use c_kzg::{Blob as CkzgBlob, Bytes32 as CkzgBytes32, Bytes48 as CkzgBytes48};

pub const VERSIONED_HASH_VERSION_KZG: u8 = 0x01;

pub type KzgField = [u8; 32];
pub type KzgCommitmentBytes = [u8; 48];

fn invalid_blob_option(context: &str, err: impl core::fmt::Display) -> RaikoError {
    RaikoError::InvalidBlobOption(format!("{context}: {err}"))
}

fn sha256(data: &[u8]) -> KzgField {
    Sha256::digest(data).into()
}

fn proof_of_equivalence_point_bytes(blob: &[u8], commitment: &KzgCommitmentBytes) -> KzgField {
    let blob_hash = sha256(blob);
    let versioned_hash = commitment_to_version_hash(commitment);
    let mut challenge_input = [0u8; 64];
    challenge_input[..32].copy_from_slice(&blob_hash);
    challenge_input[32..].copy_from_slice(versioned_hash.as_slice());
    sha256(&challenge_input)
}

#[cfg(feature = "kzg-host")]
fn proof_of_equivalence_point_canonical_bytes(
    blob: &[u8],
    commitment: &KzgCommitmentBytes,
) -> KzgField {
    let mut bytes =
        scalar_from_bytes_unchecked(proof_of_equivalence_point_bytes(blob, commitment)).to_bytes();
    bytes.reverse();
    bytes
}

fn kzg_blob(blob: &[u8]) -> RaikoResult<KzgBlob> {
    KzgBlob::from_slice(blob).map_err(|err| invalid_blob_option("Failed to convert blob", err))
}

fn kzg_bytes48(bytes: &KzgCommitmentBytes, label: &str) -> RaikoResult<Bytes48> {
    Bytes48::from_slice(bytes).map_err(|err| invalid_blob_option(&format!("Invalid {label}"), err))
}

fn proof_of_equivalence_kzg_settings() -> KzgSettings {
    KzgSettings {
        roots_of_unity: kzg_rs::get_roots_of_unity(),
        g1_points: &[],
        g2_points: kzg_rs::get_g2_verification_points(),
    }
}

#[cfg(feature = "kzg-host")]
fn ckzg_blob(blob: &[u8]) -> RaikoResult<CkzgBlob> {
    CkzgBlob::from_bytes(blob).map_err(|err| invalid_blob_option("Failed to convert blob", err))
}

#[cfg(feature = "kzg-host")]
fn ckzg_bytes48(bytes: &KzgCommitmentBytes, label: &str) -> RaikoResult<CkzgBytes48> {
    CkzgBytes48::from_bytes(bytes)
        .map_err(|err| invalid_blob_option(&format!("Invalid {label}"), err))
}

/// Convert blob bytes to a KZG commitment.
///
/// # Errors
///
/// Returns an error if the `kzg-host` feature is disabled, the blob is malformed,
/// or the commitment computation fails.
#[cfg(feature = "kzg-host")]
pub fn blob_to_commitment(blob: &[u8]) -> RaikoResult<KzgCommitmentBytes> {
    let blob = ckzg_blob(blob)?;
    let commitment = c_kzg::ethereum_kzg_settings(0)
        .blob_to_kzg_commitment(&blob)
        .map_err(|err| invalid_blob_option("Failed to compute commitment", err))?;
    Ok(*commitment.to_bytes().as_ref())
}

/// Convert blob bytes to a KZG commitment.
///
/// # Errors
///
/// Always returns an error when host-side KZG generation is not enabled.
#[cfg(not(feature = "kzg-host"))]
pub fn blob_to_commitment(_blob: &[u8]) -> RaikoResult<KzgCommitmentBytes> {
    Err(RaikoError::InvalidBlobOption(
        "blob commitment generation requires the kzg-host feature".to_string(),
    ))
}

/// Compute a standard EIP-4844 blob KZG proof.
///
/// # Errors
///
/// Returns an error if the `kzg-host` feature is disabled, the blob or commitment
/// is malformed, or proof generation fails.
#[cfg(feature = "kzg-host")]
pub fn blob_to_proof(
    blob: &[u8],
    commitment: &KzgCommitmentBytes,
) -> RaikoResult<KzgCommitmentBytes> {
    let blob = ckzg_blob(blob)?;
    let commitment = ckzg_bytes48(commitment, "commitment")?;
    let proof = c_kzg::ethereum_kzg_settings(0)
        .compute_blob_kzg_proof(&blob, &commitment)
        .map_err(|err| invalid_blob_option("Failed to compute proof", err))?;
    Ok(*proof.to_bytes().as_ref())
}

/// Compute a standard EIP-4844 blob KZG proof.
///
/// # Errors
///
/// Always returns an error when host-side KZG generation is not enabled.
#[cfg(not(feature = "kzg-host"))]
pub fn blob_to_proof(
    _blob: &[u8],
    _commitment: &KzgCommitmentBytes,
) -> RaikoResult<KzgCommitmentBytes> {
    Err(RaikoError::InvalidBlobOption(
        "blob proof generation requires the kzg-host feature".to_string(),
    ))
}

/// Compute the point-evaluation proof used by Raiko's proof-of-equivalence strategy.
///
/// # Errors
///
/// Returns an error if the `kzg-host` feature is disabled, the blob or commitment
/// is malformed, or proof generation fails.
#[cfg(feature = "kzg-host")]
pub fn blob_to_proof_of_equivalence(
    blob: &[u8],
    commitment: &KzgCommitmentBytes,
) -> RaikoResult<KzgCommitmentBytes> {
    let blob = ckzg_blob(blob)?;
    let z = CkzgBytes32::from(proof_of_equivalence_point_canonical_bytes(
        blob.as_ref(),
        commitment,
    ));
    let (proof, _) = c_kzg::ethereum_kzg_settings(0)
        .compute_kzg_proof(&blob, &z)
        .map_err(|err| invalid_blob_option("Failed to compute proof of equivalence", err))?;
    Ok(*proof.to_bytes().as_ref())
}

/// Compute the point-evaluation proof used by Raiko's proof-of-equivalence strategy.
///
/// # Errors
///
/// Always returns an error when host-side KZG generation is not enabled.
#[cfg(not(feature = "kzg-host"))]
pub fn blob_to_proof_of_equivalence(
    _blob: &[u8],
    _commitment: &KzgCommitmentBytes,
) -> RaikoResult<KzgCommitmentBytes> {
    Err(RaikoError::InvalidBlobOption(
        "proof-of-equivalence generation requires the kzg-host feature".to_string(),
    ))
}

/// Convert KZG commitment to versioned hash (EIP-4844).
#[must_use]
pub fn commitment_to_version_hash(commitment: &KzgCommitmentBytes) -> B256 {
    let mut out = sha256(commitment);
    out[0] = VERSIONED_HASH_VERSION_KZG;
    B256::from_slice(&out)
}

/// Verify a standard EIP-4844 blob KZG proof.
///
/// # Errors
///
/// Returns an error if the blob, commitment, or proof is malformed, or if
/// verification fails.
pub fn verify_blob_kzg_proof(
    blob: &[u8],
    commitment: &KzgCommitmentBytes,
    proof: &KzgCommitmentBytes,
) -> RaikoResult<()> {
    let blob = kzg_blob(blob)?;
    let commitment = kzg_bytes48(commitment, "commitment")?;
    let proof = kzg_bytes48(proof, "proof")?;
    let settings = kzg_rs::get_kzg_settings();
    let is_valid = KzgProof::verify_blob_kzg_proof(blob, &commitment, &proof, &settings)
        .map_err(|err| invalid_blob_option("KZG proof verification error", err))?;
    if !is_valid {
        return Err(RaikoError::InvalidBlobOption(
            "KZG proof verification failed".to_string(),
        ));
    }
    Ok(())
}

/// Verify the point-evaluation proof used by Raiko's proof-of-equivalence strategy.
///
/// # Errors
///
/// Returns an error if the blob, commitment, or proof is malformed, or if
/// verification fails.
pub fn verify_blob_proof_of_equivalence(
    blob: &[u8],
    commitment: &KzgCommitmentBytes,
    proof: &KzgCommitmentBytes,
) -> RaikoResult<()> {
    let commitment_array = *commitment;
    let kzg_blob = kzg_blob(blob)?;
    let commitment_bytes = kzg_bytes48(&commitment_array, "commitment")?;
    let proof_bytes = kzg_bytes48(proof, "proof")?;
    let commitment = safe_g1_affine_from_bytes(&commitment_bytes)
        .map_err(|err| invalid_blob_option("Invalid commitment", err))?;
    let proof = safe_g1_affine_from_bytes(&proof_bytes)
        .map_err(|err| invalid_blob_option("Invalid proof", err))?;

    let settings = proof_of_equivalence_kzg_settings();
    let evaluation_point =
        scalar_from_bytes_unchecked(proof_of_equivalence_point_bytes(blob, &commitment_array));
    let polynomial = kzg_blob
        .as_polynomial()
        .map_err(|err| invalid_blob_option("Failed to build polynomial", err))?;
    let evaluation_value =
        evaluate_polynomial_in_evaluation_form(polynomial, evaluation_point, &settings)
            .map_err(|err| invalid_blob_option("Failed to evaluate polynomial", err))?;

    let is_valid = verify_kzg_proof_impl(
        commitment,
        evaluation_point,
        evaluation_value,
        proof,
        &settings,
    )
    .map_err(|err| invalid_blob_option("Proof of equivalence verification error", err))?;
    if !is_valid {
        return Err(RaikoError::InvalidBlobOption(
            "Proof of equivalence verification failed".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    #[cfg(feature = "kzg-host")]
    use anyhow::{Context, Result};
    #[cfg(feature = "kzg-host")]
    use kzg_rs::BYTES_PER_BLOB;

    #[test]
    fn commitment_version_hash_sets_kzg_prefix() {
        let commitment = [7u8; 48];
        let version_hash = commitment_to_version_hash(&commitment);
        let mut expected = sha256(&commitment);
        expected[0] = VERSIONED_HASH_VERSION_KZG;
        assert_eq!(version_hash, B256::from_slice(&expected));
    }

    #[test]
    fn proof_of_equivalence_settings_skip_unused_setup_points() {
        let settings = proof_of_equivalence_kzg_settings();

        assert!(settings.g1_points.is_empty());
        assert_eq!(settings.roots_of_unity.len(), kzg_rs::NUM_ROOTS_OF_UNITY);
        assert_eq!(settings.g2_points.len(), 2);
        assert_eq!(settings.g2_points, &kzg_rs::get_g2_points()[..2]);
    }

    #[cfg(feature = "kzg-host")]
    #[test]
    fn blob_commitment_and_proof_verify() -> Result<()> {
        let blob_bytes = vec![0u8; BYTES_PER_BLOB];
        let commitment = blob_to_commitment(&blob_bytes).context("commitment")?;
        let proof = blob_to_proof(&blob_bytes, &commitment).context("proof")?;
        verify_blob_kzg_proof(&blob_bytes, &commitment, &proof).context("verify proof")?;
        Ok(())
    }

    #[cfg(feature = "kzg-host")]
    #[test]
    fn blob_proof_of_equivalence_verify() -> Result<()> {
        let blob_bytes = vec![0u8; BYTES_PER_BLOB];
        let commitment = blob_to_commitment(&blob_bytes).context("commitment")?;
        let proof = blob_to_proof_of_equivalence(&blob_bytes, &commitment)
            .context("proof of equivalence")?;
        verify_blob_proof_of_equivalence(&blob_bytes, &commitment, &proof)
            .context("verify proof of equivalence")?;
        Ok(())
    }

    #[cfg(feature = "kzg-host")]
    #[test]
    fn blob_proof_of_equivalence_handles_non_canonical_challenge() -> Result<()> {
        let (blob_bytes, commitment) = (0u16..=u16::from(u8::MAX))
            .find_map(|seed| {
                let seed = u8::try_from(seed).expect("seed is within u8 range");
                let blob_bytes = vec![seed; BYTES_PER_BLOB];
                let commitment = blob_to_commitment(&blob_bytes).ok()?;
                let raw = proof_of_equivalence_point_bytes(&blob_bytes, &commitment);
                let canonical =
                    proof_of_equivalence_point_canonical_bytes(&blob_bytes, &commitment);
                (raw != canonical).then_some((blob_bytes, commitment))
            })
            .context("missing non-canonical proof-of-equivalence challenge")?;

        let proof = blob_to_proof_of_equivalence(&blob_bytes, &commitment)
            .context("proof of equivalence")?;
        verify_blob_proof_of_equivalence(&blob_bytes, &commitment, &proof)
            .context("verify proof of equivalence")?;
        Ok(())
    }

    #[cfg(feature = "kzg-host")]
    #[test]
    fn blob_to_proof_computes_verifiable_proof_for_mainnet_blob() -> Result<()> {
        use alloy_primitives::hex;
        use std::fs;

        let blob_file = concat!(env!("CARGO_MANIFEST_DIR"), "/blob_13326465_0.bin");
        let blob_bytes = fs::read(blob_file).context("Failed to read blob file")?;

        let expected_commitment = "0xb8df58142f4397d25bf26f670fef31622428dbe4f22ad6e8c5386458ef28c698841904258320d98befd52b26edf1a26d";
        let commitment_str = expected_commitment
            .strip_prefix("0x")
            .unwrap_or(expected_commitment);
        let commitment_bytes: [u8; 48] = hex::decode(commitment_str)
            .context("Failed to decode commitment hex")?
            .try_into()
            .map_err(|_| anyhow::anyhow!("Commitment must be 48 bytes"))?;

        let computed_commitment =
            blob_to_commitment(&blob_bytes).context("Failed to compute commitment")?;
        assert_eq!(
            computed_commitment, commitment_bytes,
            "Calculated commitment does not match expected commitment"
        );

        let proof =
            blob_to_proof(&blob_bytes, &commitment_bytes).context("Failed to compute proof")?;
        verify_blob_kzg_proof(&blob_bytes, &commitment_bytes, &proof)
            .context("Failed to verify proof")?;

        Ok(())
    }
}
