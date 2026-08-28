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
pub mod boundless_config;
pub mod gaiko2;
pub mod native;
mod pending_recovery;
pub mod redact;
pub mod remote_prover;
#[cfg(feature = "risc0")]
pub mod risc0;
#[cfg(any(feature = "risc0", feature = "boundless"))]
mod risc0_aggregation;
#[cfg(feature = "sp1")]
pub mod sp1;
pub mod sp1_config;
pub use pending_recovery::{
    NetworkProverBackend, PendingProofCheckpoint, PendingProofRecoveryError,
};
pub use sp1_config::{
    Sp1FulfillmentStrategy, Sp1NetworkMetadata, Sp1NetworkMode, Sp1NetworkSubmissionProgress,
};

#[cfg(any(feature = "risc0", feature = "boundless"))]
use alloy::sol_types::SolValue;
use alloy_primitives::Bytes;
use alloy_primitives::{Address, B256};
use raiko2_pipeline::ProverBackend;
use raiko2_primitives::{AggregationGuestInput, Proof, ProverConfig, RaikoError, RaikoResult};
use raiko2_primitives_shasta::{
    ShastaZkAggregationGuestInput, encode_proof_carry_data,
    instance::{build_shasta_commitment_from_proof_carry_data_vec, shasta_aggregation_output},
    proof_carry_from_proof,
};
use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
use raiko2_protocol_shasta::shasta::ProofCarryData;
#[cfg(feature = "risc0")]
use risc0_ethereum_contracts_boundless::encode_seal;
#[cfg(feature = "risc0")]
use risc0_zkvm::Receipt as Risc0Receipt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
#[cfg(any(feature = "risc0", feature = "sp1", feature = "boundless", test))]
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

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
    /// Whether this request id already had a confirmed on-chain submission before the current
    /// rebid transaction. A missing event may fall back to market polling only in this case.
    #[serde(default)]
    pub request_id_has_confirmed_submission: bool,
    /// Exact EIP-712 signing digest of an on-chain Boundless request. This distinguishes market
    /// rebid rungs that deliberately reuse one request id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_digest: Option<String>,
    /// Earliest inclusive lower block for recovering any on-chain rung sharing this request id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broadcast_from_block: Option<u64>,
    pub expires_at: u64,
    /// Offer lock deadline (`rampUpStart + lockTimeout`), in seconds since the UNIX epoch. The
    /// client fee is zero for fulfillments past this time, so it bounds the payable window.
    pub lock_expires_at: u64,
    pub submitted_at: u64,
    pub image_ref: String,
    pub deployment: String,
    pub offchain: bool,
    pub quoted_mcycles_count: Option<u32>,
    pub evaluated_mcycles_count: Option<u32>,
    pub max_price_multiplier: u32,
    /// Exact escalated max price this submission bid, in wei, as a decimal string. The floored
    /// `max_price_multiplier` collapses the common attempt-2 (×1.5) rung to `1`, so this carries the
    /// precise bid for telemetry.
    pub max_price_wei: Option<String>,
    /// Rebid attempt number that produced this submission, starting at one. Persisted so a resume
    /// after restart restores the attempt count even when rebids reuse the same price (a flat
    /// `rebid_price_step_bps == 0` ladder), which cannot be recovered from `max_price_multiplier`
    /// alone.
    pub rebid_attempt: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoundlessSubmissionResume {
    pub provider_request_id: String,
    pub remote_tx_hash: Option<String>,
    /// Whether this request id already had a confirmed on-chain submission before the current
    /// rebid transaction. Older checkpoints default to `false` and therefore fail closed.
    #[serde(default)]
    pub request_id_has_confirmed_submission: bool,
    /// Exact EIP-712 signing digest of an on-chain Boundless request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_digest: Option<String>,
    /// Earliest inclusive lower block for recovering any on-chain rung sharing this request id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broadcast_from_block: Option<u64>,
    /// Exact guest image used by the submitted request.
    pub image_ref: String,
    /// Boundless market deployment that owns the request identifier.
    pub deployment: String,
    /// Transport used to submit the request to the Boundless market.
    pub offchain: bool,
    pub expires_at: u64,
    /// Offer lock deadline in seconds since the UNIX epoch.
    pub lock_expires_at: u64,
    pub submitted_at: u64,
    pub max_price_multiplier: u32,
    /// Exact escalated max price this submission bid, in wei, as a decimal string.
    pub max_price_wei: Option<String>,
    pub rebid_attempt: u32,
}

/// Exact persisted provider submission that may be cleared after a terminal outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingProofCheckpointIdentity {
    pub backend: NetworkProverBackend,
    pub provider_request_id: String,
    pub attempt: std::num::NonZeroU32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProverProgress {
    BoundlessSubmission(BoundlessSubmissionProgress),
    Sp1NetworkSubmission(Sp1NetworkSubmissionProgress),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgressPersistenceError {
    Retryable(String),
    Permanent(String),
}

impl std::fmt::Display for ProgressPersistenceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Retryable(error) | Self::Permanent(error) => formatter.write_str(error),
        }
    }
}

impl std::error::Error for ProgressPersistenceError {}

#[cfg(any(feature = "risc0", feature = "sp1", feature = "boundless", test))]
const CHECKPOINT_RETRY_BASE_DELAY: Duration = if cfg!(test) {
    Duration::from_millis(1)
} else {
    Duration::from_secs(1)
};
#[cfg(any(feature = "risc0", feature = "sp1", feature = "boundless", test))]
const CHECKPOINT_RETRY_MAX_DELAY: Duration = if cfg!(test) {
    Duration::from_millis(16)
} else {
    Duration::from_secs(30)
};
#[cfg(any(feature = "risc0", feature = "sp1", feature = "boundless", test))]
const CHECKPOINT_RETRY_JITTER_SLOTS: u64 = 8;
#[cfg(any(feature = "risc0", feature = "sp1", feature = "boundless", test))]
static CHECKPOINT_RETRY_INVOCATION: AtomicU64 = AtomicU64::new(0);

#[cfg(any(feature = "risc0", feature = "sp1", feature = "boundless", test))]
#[derive(Clone, Copy)]
struct CheckpointRetrySchedule {
    jitter_slot: u32,
}

#[cfg(any(feature = "risc0", feature = "sp1", feature = "boundless", test))]
impl CheckpointRetrySchedule {
    fn new() -> Self {
        Self::from_seed(CHECKPOINT_RETRY_INVOCATION.fetch_add(1, Ordering::Relaxed))
    }

    fn from_seed(seed: u64) -> Self {
        Self {
            jitter_slot: u32::try_from(seed % CHECKPOINT_RETRY_JITTER_SLOTS + 1)
                .expect("checkpoint retry jitter slot fits u32"),
        }
    }

    fn delay(self, retry: u32) -> Duration {
        let exponent = retry.min(31);
        let exponential = CHECKPOINT_RETRY_BASE_DELAY
            .saturating_mul(1_u32 << exponent)
            .min(CHECKPOINT_RETRY_MAX_DELAY);
        let jitter = (exponential / 64).saturating_mul(self.jitter_slot);
        exponential
            .saturating_add(jitter)
            .min(CHECKPOINT_RETRY_MAX_DELAY)
    }
}

#[cfg(any(feature = "risc0", feature = "sp1", feature = "boundless", test))]
pub(crate) async fn persist_prover_progress(
    observer: Option<&Arc<dyn ProverProgressObserver>>,
    progress: &ProverProgress,
    checkpoint: &'static str,
    permit: &SubmissionCheckpointPermit,
) -> RaikoResult<()> {
    let Some(observer) = observer else {
        return Ok(());
    };
    let schedule = CheckpointRetrySchedule::new();
    let mut retry = 0_u32;
    loop {
        match observer.on_progress(progress, permit).await {
            Ok(()) => return Ok(()),
            Err(ProgressPersistenceError::Retryable(error)) => {
                let delay = schedule.delay(retry);
                tracing::warn!(
                    %error,
                    checkpoint,
                    retry,
                    retry_delay_ms = delay.as_millis(),
                    "failed to persist remote submission checkpoint; retrying without resubmission"
                );
                tokio::time::sleep(delay).await;
                retry = retry.saturating_add(1);
            }
            Err(ProgressPersistenceError::Permanent(error)) => {
                return Err(RaikoError::Guest(error));
            }
        }
    }
}

#[cfg(any(feature = "risc0", feature = "sp1", feature = "boundless", test))]
pub(crate) async fn clear_pending_proof_checkpoint(
    observer: Option<&Arc<dyn ProverProgressObserver>>,
    identity: &PendingProofCheckpointIdentity,
    permit: &SubmissionCheckpointPermit,
) -> RaikoResult<()> {
    let Some(observer) = observer else {
        return Ok(());
    };
    let schedule = CheckpointRetrySchedule::new();
    let mut retry = 0_u32;
    loop {
        match observer
            .clear_pending_proof_checkpoint(identity, permit)
            .await
        {
            Ok(()) => return Ok(()),
            Err(ProgressPersistenceError::Retryable(error)) => {
                let delay = schedule.delay(retry);
                tracing::warn!(
                    %error,
                    backend = ?identity.backend,
                    provider_request_id = %identity.provider_request_id,
                    attempt = identity.attempt.get(),
                    retry,
                    retry_delay_ms = delay.as_millis(),
                    "failed to clear terminal provider checkpoint; retrying"
                );
                tokio::time::sleep(delay).await;
                retry = retry.saturating_add(1);
            }
            Err(ProgressPersistenceError::Permanent(error)) => {
                return Err(RaikoError::Guest(error));
            }
        }
    }
}

/// RAII token spanning provider acceptance through durable checkpoint persistence.
pub struct SubmissionCheckpointPermit {
    guard: Box<dyn std::any::Any + Send + Sync>,
}

impl SubmissionCheckpointPermit {
    /// Wrap a lifecycle-owned guard without exposing its implementation to prover backends.
    pub fn tracked(guard: impl std::any::Any + Send + Sync + 'static) -> Self {
        Self {
            guard: Box::new(guard),
        }
    }

    #[must_use]
    pub fn guard<T: std::any::Any>(&self) -> Option<&T> {
        self.guard.downcast_ref()
    }

    fn untracked() -> Self {
        Self::tracked(())
    }
}

#[cfg(any(feature = "risc0", feature = "sp1", feature = "boundless", test))]
pub(crate) async fn acquire_submission_checkpoint_permit(
    observer: Option<&Arc<dyn ProverProgressObserver>>,
) -> RaikoResult<SubmissionCheckpointPermit> {
    match observer {
        Some(observer) => observer
            .acquire_submission_checkpoint_permit()
            .await
            .map_err(|error| RaikoError::Guest(error.to_string())),
        None => Ok(SubmissionCheckpointPermit::untracked()),
    }
}

#[async_trait::async_trait]
pub trait ProverProgressObserver: Send + Sync {
    async fn acquire_submission_checkpoint_permit(
        &self,
    ) -> Result<SubmissionCheckpointPermit, ProgressPersistenceError> {
        Ok(SubmissionCheckpointPermit::untracked())
    }

    async fn on_progress(
        &self,
        progress: &ProverProgress,
        permit: &SubmissionCheckpointPermit,
    ) -> Result<(), ProgressPersistenceError>;

    async fn load_pending_proof_checkpoint(
        &self,
        _backend: NetworkProverBackend,
    ) -> Result<Option<PendingProofCheckpoint>, ProgressPersistenceError> {
        Ok(None)
    }

    async fn clear_pending_proof_checkpoint(
        &self,
        _identity: &PendingProofCheckpointIdentity,
        _permit: &SubmissionCheckpointPermit,
    ) -> Result<(), ProgressPersistenceError> {
        Err(ProgressPersistenceError::Permanent(
            "provider checkpoint clearing is unsupported".to_string(),
        ))
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

pub(crate) fn ensure_shasta_proposal_input_matches_carry(
    input_hash: B256,
    carry: &ProofCarryData,
    source: &str,
) -> RaikoResult<()> {
    let expected_input = hash_shasta_subproof_input(carry);
    if input_hash != expected_input {
        return Err(RaikoError::Guest(format!(
            "{source} proposal input hash mismatch: got {input_hash:#x} expected {expected_input:#x}"
        )));
    }
    Ok(())
}

pub(crate) fn expected_shasta_aggregate_input(
    carries: &[ProofCarryData],
    prover_address: Address,
) -> RaikoResult<B256> {
    let first = carries.first().ok_or_else(|| {
        RaikoError::InvalidRequestConfig(
            "cannot compute shasta aggregate input without proof carry data".to_string(),
        )
    })?;
    let commitment =
        build_shasta_commitment_from_proof_carry_data_vec(carries).ok_or_else(|| {
            RaikoError::InvalidRequestConfig("invalid shasta proof carry data".into())
        })?;

    Ok(shasta_aggregation_output(
        &commitment,
        first.chain_id,
        first.verifier,
        prover_address,
    ))
}

pub(crate) fn ensure_shasta_aggregate_input_matches_carries(
    input_hash: B256,
    carries: &[ProofCarryData],
    prover_address: Address,
    source: &str,
) -> RaikoResult<()> {
    let expected_input = expected_shasta_aggregate_input(carries, prover_address)?;
    if input_hash != expected_input {
        return Err(RaikoError::Guest(format!(
            "{source} aggregate input hash mismatch: got {input_hash:#x} expected {expected_input:#x}"
        )));
    }
    Ok(())
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

#[cfg(any(feature = "risc0", feature = "boundless"))]
pub(crate) fn encode_risc0_proposal_seal_payload(seal: &[u8], image_id: B256) -> String {
    let proof: Vec<u8> = (seal.to_vec(), image_id)
        .abi_encode()
        .into_iter()
        .skip(32)
        .collect();
    alloy_primitives::hex::encode_prefixed(proof)
}

#[cfg(any(feature = "risc0", feature = "boundless"))]
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
        proof_carry_data_vec.push(carry);
    }

    build_shasta_commitment_from_proof_carry_data_vec(&proof_carry_data_vec).ok_or_else(|| {
        RaikoError::InvalidRequestConfig("invalid shasta proof carry data".to_string())
    })?;

    let mut block_inputs = Vec::with_capacity(proofs.len());
    for (index, (proof, carry)) in proofs.iter().zip(&proof_carry_data_vec).enumerate() {
        let expected_input = hash_shasta_subproof_input(carry);
        if let Some(input_hash) = proof.input
            && input_hash != expected_input
        {
            return Err(RaikoError::InvalidRequestConfig(format!(
                "proof {index} input hash does not match shasta carry data"
            )));
        }
        block_inputs.push(expected_input);
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
    pipeline_key: raiko2_pipeline::PipelineKey,
    expected_chain_id: u64,
    proofs: &[Proof],
) -> Result<(), RaikoError> {
    validate_external_aggregate_proof_metadata(pipeline_key, proofs)?;

    for (index, proof) in proofs.iter().enumerate() {
        let carry = require_external_aggregate_proof_carry(index, proof)?;
        if carry.chain_id != expected_chain_id {
            return Err(RaikoError::InvalidRequestConfig(format!(
                "proof {index} proof carry chain_id mismatch: expected {expected_chain_id}, got {}",
                carry.chain_id
            )));
        }
    }

    Ok(())
}

/// # Errors
///
/// Returns an error when the supplied proofs do not contain the route-specific metadata needed
/// to materialize an aggregate input artifact.
pub fn validate_external_aggregate_proof_metadata(
    pipeline_key: raiko2_pipeline::PipelineKey,
    proofs: &[Proof],
) -> Result<(), RaikoError> {
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
                require_external_aggregate_proof_carry(index, proof)?;
            }
        }
    }

    Ok(())
}

fn require_external_aggregate_proof_carry(
    index: usize,
    proof: &Proof,
) -> Result<ProofCarryData, RaikoError> {
    proof_carry_from_proof(proof)
        .map_err(|err| {
            RaikoError::InvalidRequestConfig(format!(
                "proof {index} invalid shasta carry data: {err}"
            ))
        })?
        .ok_or_else(|| {
            RaikoError::InvalidRequestConfig(format!("proof {index} missing shasta carry data"))
        })
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
        CHECKPOINT_RETRY_MAX_DELAY, CheckpointRetrySchedule, NetworkProverBackend,
        PendingProofCheckpointIdentity, ProgressPersistenceError, ProverProgress,
        ProverProgressObserver, SubmissionCheckpointPermit, build_shasta_aggregation_input,
        clear_pending_proof_checkpoint, encode_proof_carry_data,
        ensure_shasta_proposal_input_matches_carry, parse_shasta_aggregation_input_hash,
        parse_shasta_proposal_input_hash, validate_external_aggregate_proofs,
    };
    #[cfg(any(feature = "risc0", feature = "boundless"))]
    use super::{
        decode_hex_payload, encode_risc0_aggregation_seal_payload,
        encode_risc0_proposal_seal_payload,
    };
    use alloy_primitives::B256;
    #[cfg(any(feature = "risc0", feature = "boundless"))]
    use alloy_sol_types::SolValue;
    use raiko2_pipeline::PipelineKey;
    use raiko2_primitives::Proof;
    use raiko2_protocol_shasta::shasta::ProofCarryData;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };

    #[derive(Default)]
    struct ClearingObserver {
        cleared: Mutex<Vec<PendingProofCheckpointIdentity>>,
    }

    #[async_trait::async_trait]
    impl ProverProgressObserver for ClearingObserver {
        async fn on_progress(
            &self,
            _progress: &ProverProgress,
            _permit: &SubmissionCheckpointPermit,
        ) -> Result<(), ProgressPersistenceError> {
            Ok(())
        }

        async fn clear_pending_proof_checkpoint(
            &self,
            identity: &PendingProofCheckpointIdentity,
            _permit: &SubmissionCheckpointPermit,
        ) -> Result<(), ProgressPersistenceError> {
            self.cleared
                .lock()
                .expect("cleared checkpoint lock")
                .push(identity.clone());
            Ok(())
        }
    }

    struct PermanentClearFailureObserver;

    #[async_trait::async_trait]
    impl ProverProgressObserver for PermanentClearFailureObserver {
        async fn on_progress(
            &self,
            _progress: &ProverProgress,
            _permit: &SubmissionCheckpointPermit,
        ) -> Result<(), ProgressPersistenceError> {
            Ok(())
        }

        async fn clear_pending_proof_checkpoint(
            &self,
            _identity: &PendingProofCheckpointIdentity,
            _permit: &SubmissionCheckpointPermit,
        ) -> Result<(), ProgressPersistenceError> {
            Err(ProgressPersistenceError::Permanent(
                "terminal checkpoint clear rejected".to_string(),
            ))
        }
    }

    #[tokio::test]
    async fn clear_pending_checkpoint_forwards_exact_identity() {
        let observer = Arc::new(ClearingObserver::default());
        let observer_dyn: Arc<dyn ProverProgressObserver> = observer.clone();
        let identity = PendingProofCheckpointIdentity {
            backend: NetworkProverBackend::Boundless,
            provider_request_id: "request-1".to_string(),
            attempt: std::num::NonZeroU32::new(5).expect("non-zero attempt"),
        };
        let permit = SubmissionCheckpointPermit::tracked(());

        clear_pending_proof_checkpoint(Some(&observer_dyn), &identity, &permit)
            .await
            .expect("clear pending checkpoint");

        assert_eq!(
            *observer.cleared.lock().expect("cleared checkpoint lock"),
            vec![identity]
        );
    }

    #[tokio::test]
    async fn clear_pending_checkpoint_without_observer_is_a_noop() {
        let identity = PendingProofCheckpointIdentity {
            backend: NetworkProverBackend::Boundless,
            provider_request_id: "request-1".to_string(),
            attempt: std::num::NonZeroU32::MIN,
        };
        let permit = SubmissionCheckpointPermit::tracked(());

        clear_pending_proof_checkpoint(None, &identity, &permit)
            .await
            .expect("unobserved clear is a no-op");
    }

    #[tokio::test]
    async fn clear_pending_checkpoint_surfaces_permanent_failure() {
        let observer: Arc<dyn ProverProgressObserver> = Arc::new(PermanentClearFailureObserver);
        let identity = PendingProofCheckpointIdentity {
            backend: NetworkProverBackend::Boundless,
            provider_request_id: "request-1".to_string(),
            attempt: std::num::NonZeroU32::MIN,
        };
        let permit = SubmissionCheckpointPermit::tracked(());

        let error = clear_pending_proof_checkpoint(Some(&observer), &identity, &permit)
            .await
            .expect_err("permanent clear failure must be returned");

        assert!(
            error
                .to_string()
                .contains("terminal checkpoint clear rejected")
        );
    }

    #[test]
    fn checkpoint_retry_schedule_grows_caps_and_jitters_per_invocation() {
        let first = CheckpointRetrySchedule::from_seed(1);
        let second = CheckpointRetrySchedule::from_seed(2);
        let first_delays = (0..16)
            .map(|attempt| first.delay(attempt))
            .collect::<Vec<_>>();

        assert!(first_delays.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(
            first_delays.last().copied(),
            Some(CHECKPOINT_RETRY_MAX_DELAY)
        );
        assert_ne!(first.delay(0), second.delay(0));
        assert!(
            first_delays
                .iter()
                .all(|delay| *delay <= CHECKPOINT_RETRY_MAX_DELAY)
        );
    }

    struct DropProbe(Arc<AtomicBool>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    #[test]
    fn submission_checkpoint_permit_drops_wrapped_guard() {
        let dropped = Arc::new(AtomicBool::new(false));
        let permit = SubmissionCheckpointPermit::tracked(DropProbe(Arc::clone(&dropped)));

        drop(permit);

        assert!(dropped.load(Ordering::Acquire));
    }

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
    fn proposal_input_binding_rejects_carry_mismatch() {
        let carry = ProofCarryData::default();
        let err =
            ensure_shasta_proposal_input_matches_carry(B256::repeat_byte(0x99), &carry, "test")
                .expect_err("mismatched carry hash");

        assert!(err.to_string().contains("input hash mismatch"));
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
    fn shasta_aggregation_input_rejects_oversized_timestamp_without_panicking() {
        let mut carry = ProofCarryData::default();
        carry.transition_input.transition.timestamp = 1_u64 << 48;
        let proof = Proof {
            input: None,
            uuid: None,
            extra_data: Some(encode_proof_carry_data(&carry).expect("encode carry data")),
            ..aggregate_proof_fixture()
        };

        let result = std::panic::catch_unwind(|| build_shasta_aggregation_input(&[proof]));

        assert!(result.is_ok(), "invalid carry data must not panic");
        let err = result
            .expect("checked above")
            .expect_err("oversized timestamp must be rejected");
        assert!(err.to_string().contains("invalid shasta proof carry data"));
    }

    #[test]
    fn aggregate_validator_accepts_native_local_proof() {
        assert!(
            validate_external_aggregate_proofs(
                PipelineKey::ShastaNative,
                0,
                &[aggregate_proof_fixture()]
            )
            .is_ok()
        );
    }

    #[test]
    fn aggregate_validator_accepts_sgx_remote_proof() {
        assert!(
            validate_external_aggregate_proofs(
                PipelineKey::ShastaSgx,
                0,
                &[aggregate_proof_fixture()]
            )
            .is_ok()
        );
    }

    #[test]
    fn aggregate_validator_rejects_missing_sgx_remote_proof_bytes() {
        let mut proof = aggregate_proof_fixture();
        proof.proof = None;

        let err = validate_external_aggregate_proofs(PipelineKey::ShastaSgx, 0, &[proof])
            .expect_err("missing proof bytes");
        assert!(
            err.to_string()
                .contains("proof 0 is missing SGX aggregation metadata")
        );
    }

    #[test]
    fn aggregate_validator_rejects_missing_sp1_fields() {
        let mut proof = aggregate_proof_fixture();
        proof.uuid = None;

        let err = validate_external_aggregate_proofs(PipelineKey::ShastaSp1, 0, &[proof])
            .expect_err("missing uuid");
        assert!(
            err.to_string()
                .contains("proof 0 is missing SP1 aggregation metadata")
        );
    }

    #[test]
    fn aggregate_validator_rejects_sp1_proof_without_quote_or_legacy_payload() {
        let mut proof = aggregate_proof_fixture();
        proof.proof = None;
        proof.quote = None;

        let err = validate_external_aggregate_proofs(PipelineKey::ShastaSp1, 0, &[proof])
            .expect_err("missing proof data");
        assert!(
            err.to_string()
                .contains("proof 0 is missing SP1 aggregation metadata")
        );
    }

    #[test]
    fn aggregate_validator_rejects_missing_risc0_local_fields() {
        let mut proof = aggregate_proof_fixture();
        proof.quote = None;

        let err = validate_external_aggregate_proofs(PipelineKey::ShastaRisc0, 0, &[proof])
            .expect_err("missing receipt");
        assert!(
            err.to_string()
                .contains("proof 0 is missing RISC0 aggregation metadata")
        );
    }

    #[test]
    fn aggregate_validator_rejects_missing_boundless_receipt() {
        let mut proof = aggregate_proof_fixture();
        proof.quote = None;

        let err = validate_external_aggregate_proofs(PipelineKey::ShastaRisc0Network, 0, &[proof])
            .expect_err("missing receipt");
        assert!(
            err.to_string()
                .contains("proof 0 is missing Boundless aggregation metadata")
        );
    }

    #[test]
    fn aggregate_validator_rejects_boundless_proof_without_carry_data() {
        let proof = Proof {
            proof: None,
            input: None,
            quote: Some("0xreceipt".to_string()),
            uuid: None,
            kzg_proof: None,
            extra_data: None,
        };

        let err = validate_external_aggregate_proofs(PipelineKey::ShastaRisc0Network, 0, &[proof])
            .expect_err("missing carry");
        assert!(
            err.to_string()
                .contains("proof 0 is missing Boundless aggregation metadata")
        );
    }

    #[test]
    fn aggregate_validator_accepts_boundless_proof_with_receipt_and_carry_data() {
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

        assert!(
            validate_external_aggregate_proofs(PipelineKey::ShastaRisc0Network, 0, &[proof])
                .is_ok()
        );
    }

    #[test]
    #[cfg(any(feature = "risc0", feature = "boundless"))]
    fn risc0_proposal_payload_encodes_seal_and_image_id() {
        let seal = vec![0x11, 0x22, 0x33];
        let image_id = B256::repeat_byte(0xaa);

        let encoded =
            decode_hex_payload(Some(&encode_risc0_proposal_seal_payload(&seal, image_id)));
        let expected: Vec<u8> = (seal, image_id).abi_encode().into_iter().skip(32).collect();

        assert_eq!(encoded, expected);
    }

    #[test]
    #[cfg(any(feature = "risc0", feature = "boundless"))]
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
