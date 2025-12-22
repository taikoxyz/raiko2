//! Taiko manifest types for proposal proofs.
//!
//! This module contains the manifest types used for Taiko proposal proof generation.
//! These types describe the input data structure for zkVM guest programs.

use alloy_consensus::Header;
use alloy_primitives::{Address, B256};
use anyhow::{Error, anyhow};
use core::str::FromStr;
use serde::{Deserialize, Serialize};

use crate::shasta::{Checkpoint, ShastaEventData};

/// Blob proof type for Taiko.
#[derive(Clone, Debug, Serialize, Deserialize, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BlobProofType {
    /// Guest runs through the entire computation from blob to Kzg commitment
    /// then to version hash.
    #[default]
    KzgVersionedHash,
    /// Simplified Proof of Equivalence with fiat input in non-aligned field.
    ProofOfEquivalence,
}

impl FromStr for BlobProofType {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "proof_of_equivalence" => Ok(BlobProofType::ProofOfEquivalence),
            "kzg_versioned_hash" => Ok(BlobProofType::KzgVersionedHash),
            _ => Err(anyhow!("invalid blob proof type")),
        }
    }
}

/// Input data source for proposal proof.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct InputDataSource {
    pub tx_data_from_calldata: Vec<u8>,
    pub tx_data_from_blob: Vec<Vec<u8>>,
    pub blob_commitments: Option<Vec<Vec<u8>>>,
    pub blob_proofs: Option<Vec<Vec<u8>>>,
    pub blob_proof_type: BlobProofType,
    pub is_forced_inclusion: bool,
}

/// Taiko prover data.
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct TaikoProverData {
    pub actual_prover: Address,
    pub designated_prover: Option<Address>,
    pub graffiti: B256,
    pub parent_transition_hash: Option<B256>,
    pub checkpoint: Option<Checkpoint>,
    pub last_anchor_block_number: Option<u64>,
}

/// Taiko chain specification for manifest.
///
/// This is a simplified chain spec for use in manifests.
/// For full chain spec, see `raiko2-primitives::ChainSpec`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ManifestChainSpec {
    pub name: String,
    pub chain_id: u64,
    pub is_taiko: bool,
}

/// Taiko proposal input manifest for guest programs.
///
/// This manifest describes all the data needed for a zkVM guest to
/// verify a proposal of Taiko L2 blocks.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct TaikoManifest {
    /// The proposal ID being proven.
    pub proposal_id: u64,
    /// The L1 header at which the proposal was proposed.
    pub l1_header: Header,
    /// The decoded proposal event data.
    pub proposal_event: ShastaEventData,
    /// Chain specification for the manifest.
    pub chain_spec: ManifestChainSpec,
    /// Prover-specific data.
    pub prover_data: TaikoProverData,
    /// Data sources for the proposal.
    pub data_sources: Vec<InputDataSource>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blob_proof_type_from_str() {
        assert_eq!(
            "kzg_versioned_hash".parse::<BlobProofType>().unwrap(),
            BlobProofType::KzgVersionedHash
        );
        assert_eq!(
            "proof_of_equivalence".parse::<BlobProofType>().unwrap(),
            BlobProofType::ProofOfEquivalence
        );
        assert!("invalid".parse::<BlobProofType>().is_err());
    }

    #[test]
    fn test_taiko_manifest_default() {
        let manifest = TaikoManifest::default();
        assert_eq!(manifest.proposal_id, 0);
        assert!(manifest.data_sources.is_empty());
    }

    #[test]
    fn test_taiko_prover_data_default() {
        let data = TaikoProverData::default();
        assert_eq!(data.actual_prover, Address::ZERO);
        assert!(data.designated_prover.is_none());
        assert_eq!(data.graffiti, B256::ZERO);
    }

    #[test]
    fn test_input_data_source_default() {
        let source = InputDataSource::default();
        assert!(source.tx_data_from_calldata.is_empty());
        assert!(!source.is_forced_inclusion);
        assert_eq!(source.blob_proof_type, BlobProofType::KzgVersionedHash);
    }
}
