//! Wire protocol between raiko2 and the reth-tdx remote prover.
//!
//! Mirrors the schema defined in `nethermind-tdx/reth-tdx/src/protocol.rs` —
//! both files must stay in sync. The schema discriminator constants make
//! cross-version mismatches fail fast at runtime.

use alloy_primitives::{Address, B256};
use raiko2_protocol_shasta::shasta::{ProofCarryData, ShastaTransitionInput};
use serde::{Deserialize, Serialize};

// ─────────────────────────── Schema discriminators ───────────────────────────

/// Request schema for a single Shasta proposal proof.
pub const RETH_TDX_SHASTA_REQUEST_SCHEMA: &str = "reth-tdx-shasta-request-v1";

/// Request schema for a Shasta aggregation proof.
pub const RETH_TDX_SHASTA_AGGREGATE_REQUEST_SCHEMA: &str = "reth-tdx-shasta-aggregate-request-v1";

/// Response schema shared by both proof endpoints.
pub const RETH_TDX_PROOF_RESPONSE_SCHEMA: &str = "reth-tdx-proof-v1";

// ─────────────────────────── Proposal proof ───────────────────────────

/// Top-level request envelope for `POST /prove/shasta`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShastaProveRequest {
    /// Schema discriminator — must equal [`RETH_TDX_SHASTA_REQUEST_SCHEMA`].
    pub schema: String,
    /// L1-derived proposal data. reth-tdx fetches the corresponding L2 block
    /// itself from the local Nethermind RPC.
    pub payload: ShastaProvePayload,
}

/// L1-derived proposal data passed by the caller. Everything in here is
/// independently verifiable by the on-chain Shasta verifier against L1 state
/// at proof-submission time, so it is safe to trust without reth-tdx having
/// its own L1 RPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShastaProvePayload {
    /// L2 chain id.
    pub chain_id: u64,
    /// On-chain Shasta verifier address.
    pub verifier: Address,
    /// Shasta proposal id — used as the L2 block number to fetch locally
    /// (`proposal_id` ↔ L2 block number is 1:1 in Shasta).
    pub proposal_id: u64,
    /// L1 proposal hash.
    pub proposal_hash: B256,
    /// Parent proposal hash from the Shasta proposal chain.
    pub parent_proposal_hash: B256,
    /// The EOA the prover will submit the proof from.
    pub actual_prover: Address,
    /// Transition input (proposer + timestamp) from the L1 proposal event.
    pub transition: ShastaTransitionInput,
}

// ─────────────────────────── Aggregation proof ───────────────────────────

/// Top-level request envelope for `POST /prove/shasta-aggregate`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShastaAggregateRequest {
    /// Schema discriminator — must equal
    /// [`RETH_TDX_SHASTA_AGGREGATE_REQUEST_SCHEMA`].
    pub schema: String,
    /// One entry per sub-proof being aggregated.
    pub payload: ShastaAggregatePayload,
}

/// Aggregation payload — a flat list of previously-signed sub-proofs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShastaAggregatePayload {
    /// Sub-proofs to fold into a single aggregation signature.
    pub proofs: Vec<ShastaAggregateProof>,
}

/// One previously-signed sub-proof. Carries its own [`ShastaProvePayload`]
/// plus the original signing hash and proof bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShastaAggregateProof {
    /// L1-derived data the sub-proof was bound to.
    pub payload: ShastaProvePayload,
    /// Original signing hash.
    pub input: B256,
    /// Hex-encoded 85-byte sub-proof: `instance_address(20) || signature(65)`.
    pub proof: String,
}

// ─────────────────────────── Response ───────────────────────────

/// Response envelope shared by both proof endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofResponse {
    /// Schema discriminator — equals [`RETH_TDX_PROOF_RESPONSE_SCHEMA`].
    pub schema: String,
    /// `ok` or `error`.
    pub status: ProofStatus,
    /// Present on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<ProofResult>,
    /// Present on error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ProofError>,
}

/// Response status.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProofStatus {
    /// Proof generated successfully.
    Ok,
    /// Proof generation failed.
    Error,
}

/// Successful proof payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofResult {
    /// Hex-encoded 85-byte TDX proof: `instance_address(20) || signature(65)`.
    pub proof: String,
    /// Hex-encoded TDX attestation quote bound to the signing hash.
    pub quote: String,
    /// Hex-encoded signing hash (the value the ECDSA signature was made over).
    pub input: String,
    /// Bootstrap public key / instance address (echoed for caller convenience).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_address: Option<String>,
    /// The exact `ProofCarryData` vector reth-tdx built and signed over. Needed
    /// to compute `hashCommitment(commitment)` for on-chain `verifyProof`.
    /// Length 1 for a proposal proof, N for an aggregation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof_carry_data_vec: Option<Vec<ProofCarryData>>,
}

/// Error payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofError {
    /// Short machine-readable code.
    pub code: String,
    /// Human-readable explanation.
    pub message: String,
}
