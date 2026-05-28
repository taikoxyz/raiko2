//! Remote TDX prover — HTTP client that talks to the
//! [`reth-tdx`](https://github.com/NethermindEth/reth-tdx) binary running inside
//! a Nethermind TDX VM.
//!
//! ## Why a remote prover (vs in-process TDX)
//!
//! The previous design ran raiko2 inside the TDX VM and produced TDX proofs
//! in-process. That gave the operator the freedom to point raiko2 at any L2
//! RPC — but it also meant the attestation quote did not actually constrain
//! where the proven blocks came from. A malicious operator could direct the
//! in-VM raiko2 at an untrusted RPC and still emit a TEE-signed proof.
//!
//! `reth-tdx` closes that gap by running inside the TDX VM and fetching L2
//! blocks itself from a co-resident Nethermind. raiko2 (running outside the
//! VM) sends only L1-derived proposal fields; reth-tdx sources L2 state
//! locally and signs.
//!
//! ## Wire format
//!
//! See [`protocol`]. Each request envelope carries a schema discriminator
//! (`reth-tdx-shasta-request-v1`, etc.) so mismatched versions fail fast.

pub mod protocol;

use std::{str::FromStr, time::Duration};

use alloy_primitives::{B256, Bytes};
use raiko2_pipeline::ProverBackend;
use raiko2_primitives::{AggregationGuestInput, Proof, ProverConfig, RaikoError, RaikoResult};
use raiko2_primitives_shasta::{GuestInput, proof_carry_from_proof};
use reqwest::{
    Client, Url,
    header::{CONTENT_TYPE, HeaderValue},
};
use serde_json::{Map, Value};

use crate::{GuestInputCodec, Prover, with_shasta_extra_data};

use protocol::{
    ProofResponse, ProofResult, ProofStatus, RETH_TDX_PROOF_RESPONSE_SCHEMA,
    RETH_TDX_SHASTA_AGGREGATE_REQUEST_SCHEMA, RETH_TDX_SHASTA_REQUEST_SCHEMA,
    ShastaAggregatePayload, ShastaAggregateProof, ShastaAggregateRequest, ShastaProvePayload,
    ShastaProveRequest,
};

const SHASTA_PROPOSAL_PATH: &str = "/prove/shasta";
const SHASTA_AGGREGATE_PATH: &str = "/prove/shasta-aggregate";

/// Static configuration for the [`RethTdxProver`] HTTP client.
#[derive(Debug, Clone, Default)]
pub struct RethTdxConfig {
    /// Base URL of the reth-tdx server (e.g. `http://localhost:8080`).
    pub base_url: String,
    /// Per-request timeout. Long enough to accommodate the tdxs daemon
    /// round-trip + the local L2 fetch.
    pub timeout_ms: u64,
}

impl RethTdxConfig {
    /// Convert the millisecond budget into a [`Duration`] for reqwest.
    #[must_use]
    pub const fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms)
    }
}

/// HTTP client for the reth-tdx remote prover.
#[derive(Clone)]
pub struct RethTdxProver {
    client: Client,
    prove_url: Url,
    aggregate_url: Url,
}

impl RethTdxProver {
    /// # Errors
    ///
    /// Returns an error when the base URL is empty or malformed, or when the
    /// HTTP client cannot be constructed.
    pub fn new(config: &RethTdxConfig) -> RaikoResult<Self> {
        if config.base_url.trim().is_empty() {
            return Err(RaikoError::InvalidRequestConfig(
                "reth_tdx.base_url must not be empty".to_string(),
            ));
        }

        let base_url = Url::parse(&config.base_url).map_err(|err| {
            RaikoError::InvalidRequestConfig(format!("invalid reth_tdx.base_url: {err}"))
        })?;
        let prove_url = base_url.join(SHASTA_PROPOSAL_PATH).map_err(|err| {
            RaikoError::InvalidRequestConfig(format!("invalid reth_tdx prove URL: {err}"))
        })?;
        let aggregate_url = base_url.join(SHASTA_AGGREGATE_PATH).map_err(|err| {
            RaikoError::InvalidRequestConfig(format!("invalid reth_tdx aggregate URL: {err}"))
        })?;
        let client = Client::builder()
            .timeout(config.timeout())
            .build()
            .map_err(|err| {
                RaikoError::InvalidRequestConfig(format!("failed to build reth_tdx client: {err}"))
            })?;

        Ok(Self {
            client,
            prove_url,
            aggregate_url,
        })
    }
}

impl GuestInputCodec<GuestInput> for RethTdxProver {
    fn encode(&self, input: &GuestInput, _config: &ProverConfig) -> RaikoResult<Bytes> {
        let request = build_shasta_request(input)?;
        let payload = serde_json::to_vec(&request).map_err(|err| {
            RaikoError::Guest(format!("failed to encode reth-tdx request: {err}"))
        })?;
        Ok(Bytes::from(payload))
    }
}

#[async_trait::async_trait]
impl<B> Prover<B> for RethTdxProver
where
    B: ProverBackend,
{
    type GuestInput = GuestInput;

    fn encode(&self, input: &Self::GuestInput, config: &ProverConfig) -> RaikoResult<Bytes> {
        GuestInputCodec::encode(self, input, config)
    }

    async fn prove_encoded(
        &self,
        input: Bytes,
        _config: &ProverConfig,
        _backend: &B,
    ) -> RaikoResult<Proof> {
        let packet: ShastaProveRequest = serde_json::from_slice(input.as_ref())
            .map_err(|err| RaikoError::Guest(format!("failed to decode reth-tdx packet: {err}")))?;
        if packet.schema != RETH_TDX_SHASTA_REQUEST_SCHEMA {
            return Err(RaikoError::Guest(format!(
                "unsupported reth-tdx request schema: {}",
                packet.schema
            )));
        }

        let (envelope, result) = self
            .post_request(self.prove_url.clone(), input.to_vec())
            .await?;
        let input_hash = B256::from_str(&result.input).map_err(|err| {
            RaikoError::Guest(format!(
                "invalid reth-tdx input hash '{}': {err}",
                result.input
            ))
        })?;

        // Prefer the carry_data the prover actually signed over (returned by
        // reth-tdx in `proof_carry_data_vec`). Fall back to a stub
        // reconstructed from the L1-only request payload only if the prover is
        // running an older release without that field — in that case the
        // checkpoint/parent_block_hash fields will be zero and downstream
        // on-chain verification will not be possible.
        let carry = result
            .proof_carry_data_vec
            .as_ref()
            .and_then(|v| v.first().cloned())
            .unwrap_or_else(|| reconstruct_proof_carry_data(&packet.payload));

        let extra_data = with_shasta_extra_data(
            &carry,
            "reth_tdx",
            Some(reth_tdx_metadata(&envelope.schema, &result)),
        )?;

        Ok(Proof {
            proof: Some(result.proof),
            input: Some(input_hash),
            quote: Some(result.quote),
            extra_data,
            ..Default::default()
        })
    }

    async fn aggregate(
        &self,
        input: AggregationGuestInput,
        _config: &ProverConfig,
        _backend: &B,
    ) -> RaikoResult<Proof> {
        let request = build_shasta_aggregate_request(&input.proofs)?;
        let payload = serde_json::to_vec(&request).map_err(|err| {
            RaikoError::Guest(format!(
                "failed to encode reth-tdx aggregate request: {err}"
            ))
        })?;

        let (envelope, result) = self
            .post_request(self.aggregate_url.clone(), payload)
            .await?;
        let input_hash = B256::from_str(&result.input).map_err(|err| {
            RaikoError::Guest(format!(
                "invalid reth-tdx input hash '{}': {err}",
                result.input
            ))
        })?;
        let metadata = reth_tdx_metadata(&envelope.schema, &result);

        Ok(Proof {
            proof: Some(result.proof),
            input: Some(input_hash),
            quote: Some(result.quote),
            extra_data: Some(serde_json::json!({ "reth_tdx": metadata })),
            ..Default::default()
        })
    }
}

impl RethTdxProver {
    async fn post_request(
        &self,
        url: Url,
        body: Vec<u8>,
    ) -> RaikoResult<(ProofResponse, ProofResult)> {
        let response = self
            .client
            .post(url)
            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .body(body)
            .send()
            .await
            .map_err(|err| RaikoError::Guest(format!("reth-tdx request failed: {err}")))?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|err| RaikoError::Guest(format!("reth-tdx read failed: {err}")))?;
        let envelope: ProofResponse = serde_json::from_slice(&body).map_err(|err| {
            RaikoError::Guest(format!(
                "reth-tdx response decode failed (status {status}): {err}"
            ))
        })?;

        if envelope.schema != RETH_TDX_PROOF_RESPONSE_SCHEMA {
            return Err(RaikoError::Guest(format!(
                "unsupported reth-tdx response schema: {}",
                envelope.schema
            )));
        }

        if !status.is_success() || envelope.status == ProofStatus::Error {
            let error = envelope.error.as_ref().ok_or_else(|| {
                RaikoError::Guest(format!(
                    "reth-tdx request failed with status {status} and no error payload"
                ))
            })?;
            return Err(RaikoError::Guest(format!(
                "reth-tdx {}: {}",
                error.code, error.message
            )));
        }

        let result = envelope.result.clone().ok_or_else(|| {
            RaikoError::Guest("reth-tdx response missing result payload".to_string())
        })?;

        Ok((envelope, result))
    }
}

// ─────────────────────────── Helpers ───────────────────────────

fn build_shasta_request(input: &GuestInput) -> RaikoResult<ShastaProveRequest> {
    let carry = &input.proof_carry_data;
    let payload = ShastaProvePayload {
        chain_id: carry.chain_id,
        verifier: carry.verifier,
        proposal_id: carry.transition_input.proposal_id,
        proposal_hash: carry.transition_input.proposal_hash,
        parent_proposal_hash: carry.transition_input.parent_proposal_hash,
        actual_prover: carry.transition_input.actual_prover,
        transition: carry.transition_input.transition.clone(),
    };
    Ok(ShastaProveRequest {
        schema: RETH_TDX_SHASTA_REQUEST_SCHEMA.to_string(),
        payload,
    })
}

fn build_shasta_aggregate_request(proofs: &[Proof]) -> RaikoResult<ShastaAggregateRequest> {
    if proofs.is_empty() {
        return Err(RaikoError::InvalidRequestConfig(
            "cannot build reth-tdx aggregate request without proofs".to_string(),
        ));
    }

    let entries = proofs
        .iter()
        .map(|proof| {
            let input = proof.input.ok_or_else(|| {
                RaikoError::InvalidRequestConfig(
                    "reth-tdx aggregate sub-proof missing input hash".to_string(),
                )
            })?;
            let proof_hex = proof.proof.clone().ok_or_else(|| {
                RaikoError::InvalidRequestConfig(
                    "reth-tdx aggregate sub-proof missing proof bytes".to_string(),
                )
            })?;
            let carry = proof_carry_from_proof(proof)?.ok_or_else(|| {
                RaikoError::InvalidRequestConfig(
                    "reth-tdx aggregate sub-proof missing shasta carry data".to_string(),
                )
            })?;
            Ok(ShastaAggregateProof {
                payload: ShastaProvePayload {
                    chain_id: carry.chain_id,
                    verifier: carry.verifier,
                    proposal_id: carry.transition_input.proposal_id,
                    proposal_hash: carry.transition_input.proposal_hash,
                    parent_proposal_hash: carry.transition_input.parent_proposal_hash,
                    actual_prover: carry.transition_input.actual_prover,
                    transition: carry.transition_input.transition.clone(),
                },
                input,
                proof: proof_hex,
            })
        })
        .collect::<RaikoResult<Vec<_>>>()?;

    Ok(ShastaAggregateRequest {
        schema: RETH_TDX_SHASTA_AGGREGATE_REQUEST_SCHEMA.to_string(),
        payload: ShastaAggregatePayload { proofs: entries },
    })
}

/// Reconstruct a minimal [`ProofCarryData`](raiko2_protocol_shasta::shasta::ProofCarryData)
/// from a `ShastaProvePayload` for `extra_data` annotation. The L2-derived
/// fields (parent_block_hash, checkpoint) are populated by reth-tdx inside the
/// TEE and are not carried in raiko2's view of the request — this stub is
/// only used to label the returned `Proof` with chain-id / proposal-id for
/// downstream aggregation lookup.
fn reconstruct_proof_carry_data(
    payload: &ShastaProvePayload,
) -> raiko2_protocol_shasta::shasta::ProofCarryData {
    raiko2_protocol_shasta::shasta::ProofCarryData {
        chain_id: payload.chain_id,
        verifier: payload.verifier,
        transition_input: raiko2_protocol_shasta::shasta::TransitionInputData {
            proposal_id: payload.proposal_id,
            proposal_hash: payload.proposal_hash,
            parent_proposal_hash: payload.parent_proposal_hash,
            parent_block_hash: B256::ZERO,
            actual_prover: payload.actual_prover,
            transition: payload.transition.clone(),
            checkpoint: Default::default(),
        },
    }
}

fn reth_tdx_metadata(response_schema: &str, result: &ProofResult) -> Value {
    let mut metadata = Map::from_iter([(
        "schema".to_string(),
        Value::String(response_schema.to_string()),
    )]);
    if let Some(instance_address) = result.instance_address.clone() {
        metadata.insert(
            "instance_address".to_string(),
            Value::String(instance_address),
        );
    }
    Value::Object(metadata)
}
