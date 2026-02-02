#![allow(missing_docs)]

use raiko2_primitives::proof::{AggregationInput, VerifierArtifact};
use raiko2_primitives::{RaikoError, RaikoResult};
use risc0_zkvm::{Digest, Receipt as ZkvmReceipt};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct BoundlessAggregationGuestInput {
    image_id: Digest,
    receipts: Vec<ZkvmReceipt>,
}

/// Build the boundless aggregation input for a risc0 aggregation proof.
///
/// # Errors
/// Returns an error if any receipt artifact is missing or invalid, if the
/// expected image id is malformed, or if serialization fails.
pub fn build_risc0_aggregation_input(agg: &AggregationInput) -> RaikoResult<Vec<u8>> {
    let receipts = agg
        .proofs
        .iter()
        .map(|proof| extract_receipt(&proof.verifier_artifacts))
        .collect::<RaikoResult<Vec<_>>>()?;

    let image_id = match agg.expected_image_id.as_deref() {
        Some(raw) => parse_expected_image_id(raw)?,
        None => Digest::ZERO,
    };

    let input = BoundlessAggregationGuestInput {
        image_id,
        receipts,
    };

    bincode::serialize(&input)
        .map_err(|e| RaikoError::InvalidRequestConfig(format!("Failed to encode input: {e}")))
}

fn extract_receipt(artifacts: &[VerifierArtifact]) -> RaikoResult<ZkvmReceipt> {
    let artifact = artifacts
        .iter()
        .find(|artifact| artifact.kind == "receipt_json")
        .ok_or_else(|| {
            RaikoError::InvalidRequestConfig("Missing receipt_json verifier artifact".to_string())
        })?;

    let receipt_json = match &artifact.value {
        Value::String(value) => value.clone(),
        other => serde_json::to_string(other).map_err(|e| {
            RaikoError::InvalidRequestConfig(format!(
                "Failed to serialize receipt artifact: {e}"
            ))
        })?,
    };

    serde_json::from_str(&receipt_json).map_err(|e| {
        RaikoError::InvalidRequestConfig(format!("Failed to parse receipt JSON: {e}"))
    })
}

fn parse_expected_image_id(raw: &str) -> RaikoResult<Digest> {
    let trimmed = raw.strip_prefix("0x").unwrap_or(raw);
    let bytes = alloy_primitives::hex::decode(trimmed).map_err(|e| {
        RaikoError::InvalidRequestConfig(format!("Invalid expected_image_id hex: {e}"))
    })?;
    if bytes.len() != 32 {
        return Err(RaikoError::InvalidRequestConfig(format!(
            "Invalid expected_image_id length: expected 32 bytes, got {}",
            bytes.len()
        )));
    }
    Digest::try_from(bytes.as_slice()).map_err(|e| {
        RaikoError::InvalidRequestConfig(format!("Invalid expected_image_id bytes: {e}"))
    })
}
