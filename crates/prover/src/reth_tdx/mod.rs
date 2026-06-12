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

use alloy_primitives::{Address, B256, Bytes};
use raiko2_pipeline::ProverBackend;
use raiko2_primitives::{
    AggregationGuestInput, Proof, ProverConfig, RaikoError, RaikoResult,
    TdxDirectAggregateGuestInput, TdxDirectAggregateProposal,
};
use raiko2_primitives_shasta::{
    GuestInput, encode_proof_carry_data_vec,
    instance::{build_shasta_commitment_from_proof_carry_data_vec, shasta_aggregation_output},
    proof_carry_from_proof,
};
use raiko2_protocol_shasta::shasta::ProofCarryData;
use reqwest::{
    Client, Url,
    header::{CONTENT_TYPE, HeaderValue},
    redirect::Policy,
};
use serde_json::{Map, Value};

use crate::{GuestInputCodec, Prover, with_shasta_extra_data};

use protocol::{
    ProofResponse, ProofResult, ProofStatus, RETH_TDX_PROOF_RESPONSE_SCHEMA,
    RETH_TDX_SHASTA_AGGREGATE_REQUEST_SCHEMA, RETH_TDX_SHASTA_DIRECT_AGGREGATE_REQUEST_SCHEMA,
    RETH_TDX_SHASTA_REQUEST_SCHEMA, ShastaAggregatePayload, ShastaAggregateProof,
    ShastaAggregateRequest, ShastaDirectAggregatePayload, ShastaDirectAggregateRequest,
    ShastaProvePayload, ShastaProveRequest,
};

const SHASTA_PROPOSAL_PATH: &str = "/prove/shasta";
const SHASTA_AGGREGATE_PATH: &str = "/prove/shasta-aggregate";
const SHASTA_DIRECT_AGGREGATE_PATH: &str = "/prove/shasta-direct-aggregate";
const TDX_PROOF_BYTES: usize = 85;

/// Cap on the response body we will buffer from reth-tdx. A proof + quote +
/// carry payload is at most a few tens of KB even for a large aggregation;
/// this only exists to stop a hostile/broken prover from OOM-ing the process.
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

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
    direct_aggregate_url: Url,
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
        if !matches!(base_url.scheme(), "http" | "https") {
            return Err(RaikoError::InvalidRequestConfig(format!(
                "reth_tdx.base_url must use the http or https scheme, got '{}'",
                base_url.scheme()
            )));
        }
        // Append the endpoint onto the base URL's path. We deliberately avoid
        // `Url::join("/prove/...")`, whose absolute-path semantics silently
        // discard any path prefix on `base_url` (e.g. `http://h/api` → `http://h/prove/...`).
        let prove_url = endpoint_url(&base_url, SHASTA_PROPOSAL_PATH)?;
        let aggregate_url = endpoint_url(&base_url, SHASTA_AGGREGATE_PATH)?;
        let direct_aggregate_url = endpoint_url(&base_url, SHASTA_DIRECT_AGGREGATE_PATH)?;
        // `redirect(none)` stops a misconfigured/redirecting endpoint from being
        // silently followed to a different (possibly internal) host.
        let client = Client::builder()
            .timeout(config.timeout())
            .redirect(Policy::none())
            .build()
            .map_err(|err| {
                RaikoError::InvalidRequestConfig(format!("failed to build reth_tdx client: {err}"))
            })?;

        Ok(Self {
            client,
            prove_url,
            aggregate_url,
            direct_aggregate_url,
        })
    }
}

impl GuestInputCodec<GuestInput> for RethTdxProver {
    fn encode(&self, input: &GuestInput, _config: &ProverConfig) -> RaikoResult<Bytes> {
        let request = build_shasta_request(input);
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
        let instance = decode_tdx_instance_address(&result.proof)?;

        // reth-tdx must echo back the exact ProofCarryData it signed over so
        // raiko2 can compute the on-chain commitment hash. A missing or empty
        // `proof_carry_data_vec` means the remote prover is too old (or
        // misbehaving) and the resulting proof cannot be verified on-chain;
        // surface that as an error instead of returning a known-unverifiable
        // payload.
        let carry = result
            .proof_carry_data_vec
            .as_ref()
            .and_then(|v| v.first().cloned())
            .ok_or_else(|| {
                RaikoError::Guest(
                    "reth-tdx response is missing `proof_carry_data_vec`; the remote prover \
                     must echo the carry data it signed over for on-chain verification"
                        .to_string(),
                )
            })?;

        // Defense-in-depth: confirm the remote signed the proposal we asked
        // for, not a substituted one (see `ensure_carry_matches_request`).
        ensure_carry_matches_request(&carry, &packet.payload)?;
        ensure_tdx_input_matches(&[carry.clone()], instance, input_hash)?;

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
        if let Some(direct) = input.tdx_direct {
            return self.aggregate_direct(direct).await;
        }

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
        let instance = decode_tdx_instance_address(&result.proof)?;

        let carry_vec = result
            .proof_carry_data_vec
            .as_ref()
            .filter(|vec| !vec.is_empty())
            .ok_or_else(|| {
                RaikoError::Guest(
                    "reth-tdx aggregation response is missing `proof_carry_data_vec`; \
                     the remote prover must echo the carry data it signed over for \
                     on-chain verification"
                        .to_string(),
                )
            })?;

        // Defense-in-depth: the echoed carry vector must line up 1:1 with the
        // sub-proofs we asked to aggregate, and each entry must describe the
        // proposal we requested (not a substituted one).
        if carry_vec.len() != request.payload.proofs.len() {
            return Err(RaikoError::Guest(format!(
                "reth-tdx aggregation echoed {} carry entries but {} sub-proofs were requested",
                carry_vec.len(),
                request.payload.proofs.len()
            )));
        }
        for (carry, requested) in carry_vec.iter().zip(request.payload.proofs.iter()) {
            ensure_carry_matches_request(carry, &requested.payload)?;
        }
        ensure_tdx_input_matches(carry_vec, instance, input_hash)?;

        let metadata = reth_tdx_metadata(&envelope.schema, &result);
        let mut extra_data = encode_proof_carry_data_vec(carry_vec)?;
        if let Some(root) = extra_data.as_object_mut() {
            root.insert("reth_tdx".to_string(), metadata);
        }

        Ok(Proof {
            proof: Some(result.proof),
            input: Some(input_hash),
            quote: Some(result.quote),
            extra_data: Some(extra_data),
            ..Default::default()
        })
    }
}

impl RethTdxProver {
    async fn aggregate_direct(&self, input: TdxDirectAggregateGuestInput) -> RaikoResult<Proof> {
        let request = build_shasta_direct_aggregate_request(input)?;
        let payload = serde_json::to_vec(&request).map_err(|err| {
            RaikoError::Guest(format!(
                "failed to encode reth-tdx direct aggregate request: {err}"
            ))
        })?;

        let (envelope, result) = self
            .post_request(self.direct_aggregate_url.clone(), payload)
            .await?;
        let input_hash = parse_result_input_hash(&result)?;
        let instance = decode_tdx_instance_address(&result.proof)?;
        let carry_vec = result
            .proof_carry_data_vec
            .as_ref()
            .filter(|vec| !vec.is_empty())
            .ok_or_else(|| {
                RaikoError::Guest(
                    "reth-tdx direct aggregation response is missing `proof_carry_data_vec`; \
                     the remote prover must echo the carry data it signed over for \
                     on-chain verification"
                        .to_string(),
                )
            })?;

        if carry_vec.len() != request.payload.proposals.len() {
            return Err(RaikoError::Guest(format!(
                "reth-tdx direct aggregation echoed {} carry entries but {} proposals were requested",
                carry_vec.len(),
                request.payload.proposals.len()
            )));
        }
        for (carry, requested) in carry_vec.iter().zip(request.payload.proposals.iter()) {
            ensure_carry_matches_direct_request(carry, requested)?;
        }
        ensure_tdx_input_matches(carry_vec, instance, input_hash)?;

        proof_from_aggregate_result(envelope.schema.as_str(), &result, input_hash, carry_vec)
    }

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
        let body = read_body_capped(response).await?;
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

/// Append `path` onto `base`'s path without discarding an existing path prefix.
///
/// `Url::join("/prove/shasta")` treats the leading slash as absolute and drops
/// any prefix on `base` (so `http://h/api` becomes `http://h/prove/shasta`).
/// This instead extends the base path's segments, so `http://h/api` →
/// `http://h/api/prove/shasta` and `http://h` → `http://h/prove/shasta`.
fn endpoint_url(base: &Url, path: &str) -> RaikoResult<Url> {
    let mut url = base.clone();
    url.path_segments_mut()
        .map_err(|()| {
            RaikoError::InvalidRequestConfig("reth_tdx.base_url cannot be a base URL".to_string())
        })?
        .pop_if_empty() // drop a trailing empty segment from a trailing slash
        .extend(path.split('/').filter(|seg| !seg.is_empty()));
    Ok(url)
}

fn decode_tdx_instance_address(proof: &str) -> RaikoResult<Address> {
    let proof = proof.strip_prefix("0x").unwrap_or(proof);
    let proof = hex::decode(proof)
        .map_err(|err| RaikoError::Guest(format!("invalid reth-tdx proof hex: {err}")))?;
    if proof.len() != TDX_PROOF_BYTES {
        return Err(RaikoError::Guest(format!(
            "invalid reth-tdx proof length: got {} bytes, expected {TDX_PROOF_BYTES} \
             bytes (instance_address(20) || signature(65))",
            proof.len()
        )));
    }
    Ok(Address::from_slice(&proof[..20]))
}

fn parse_result_input_hash(result: &ProofResult) -> RaikoResult<B256> {
    B256::from_str(&result.input).map_err(|err| {
        RaikoError::Guest(format!(
            "invalid reth-tdx input hash '{}': {err}",
            result.input
        ))
    })
}

fn ensure_tdx_input_matches(
    carry_vec: &[ProofCarryData],
    instance: Address,
    input_hash: B256,
) -> RaikoResult<()> {
    let commitment =
        build_shasta_commitment_from_proof_carry_data_vec(carry_vec).ok_or_else(|| {
            RaikoError::Guest(
                "reth-tdx response carry data cannot build a Shasta commitment".to_string(),
            )
        })?;
    let first = carry_vec.first().ok_or_else(|| {
        RaikoError::Guest("reth-tdx response carry data cannot be empty".to_string())
    })?;
    let expected = shasta_aggregation_output(&commitment, first.chain_id, first.verifier, instance);
    if input_hash != expected {
        return Err(RaikoError::Guest(format!(
            "reth-tdx input hash does not match echoed carry data and instance address: \
             got {input_hash}, expected {expected}"
        )));
    }
    Ok(())
}

fn proof_from_aggregate_result(
    response_schema: &str,
    result: &ProofResult,
    input_hash: B256,
    carry_vec: &[ProofCarryData],
) -> RaikoResult<Proof> {
    let metadata = reth_tdx_metadata(response_schema, result);
    let mut extra_data = encode_proof_carry_data_vec(carry_vec)?;
    if let Some(root) = extra_data.as_object_mut() {
        root.insert("reth_tdx".to_string(), metadata);
    }

    Ok(Proof {
        proof: Some(result.proof.clone()),
        input: Some(input_hash),
        quote: Some(result.quote.clone()),
        extra_data: Some(extra_data),
        ..Default::default()
    })
}

/// Read a reth-tdx response body, refusing to buffer more than
/// [`MAX_RESPONSE_BYTES`] so a hostile/broken prover cannot OOM the process.
/// The cap is enforced on both the advertised `Content-Length` and the actual
/// streamed length (a missing or lying header is still bounded).
async fn read_body_capped(mut response: reqwest::Response) -> RaikoResult<Vec<u8>> {
    if let Some(len) = response.content_length()
        && len > MAX_RESPONSE_BYTES as u64
    {
        return Err(RaikoError::Guest(format!(
            "reth-tdx response too large: {len} bytes (max {MAX_RESPONSE_BYTES})"
        )));
    }
    let mut buf = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| RaikoError::Guest(format!("reth-tdx read failed: {err}")))?
    {
        if buf.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(RaikoError::Guest(format!(
                "reth-tdx response exceeds {MAX_RESPONSE_BYTES} bytes"
            )));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// Confirm the carry data reth-tdx echoed back describes the *same* proposal
/// raiko2 asked it to prove.
///
/// reth-tdx is trusted to source the L2-derived fields (`parent_block_hash`,
/// `checkpoint`) locally, but the L1-derived fields below are values raiko2
/// supplied and independently knows. A mismatch means the remote signed a
/// different proposal than requested — reject it rather than silently package
/// a substituted proof (which would still pass on-chain `verifyProof` because
/// it is internally self-consistent).
fn ensure_carry_matches_request(
    carry: &ProofCarryData,
    requested: &ShastaProvePayload,
) -> RaikoResult<()> {
    let mismatch = |field: &str| {
        Err(RaikoError::Guest(format!(
            "reth-tdx echoed carry data does not match the requested proposal (field `{field}` \
             differs); refusing to package a substituted proof"
        )))
    };
    let ti = &carry.transition_input;
    if carry.chain_id != requested.chain_id {
        return mismatch("chain_id");
    }
    if carry.verifier != requested.verifier {
        return mismatch("verifier");
    }
    if ti.proposal_id != requested.proposal_id {
        return mismatch("proposal_id");
    }
    if ti.proposal_hash != requested.proposal_hash {
        return mismatch("proposal_hash");
    }
    if ti.parent_proposal_hash != requested.parent_proposal_hash {
        return mismatch("parent_proposal_hash");
    }
    if ti.actual_prover != requested.actual_prover {
        return mismatch("actual_prover");
    }
    if ti.transition != requested.transition {
        return mismatch("transition");
    }
    Ok(())
}

fn ensure_carry_matches_direct_request(
    carry: &ProofCarryData,
    requested: &TdxDirectAggregateProposal,
) -> RaikoResult<()> {
    let mismatch = |field: &str| {
        Err(RaikoError::Guest(format!(
            "reth-tdx echoed carry data does not match the requested direct aggregate proposal \
             (field `{field}` differs); refusing to package a substituted proof"
        )))
    };
    let ti = &carry.transition_input;
    if carry.chain_id != requested.chain_id {
        return mismatch("chain_id");
    }
    if carry.verifier != requested.verifier {
        return mismatch("verifier");
    }
    if ti.proposal_id != requested.proposal_id {
        return mismatch("proposal_id");
    }
    if ti.proposal_hash != requested.proposal_hash {
        return mismatch("proposal_hash");
    }
    if ti.parent_proposal_hash != requested.parent_proposal_hash {
        return mismatch("parent_proposal_hash");
    }
    if ti.actual_prover != requested.actual_prover {
        return mismatch("actual_prover");
    }
    if ti.transition.proposer != requested.transition.proposer {
        return mismatch("transition.proposer");
    }
    if ti.transition.timestamp != requested.transition.timestamp {
        return mismatch("transition.timestamp");
    }
    let Some(last_block_number) = requested.l2_block_numbers.last().copied() else {
        return Err(RaikoError::InvalidRequestConfig(
            "TDX direct aggregate proposal l2_block_numbers must not be empty".to_string(),
        ));
    };
    if ti.checkpoint.blockNumber.to::<u64>() != last_block_number {
        return mismatch("checkpoint.blockNumber");
    }
    Ok(())
}

fn build_shasta_request(input: &GuestInput) -> ShastaProveRequest {
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
    ShastaProveRequest {
        schema: RETH_TDX_SHASTA_REQUEST_SCHEMA.to_string(),
        payload,
    }
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

fn build_shasta_direct_aggregate_request(
    input: TdxDirectAggregateGuestInput,
) -> RaikoResult<ShastaDirectAggregateRequest> {
    if input.proposals.is_empty() {
        return Err(RaikoError::InvalidRequestConfig(
            "cannot build reth-tdx direct aggregate request without proposals".to_string(),
        ));
    }
    let mut previous_proposal = None;
    for (index, proposal) in input.proposals.iter().enumerate() {
        validate_direct_l2_block_numbers(index, &proposal.l2_block_numbers)?;
        if let Some(previous) = previous_proposal {
            validate_direct_proposal_continuity(previous, proposal, index)?;
        }
        previous_proposal = Some(proposal);
    }

    Ok(ShastaDirectAggregateRequest {
        schema: RETH_TDX_SHASTA_DIRECT_AGGREGATE_REQUEST_SCHEMA.to_string(),
        payload: ShastaDirectAggregatePayload {
            proposals: input.proposals,
        },
    })
}

fn validate_direct_proposal_continuity(
    previous: &TdxDirectAggregateProposal,
    current: &TdxDirectAggregateProposal,
    index: usize,
) -> RaikoResult<()> {
    if current.proposal_id != previous.proposal_id + 1 {
        return Err(RaikoError::InvalidRequestConfig(format!(
            "TDX direct aggregate proposal {index} proposal_id must be contiguous"
        )));
    }
    let previous_last_block = previous.l2_block_numbers.last().copied().ok_or_else(|| {
        RaikoError::InvalidRequestConfig(format!(
            "TDX direct aggregate proposal {} l2_block_numbers must not be empty",
            index - 1
        ))
    })?;
    let current_first_block = current.l2_block_numbers.first().copied().ok_or_else(|| {
        RaikoError::InvalidRequestConfig(format!(
            "TDX direct aggregate proposal {index} l2_block_numbers must not be empty"
        ))
    })?;
    if current_first_block != previous_last_block + 1 {
        return Err(RaikoError::InvalidRequestConfig(format!(
            "TDX direct aggregate proposal {index} l2_block_numbers must continue the previous proposal"
        )));
    }
    Ok(())
}

fn validate_direct_l2_block_numbers(index: usize, numbers: &[u64]) -> RaikoResult<()> {
    let Some((&start, rest)) = numbers.split_first() else {
        return Err(RaikoError::InvalidRequestConfig(format!(
            "TDX direct aggregate proposal {index} l2_block_numbers must not be empty"
        )));
    };
    let mut previous = start;
    for number in rest {
        if *number <= previous {
            return Err(RaikoError::InvalidRequestConfig(format!(
                "TDX direct aggregate proposal {index} l2_block_numbers must be strictly increasing"
            )));
        }
        if *number != previous + 1 {
            return Err(RaikoError::InvalidRequestConfig(format!(
                "TDX direct aggregate proposal {index} l2_block_numbers must be contiguous"
            )));
        }
        previous = *number;
    }
    Ok(())
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
