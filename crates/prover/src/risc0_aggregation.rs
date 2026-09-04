use raiko2_primitives::proof::{
    AggregationInput, ProofEnvelope, ProofPayload, PublicInputs, VerifierArtifact,
};
use raiko2_primitives::{Proof, RaikoError, RaikoResult};
use raiko2_primitives_shasta::ShastaRisc0AggregationGuestInput;
use risc0_zkvm::{
    Digest as ZkvmDigest, InnerReceipt, MaybePruned, Receipt as ZkvmReceipt, VerifierContext,
};
use serde_json::Value;

use crate::{
    RISC0_SEAL_PAYLOAD_KIND, build_shasta_aggregation_input, decode_hex_payload,
    shasta_image_id_words_from_uuid,
};

/// Build the receipt-backed RISC0 aggregation input expected by the RISC0 aggregation guest.
///
/// # Errors
///
/// Returns an error if a receipt artifact is missing or invalid, if the expected image id does not
/// match the proof set, or if serialization fails.
pub fn build_risc0_aggregation_input(agg: &AggregationInput) -> RaikoResult<Vec<u8>> {
    let expected_image_id = agg.expected_image_id.as_deref().ok_or_else(|| {
        RaikoError::InvalidRequestConfig("RISC0 aggregation requires expected_image_id".to_string())
    })?;
    let proofs = agg
        .proofs
        .iter()
        .map(|proof| proof_from_envelope(proof, expected_image_id))
        .collect::<RaikoResult<Vec<_>>>()?;
    let canonical_input = build_shasta_aggregation_input(&proofs)?;

    let expected_words = shasta_image_id_words_from_uuid(expected_image_id).map_err(|err| {
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
                    RaikoError::InvalidRequestConfig(format!("Failed to encode receipt: {e}"))
                })
            })
        })
        .collect::<RaikoResult<Vec<_>>>()?;

    let input = ShastaRisc0AggregationGuestInput {
        image_id: canonical_input.image_id,
        proof_carry_data_vec: canonical_input.proof_carry_data_vec,
        receipts,
        prover_address: canonical_input.prover_address,
    };
    bincode::serialize(&input)
        .map_err(|e| RaikoError::InvalidRequestConfig(format!("Failed to encode input: {e}")))
}

pub(crate) fn build_risc0_aggregation_input_from_proofs(
    proofs: Vec<Proof>,
    expected_image_id: ZkvmDigest,
) -> RaikoResult<Vec<u8>> {
    let agg = AggregationInput {
        proofs: proofs.into_iter().map(proof_to_envelope).collect(),
        expected_image_id: Some(alloy_primitives::hex::encode_prefixed(
            expected_image_id.as_bytes(),
        )),
        metadata: None,
    };
    build_risc0_aggregation_input(&agg)
}

pub(crate) fn proof_to_envelope(proof: Proof) -> ProofEnvelope {
    let mut verifier_artifacts = Vec::new();
    if let Some(receipt) = proof.quote {
        verifier_artifacts.push(VerifierArtifact {
            kind: "receipt_json".to_string(),
            value: serde_json::Value::String(receipt),
        });
    }
    let input_hash = proof
        .input
        .map(|value| alloy_primitives::hex::encode_prefixed(value.as_slice()));
    let carry_data = proof.extra_data;
    let payload_bytes = decode_hex_payload(proof.proof.as_deref());

    ProofEnvelope {
        backend: "risc0".to_string(),
        public_inputs: PublicInputs {
            input_hash,
            instance_hash: None,
        },
        payload: ProofPayload {
            payload_kind: RISC0_SEAL_PAYLOAD_KIND.to_string(),
            bytes: payload_bytes,
        },
        verifier_artifacts,
        carry_data,
        metadata: None,
    }
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

fn proof_from_envelope(envelope: &ProofEnvelope, expected_image_id: &str) -> RaikoResult<Proof> {
    let receipt = extract_receipt(&envelope.verifier_artifacts)?;
    let actual_image_id = validate_receipt_image_id(&receipt, expected_image_id)?;
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

    Ok(Proof {
        proof: Some(alloy_primitives::hex::encode_prefixed(
            &envelope.payload.bytes,
        )),
        input,
        quote,
        uuid: Some(alloy_primitives::hex::encode_prefixed(
            actual_image_id.as_bytes(),
        )),
        kzg_proof: None,
        extra_data: envelope.carry_data.clone(),
    })
}

fn validate_receipt_image_id(
    receipt: &ZkvmReceipt,
    expected_image_id: &str,
) -> RaikoResult<ZkvmDigest> {
    let expected = parse_expected_image_id(expected_image_id)?;
    match receipt_image_id(receipt) {
        Ok(actual) => {
            if actual != expected {
                return Err(RaikoError::InvalidRequestConfig(format!(
                    "receipt image id does not match expected_image_id: expected {}, got {}",
                    alloy_primitives::hex::encode_prefixed(expected.as_bytes()),
                    alloy_primitives::hex::encode_prefixed(actual.as_bytes()),
                )));
            }
            Ok(actual)
        }
        Err(image_id_error) => {
            verify_receipt_against_expected_image_id(receipt, expected).map_err(
                |verify_error| {
                    RaikoError::InvalidRequestConfig(format!(
                        "receipt image id could not be determined ({image_id_error}) and \
                     verification against expected_image_id failed: {verify_error}"
                    ))
                },
            )?;
            Ok(expected)
        }
    }
}

fn parse_expected_image_id(raw: &str) -> RaikoResult<ZkvmDigest> {
    let trimmed = raw.strip_prefix("0x").unwrap_or(raw);
    let bytes = alloy_primitives::hex::decode(trimmed).map_err(|err| {
        RaikoError::InvalidRequestConfig(format!("Invalid expected_image_id/image id: {err}"))
    })?;
    if bytes.len() != 32 {
        return Err(RaikoError::InvalidRequestConfig(format!(
            "Invalid expected_image_id/image id length: expected 32 bytes, got {}",
            bytes.len()
        )));
    }
    ZkvmDigest::try_from(bytes.as_slice()).map_err(|err| {
        RaikoError::InvalidRequestConfig(format!("Invalid expected_image_id/image id: {err}"))
    })
}

fn receipt_image_id(receipt: &ZkvmReceipt) -> RaikoResult<ZkvmDigest> {
    let claim = receipt.claim().map_err(|err| {
        RaikoError::InvalidRequestConfig(format!("Failed to inspect receipt claim: {err}"))
    })?;
    let claim = match claim {
        MaybePruned::Value(claim) => claim,
        MaybePruned::Pruned(_) => {
            return Err(RaikoError::InvalidRequestConfig(
                "receipt claim is pruned and does not expose an image id".to_string(),
            ));
        }
    };

    Ok(match claim.pre {
        MaybePruned::Value(pre_state) => pre_state.merkle_root,
        MaybePruned::Pruned(image_id) => image_id,
    })
}

fn allow_fake_receipts_for_verification() -> bool {
    cfg!(debug_assertions)
        || std::env::var("RAIKO_ALLOW_FAKE_RISC0_RECEIPTS").is_ok_and(|value| {
            value.eq_ignore_ascii_case("1")
                || value.eq_ignore_ascii_case("true")
                || value.eq_ignore_ascii_case("yes")
                || value.eq_ignore_ascii_case("on")
        })
}

fn verify_receipt_against_expected_image_id(
    receipt: &ZkvmReceipt,
    expected_image_id: ZkvmDigest,
) -> RaikoResult<()> {
    let result = match &receipt.inner {
        InnerReceipt::Fake(_) => {
            if !allow_fake_receipts_for_verification() {
                return Err(RaikoError::InvalidRequestConfig(
                    "fake RISC0 receipts are not accepted unless explicitly enabled for development"
                        .to_string(),
                ));
            }
            receipt.verify_with_context(
                &VerifierContext::default().with_dev_mode(true),
                expected_image_id,
            )
        }
        _ => receipt.verify(expected_image_id),
    };
    result.map_err(|err| {
        RaikoError::InvalidRequestConfig(format!(
            "receipt image id does not match expected_image_id: {err}"
        ))
    })
}
