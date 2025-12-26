use crate::{RaikoError, RaikoResult};
use alloy_primitives::B256;
use kzg::kzg_proofs::KZGSettings;
use kzg_traits::G1;
use kzg_traits::eip_4844::{
    blob_to_kzg_commitment_rust, bytes_to_blob, hash, verify_blob_kzg_proof_rust,
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

pub type KzgGroup = [u8; 48];
pub type KzgField = [u8; 32];
pub type KzgCommitmentBytes = KzgGroup;

fn blob_to_commitment_with_settings(
    blob: &[u8],
    kzg_settings: &KZGSettings,
) -> RaikoResult<KzgCommitmentBytes> {
    let blob_fields = bytes_to_blob(blob)
        .map_err(|e| RaikoError::InvalidBlobOption(format!("Failed to convert blob: {}", e)))?;

    let commitment = blob_to_kzg_commitment_rust(&blob_fields, kzg_settings).map_err(|e| {
        RaikoError::InvalidBlobOption(format!("Failed to compute commitment: {}", e))
    })?;

    let commitment_bytes = commitment.to_bytes();
    let mut result = [0u8; 48];
    result.copy_from_slice(&commitment_bytes);
    Ok(result)
}

/// Convert blob (Vec<u8>) to KZG commitment using the static KZG settings.
pub fn blob_to_commitment(blob: &[u8]) -> RaikoResult<KzgCommitmentBytes> {
    blob_to_commitment_with_settings(blob, get_kzg_settings()?)
}

/// Convert KZG commitment to versioned hash (EIP-4844).
///
/// Computes SHA256 hash of the commitment and sets the first byte to VERSIONED_HASH_VERSION_KZG.
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
        .map_err(|e| RaikoError::InvalidBlobOption(format!("Failed to convert blob: {}", e)))?;

    let kzg_commitment = G1::from_bytes(commitment)
        .map_err(|e| RaikoError::InvalidBlobOption(format!("Invalid commitment: {}", e)))?;
    let kzg_proof = G1::from_bytes(proof)
        .map_err(|e| RaikoError::InvalidBlobOption(format!("Invalid proof: {}", e)))?;

    let is_valid =
        verify_blob_kzg_proof_rust(&blob_fields, &kzg_commitment, &kzg_proof, kzg_settings)
            .map_err(|e| RaikoError::InvalidBlobOption(format!("Verification error: {}", e)))?;
    if !is_valid {
        return Err(RaikoError::InvalidBlobOption(
            "KZG proof verification failed".to_string(),
        ));
    }
    Ok(())
}

/// Verify blob KZG proof using the static KZG settings.
pub fn verify_blob_kzg_proof(
    blob: &[u8],
    commitment: &KzgCommitmentBytes,
    proof: &KzgCommitmentBytes,
) -> RaikoResult<()> {
    verify_blob_kzg_proof_with_settings(blob, commitment, proof, get_kzg_settings()?)
}

// Note: `get_kzg_settings` and `verify_blob_kzg_proof_with_settings` are `pub(crate)` so
// `verification.rs` can call them without making them public API.

#[cfg(test)]
mod test {
    use super::*;
    use kzg_traits::eip_4844::{BYTES_PER_BLOB, compute_blob_kzg_proof_rust};

    #[test]
    fn blob_commitment_version_hash_and_proof_verify() {
        let kzg_settings = get_kzg_settings().expect("embedded settings load");

        let blob_bytes = vec![0u8; BYTES_PER_BLOB];
        let commitment_bytes =
            blob_to_commitment_with_settings(&blob_bytes, kzg_settings).expect("commitment");

        let blob_fr = bytes_to_blob(&blob_bytes).expect("bytes_to_blob");
        let commitment_point =
            blob_to_kzg_commitment_rust(&blob_fr, kzg_settings).expect("commitment point");

        let proof_point =
            compute_blob_kzg_proof_rust(&blob_fr, &commitment_point, kzg_settings).expect("proof");

        verify_blob_kzg_proof_with_settings(
            &blob_bytes,
            &commitment_bytes,
            &proof_point.to_bytes(),
            kzg_settings,
        )
        .expect("verify proof");

        let version_hash = commitment_to_version_hash(&commitment_bytes);
        let mut expected = hash(&commitment_bytes);
        expected[0] = VERSIONED_HASH_VERSION_KZG;
        assert_eq!(version_hash, B256::from_slice(&expected));
    }

    #[test]
    fn bincode_deserialize_kzg_settings_bin() {
        static BIN: &[u8] = include_bytes!("../../kzg_settings.bin");
        let start = std::time::Instant::now();
        let deserialized_settings: KZGSettings =
            bincode::deserialize(BIN).expect("Failed to deserialize KZGSettings from binary");
        println!(
            "✓ bincode deserialized KZGSettings in {:.2}s ({} bytes)",
            start.elapsed().as_secs_f64(),
            BIN.len()
        );

        let blob_bytes = vec![0u8; BYTES_PER_BLOB];
        let _commitment =
            blob_to_commitment_with_settings(&blob_bytes, &deserialized_settings).expect("commit");
    }

    #[test]
    fn bincode_settings_matches_embedded_settings_commit_proof_verify() {
        static BIN: &[u8] = include_bytes!("../../kzg_settings.bin");
        let deserialized_settings: KZGSettings =
            bincode::deserialize(BIN).expect("Failed to deserialize KZGSettings from binary");

        let embedded_settings = get_kzg_settings().expect("load embedded settings");

        let blob_bytes = vec![0u8; BYTES_PER_BLOB];
        let blob_fr = bytes_to_blob(&blob_bytes).expect("bytes_to_blob");

        let commitment_embedded =
            blob_to_kzg_commitment_rust(&blob_fr, embedded_settings).expect("commit embedded");
        let commitment_deser =
            blob_to_kzg_commitment_rust(&blob_fr, &deserialized_settings).expect("commit deser");
        assert_eq!(
            commitment_embedded.to_bytes(),
            commitment_deser.to_bytes(),
            "commitment mismatch: embedded vs bincode-deserialized"
        );

        let proof_embedded =
            compute_blob_kzg_proof_rust(&blob_fr, &commitment_embedded, embedded_settings)
                .expect("proof embedded");
        let proof_deser =
            compute_blob_kzg_proof_rust(&blob_fr, &commitment_deser, &deserialized_settings)
                .expect("proof deser");
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
        .expect("verify with embedded settings");
        verify_blob_kzg_proof_with_settings(
            &blob_bytes,
            &commitment_bytes,
            &proof_bytes,
            &deserialized_settings,
        )
        .expect("verify with bincode-deserialized settings");
    }
}
