#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

//! Raiko V2 Prover SDKs
//!
//! This crate provides the prover implementations for generating zero-knowledge proofs
//! of Taiko block execution. It supports multiple proving backends:
//!
//! - **RISC0**: RISC-V zkVM prover
//! - **SP1**: Succinct zkVM prover
//! - **Gaiko2**: remote geth-backed TEE prover
//!
//! ## Usage
//!
//! ```rust,ignore
//! use raiko2_prover::gaiko2::Gaiko2Prover;
//! use raiko2_prover::risc0::Risc0Prover;
//! use raiko2_prover::sp1::Sp1Prover;
//!
//! // Create RISC0 prover
//! let risc0_prover = Risc0Prover::new(Default::default());
//!
//! // Create SP1 prover after loading the SP1 backend ELFs.
//! let sp1_prover = Sp1Prover::new_with_backend(Default::default(), &sp1_backend)?;
//! // Create a gaiko2 prover client
//! let gaiko2_prover = Gaiko2Prover::new(Default::default());
//! ```

#[cfg(feature = "boundless")]
pub mod boundless;
pub mod gaiko2;
pub mod native;
pub mod remote_prover;
#[cfg(feature = "risc0")]
pub mod risc0;
#[cfg(any(feature = "risc0", feature = "boundless", test))]
mod risc0_aggregation;
#[cfg(feature = "sp1")]
pub mod sp1;
#[cfg(feature = "sp1")]
pub use sp1::{
    Sp1FulfillmentStrategy, Sp1NetworkMetadata, Sp1NetworkMode, Sp1NetworkSubmissionProgress,
};

#[cfg(any(feature = "risc0", feature = "boundless", test))]
use alloy::sol_types::SolValue;
#[cfg(any(feature = "risc0", feature = "sp1", feature = "boundless", test))]
use alloy_primitives::B256;
use alloy_primitives::Bytes;
use raiko2_pipeline::{PipelineRoute, ProverBackend};
use raiko2_primitives::{AggregationGuestInput, Proof, ProverConfig, RaikoError, RaikoResult};
use raiko2_primitives_shasta::{
    ShastaZkAggregationGuestInput, encode_proof_carry_data, proof_carry_from_proof,
};
use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
use raiko2_protocol_shasta::shasta::ProofCarryData;
#[cfg(feature = "risc0")]
use risc0_ethereum_contracts_boundless::encode_seal;
#[cfg(feature = "risc0")]
use risc0_zkvm::Receipt as Risc0Receipt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Encoding helper for guest inputs.
pub trait GuestInputCodec<I>: Send + Sync {
    /// # Errors
    ///
    /// Returns an error if the input cannot be encoded.
    fn encode(&self, input: &I, config: &ProverConfig) -> RaikoResult<Bytes>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoundlessSubmissionProgress {
    pub provider_request_id: String,
    pub remote_tx_hash: Option<String>,
    pub expires_at: u64,
    pub image_ref: String,
    pub deployment: String,
    pub offchain: bool,
    pub quoted_mcycles_count: Option<u32>,
    pub evaluated_mcycles_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoundlessSubmissionResume {
    pub provider_request_id: String,
    pub remote_tx_hash: Option<String>,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProverProgress {
    BoundlessSubmission(BoundlessSubmissionProgress),
    #[cfg(feature = "sp1")]
    Sp1NetworkSubmission(Sp1NetworkSubmissionProgress),
}

#[async_trait::async_trait]
pub trait ProverProgressObserver: Send + Sync {
    async fn on_progress(&self, progress: &ProverProgress);

    async fn load_sp1_network_request_id(&self) -> Option<String> {
        None
    }

    async fn load_boundless_submission(&self) -> Option<BoundlessSubmissionResume> {
        None
    }
}

#[cfg(any(feature = "risc0", feature = "sp1", feature = "boundless", test))]
const B256_BYTES: usize = 32;
#[cfg(any(feature = "risc0", feature = "boundless"))]
pub(crate) const RISC0_SEAL_PAYLOAD_KIND: &str = "risc0_seal";

#[cfg(any(feature = "risc0", feature = "sp1", feature = "boundless", test))]
pub(crate) fn parse_shasta_proposal_input_hash(public_values: &[u8]) -> RaikoResult<B256> {
    if public_values.len() == B256_BYTES {
        Ok(B256::from_slice(public_values))
    } else {
        Err(RaikoError::Guest(format!(
            "invalid Shasta proposal journal length: expected {B256_BYTES} bytes, got {}",
            public_values.len()
        )))
    }
}

#[cfg(any(feature = "risc0", feature = "sp1", feature = "boundless", test))]
pub(crate) fn parse_shasta_aggregation_input_hash(public_values: &[u8]) -> RaikoResult<B256> {
    if public_values.len() >= B256_BYTES {
        Ok(B256::from_slice(&public_values[..B256_BYTES]))
    } else {
        Err(RaikoError::Guest(format!(
            "invalid Shasta aggregation journal length: expected at least {B256_BYTES} bytes, got {}",
            public_values.len()
        )))
    }
}

#[cfg(any(feature = "risc0", feature = "boundless", test))]
pub(crate) fn encode_risc0_proposal_seal_payload(seal: &[u8], image_id: B256) -> String {
    let proof: Vec<u8> = (seal.to_vec(), image_id)
        .abi_encode()
        .into_iter()
        .skip(32)
        .collect();
    alloy_primitives::hex::encode_prefixed(proof)
}

#[cfg(any(feature = "risc0", feature = "boundless", test))]
pub(crate) fn encode_risc0_aggregation_seal_payload(
    seal: &[u8],
    block_image_id: B256,
    aggregation_image_id: B256,
) -> String {
    let proof: Vec<u8> = (seal.to_vec(), block_image_id, aggregation_image_id)
        .abi_encode()
        .into_iter()
        .skip(32)
        .collect();
    alloy_primitives::hex::encode_prefixed(proof)
}

#[cfg(feature = "risc0")]
pub(crate) fn encode_risc0_proposal_proof_payload(
    receipt: &Risc0Receipt,
    image_id: B256,
) -> String {
    encode_seal(receipt).map_or_else(
        |_| alloy_primitives::hex::encode_prefixed(&receipt.journal.bytes),
        |seal| encode_risc0_proposal_seal_payload(&seal, image_id),
    )
}

#[cfg(feature = "risc0")]
pub(crate) fn encode_risc0_aggregation_proof_payload(
    receipt: &Risc0Receipt,
    block_image_id: B256,
    aggregation_image_id: B256,
) -> String {
    encode_seal(receipt).map_or_else(
        |_| alloy_primitives::hex::encode_prefixed(&receipt.journal.bytes),
        |seal| encode_risc0_aggregation_seal_payload(&seal, block_image_id, aggregation_image_id),
    )
}

#[cfg(any(feature = "risc0", feature = "boundless"))]
pub(crate) fn decode_hex_payload(value: Option<&str>) -> Vec<u8> {
    value
        .and_then(|raw| alloy_primitives::hex::decode(raw.strip_prefix("0x").unwrap_or(raw)).ok())
        .unwrap_or_default()
}

pub(crate) fn build_shasta_aggregation_input(
    proofs: &[Proof],
) -> Result<ShastaZkAggregationGuestInput, RaikoError> {
    let image_id = shasta_aggregation_image_id_words(proofs)?;
    let mut block_inputs = Vec::with_capacity(proofs.len());
    let mut proof_carry_data_vec = Vec::with_capacity(proofs.len());

    for (index, proof) in proofs.iter().enumerate() {
        let carry = proof_carry_from_proof(proof)
            .map_err(|err| {
                RaikoError::InvalidRequestConfig(format!(
                    "proof {index} invalid shasta carry data: {err}"
                ))
            })?
            .ok_or_else(|| {
                RaikoError::InvalidRequestConfig(format!("proof {index} missing shasta carry data"))
            })?;
        let expected_input = hash_shasta_subproof_input(&carry);
        if let Some(input_hash) = proof.input
            && input_hash != expected_input
        {
            return Err(RaikoError::InvalidRequestConfig(format!(
                "proof {index} input hash does not match shasta carry data"
            )));
        }
        block_inputs.push(expected_input);
        proof_carry_data_vec.push(carry);
    }

    Ok(ShastaZkAggregationGuestInput {
        image_id,
        block_inputs,
        proof_carry_data_vec,
        prover_address: alloy_primitives::Address::ZERO,
    })
}

pub(crate) fn with_shasta_extra_data(
    carry: &ProofCarryData,
    namespace: &str,
    metadata: Option<serde_json::Value>,
) -> RaikoResult<Option<serde_json::Value>> {
    let mut extra_data = encode_proof_carry_data(carry)?;
    if let Some(metadata) = metadata
        && let Some(root) = extra_data.as_object_mut()
    {
        root.insert(namespace.to_string(), metadata);
    }
    Ok(Some(extra_data))
}

fn shasta_aggregation_image_id_words(proofs: &[Proof]) -> Result<[u32; 8], RaikoError> {
    let mut image_id = None;
    for (index, proof) in proofs.iter().enumerate() {
        let Some(uuid) = proof.uuid.as_deref() else {
            continue;
        };
        let words = shasta_image_id_words_from_uuid(uuid).map_err(|err| {
            RaikoError::InvalidRequestConfig(format!("proof {index} invalid uuid/image id: {err}"))
        })?;
        match image_id {
            Some(existing) if existing != words => {
                return Err(RaikoError::InvalidRequestConfig(
                    "proofs do not share the same image id".to_string(),
                ));
            }
            Some(_) => {}
            None => image_id = Some(words),
        }
    }

    Ok(image_id.unwrap_or([0; 8]))
}

pub(crate) fn shasta_image_id_words_from_uuid(raw: &str) -> Result<[u32; 8], String> {
    #[cfg(feature = "sp1")]
    {
        crate::sp1::sp1_image_id_words_from_uuid(raw)
    }

    #[cfg(not(feature = "sp1"))]
    {
        let bytes =
            alloy_primitives::hex::decode(raw).map_err(|err| format!("invalid hex uuid: {err}"))?;
        if bytes.len() != 32 {
            return Err(format!(
                "expected 32-byte hex image id, got {}",
                bytes.len()
            ));
        }

        let mut words = [0u32; 8];
        for (index, chunk) in bytes.chunks_exact(4).enumerate() {
            let mut word = [0u8; 4];
            word.copy_from_slice(chunk);
            words[index] = u32::from_le_bytes(word);
        }
        Ok(words)
    }
}

/// # Errors
///
/// Returns an error when the supplied proofs do not satisfy the route-specific external
/// aggregation admission contract.
pub fn validate_external_aggregate_proofs(
    route: PipelineRoute,
    proofs: &[Proof],
) -> Result<(), RaikoError> {
    let pipeline_key = route
        .pipeline_key()
        .map_err(RaikoError::InvalidRequestConfig)?;

    for (index, proof) in proofs.iter().enumerate() {
        match pipeline_key {
            raiko2_pipeline::PipelineKey::ShastaNative => {
                if proof.input.is_none() || proof.extra_data.is_none() {
                    return Err(RaikoError::InvalidRequestConfig(format!(
                        "proof {index} is missing native aggregation metadata"
                    )));
                }
            }
            raiko2_pipeline::PipelineKey::ShastaSgx
            | raiko2_pipeline::PipelineKey::ShastaSgxGeth => {
                if proof.input.is_none() || proof.extra_data.is_none() || proof.proof.is_none() {
                    return Err(RaikoError::InvalidRequestConfig(format!(
                        "proof {index} is missing SGX aggregation metadata"
                    )));
                }
            }
            raiko2_pipeline::PipelineKey::ShastaSp1 => {
                if proof.input.is_none()
                    || proof.extra_data.is_none()
                    || proof.uuid.is_none()
                    || (proof.quote.is_none() && proof.proof.is_none())
                {
                    return Err(RaikoError::InvalidRequestConfig(format!(
                        "proof {index} is missing SP1 aggregation metadata"
                    )));
                }
            }
            raiko2_pipeline::PipelineKey::ShastaRisc0 => {
                if proof.input.is_none()
                    || proof.extra_data.is_none()
                    || proof.uuid.is_none()
                    || proof.quote.is_none()
                {
                    return Err(RaikoError::InvalidRequestConfig(format!(
                        "proof {index} is missing RISC0 aggregation metadata"
                    )));
                }
            }
            raiko2_pipeline::PipelineKey::ShastaRisc0Network => {
                if proof.quote.is_none() || proof.extra_data.is_none() {
                    return Err(RaikoError::InvalidRequestConfig(format!(
                        "proof {index} is missing Boundless aggregation metadata"
                    )));
                }
                proof_carry_from_proof(proof)
                    .map_err(|err| {
                        RaikoError::InvalidRequestConfig(format!(
                            "proof {index} invalid shasta carry data: {err}"
                        ))
                    })?
                    .ok_or_else(|| {
                        RaikoError::InvalidRequestConfig(format!(
                            "proof {index} missing shasta carry data"
                        ))
                    })?;
            }
        }
    }

    Ok(())
}

/// Common prover trait for all proving backends.
#[async_trait::async_trait]
pub trait Prover<B>: Send + Sync
where
    B: ProverBackend,
{
    type GuestInput: Send + Sync + 'static;

    /// # Errors
    ///
    /// Returns an error if the input cannot be encoded.
    fn encode(&self, input: &Self::GuestInput, config: &ProverConfig) -> RaikoResult<Bytes>;

    /// Generate a proof for the given input.
    async fn prove_encoded(
        &self,
        input: Bytes,
        config: &ProverConfig,
        backend: &B,
    ) -> RaikoResult<Proof>;

    async fn prove_encoded_with_observer(
        &self,
        input: Bytes,
        config: &ProverConfig,
        backend: &B,
        observer: Option<Arc<dyn ProverProgressObserver>>,
    ) -> RaikoResult<Proof> {
        let _ = observer;
        self.prove_encoded(input, config, backend).await
    }

    async fn prove(
        &self,
        input: Self::GuestInput,
        config: &ProverConfig,
        backend: &B,
    ) -> RaikoResult<Proof> {
        let encoded = self.encode(&input, config)?;
        self.prove_encoded(encoded, config, backend).await
    }

    /// Generate an aggregation proof.
    async fn aggregate(
        &self,
        input: AggregationGuestInput,
        config: &ProverConfig,
        backend: &B,
    ) -> RaikoResult<Proof>;

    async fn aggregate_with_observer(
        &self,
        input: AggregationGuestInput,
        config: &ProverConfig,
        backend: &B,
        observer: Option<Arc<dyn ProverProgressObserver>>,
    ) -> RaikoResult<Proof> {
        let _ = observer;
        self.aggregate(input, config, backend).await
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_hex_payload, encode_proof_carry_data, encode_risc0_aggregation_seal_payload,
        encode_risc0_proposal_seal_payload, parse_shasta_aggregation_input_hash,
        parse_shasta_proposal_input_hash, validate_external_aggregate_proofs,
    };
    use alloy_primitives::B256;
    use alloy_sol_types::SolValue;
    use raiko2_pipeline::PipelineRoute;
    use raiko2_primitives::Proof;
    use raiko2_protocol_shasta::shasta::ProofCarryData;

    #[test]
    fn parses_shasta_proposal_input_hash_from_first_committed_word() {
        let subproof_input_hash = B256::repeat_byte(0x22);
        let public_values = subproof_input_hash.as_slice().to_vec();

        assert_eq!(
            parse_shasta_proposal_input_hash(&public_values).expect("parse proposal input hash"),
            subproof_input_hash
        );
    }

    #[test]
    fn rejects_non_exact_shasta_proposal_public_input_length() {
        let err = parse_shasta_proposal_input_hash(&[0u8; 64]).expect_err("reject");
        assert!(err.to_string().contains("expected 32 bytes"));
    }

    #[test]
    fn parses_shasta_aggregation_input_hash_from_first_committed_word() {
        let agg_input_hash = B256::repeat_byte(0x33);
        let public_values = agg_input_hash.as_slice().to_vec();

        assert_eq!(
            parse_shasta_aggregation_input_hash(&public_values)
                .expect("parse aggregation input hash"),
            agg_input_hash
        );
    }

    #[test]
    fn rejects_short_shasta_aggregation_public_input_length() {
        let err = parse_shasta_aggregation_input_hash(&[0u8; 31]).expect_err("reject");
        assert!(err.to_string().contains("expected at least 32 bytes"));
    }

    fn aggregate_proof_fixture() -> Proof {
        Proof {
            proof: Some("0xproof".to_string()),
            input: Some(B256::repeat_byte(0x11)),
            quote: Some("0xquote".to_string()),
            uuid: Some("0xuuid".to_string()),
            kzg_proof: None,
            extra_data: Some(
                encode_proof_carry_data(&ProofCarryData::default()).expect("encode carry data"),
            ),
        }
    }

    #[test]
    fn aggregate_validator_accepts_native_local_proof() {
        let route = "native/local"
            .parse::<PipelineRoute>()
            .expect("parse route");
        assert!(validate_external_aggregate_proofs(route, &[aggregate_proof_fixture()]).is_ok());
    }

    #[test]
    fn aggregate_validator_accepts_sgx_remote_proof() {
        let route = "sgx/remote".parse::<PipelineRoute>().expect("parse route");
        assert!(validate_external_aggregate_proofs(route, &[aggregate_proof_fixture()]).is_ok());
    }

    #[test]
    fn aggregate_validator_rejects_missing_sgx_remote_proof_bytes() {
        let route = "sgx/remote".parse::<PipelineRoute>().expect("parse route");
        let mut proof = aggregate_proof_fixture();
        proof.proof = None;

        let err =
            validate_external_aggregate_proofs(route, &[proof]).expect_err("missing proof bytes");
        assert!(
            err.to_string()
                .contains("proof 0 is missing SGX aggregation metadata")
        );
    }

    #[test]
    fn aggregate_validator_rejects_missing_sp1_fields() {
        let route = "sp1/local".parse::<PipelineRoute>().expect("parse route");
        let mut proof = aggregate_proof_fixture();
        proof.uuid = None;

        let err = validate_external_aggregate_proofs(route, &[proof]).expect_err("missing uuid");
        assert!(
            err.to_string()
                .contains("proof 0 is missing SP1 aggregation metadata")
        );
    }

    #[test]
    fn aggregate_validator_rejects_sp1_proof_without_quote_or_legacy_payload() {
        let route = "sp1/local".parse::<PipelineRoute>().expect("parse route");
        let mut proof = aggregate_proof_fixture();
        proof.proof = None;
        proof.quote = None;

        let err =
            validate_external_aggregate_proofs(route, &[proof]).expect_err("missing proof data");
        assert!(
            err.to_string()
                .contains("proof 0 is missing SP1 aggregation metadata")
        );
    }

    #[test]
    fn aggregate_validator_rejects_missing_risc0_local_fields() {
        let route = "risc0/local".parse::<PipelineRoute>().expect("parse route");
        let mut proof = aggregate_proof_fixture();
        proof.quote = None;

        let err = validate_external_aggregate_proofs(route, &[proof]).expect_err("missing receipt");
        assert!(
            err.to_string()
                .contains("proof 0 is missing RISC0 aggregation metadata")
        );
    }

    #[test]
    fn aggregate_validator_rejects_missing_boundless_receipt() {
        let route = "risc0/network"
            .parse::<PipelineRoute>()
            .expect("parse route");
        let mut proof = aggregate_proof_fixture();
        proof.quote = None;

        let err = validate_external_aggregate_proofs(route, &[proof]).expect_err("missing receipt");
        assert!(
            err.to_string()
                .contains("proof 0 is missing Boundless aggregation metadata")
        );
    }

    #[test]
    fn aggregate_validator_rejects_boundless_proof_without_carry_data() {
        let route = "risc0/network"
            .parse::<PipelineRoute>()
            .expect("parse route");
        let proof = Proof {
            proof: None,
            input: None,
            quote: Some("0xreceipt".to_string()),
            uuid: None,
            kzg_proof: None,
            extra_data: None,
        };

        let err = validate_external_aggregate_proofs(route, &[proof]).expect_err("missing carry");
        assert!(
            err.to_string()
                .contains("proof 0 is missing Boundless aggregation metadata")
        );
    }

    #[test]
    fn aggregate_validator_accepts_boundless_proof_with_receipt_and_carry_data() {
        let route = "risc0/network"
            .parse::<PipelineRoute>()
            .expect("parse route");
        let proof = Proof {
            proof: None,
            input: None,
            quote: Some("0xreceipt".to_string()),
            uuid: None,
            kzg_proof: None,
            extra_data: Some(
                encode_proof_carry_data(&ProofCarryData::default()).expect("encode carry data"),
            ),
        };

        assert!(validate_external_aggregate_proofs(route, &[proof]).is_ok());
    }

    #[test]
    fn risc0_proposal_payload_encodes_seal_and_image_id() {
        let seal = vec![0x11, 0x22, 0x33];
        let image_id = B256::repeat_byte(0xaa);

        let encoded =
            decode_hex_payload(Some(&encode_risc0_proposal_seal_payload(&seal, image_id)));
        let expected: Vec<u8> = (seal, image_id).abi_encode().into_iter().skip(32).collect();

        assert_eq!(encoded, expected);
    }

    #[test]
    fn risc0_aggregation_payload_encodes_seal_and_both_image_ids() {
        let seal = vec![0x44, 0x55, 0x66];
        let block_image_id = B256::repeat_byte(0xbb);
        let aggregation_image_id = B256::repeat_byte(0xcc);

        let encoded = decode_hex_payload(Some(&encode_risc0_aggregation_seal_payload(
            &seal,
            block_image_id,
            aggregation_image_id,
        )));
        let expected: Vec<u8> = (seal, block_image_id, aggregation_image_id)
            .abi_encode()
            .into_iter()
            .skip(32)
            .collect();

        assert_eq!(encoded, expected);
    }
}
