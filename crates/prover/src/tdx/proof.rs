//! TDX proof construction and attestation quote generation.
//!
//! `TdxProof` (89 bytes: `instance_id(4) || address(20) || signature(65)`) is the canonical
//! wire format for both proposal proofs and aggregation proofs. This matches the `SgxVerifier`
//! wire format so TDX proofs can slot into any `ComposeVerifier` configuration.

use alloy_primitives::{Address, B256};
use anyhow::{Result, anyhow};
use rand::Rng;
use tracing::info;

use crate::tdx::{
    attestation_client,
    config::load_private_key,
    signature::{address_from_private_key, recover_signer, sign_message},
};

/// Wire size: `instance_id(4) || address(20) || signature(65)`.
pub const TDX_PROOF_SIZE: usize = 89;

// ────────────────────────── Proof structures ──────────────────────────

/// A single TDX proof (89 bytes).
#[derive(Debug)]
pub struct TdxProof {
    data: [u8; TDX_PROOF_SIZE],
}

impl TdxProof {
    /// Build a new proof from its components.
    #[must_use]
    pub fn new(instance_id: u32, public_key: &Address, signature: &[u8; 65]) -> Self {
        let mut data = [0u8; TDX_PROOF_SIZE];
        data[0..4].copy_from_slice(&instance_id.to_be_bytes());
        data[4..24].copy_from_slice(public_key.as_slice());
        data[24..89].copy_from_slice(signature);
        Self { data }
    }

    /// Parse from a byte slice.
    ///
    /// # Errors
    ///
    /// Returns an error if the byte slice length does not match the expected proof size.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != TDX_PROOF_SIZE {
            return Err(anyhow!(
                "Invalid proof size: expected {TDX_PROOF_SIZE}, got {}",
                bytes.len()
            ));
        }
        let mut data = [0u8; TDX_PROOF_SIZE];
        data.copy_from_slice(bytes);
        Ok(Self { data })
    }

    /// Extract the `instance_id` field (bytes 0..4, big-endian).
    #[must_use]
    pub fn instance_id(&self) -> u32 {
        u32::from_be_bytes(self.data[0..4].try_into().unwrap())
    }

    /// Extract the prover address (bytes 4..24).
    #[must_use]
    pub fn public_key(&self) -> Address {
        Address::from_slice(&self.data[4..24])
    }

    /// Extract the ECDSA signature (bytes 24..89).
    #[must_use]
    pub fn signature(&self) -> [u8; 65] {
        self.data[24..89].try_into().unwrap()
    }

    /// Consume and return the raw bytes.
    #[must_use]
    pub fn into_vec(self) -> Vec<u8> {
        self.data.to_vec()
    }
}

// ────────────────────────── Quote generation ──────────────────────────

/// Generate a TDX attestation quote for arbitrary 32-byte user data.
///
/// # Errors
///
/// Returns an error if the attestation service is unreachable.
pub fn generate_tdx_quote(
    socket_path: &str,
    user_report_data: &B256,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let nonce: [u8; 32] = rand::thread_rng().r#gen();
    let nonce = nonce.to_vec();

    info!("Requesting TDX attestation from: {socket_path}");
    let attestation_doc =
        attestation_client::issue_attestation(socket_path, user_report_data.as_slice(), &nonce)?;

    Ok((attestation_doc, nonce))
}

/// Generate a TDX attestation quote embedding the prover's public key (for bootstrap).
///
/// # Errors
///
/// Returns an error if the attestation service is unreachable.
pub fn generate_tdx_quote_from_public_key(
    socket_path: &str,
    public_key: &Address,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut padded = [0u8; 32];
    padded[..20].copy_from_slice(public_key.as_slice());
    generate_tdx_quote(socket_path, &B256::from(padded))
}

/// Retrieve metadata from the attestation service.
///
/// # Errors
///
/// Returns an error if the attestation service is unreachable.
pub fn get_tdx_metadata(socket_path: &str) -> Result<attestation_client::MetadataResponseData> {
    attestation_client::metadata(socket_path)
}

// ────────────────────────── Prove (single / batch) ──────────────────────────

/// Output of a single/batch proof generation.
pub struct ProveData {
    pub proof: Vec<u8>,
    pub quote: Vec<u8>,
    pub instance_hash: B256,
}

/// Generate a TDX proof for the given instance hash.
///
/// Signs the hash with the bootstrapped private key, builds the 89-byte proof,
/// and generates a TDX attestation quote over the hash.
///
/// # Errors
///
/// Returns an error if the private key cannot be loaded, signing fails, or the
/// attestation service is unreachable.
pub fn prove(socket_path: &str, instance_id: u32, instance_hash: B256) -> Result<ProveData> {
    let private_key = load_private_key()?;
    let address = address_from_private_key(&private_key);

    let signature = sign_message(&private_key, &instance_hash)?;
    let proof = TdxProof::new(instance_id, &address, &signature).into_vec();
    let (quote, _nonce) = generate_tdx_quote(socket_path, &instance_hash)?;

    Ok(ProveData {
        proof,
        quote,
        instance_hash,
    })
}

// ────────────────────────── Prove aggregation ──────────────────────────

/// Output of an aggregation proof generation.
pub struct ProveAggregationData {
    pub proof: Vec<u8>,
    pub quote: Vec<u8>,
    pub aggregation_hash: B256,
}

/// Generate a Shasta TDX aggregation proof.
///
/// Verifies that all sub-proofs were signed by the same instance (no key rotation
/// in Shasta), computes the aggregation hash, signs it, and generates a TDX quote.
///
/// # Errors
///
/// Returns an error if sub-proof verification fails, the private key cannot be
/// loaded, or the attestation service is unreachable.
pub fn prove_shasta_aggregation(
    socket_path: &str,
    instance_id: u32,
    sub_proofs: &[(Vec<u8>, B256)],
    aggregation_hash: B256,
) -> Result<ProveAggregationData> {
    let expected_instance = verify_sub_proofs(sub_proofs)?;

    // Sign the aggregation hash with the current instance's key.
    // In Shasta, key rotation is not allowed: the local key must match the
    // sub-proofs' instance, otherwise the emitted aggregation proof would
    // reference a different prover than the sub-proofs and fail to verify.
    let private_key = load_private_key()?;
    let new_instance = address_from_private_key(&private_key);

    if new_instance != expected_instance {
        return Err(anyhow!(
            "Shasta aggregation does not allow key rotation: local instance {new_instance} does not match sub-proofs instance {expected_instance}"
        ));
    }

    let signature = sign_message(&private_key, &aggregation_hash)?;
    let proof = TdxProof::new(instance_id, &new_instance, &signature).into_vec();
    let (quote, _nonce) = generate_tdx_quote(socket_path, &aggregation_hash)?;

    Ok(ProveAggregationData {
        proof,
        quote,
        aggregation_hash,
    })
}

/// Verify that all sub-proofs share the same instance and have valid signatures,
/// returning that instance address.
///
/// # Errors
///
/// Returns an error if the list is empty, any proof is malformed, any signature
/// fails to recover the expected signer, or proofs disagree on the prover instance.
fn verify_sub_proofs(sub_proofs: &[(Vec<u8>, B256)]) -> Result<Address> {
    if sub_proofs.is_empty() {
        return Err(anyhow!("No sub-proofs provided for aggregation"));
    }

    let first_proof = TdxProof::from_bytes(&sub_proofs[0].0)?;
    let expected_instance = first_proof.public_key();

    for (i, (proof_bytes, input_hash)) in sub_proofs.iter().enumerate() {
        let tdx_proof = TdxProof::from_bytes(proof_bytes)?;
        let instance = tdx_proof.public_key();

        if instance != expected_instance {
            return Err(anyhow!(
                "Shasta aggregation does not allow key rotation: proof {i} has instance {instance}, expected {expected_instance}"
            ));
        }

        let signature = tdx_proof.signature();
        let recovered = recover_signer(&signature, input_hash)?;
        if recovered != expected_instance {
            return Err(anyhow!(
                "Proof {i} signature verification failed: expected signer {expected_instance}, got {recovered}"
            ));
        }
    }

    Ok(expected_instance)
}

#[cfg(test)]
mod tests {
    use super::{TDX_PROOF_SIZE, TdxProof, verify_sub_proofs};
    use crate::tdx::signature::{address_from_private_key, sign_message};
    use alloy_primitives::{Address, B256};
    use secp256k1::Secp256k1;

    fn signed_proof(
        secret: &secp256k1::SecretKey,
        instance_id: u32,
        input_hash: B256,
    ) -> (Vec<u8>, B256) {
        let address = address_from_private_key(secret);
        let signature = sign_message(secret, &input_hash).expect("sign");
        let proof = TdxProof::new(instance_id, &address, &signature).into_vec();
        (proof, input_hash)
    }

    #[test]
    fn from_bytes_rejects_short_buffer() {
        let err = TdxProof::from_bytes(&[0u8; TDX_PROOF_SIZE - 1]).expect_err("short");
        assert!(err.to_string().contains("Invalid proof size"));
    }

    #[test]
    fn from_bytes_rejects_long_buffer() {
        let err = TdxProof::from_bytes(&[0u8; TDX_PROOF_SIZE + 1]).expect_err("long");
        assert!(err.to_string().contains("Invalid proof size"));
    }

    #[test]
    fn proof_round_trip_preserves_fields() {
        let address = Address::repeat_byte(0xab);
        let signature = [0x42u8; 65];
        let proof = TdxProof::new(7, &address, &signature);
        let bytes = proof.into_vec();
        let parsed = TdxProof::from_bytes(&bytes).expect("parse");

        assert_eq!(parsed.instance_id(), 7);
        assert_eq!(parsed.public_key(), address);
        assert_eq!(parsed.signature(), signature);
    }

    #[test]
    fn verify_sub_proofs_rejects_empty() {
        let err = verify_sub_proofs(&[]).expect_err("empty");
        assert!(err.to_string().contains("No sub-proofs"));
    }

    #[test]
    fn verify_sub_proofs_accepts_consistent_signatures() {
        let secp = Secp256k1::new();
        let (secret, _) = secp.generate_keypair(&mut rand::thread_rng());
        let expected = address_from_private_key(&secret);

        let sub_proofs = vec![
            signed_proof(&secret, 1, B256::repeat_byte(0x11)),
            signed_proof(&secret, 1, B256::repeat_byte(0x22)),
        ];

        let instance = verify_sub_proofs(&sub_proofs).expect("verify");
        assert_eq!(instance, expected);
    }

    #[test]
    fn verify_sub_proofs_rejects_key_rotation() {
        let secp = Secp256k1::new();
        let (secret_a, _) = secp.generate_keypair(&mut rand::thread_rng());
        let (secret_b, _) = secp.generate_keypair(&mut rand::thread_rng());

        let sub_proofs = vec![
            signed_proof(&secret_a, 1, B256::repeat_byte(0x11)),
            signed_proof(&secret_b, 1, B256::repeat_byte(0x22)),
        ];

        let err = verify_sub_proofs(&sub_proofs).expect_err("rotation");
        assert!(
            err.to_string().contains("key rotation"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn verify_sub_proofs_rejects_signature_against_wrong_hash() {
        let secp = Secp256k1::new();
        let (secret, _) = secp.generate_keypair(&mut rand::thread_rng());

        // Sign one hash but claim the proof is for a different hash — recover will
        // not return the expected signer.
        let (proof_bytes, _) = signed_proof(&secret, 1, B256::repeat_byte(0x11));
        let sub_proofs = vec![(proof_bytes, B256::repeat_byte(0x22))];

        let err = verify_sub_proofs(&sub_proofs).expect_err("bad sig");
        assert!(
            err.to_string().contains("signature verification failed"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn verify_sub_proofs_rejects_malformed_proof_bytes() {
        let sub_proofs = vec![(vec![0u8; TDX_PROOF_SIZE - 1], B256::ZERO)];
        let err = verify_sub_proofs(&sub_proofs).expect_err("malformed");
        assert!(err.to_string().contains("Invalid proof size"));
    }
}
