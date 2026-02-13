//! TDX proof construction and attestation quote generation.
//!
//! Contains structured proof types (`TdxProof`, `TdxAggregationProof`) and
//! functions for generating TDX attestation quotes and proving.

#![allow(dead_code)]

use alloy_primitives::{Address, B256};
use anyhow::{Result, anyhow};
use rand::Rng;
use tracing::info;

use crate::tdx::{
    attestation_client,
    config::load_private_key,
    signature::{address_from_private_key, recover_signer, sign_message},
};

/// Size of a TDX single proof: 4 (`instance_id`) + 20 (address) + 65 (signature).
pub const TDX_PROOF_SIZE: usize = 89;

/// Size of a TDX aggregation proof: 4 (`instance_id`) + 20 (old) + 20 (new) + 65 (signature).
pub const TDX_AGGREGATION_PROOF_SIZE: usize = 109;

// ────────────────────────── Proof structures ──────────────────────────

/// A single TDX proof (89 bytes).
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

/// A TDX aggregation proof (109 bytes).
pub struct TdxAggregationProof {
    data: [u8; TDX_AGGREGATION_PROOF_SIZE],
}

impl TdxAggregationProof {
    /// Build an aggregation proof from its components.
    #[must_use]
    pub fn new(
        instance_id: u32,
        old_instance: &Address,
        new_instance: &Address,
        signature: &[u8; 65],
    ) -> Self {
        let mut data = [0u8; TDX_AGGREGATION_PROOF_SIZE];
        data[0..4].copy_from_slice(&instance_id.to_be_bytes());
        data[4..24].copy_from_slice(old_instance.as_slice());
        data[24..44].copy_from_slice(new_instance.as_slice());
        data[44..109].copy_from_slice(signature);
        Self { data }
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
    // Verify all sub-proofs are signed by the same instance
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

    // Sign the aggregation hash with the current instance's key
    let private_key = load_private_key()?;
    let new_instance = address_from_private_key(&private_key);

    let signature = sign_message(&private_key, &aggregation_hash)?;
    let proof = TdxProof::new(instance_id, &new_instance, &signature).into_vec();
    let (quote, _nonce) = generate_tdx_quote(socket_path, &aggregation_hash)?;

    Ok(ProveAggregationData {
        proof,
        quote,
        aggregation_hash,
    })
}
