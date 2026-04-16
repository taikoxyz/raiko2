use raiko2_primitives::proof::{AggregationInput, VerifierArtifact};
use raiko2_primitives::{RaikoError, RaikoResult};
use raiko2_primitives_shasta::ShastaBoundlessAggregationGuestInput;
use risc0_zkvm::Receipt as ZkvmReceipt;
use serde_json::Value;

use crate::{build_shasta_aggregation_input, sp1::sp1_image_id_words_from_uuid};

/// Build the receipt-backed RISC0 aggregation input expected by the Boundless aggregation guest.
///
/// # Errors
///
/// Returns an error if a receipt artifact is missing or invalid, if the expected image id does not
/// match the proof set, or if serialization fails.
pub fn build_risc0_aggregation_input(agg: &AggregationInput) -> RaikoResult<Vec<u8>> {
    let expected_image_id = agg.expected_image_id.as_deref().ok_or_else(|| {
        RaikoError::InvalidRequestConfig(
            "boundless aggregation requires expected_image_id".to_string(),
        )
    })?;
    let proofs = agg
        .proofs
        .iter()
        .map(|proof| proof_from_envelope(proof, expected_image_id))
        .collect::<RaikoResult<Vec<_>>>()?;
    let canonical_input = build_shasta_aggregation_input(&proofs)?;

    let expected_words = sp1_image_id_words_from_uuid(expected_image_id).map_err(|err| {
        RaikoError::InvalidRequestConfig(format!("Invalid expected_image_id/image id: {err}"))
    })?;
    if expected_words != canonical_input.image_id {
        return Err(RaikoError::InvalidRequestConfig(
            "expected_image_id does not match aggregation proof image id".to_string(),
        ));
    }

    let receipts = agg
        .proofs
        .iter()
        .map(|proof| extract_receipt(&proof.verifier_artifacts))
        .map(|receipt| {
            receipt.and_then(|receipt| {
                bincode::serialize(&receipt).map_err(|e| {
                    RaikoError::InvalidRequestConfig(format!(
                        "Failed to encode receipt for boundless aggregation: {e}"
                    ))
                })
            })
        })
        .collect::<RaikoResult<Vec<_>>>()?;

    let input = ShastaBoundlessAggregationGuestInput {
        image_id: canonical_input.image_id,
        proof_carry_data_vec: canonical_input.proof_carry_data_vec,
        receipts,
        prover_address: canonical_input.prover_address,
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
            RaikoError::InvalidRequestConfig(format!("Failed to serialize receipt artifact: {e}"))
        })?,
    };

    serde_json::from_str(&receipt_json)
        .map_err(|e| RaikoError::InvalidRequestConfig(format!("Failed to parse receipt JSON: {e}")))
}

fn proof_from_envelope(
    envelope: &raiko2_primitives::proof::ProofEnvelope,
    expected_image_id: &str,
) -> RaikoResult<raiko2_primitives::Proof> {
    let input = envelope
        .public_inputs
        .input_hash
        .as_deref()
        .map(|value| {
            let trimmed = value.strip_prefix("0x").unwrap_or(value);
            let bytes = alloy_primitives::hex::decode(trimmed).map_err(|err| {
                RaikoError::InvalidRequestConfig(format!(
                    "Invalid aggregation proof input hash hex: {err}"
                ))
            })?;
            if bytes.len() != 32 {
                return Err(RaikoError::InvalidRequestConfig(format!(
                    "Invalid aggregation proof input hash length: expected 32 bytes, got {}",
                    bytes.len()
                )));
            }
            Ok(alloy_primitives::B256::from_slice(&bytes))
        })
        .transpose()?;

    let quote = envelope
        .verifier_artifacts
        .iter()
        .find(|artifact| artifact.kind == "receipt_json")
        .map(|artifact| match &artifact.value {
            Value::String(value) => Ok(value.clone()),
            other => serde_json::to_string(other).map_err(|err| {
                RaikoError::InvalidRequestConfig(format!(
                    "Failed to serialize receipt_json verifier artifact: {err}"
                ))
            }),
        })
        .transpose()?;

    Ok(raiko2_primitives::Proof {
        proof: Some(alloy_primitives::hex::encode_prefixed(
            &envelope.payload.bytes,
        )),
        input,
        quote,
        uuid: Some(expected_image_id.to_string()),
        kzg_proof: None,
        extra_data: envelope.carry_data.clone(),
    })
}
