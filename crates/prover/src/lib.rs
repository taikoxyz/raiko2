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

use alloy_primitives::Bytes;
use raiko2_pipeline::ProverBackend;
use raiko2_primitives::{AggregationGuestInput, Proof, ProverConfig, RaikoError, RaikoResult};
use raiko2_primitives_shasta::{ShastaZkAggregationGuestInput, encode_proof_carry_data};
use raiko2_protocol_shasta::shasta::ProofCarryData;
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
    pub image_ref: String,
    pub deployment: String,
    pub offchain: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProverProgress {
    BoundlessSubmission(BoundlessSubmissionProgress),
}

#[async_trait::async_trait]
pub trait ProverProgressObserver: Send + Sync {
    async fn on_progress(&self, progress: &ProverProgress);
}

pub(crate) fn parse_shasta_aggregation_input(
    config: &ProverConfig,
) -> Result<ShastaZkAggregationGuestInput, RaikoError> {
    serde_json::from_value(
        config
            .get("shasta_zk_aggregation_input")
            .cloned()
            .ok_or_else(|| {
                RaikoError::InvalidRequestConfig(
                    "Missing 'shasta_zk_aggregation_input' in config".to_string(),
                )
            })?,
    )
    .map_err(|e| {
        RaikoError::InvalidRequestConfig(format!("Failed to parse aggregation input: {e}"))
    })
}

pub(crate) fn validate_shasta_aggregation_lengths(
    aggregation_input: &ShastaZkAggregationGuestInput,
) -> Result<(), RaikoError> {
    if aggregation_input.block_inputs.len() != aggregation_input.proof_carry_data_vec.len() {
        return Err(RaikoError::InvalidRequestConfig(
            "Mismatched block_inputs and proof_carry_data_vec lengths".to_string(),
        ));
    }

    Ok(())
}

pub(crate) fn parse_shasta_proof_carry_data(
    config: &ProverConfig,
) -> Result<ProofCarryData, RaikoError> {
    serde_json::from_value(
        config
            .get("shasta_proof_carry_data")
            .cloned()
            .ok_or_else(|| {
                RaikoError::InvalidRequestConfig(
                    "Missing 'shasta_proof_carry_data' in config".to_string(),
                )
            })?,
    )
    .map_err(|e| RaikoError::InvalidRequestConfig(format!("Failed to parse proof carry data: {e}")))
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

    /// Update request-scoped prover config derived from the validated guest input.
    ///
    /// Backends that need extra request metadata, such as `ProofCarryData`, should populate it
    /// here so `prove_encoded` and `aggregate` can read a canonical config shape.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend cannot derive a valid request-scoped configuration from
    /// the guest input.
    fn prepare_config_for_input(
        &self,
        _input: &Self::GuestInput,
        _config: &mut ProverConfig,
    ) -> RaikoResult<()> {
        Ok(())
    }

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
        let mut request_config = config.clone();
        self.prepare_config_for_input(&input, &mut request_config)?;
        let encoded = self.encode(&input, &request_config)?;
        self.prove_encoded(encoded, &request_config, backend).await
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
