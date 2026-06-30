//! Proof context for raiko2.

use alloy_primitives::B256;
use std::sync::Arc;

use crate::ProofType;
use crate::chain_spec::TaikoChainSpec;
use crate::proof::ProverConfig;
use reth_chainspec::ChainSpec as RethChainSpec;
use serde::{Deserialize, Serialize};

/// Explicit L2 block range used to build proving inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct L2BlockRange {
    pub start: u64,
    pub end: u64,
}

impl L2BlockRange {
    /// Returns true when the range is well-formed.
    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.start <= self.end
    }
}

/// Shasta checkpoint committed by the client for the final proven block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ShastaCheckpoint {
    pub block_number: u64,
    pub block_hash: B256,
    pub state_root: B256,
}

/// Shasta-specific request metadata required during preflight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ShastaRequest {
    /// The L1 block containing the proposal event.
    pub l1_inclusion_block_number: u64,
    /// The previously committed anchor block number.
    pub last_anchor_block_number: u64,
    /// Optional checkpoint expected by the caller for the final witness block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<ShastaCheckpoint>,
}

/// Proof request parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofRequest {
    /// The L1 chain ID.
    pub l1_chain_id: u64,
    /// The L2 chain ID.
    pub l2_chain_id: u64,
    /// The proposal ID to prove.
    pub proposal_id: u64,
    /// Optional explicit L2 block span used during preflight.
    pub l2_block_range: Option<L2BlockRange>,
    /// Optional Shasta-specific request metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shasta: Option<ShastaRequest>,
    /// The proof type (risc0, sp1).
    #[serde(with = "crate::proof_type::lowercase")]
    pub proof_type: ProofType,
    /// The blob proof type.
    pub blob_proof_type: Option<String>,
    /// The prover address.
    pub prover: Option<String>,
    /// The graffiti.
    pub graffiti: Option<String>,
}

impl Default for ProofRequest {
    fn default() -> Self {
        Self {
            l1_chain_id: 1,
            l2_chain_id: 167_000,
            proposal_id: 0,
            l2_block_range: None,
            shasta: None,
            proof_type: ProofType::Risc0,
            blob_proof_type: None,
            prover: None,
            graffiti: None,
        }
    }
}

/// Context config key carrying an optional L1 beacon RPC endpoint override.
pub const PROOF_CONTEXT_L1_BEACON_RPC_KEY: &str = "l1_beacon_rpc";

/// Proof context containing chain specs and request parameters.
#[derive(Debug, Clone)]
pub struct ProofContext {
    pub l1_chain_spec: Arc<RethChainSpec>,
    pub l2_chain_spec: Arc<TaikoChainSpec>,
    pub request: ProofRequest,
    pub config: ProverConfig,
}

impl ProofContext {
    #[must_use]
    pub fn new(request: ProofRequest, config: ProverConfig) -> Self {
        Self {
            l1_chain_spec: Arc::new(RethChainSpec::default()),
            l2_chain_spec: Arc::new(TaikoChainSpec::default()),
            request,
            config,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_proof_request_default() {
        let req = ProofRequest::default();
        assert_eq!(req.l1_chain_id, 1);
        assert_eq!(req.l2_chain_id, 167_000);
        assert_eq!(req.proposal_id, 0);
        assert_eq!(req.l2_block_range, None);
        assert_eq!(req.shasta, None);
        assert_eq!(req.proof_type, ProofType::Risc0);
        assert!(req.blob_proof_type.is_none());
        assert!(req.prover.is_none());
        assert!(req.graffiti.is_none());
    }

    #[test]
    fn test_proof_request_serialization() -> Result<(), Box<dyn std::error::Error>> {
        let req = ProofRequest {
            l1_chain_id: 1,
            l2_chain_id: 167_000,
            proposal_id: 123,
            l2_block_range: Some(L2BlockRange { start: 10, end: 12 }),
            shasta: Some(ShastaRequest {
                l1_inclusion_block_number: 456,
                last_anchor_block_number: 455,
                checkpoint: None,
            }),
            proof_type: ProofType::Sp1,
            blob_proof_type: Some("kzg".to_string()),
            prover: Some("0x1234".to_string()),
            graffiti: Some("test".to_string()),
        };

        let json = serde_json::to_string(&req)?;
        let deserialized: ProofRequest = serde_json::from_str(&json)?;

        assert_eq!(req.proposal_id, deserialized.proposal_id);
        assert_eq!(req.l2_block_range, deserialized.l2_block_range);
        assert_eq!(req.shasta, deserialized.shasta);
        assert_eq!(req.proof_type, deserialized.proof_type);
        assert_eq!(req.blob_proof_type, deserialized.blob_proof_type);
        Ok(())
    }
}
