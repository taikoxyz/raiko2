use crate::{GuestInput, RaikoError, RaikoResult};
use alloy_primitives::B256;
use kzg::kzg_proofs::KZGSettings;
use kzg_traits::G1;
use kzg_traits::eip_4844::{
    blob_to_kzg_commitment_rust, bytes_to_blob, hash, load_trusted_setup_rust,
    load_trusted_setup_string, verify_blob_kzg_proof_rust,
};
#[cfg(test)]
use std::path::Path;
use std::sync::OnceLock;

pub const VERSIONED_HASH_VERSION_KZG: u8 = 0x01;

/// Static KZG settings loaded from the default trusted setup.
/// This is initialized lazily on first access.
static KZG_SETTINGS: OnceLock<Result<KZGSettings, String>> = OnceLock::new();

/// Get or initialize the KZG settings.
/// Loads the trusted setup from the default path on first call.
fn get_kzg_settings() -> RaikoResult<&'static KZGSettings> {
    match KZG_SETTINGS.get_or_init(init_kzg_settings) {
        Ok(settings) => Ok(settings),
        Err(e) => Err(RaikoError::InvalidBlobOption(format!(
            "KZG settings initialization failed: {e}"
        ))),
    }
}

/// Default embedded trusted setup in `RKZG` binary format.
///
/// This makes KZG settings initialization fast and deterministic (no filesystem IO), suitable for
/// zk guest builds.
static DEFAULT_RKZG_BYTES: &[u8] = include_bytes!("../kzg_settings_raw.rkzg");

/// Initialize KZG settings from the trusted setup file.
fn init_kzg_settings() -> Result<KZGSettings, String> {
    // NOTE: zk guests cannot read files. Keep this strictly embedded.
    let bytes: &[u8] = DEFAULT_RKZG_BYTES;
    let (g1_monomial_bytes, g1_lagrange_bytes, g2_monomial_bytes) =
        decode_trusted_setup_bytes(bytes)?;

    load_trusted_setup_rust(&g1_monomial_bytes, &g1_lagrange_bytes, &g2_monomial_bytes)
}

const TRUSTED_SETUP_BIN_MAGIC: &[u8; 4] = b"RKZG";
const TRUSTED_SETUP_BIN_VERSION: u8 = 1;

/// Decode trusted setup either from a fast binary format (preferred) or from the standard text
/// trusted setup format.
///
/// Binary format v1:
/// - magic: "RKZG" (4 bytes)
/// - version: u8 (1)
/// - reserved: [u8; 3] (zeros)
/// - g1_monomial_len: u32 LE
/// - g1_lagrange_len: u32 LE
/// - g2_monomial_len: u32 LE
/// - payload bytes: g1_monomial || g1_lagrange || g2_monomial
fn decode_trusted_setup_bytes(bytes: &[u8]) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), String> {
    use kzg_traits::eip_4844::{
        BYTES_PER_G1, BYTES_PER_G2, FIELD_ELEMENTS_PER_BLOB, TRUSTED_SETUP_NUM_G2_POINTS,
    };

    if bytes.len() >= 4 && &bytes[0..4] == TRUSTED_SETUP_BIN_MAGIC {
        if bytes.len() < 4 + 1 + 3 + 12 {
            return Err("trusted setup bin too small".to_string());
        }
        let version = bytes[4];
        if version != TRUSTED_SETUP_BIN_VERSION {
            return Err(format!("unsupported trusted setup bin version: {version}"));
        }
        let g1_m_len = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let g1_l_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let g2_m_len = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize;

        let start = 20;
        let end1 = start + g1_m_len;
        let end2 = end1 + g1_l_len;
        let end3 = end2 + g2_m_len;
        if end3 != bytes.len() {
            return Err(format!(
                "trusted setup bin length mismatch: expected payload {}, got {}",
                end3,
                bytes.len()
            ));
        }

        // Basic sanity checks to avoid obvious bad inputs.
        if g1_m_len % BYTES_PER_G1 != 0
            || g1_l_len % BYTES_PER_G1 != 0
            || g2_m_len % BYTES_PER_G2 != 0
        {
            return Err("trusted setup bin has invalid chunk sizes".to_string());
        }
        let g1_points = g1_m_len / BYTES_PER_G1;
        let g2_points = g2_m_len / BYTES_PER_G2;
        if g1_points != FIELD_ELEMENTS_PER_BLOB {
            return Err(format!(
                "trusted setup bin wrong g1 points: {g1_points} (expected {FIELD_ELEMENTS_PER_BLOB})"
            ));
        }
        if g1_l_len / BYTES_PER_G1 != FIELD_ELEMENTS_PER_BLOB {
            return Err("trusted setup bin wrong g1 lagrange size".to_string());
        }
        if g2_points != TRUSTED_SETUP_NUM_G2_POINTS {
            return Err(format!(
                "trusted setup bin wrong g2 points: {g2_points} (expected {TRUSTED_SETUP_NUM_G2_POINTS})"
            ));
        }

        return Ok((
            bytes[start..end1].to_vec(),
            bytes[end1..end2].to_vec(),
            bytes[end2..end3].to_vec(),
        ));
    }

    // Fallback: treat as UTF-8 text setup.
    let contents =
        std::str::from_utf8(bytes).map_err(|e| format!("trusted setup not utf8: {e}"))?;
    load_trusted_setup_string(contents)
}

fn encode_trusted_setup_bin(g1_m: &[u8], g1_l: &[u8], g2_m: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(20 + g1_m.len() + g1_l.len() + g2_m.len());
    out.extend_from_slice(TRUSTED_SETUP_BIN_MAGIC);
    out.push(TRUSTED_SETUP_BIN_VERSION);
    out.extend_from_slice(&[0u8; 3]); // reserved
    out.extend_from_slice(&(g1_m.len() as u32).to_le_bytes());
    out.extend_from_slice(&(g1_l.len() as u32).to_le_bytes());
    out.extend_from_slice(&(g2_m.len() as u32).to_le_bytes());
    out.extend_from_slice(g1_m);
    out.extend_from_slice(g1_l);
    out.extend_from_slice(g2_m);
    out
}

pub type KzgGroup = [u8; 48];
pub type KzgField = [u8; 32];
pub type KzgCommitmentBytes = KzgGroup;

fn blob_to_commitment_with_settings(
    blob: &[u8],
    kzg_settings: &KZGSettings,
) -> RaikoResult<KzgCommitmentBytes> {
    // Convert blob bytes to blob format expected by kzg
    let blob_fields = bytes_to_blob(blob)
        .map_err(|e| RaikoError::InvalidBlobOption(format!("Failed to convert blob: {}", e)))?;

    // Compute KZG commitment
    let commitment = blob_to_kzg_commitment_rust(&blob_fields, kzg_settings).map_err(|e| {
        RaikoError::InvalidBlobOption(format!("Failed to compute commitment: {}", e))
    })?;

    // Convert commitment to bytes (48 bytes)
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

fn verify_blob_kzg_proof_with_settings(
    blob: &[u8],
    commitment: &KzgCommitmentBytes,
    proof: &KzgCommitmentBytes,
    kzg_settings: &KZGSettings,
) -> RaikoResult<()> {
    // Convert blob bytes to blob format
    let blob_fields = bytes_to_blob(blob)
        .map_err(|e| RaikoError::InvalidBlobOption(format!("Failed to convert blob: {}", e)))?;

    // Convert commitment bytes to G1 point
    let kzg_commitment = G1::from_bytes(commitment)
        .map_err(|e| RaikoError::InvalidBlobOption(format!("Invalid commitment: {}", e)))?;

    // Convert proof bytes to G1 point
    let kzg_proof = G1::from_bytes(proof)
        .map_err(|e| RaikoError::InvalidBlobOption(format!("Invalid proof: {}", e)))?;

    // Verify the proof
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

/// Verify blob usage in proposal mode.
///
/// Checks that raw blob commitment matches input blob commitment,
/// then verifies the blob version hash.
/// Iterates through each data source and verifies each blob with its commitment and proof.
///
/// This function uses a static KZG settings loaded from the default trusted setup path.
pub fn verify_proposal_mode_blob_usage(guest_input: &GuestInput) -> RaikoResult<()> {
    // Get the static KZG settings
    let kzg_settings = get_kzg_settings()?;

    for data_source in &guest_input.taiko.data_sources {
        // Skip if no blobs in this data source
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

        // Iterate and verify each blob with its commitment and proof
        for (idx, blob_data) in data_source.tx_data_from_blob.iter().enumerate() {
            // Convert commitment Vec<u8> to [u8; 48]
            if commitments[idx].len() != 48 {
                return Err(RaikoError::InvalidBlobOption(format!(
                    "Commitment at index {} has invalid length (expected 48 bytes, got {})",
                    idx,
                    commitments[idx].len()
                )));
            }
            let mut commitment_array = [0u8; 48];
            commitment_array.copy_from_slice(&commitments[idx]);

            // Convert proof Vec<u8> to [u8; 48]
            if proofs[idx].len() != 48 {
                return Err(RaikoError::InvalidBlobOption(format!(
                    "Proof at index {} has invalid length (expected 48 bytes, got {})",
                    idx,
                    proofs[idx].len()
                )));
            }
            let mut proof_array = [0u8; 48];
            proof_array.copy_from_slice(&proofs[idx]);

            // Verify the blob KZG proof
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

#[cfg(test)]
mod test {
    use super::*;
    use kzg::kzg_proofs::generate_trusted_setup;
    use kzg_traits::G2;
    use kzg_traits::eip_4844::TRUSTED_SETUP_NUM_G2_POINTS;
    use kzg_traits::eip_4844::{
        BYTES_PER_BLOB, FIELD_ELEMENTS_PER_BLOB, compute_blob_kzg_proof_rust,
    };

    #[test]
    fn blob_commitment_version_hash_and_proof_verify() {
        // Build a valid EIP-4844-sized blob (all zeros).
        let blob_bytes = vec![0u8; BYTES_PER_BLOB];

        // Build KZGSettings in-memory (no filesystem dependency).
        let (g1_monomial, g1_lagrange, g2_monomial_full) =
            generate_trusted_setup(FIELD_ELEMENTS_PER_BLOB, [7u8; 32]);
        // EIP-4844 trusted setup only requires 65 G2 points.
        let g2_monomial = &g2_monomial_full[..TRUSTED_SETUP_NUM_G2_POINTS];

        let mut g1_monomial_bytes = Vec::with_capacity(FIELD_ELEMENTS_PER_BLOB * 48);
        for p in &g1_monomial {
            g1_monomial_bytes.extend_from_slice(&p.to_bytes());
        }

        let mut g1_lagrange_bytes = Vec::with_capacity(FIELD_ELEMENTS_PER_BLOB * 48);
        for p in &g1_lagrange {
            g1_lagrange_bytes.extend_from_slice(&p.to_bytes());
        }

        let mut g2_monomial_bytes = Vec::with_capacity(TRUSTED_SETUP_NUM_G2_POINTS * 96);
        for p in g2_monomial {
            g2_monomial_bytes.extend_from_slice(&p.to_bytes());
        }

        let kzg_settings: KZGSettings =
            load_trusted_setup_rust(&g1_monomial_bytes, &g1_lagrange_bytes, &g2_monomial_bytes)
                .expect("load trusted setup into KZGSettings");

        // Compute commitment (bytes) via our helper.
        let commitment_bytes =
            blob_to_commitment_with_settings(&blob_bytes, &kzg_settings).expect("commitment");

        // Compute commitment (point) and proof via rust-kzg, and verify they match our bytes wrapper.
        let blob_fr = bytes_to_blob(&blob_bytes).expect("bytes_to_blob");
        let commitment_point =
            blob_to_kzg_commitment_rust(&blob_fr, &kzg_settings).expect("commitment point");
        assert_eq!(commitment_bytes, commitment_point.to_bytes());

        let proof_point =
            compute_blob_kzg_proof_rust(&blob_fr, &commitment_point, &kzg_settings).expect("proof");
        let proof_bytes = proof_point.to_bytes();

        verify_blob_kzg_proof_with_settings(
            &blob_bytes,
            &commitment_bytes,
            &proof_bytes,
            &kzg_settings,
        )
        .expect("verify proof");

        // Versioned hash: sha256(commitment) with first byte set to 0x01.
        let version_hash = commitment_to_version_hash(&commitment_bytes);
        let mut expected = hash(&commitment_bytes);
        expected[0] = VERSIONED_HASH_VERSION_KZG;
        assert_eq!(version_hash, B256::from_slice(&expected));
    }

    #[test]
    fn trusted_setup_binary_format_roundtrip_and_loads() {
        let (g1_monomial, g1_lagrange, g2_monomial_full) =
            generate_trusted_setup(FIELD_ELEMENTS_PER_BLOB, [9u8; 32]);
        let g2_monomial = &g2_monomial_full[..TRUSTED_SETUP_NUM_G2_POINTS];

        let mut g1_monomial_bytes = Vec::with_capacity(FIELD_ELEMENTS_PER_BLOB * 48);
        for p in &g1_monomial {
            g1_monomial_bytes.extend_from_slice(&p.to_bytes());
        }

        let mut g1_lagrange_bytes = Vec::with_capacity(FIELD_ELEMENTS_PER_BLOB * 48);
        for p in &g1_lagrange {
            g1_lagrange_bytes.extend_from_slice(&p.to_bytes());
        }

        let mut g2_monomial_bytes = Vec::with_capacity(TRUSTED_SETUP_NUM_G2_POINTS * 96);
        for p in g2_monomial {
            g2_monomial_bytes.extend_from_slice(&p.to_bytes());
        }

        let bin =
            encode_trusted_setup_bin(&g1_monomial_bytes, &g1_lagrange_bytes, &g2_monomial_bytes);
        let (a, b, c) = decode_trusted_setup_bytes(&bin).expect("decode bin");
        assert_eq!(a, g1_monomial_bytes);
        assert_eq!(b, g1_lagrange_bytes);
        assert_eq!(c, g2_monomial_bytes);

        let _settings: KZGSettings = load_trusted_setup_rust(&a, &b, &c).expect("load settings");
    }

    #[test]
    fn load_repo_rkzg_and_compute_commitment() {
        // This file is expected to be generated offline (from `trusted_setup.txt`) and checked in.
        // It uses the fast `RKZG` binary format defined in this module.
        static RKZG_BYTES: &[u8] = include_bytes!("../kzg_settings_raw.rkzg");

        let (g1_m, g1_l, g2_m) = decode_trusted_setup_bytes(RKZG_BYTES).expect("decode rkzg");
        let kzg_settings: KZGSettings =
            load_trusted_setup_rust(&g1_m, &g1_l, &g2_m).expect("load KZGSettings");

        // Compute commitment from a deterministic blob.
        let blob_bytes = vec![0u8; BYTES_PER_BLOB];
        let commitment =
            blob_to_commitment_with_settings(&blob_bytes, &kzg_settings).expect("commitment");

        // Basic sanity: version hash should be stable and not all-zero.
        let version_hash = commitment_to_version_hash(&commitment);
        assert_ne!(version_hash, B256::ZERO);
    }

    /// Offline helper: convert an Ethereum `trusted_setup.txt` into an `RKZG` binary file.
    ///
    /// Usage:
    /// - `RAIKO2_TRUSTED_SETUP_TXT=/path/to/trusted_setup.txt`
    /// - `RAIKO2_TRUSTED_SETUP_BIN=/path/to/kzg_settings_raw.rkzg` (optional; default shown)
    ///
    /// Run:
    /// `cargo test -p raiko2-primitives convert_eth_trusted_setup_txt_to_bin -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn convert_eth_trusted_setup_txt_to_bin() {
        let txt_path = std::env::var("RAIKO2_TRUSTED_SETUP_TXT")
            .expect("set RAIKO2_TRUSTED_SETUP_TXT=/path/to/trusted_setup.txt");
        let out_path = std::env::var("RAIKO2_TRUSTED_SETUP_BIN")
            .unwrap_or_else(|_| "kzg_settings_raw.rkzg".to_string());

        write_trusted_setup_bin_from_txt(&txt_path, &out_path).expect("convert and write rkzg");

        // Sanity: ensure the produced file decodes and loads.
        let bytes = std::fs::read(&out_path).expect("read rkzg output");
        let (g1m, g1l, g2m) = decode_trusted_setup_bytes(&bytes).expect("decode rkzg output");
        let _settings: KZGSettings =
            load_trusted_setup_rust(&g1m, &g1l, &g2m).expect("load settings from rkzg output");
    }

    /// Convert an EIP-4844 trusted setup **text** file (e.g. Ethereum `trusted_setup.txt`) into the
    /// fast `RKZG` binary format used by `decode_trusted_setup_bytes`.
    ///
    /// This is intended as an offline build step: run once, then ship the `.rkzg` file.
    fn trusted_setup_txt_to_rkzg_bytes(txt_contents: &str) -> Result<Vec<u8>, String> {
        let (g1_monomial_bytes, g1_lagrange_bytes, g2_monomial_bytes) =
            load_trusted_setup_string(txt_contents)?;
        Ok(encode_trusted_setup_bin(
            &g1_monomial_bytes,
            &g1_lagrange_bytes,
            &g2_monomial_bytes,
        ))
    }

    fn write_trusted_setup_bin_from_txt(
        txt_path: impl AsRef<Path>,
        out_path: impl AsRef<Path>,
    ) -> Result<(), String> {
        let txt_path = txt_path.as_ref();
        let out_path = out_path.as_ref();

        let txt = std::fs::read_to_string(txt_path)
            .map_err(|e| format!("read trusted setup txt {:?}: {e}", txt_path))?;
        let bin = trusted_setup_txt_to_rkzg_bytes(&txt)?;
        std::fs::write(out_path, &bin)
            .map_err(|e| format!("write rkzg bin {:?}: {e}", out_path))?;
        Ok(())
    }
}
