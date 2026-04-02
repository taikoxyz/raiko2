use crate::{RaikoError, RaikoResult};
use alloy_primitives::B256;
use kzg::kzg_proofs::KZGSettings;
use kzg::kzg_types::ZFr;
use kzg_traits::G1;
use kzg_traits::eip_4844::{
    blob_to_kzg_commitment_rust, blob_to_polynomial, bytes_to_blob, compute_blob_kzg_proof_rust,
    compute_kzg_proof_rust, evaluate_polynomial_in_evaluation_form, hash, hash_to_bls_field,
    verify_blob_kzg_proof_rust, verify_kzg_proof_rust,
};
use std::sync::OnceLock;

pub const VERSIONED_HASH_VERSION_KZG: u8 = 0x01;

/// Static KZG settings loaded from the default trusted setup.
/// This is initialized lazily on first access.
static KZG_SETTINGS: OnceLock<Result<KZGSettings, String>> = OnceLock::new();

/// Default embedded KZG settings in `bincode` format.
///
/// This avoids any filesystem IO, suitable for zk guest builds.
static DEFAULT_KZG_SETTINGS_BIN: &[u8] = include_bytes!("../../kzg_settings.bin");

/// Get or initialize the KZG settings.
pub(crate) fn get_kzg_settings() -> RaikoResult<&'static KZGSettings> {
    match KZG_SETTINGS.get_or_init(init_kzg_settings) {
        Ok(settings) => Ok(settings),
        Err(e) => Err(RaikoError::InvalidBlobOption(format!(
            "KZG settings initialization failed: {e}"
        ))),
    }
}

/// Initialize KZG settings from the embedded trusted setup bytes.
fn init_kzg_settings() -> Result<KZGSettings, String> {
    bincode::deserialize(DEFAULT_KZG_SETTINGS_BIN)
        .map_err(|e| format!("bincode deserialize KZGSettings failed: {e}"))
}

pub type KzgField = [u8; 32];
pub type KzgCommitmentBytes = [u8; 48];

fn g1_to_kzg_commitment_bytes(g1_point: &impl G1) -> KzgCommitmentBytes {
    let bytes = g1_point.to_bytes();
    let mut result = [0u8; 48];
    result.copy_from_slice(&bytes);
    result
}

fn blob_to_commitment_with_settings(
    blob: &[u8],
    kzg_settings: &KZGSettings,
) -> RaikoResult<KzgCommitmentBytes> {
    let blob_fields = bytes_to_blob(blob)
        .map_err(|e| RaikoError::InvalidBlobOption(format!("Failed to convert blob: {e}")))?;

    let commitment = blob_to_kzg_commitment_rust(&blob_fields, kzg_settings)
        .map_err(|e| RaikoError::InvalidBlobOption(format!("Failed to compute commitment: {e}")))?;

    Ok(g1_to_kzg_commitment_bytes(&commitment))
}

fn proof_of_equivalence_point(blob: &[u8], commitment: &KzgCommitmentBytes) -> ZFr {
    let blob_hash = hash(blob);
    let versioned_hash = commitment_to_version_hash(commitment);
    let mut challenge_input = [0u8; 64];
    challenge_input[..32].copy_from_slice(&blob_hash);
    challenge_input[32..].copy_from_slice(versioned_hash.as_slice());
    let evaluation_challenge = hash(&challenge_input);
    hash_to_bls_field(&evaluation_challenge)
}

/// Convert blob (Vec<u8>) to KZG commitment using the static KZG settings.
///
/// # Errors
///
/// Returns an error if the blob cannot be converted or the commitment computation fails.
pub fn blob_to_commitment(blob: &[u8]) -> RaikoResult<KzgCommitmentBytes> {
    blob_to_commitment_with_settings(blob, get_kzg_settings()?)
}

fn blob_to_proof_with_settings(
    blob: &[u8],
    commitment: &KzgCommitmentBytes,
    kzg_settings: &KZGSettings,
) -> RaikoResult<KzgCommitmentBytes> {
    let blob_fields = bytes_to_blob(blob)
        .map_err(|e| RaikoError::InvalidBlobOption(format!("Failed to convert blob: {e}")))?;

    let kzg_commitment = G1::from_bytes(commitment)
        .map_err(|e| RaikoError::InvalidBlobOption(format!("Invalid commitment: {e}")))?;

    let proof = compute_blob_kzg_proof_rust(&blob_fields, &kzg_commitment, kzg_settings)
        .map_err(|e| RaikoError::InvalidBlobOption(format!("Failed to compute proof: {e}")))?;

    Ok(g1_to_kzg_commitment_bytes(&proof))
}

/// Compute a KZG proof for a blob and its corresponding commitment using the static KZG settings.
///
/// # Parameters
///
/// - `blob`: The raw blob data as a byte slice. In the context of [EIP-4844],
///   this should be the encoded blob whose polynomial commitment is being proven.
/// - `commitment`: The KZG commitment to the exact same blob, encoded as
///   48-byte `KzgCommitmentBytes`. This should typically be produced by
///   [`blob_to_commitment`] using the same `blob` input.
///
/// The `commitment` **must** correspond to the provided `blob`. Passing a
/// commitment that was computed from different data will result in a proof
/// that fails verification.
///
/// # Returns
///
/// Returns the KZG proof bytes (`KzgCommitmentBytes`) for the given `blob` and
/// `commitment`. This proof can be used with EIP-4844-compatible verification
/// routines (e.g. [`verify_blob_kzg_proof_with_settings`] or the underlying
/// `verify_blob_kzg_proof_rust`) to prove that the blob data matches its
/// published KZG commitment.
///
/// For more details on how blob commitments and proofs are used in the
/// protocol, see [EIP-4844].
///
/// [EIP-4844]: https://eips.ethereum.org/EIPS/eip-4844
///
/// # Errors
///
/// Returns an error if the blob cannot be converted, the commitment is invalid,
/// or the proof computation fails.
pub fn blob_to_proof(
    blob: &[u8],
    commitment: &KzgCommitmentBytes,
) -> RaikoResult<KzgCommitmentBytes> {
    blob_to_proof_with_settings(blob, commitment, get_kzg_settings()?)
}

fn blob_to_proof_of_equivalence_with_settings(
    blob: &[u8],
    commitment: &KzgCommitmentBytes,
    kzg_settings: &KZGSettings,
) -> RaikoResult<KzgCommitmentBytes> {
    let blob_fields = bytes_to_blob(blob)
        .map_err(|e| RaikoError::InvalidBlobOption(format!("Failed to convert blob: {e}")))?;
    let evaluation_point = proof_of_equivalence_point(blob, commitment);
    let (proof, _) = compute_kzg_proof_rust(&blob_fields, &evaluation_point, kzg_settings)
        .map_err(|e| {
            RaikoError::InvalidBlobOption(format!("Failed to compute proof of equivalence: {e}"))
        })?;

    Ok(g1_to_kzg_commitment_bytes(&proof))
}

/// Compute the point-evaluation proof used by Raiko's proof-of-equivalence strategy.
///
/// # Errors
///
/// Returns an error if the blob cannot be converted or the proof computation fails.
pub fn blob_to_proof_of_equivalence(
    blob: &[u8],
    commitment: &KzgCommitmentBytes,
) -> RaikoResult<KzgCommitmentBytes> {
    blob_to_proof_of_equivalence_with_settings(blob, commitment, get_kzg_settings()?)
}

/// Convert KZG commitment to versioned hash (EIP-4844).
///
/// Computes SHA256 hash of the commitment and sets the first byte to `VERSIONED_HASH_VERSION_KZG`.
#[must_use]
pub fn commitment_to_version_hash(commitment: &KzgCommitmentBytes) -> B256 {
    let mut out = hash(commitment);
    out[0] = VERSIONED_HASH_VERSION_KZG;
    B256::from_slice(&out)
}

pub(crate) fn verify_blob_kzg_proof_with_settings(
    blob: &[u8],
    commitment: &KzgCommitmentBytes,
    proof: &KzgCommitmentBytes,
    kzg_settings: &KZGSettings,
) -> RaikoResult<()> {
    let blob_fields = bytes_to_blob(blob)
        .map_err(|e| RaikoError::InvalidBlobOption(format!("Failed to convert blob: {e}")))?;

    let kzg_commitment = G1::from_bytes(commitment)
        .map_err(|e| RaikoError::InvalidBlobOption(format!("Invalid commitment: {e}")))?;
    let kzg_proof = G1::from_bytes(proof)
        .map_err(|e| RaikoError::InvalidBlobOption(format!("Invalid proof: {e}")))?;

    let is_valid =
        verify_blob_kzg_proof_rust(&blob_fields, &kzg_commitment, &kzg_proof, kzg_settings)
            .map_err(|e| RaikoError::InvalidBlobOption(format!("Verification error: {e}")))?;
    if !is_valid {
        return Err(RaikoError::InvalidBlobOption(
            "KZG proof verification failed".to_string(),
        ));
    }
    Ok(())
}

/// Verify blob KZG proof using the static KZG settings.
///
/// # Errors
///
/// Returns an error if the proof is invalid or verification fails.
pub fn verify_blob_kzg_proof(
    blob: &[u8],
    commitment: &KzgCommitmentBytes,
    proof: &KzgCommitmentBytes,
) -> RaikoResult<()> {
    verify_blob_kzg_proof_with_settings(blob, commitment, proof, get_kzg_settings()?)
}

fn verify_blob_proof_of_equivalence_with_settings(
    blob: &[u8],
    commitment: &KzgCommitmentBytes,
    proof: &KzgCommitmentBytes,
    kzg_settings: &KZGSettings,
) -> RaikoResult<()> {
    let blob_fields = bytes_to_blob(blob)
        .map_err(|e| RaikoError::InvalidBlobOption(format!("Failed to convert blob: {e}")))?;
    let polynomial = blob_to_polynomial(&blob_fields).map_err(|e| {
        RaikoError::InvalidBlobOption(format!(
            "Failed to build polynomial for proof of equivalence: {e}"
        ))
    })?;
    let evaluation_point = proof_of_equivalence_point(blob, commitment);
    let evaluation_value =
        evaluate_polynomial_in_evaluation_form(&polynomial, &evaluation_point, kzg_settings)
            .map_err(|e| {
                RaikoError::InvalidBlobOption(format!(
                    "Failed to evaluate polynomial for proof of equivalence: {e}"
                ))
            })?;

    let kzg_commitment = G1::from_bytes(commitment)
        .map_err(|e| RaikoError::InvalidBlobOption(format!("Invalid commitment: {e}")))?;
    let kzg_proof = G1::from_bytes(proof)
        .map_err(|e| RaikoError::InvalidBlobOption(format!("Invalid proof: {e}")))?;

    let is_valid = verify_kzg_proof_rust(
        &kzg_commitment,
        &evaluation_point,
        &evaluation_value,
        &kzg_proof,
        kzg_settings,
    )
    .map_err(|e| {
        RaikoError::InvalidBlobOption(format!("Proof of equivalence verification error: {e}"))
    })?;
    if !is_valid {
        return Err(RaikoError::InvalidBlobOption(
            "Proof of equivalence verification failed".to_string(),
        ));
    }

    Ok(())
}

/// Verify the point-evaluation proof used by Raiko's proof-of-equivalence strategy.
///
/// # Errors
///
/// Returns an error if the proof is invalid or verification fails.
pub fn verify_blob_proof_of_equivalence(
    blob: &[u8],
    commitment: &KzgCommitmentBytes,
    proof: &KzgCommitmentBytes,
) -> RaikoResult<()> {
    verify_blob_proof_of_equivalence_with_settings(blob, commitment, proof, get_kzg_settings()?)
}

// Note: `get_kzg_settings` and `verify_blob_kzg_proof_with_settings` are `pub(crate)` so
// `verification.rs` can call them without making them public API.

#[cfg(test)]
mod test {
    use super::*;
    use alloy_primitives::hex;
    use anyhow::{Context, Result};
    use kzg_traits::eip_4844::BYTES_PER_BLOB;

    #[test]
    fn blob_commitment_version_hash_and_proof_verify() -> Result<()> {
        let kzg_settings = get_kzg_settings().context("embedded settings load")?;

        let blob_bytes = vec![0u8; BYTES_PER_BLOB];
        let commitment_bytes =
            blob_to_commitment_with_settings(&blob_bytes, kzg_settings).context("commitment")?;

        let blob_fr = bytes_to_blob(&blob_bytes)
            .map_err(anyhow::Error::msg)
            .context("bytes_to_blob")?;
        let commitment_point = blob_to_kzg_commitment_rust(&blob_fr, kzg_settings)
            .map_err(anyhow::Error::msg)
            .context("commitment point")?;

        let proof_point = compute_blob_kzg_proof_rust(&blob_fr, &commitment_point, kzg_settings)
            .map_err(anyhow::Error::msg)
            .context("proof")?;

        verify_blob_kzg_proof_with_settings(
            &blob_bytes,
            &commitment_bytes,
            &proof_point.to_bytes(),
            kzg_settings,
        )
        .context("verify proof")?;

        let version_hash = commitment_to_version_hash(&commitment_bytes);
        let mut expected = hash(&commitment_bytes);
        expected[0] = VERSIONED_HASH_VERSION_KZG;
        assert_eq!(version_hash, B256::from_slice(&expected));
        Ok(())
    }

    #[test]
    fn blob_proof_of_equivalence_verify() -> Result<()> {
        let kzg_settings = get_kzg_settings().context("embedded settings load")?;

        let blob_bytes = vec![0u8; BYTES_PER_BLOB];
        let commitment_bytes =
            blob_to_commitment_with_settings(&blob_bytes, kzg_settings).context("commitment")?;
        let proof_bytes = blob_to_proof_of_equivalence_with_settings(
            &blob_bytes,
            &commitment_bytes,
            kzg_settings,
        )
        .context("proof of equivalence")?;

        verify_blob_proof_of_equivalence_with_settings(
            &blob_bytes,
            &commitment_bytes,
            &proof_bytes,
            kzg_settings,
        )
        .context("verify proof of equivalence")?;

        Ok(())
    }

    #[test]
    fn bincode_deserialize_kzg_settings_bin() -> Result<()> {
        static BIN: &[u8] = include_bytes!("../../kzg_settings.bin");
        let start = std::time::Instant::now();
        let deserialized_settings: KZGSettings =
            bincode::deserialize(BIN).context("Failed to deserialize KZGSettings from binary")?;
        println!(
            "✓ bincode deserialized KZGSettings in {:.2}s ({} bytes)",
            start.elapsed().as_secs_f64(),
            BIN.len()
        );

        let blob_bytes = vec![0u8; BYTES_PER_BLOB];
        let _commitment = blob_to_commitment_with_settings(&blob_bytes, &deserialized_settings)
            .context("commit")?;
        Ok(())
    }

    #[test]
    fn bincode_settings_matches_embedded_settings_commit_proof_verify() -> Result<()> {
        static BIN: &[u8] = include_bytes!("../../kzg_settings.bin");
        let deserialized_settings: KZGSettings =
            bincode::deserialize(BIN).context("Failed to deserialize KZGSettings from binary")?;

        let embedded_settings = get_kzg_settings().context("load embedded settings")?;

        let blob_bytes = vec![0u8; BYTES_PER_BLOB];
        let blob_fr = bytes_to_blob(&blob_bytes)
            .map_err(anyhow::Error::msg)
            .context("bytes_to_blob")?;

        let commitment_embedded = blob_to_kzg_commitment_rust(&blob_fr, embedded_settings)
            .map_err(anyhow::Error::msg)
            .context("commit embedded")?;
        let commitment_deser = blob_to_kzg_commitment_rust(&blob_fr, &deserialized_settings)
            .map_err(anyhow::Error::msg)
            .context("commit deser")?;
        assert_eq!(
            commitment_embedded.to_bytes(),
            commitment_deser.to_bytes(),
            "commitment mismatch: embedded vs bincode-deserialized"
        );

        let proof_embedded =
            compute_blob_kzg_proof_rust(&blob_fr, &commitment_embedded, embedded_settings)
                .map_err(anyhow::Error::msg)
                .context("proof embedded")?;
        let proof_deser =
            compute_blob_kzg_proof_rust(&blob_fr, &commitment_deser, &deserialized_settings)
                .map_err(anyhow::Error::msg)
                .context("proof deser")?;
        assert_eq!(
            proof_embedded.to_bytes(),
            proof_deser.to_bytes(),
            "proof mismatch: embedded vs bincode-deserialized"
        );

        let commitment_bytes = commitment_embedded.to_bytes();
        let proof_bytes = proof_embedded.to_bytes();
        verify_blob_kzg_proof_with_settings(
            &blob_bytes,
            &commitment_bytes,
            &proof_bytes,
            embedded_settings,
        )
        .context("verify with embedded settings")?;
        verify_blob_kzg_proof_with_settings(
            &blob_bytes,
            &commitment_bytes,
            &proof_bytes,
            &deserialized_settings,
        )
        .context("verify with bincode-deserialized settings")?;
        Ok(())
    }

    #[test]
    fn blob_to_proof_computes_verifiable_proof_for_mainnet_blob() -> Result<()> {
        use std::fs;

        // Read blob from file
        let blob_file = concat!(env!("CARGO_MANIFEST_DIR"), "/blob_13326465_0.bin");
        let blob_bytes = fs::read(blob_file).context("Failed to read blob file")?;
        println!("Read blob: {} bytes", blob_bytes.len());

        // Given commitment from mainnet
        let expected_commitment = "0xb8df58142f4397d25bf26f670fef31622428dbe4f22ad6e8c5386458ef28c698841904258320d98befd52b26edf1a26d";
        let commitment_str = expected_commitment
            .strip_prefix("0x")
            .unwrap_or(expected_commitment);
        let commitment_bytes: [u8; 48] = hex::decode(commitment_str)
            .context("Failed to decode commitment hex")?
            .try_into()
            .map_err(|_| anyhow::anyhow!("Commitment must be 48 bytes"))?;

        // Check that our computed commitment matches the expected commitment_hex above
        let computed_commitment =
            blob_to_commitment(&blob_bytes).context("Failed to compute commitment")?;
        assert_eq!(
            computed_commitment, commitment_bytes,
            "Calculated commitment does not match expected commitment!"
        );

        // Compute proof
        let proof =
            blob_to_proof(&blob_bytes, &commitment_bytes).context("Failed to compute proof")?;
        println!("Computed proof: 0x{}", hex::encode(proof));

        // Verify proof
        verify_blob_kzg_proof(&blob_bytes, &commitment_bytes, &proof)
            .context("Failed to verify proof")?;

        println!("✓ Proof verified successfully!");
        Ok(())
    }
}
