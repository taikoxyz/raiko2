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
//!
//! ## Usage
//!
//! ```rust,ignore
//! use raiko2_prover::{risc0::Risc0Prover, sp1::Sp1Prover};
//!
//! // Create RISC0 prover
//! let risc0_prover = Risc0Prover::new(Default::default());
//!
//! // Create SP1 prover
//! let sp1_prover = Sp1Prover::new(Default::default());
//! ```

pub mod boundless;
pub mod native;
pub mod risc0;
pub mod sp1;
pub use sp1::{
    Sp1FulfillmentStrategy, Sp1NetworkMetadata, Sp1NetworkMode, Sp1NetworkSubmissionProgress,
};

use alloy_primitives::{B256, Bytes};
use raiko2_pipeline::ProverBackend;
use raiko2_primitives::{AggregationGuestInput, Proof, ProverConfig, RaikoError, RaikoResult};
use raiko2_primitives_shasta::{
    ShastaZkAggregationGuestInput, encode_proof_carry_data, proof_carry_from_proof,
};
use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
use raiko2_protocol_shasta::shasta::ProofCarryData;
use risc0_ethereum_contracts_boundless::encode_seal;
use risc0_zkvm::Receipt as Risc0Receipt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::sp1::sp1_image_id_words_from_uuid;
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
    pub image_ref: String,
    pub deployment: String,
    pub offchain: bool,
    pub quoted_mcycles_count: Option<u32>,
    pub evaluated_mcycles_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProverProgress {
    BoundlessSubmission(BoundlessSubmissionProgress),
    Sp1NetworkSubmission(Sp1NetworkSubmissionProgress),
}

#[async_trait::async_trait]
pub trait ProverProgressObserver: Send + Sync {
    async fn on_progress(&self, progress: &ProverProgress);

    async fn load_sp1_network_request_id(&self) -> Option<String> {
        None
    }
}

const B256_BYTES: usize = 32;
pub(crate) const RISC0_SEAL_PAYLOAD_KIND: &str = "risc0_seal";

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

pub(crate) fn parse_shasta_aggregation_input_hash(public_values: &[u8]) -> B256 {
    if public_values.len() >= B256_BYTES {
        B256::from_slice(&public_values[..B256_BYTES])
    } else {
        B256::default()
    }
}

pub(crate) fn encode_risc0_proof_payload(receipt: &Risc0Receipt) -> String {
    encode_seal(receipt).map_or_else(
        |_| alloy_primitives::hex::encode_prefixed(&receipt.journal.bytes),
        alloy_primitives::hex::encode_prefixed,
    )
}

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
        let words = sp1_image_id_words_from_uuid(uuid).map_err(|err| {
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
    use super::{parse_shasta_aggregation_input_hash, parse_shasta_proposal_input_hash};
    use alloy_primitives::B256;

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
            parse_shasta_aggregation_input_hash(&public_values),
            agg_input_hash
        );
    }
}
