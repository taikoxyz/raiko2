//! Shasta proof helpers for carrying protocol data.

use raiko2_primitives::{Proof, RaikoResult};
use raiko2_protocol_shasta::shasta::ProofCarryData;

/// Encode `ProofCarryData` into JSON for storage in `Proof.extra_data`.
///
/// # Errors
///
/// Returns an error if `ProofCarryData` cannot be serialized to JSON.
pub fn encode_proof_carry_data(carry: &ProofCarryData) -> RaikoResult<serde_json::Value> {
    Ok(serde_json::to_value(carry)?)
}

/// Decode `ProofCarryData` from JSON stored in `Proof.extra_data`.
///
/// # Errors
///
/// Returns an error if the JSON payload cannot be deserialized into `ProofCarryData`.
pub fn decode_proof_carry_data(value: &serde_json::Value) -> RaikoResult<ProofCarryData> {
    Ok(serde_json::from_value(value.clone())?)
}

/// Decode optional `ProofCarryData` from `Proof.extra_data`.
///
/// # Errors
///
/// Returns an error if the JSON payload cannot be deserialized into `ProofCarryData`.
pub fn decode_proof_carry_data_opt(
    value: Option<&serde_json::Value>,
) -> RaikoResult<Option<ProofCarryData>> {
    match value {
        Some(v) => Ok(Some(decode_proof_carry_data(v)?)),
        None => Ok(None),
    }
}

/// Decode `ProofCarryData` from a `Proof` if present.
///
/// # Errors
///
/// Returns an error if `Proof.extra_data` cannot be deserialized into `ProofCarryData`.
#[allow(dead_code)]
pub fn proof_carry_from_proof(proof: &Proof) -> RaikoResult<Option<ProofCarryData>> {
    decode_proof_carry_data_opt(proof.extra_data.as_ref())
}
