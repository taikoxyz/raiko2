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

pub mod agent;
pub mod native;
pub mod risc0;
pub mod sp1;

use alloy_primitives::Bytes;
use raiko2_pipeline::ProverBackend;
use raiko2_primitives::{AggregationGuestInput, Proof, ProverConfig, RaikoError, RaikoResult};
use raiko2_primitives_shasta::ShastaZkAggregationGuestInput;
use raiko2_protocol_shasta::shasta::ProofCarryData;
use serde_json::Value;

/// Encoding helper for guest inputs.
pub trait GuestInputCodec<I>: Send + Sync {
    /// # Errors
    ///
    /// Returns an error if the input cannot be encoded.
    fn encode(&self, input: &I, config: &ProverConfig) -> RaikoResult<Bytes>;
}

fn config_value(config: &ProverConfig, key: &str) -> Value {
    config.get(key).cloned().unwrap_or(Value::Null)
}

pub(crate) fn parse_proof_carry_data(config: &ProverConfig) -> ProofCarryData {
    serde_json::from_value(config_value(config, "proof_carry_data")).unwrap_or_default()
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
}
