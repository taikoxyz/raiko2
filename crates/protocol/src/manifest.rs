//! Taiko manifest types for proposal proofs.
//!
//! This module contains the manifest types used for Taiko proposal proof generation.
//! These types describe the input data structure for zkVM guest programs.

use alloy_consensus::Header;
use alloy_primitives::{Address, B256};
use anyhow::{Error, anyhow};
use core::str::FromStr;
use serde::{Deserialize, Serialize};

/// Blob proof type for Taiko.
#[derive(
    Clone, Copy, Debug, Default, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd, Hash,
)]
#[serde(rename_all = "snake_case")]
pub enum BlobProofType {
    /// Simplified Proof of Equivalence with fiat input in non-aligned field.
    #[default]
    ProofOfEquivalence,
}

impl FromStr for BlobProofType {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "proof_of_equivalence" => Ok(BlobProofType::ProofOfEquivalence),
            _ => Err(anyhow!("invalid blob proof type")),
        }
    }
}

/// Input data source for proposal proof.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct InputDataSource {
    pub tx_data_from_calldata: Vec<u8>,
    pub tx_data_from_blob: Vec<Vec<u8>>,
    pub blob_commitments: Vec<Vec<u8>>,
    pub blob_proofs: Vec<Vec<u8>>,
    pub is_forced_inclusion: bool,
}

/// Taiko prover data.
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct TaikoProverData<Cp = ()> {
    pub actual_prover: Address,
    pub designated_prover: Option<Address>,
    pub graffiti: B256,
    pub parent_transition_hash: Option<B256>,
    pub checkpoint: Option<Cp>,
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
pub struct TaikoManifest<E = (), Cp = ()> {
    /// The proposal ID being proven.
    pub proposal_id: u64,
    /// The L1 header at which the proposal was proposed.
    #[serde(with = "l1_header_bincode_compat")]
    pub l1_header: Header,
    /// The decoded proposal event data.
    pub proposal_event: E,
    /// Chain specification for the manifest.
    pub chain_spec: ManifestChainSpec,
    /// Prover-specific data.
    pub prover_data: TaikoProverData<Cp>,
    /// The blob proof strategy resolved for the current prover backend.
    #[serde(default)]
    pub blob_proof_type: BlobProofType,
    /// Data sources for the proposal.
    pub data_sources: Vec<InputDataSource>,
    /// L1 header chain covering Shasta anchor checkpoints through the proposal origin block.
    #[serde(default, with = "l1_header_vec_bincode_compat")]
    pub l1_ancestor_headers: Vec<Header>,
}

mod l1_header_bincode_compat {
    use super::Header;
    use alloy_rlp::Decodable;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(h: &Header, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if s.is_human_readable() {
            return h.serialize(s);
        }

        // For bincode (SP1 stdin), encode the header as canonical RLP bytes.
        let bytes = alloy_rlp::encode(h);
        let v: Vec<u8> = bytes.clone();
        v.serialize(s)
    }

    pub fn deserialize<'de, D>(d: D) -> Result<Header, D::Error>
    where
        D: Deserializer<'de>,
    {
        if d.is_human_readable() {
            return Header::deserialize(d);
        }

        // For bincode (SP1 stdin), decode from RLP bytes.
        let bytes = Vec::<u8>::deserialize(d)?;
        let mut slice = bytes.as_slice();
        Header::decode(&mut slice).map_err(serde::de::Error::custom)
    }
}

mod l1_header_vec_bincode_compat {
    use super::Header;
    use alloy_rlp::Decodable;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(headers: &[Header], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            return headers.serialize(serializer);
        }

        let encoded = headers
            .iter()
            .map(|header| alloy_rlp::encode(header).clone())
            .collect::<Vec<_>>();
        encoded.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<Header>, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            return Vec::<Header>::deserialize(deserializer);
        }

        let encoded = Vec::<Vec<u8>>::deserialize(deserializer)?;
        encoded
            .into_iter()
            .map(|bytes| {
                let mut slice = bytes.as_slice();
                Header::decode(&mut slice).map_err(serde::de::Error::custom)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blob_proof_type_from_str() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            "proof_of_equivalence".parse::<BlobProofType>()?,
            BlobProofType::ProofOfEquivalence
        );
        assert!("kzg_versioned_hash".parse::<BlobProofType>().is_err());
        assert!("invalid".parse::<BlobProofType>().is_err());
        Ok(())
    }

    #[test]
    fn test_taiko_manifest_default() {
        let manifest: TaikoManifest<(), ()> = TaikoManifest::default();
        assert_eq!(manifest.proposal_id, 0);
        assert!(manifest.data_sources.is_empty());
        assert!(manifest.l1_ancestor_headers.is_empty());
        assert_eq!(manifest.blob_proof_type, BlobProofType::ProofOfEquivalence);
    }

    #[test]
    fn test_taiko_prover_data_default() {
        let data: TaikoProverData<()> = TaikoProverData::default();
        assert_eq!(data.actual_prover, Address::ZERO);
        assert!(data.designated_prover.is_none());
        assert_eq!(data.graffiti, B256::ZERO);
    }

    #[test]
    fn test_input_data_source_default() {
        let source = InputDataSource::default();
        assert!(source.tx_data_from_calldata.is_empty());
        assert!(source.tx_data_from_blob.is_empty());
        assert!(source.blob_commitments.is_empty());
        assert!(source.blob_proofs.is_empty());
        assert!(!source.is_forced_inclusion);
    }
}
