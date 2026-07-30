use alloy_consensus::{Header, TrieAccount};
use alloy_primitives::{B256, map::AddressMap};
use anyhow::{Context, Result};
use raiko2_primitives::{
    ChainSpec, ExecutionWitness, L2BlockRange, ShastaCheckpoint, WitnessHeader, WitnessStateNode,
};
use raiko2_protocol::{BlobProofType, InputDataSource};
use raiko2_protocol_shasta::shasta::ShastaEventData;
use reth_ethereum_primitives::Block;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use sha2::{Digest, Sha256};

pub const CANONICAL_PREFLIGHT_SCHEMA_V1: u16 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalPreflightKeyV1 {
    pub schema: u16,
    pub l1_chain_id: u64,
    pub l2_chain_id: u64,
    pub proposal_id: u64,
    pub l2_block_range: L2BlockRange,
    pub l1_inclusion_block_number: u64,
    pub last_anchor_block_number: u64,
    pub checkpoint: Option<ShastaCheckpoint>,
    pub l1_inclusion_hash: B256,
    pub proposal_event_digest: B256,
    pub chain_rules_fingerprint: B256,
}

impl CanonicalPreflightKeyV1 {
    /// Returns the deterministic digest used to locate this full cache key.
    ///
    /// # Errors
    ///
    /// Returns an error when the key cannot be serialized.
    pub fn digest(&self) -> Result<B256> {
        sha256_serialized(self, "canonical preflight key")
    }
}

#[serde_as]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CanonicalStatelessInputV1 {
    #[serde_as(as = "raiko2_primitives::EthereumBlock<'_>")]
    pub block: Block,
    pub witness: ExecutionWitness,
    pub accounts: AddressMap<TrieAccount>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CanonicalShastaManifestV1 {
    pub proposal_id: u64,
    #[serde(with = "header_bincode_compat")]
    pub l1_header: Header,
    pub proposal_event: ShastaEventData,
    pub blob_proof_type: BlobProofType,
    pub data_sources: Vec<InputDataSource>,
    #[serde(default, with = "header_vec_bincode_compat")]
    pub l1_ancestor_headers: Vec<Header>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CanonicalShastaPreflightV1 {
    pub manifest: CanonicalShastaManifestV1,
    pub witnesses: Vec<CanonicalStatelessInputV1>,
    pub proposal_ancestor_headers: Vec<WitnessHeader>,
    pub proposal_state_nodes: Vec<WitnessStateNode>,
}

/// Returns a deterministic digest of the normalized Shasta proposal event.
///
/// # Errors
///
/// Returns an error when the event cannot be serialized.
pub fn proposal_event_digest(event: &ShastaEventData) -> Result<B256> {
    sha256_serialized(event, "Shasta proposal event")
}

/// Hashes only chain rules that can affect Shasta discovery, derivation, or execution.
///
/// Verifier maps, RPC endpoints, beacon endpoints, and display names are deliberately excluded.
///
/// # Errors
///
/// Returns an error when a fingerprinted chain-rule field cannot be serialized.
pub fn chain_rules_fingerprint(l1: &ChainSpec, l2: &ChainSpec) -> Result<B256> {
    let mut hasher = Sha256::new();
    update_rule_field(&mut hasher, "l1.chain_id", &l1.chain_id)?;
    update_rule_field(&mut hasher, "l1.max_spec_id", &l1.max_spec_id)?;
    update_rule_field(&mut hasher, "l1.hard_forks", &l1.hard_forks)?;
    update_rule_field(&mut hasher, "l1.eip_1559_constants", &l1.eip_1559_constants)?;
    update_rule_field(&mut hasher, "l1.l1_contract", &l1.l1_contract)?;
    update_rule_field(&mut hasher, "l1.l2_contract", &l1.l2_contract)?;
    update_rule_field(
        &mut hasher,
        "l1.checkpoint_store_contract",
        &l1.checkpoint_store_contract,
    )?;
    update_rule_field(&mut hasher, "l1.genesis_time", &l1.genesis_time)?;
    update_rule_field(&mut hasher, "l1.seconds_per_slot", &l1.seconds_per_slot)?;
    update_rule_field(&mut hasher, "l1.is_taiko", &l1.is_taiko)?;

    update_rule_field(&mut hasher, "l2.chain_id", &l2.chain_id)?;
    update_rule_field(&mut hasher, "l2.max_spec_id", &l2.max_spec_id)?;
    update_rule_field(&mut hasher, "l2.hard_forks", &l2.hard_forks)?;
    update_rule_field(&mut hasher, "l2.eip_1559_constants", &l2.eip_1559_constants)?;
    update_rule_field(&mut hasher, "l2.l1_contract", &l2.l1_contract)?;
    update_rule_field(&mut hasher, "l2.l2_contract", &l2.l2_contract)?;
    update_rule_field(
        &mut hasher,
        "l2.checkpoint_store_contract",
        &l2.checkpoint_store_contract,
    )?;
    update_rule_field(&mut hasher, "l2.genesis_time", &l2.genesis_time)?;
    update_rule_field(&mut hasher, "l2.seconds_per_slot", &l2.seconds_per_slot)?;
    update_rule_field(&mut hasher, "l2.is_taiko", &l2.is_taiko)?;

    Ok(B256::from_slice(&hasher.finalize()))
}

fn update_rule_field<T: Serialize>(hasher: &mut Sha256, label: &str, value: &T) -> Result<()> {
    let bytes = bincode::serialize(value)
        .with_context(|| format!("failed to encode chain rule field {label}"))?;
    hasher.update((label.len() as u64).to_be_bytes());
    hasher.update(label.as_bytes());
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
    Ok(())
}

fn sha256_serialized<T: Serialize>(value: &T, label: &str) -> Result<B256> {
    let bytes = bincode::serialize(value).with_context(|| format!("failed to encode {label}"))?;
    Ok(B256::from_slice(&Sha256::digest(bytes)))
}

mod header_bincode_compat {
    use alloy_consensus::Header;
    use alloy_rlp::Decodable;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(header: &Header, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            return header.serialize(serializer);
        }
        alloy_rlp::encode(header).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Header, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            return Header::deserialize(deserializer);
        }
        let bytes = Vec::<u8>::deserialize(deserializer)?;
        Header::decode(&mut bytes.as_slice()).map_err(serde::de::Error::custom)
    }
}

mod header_vec_bincode_compat {
    use alloy_consensus::Header;
    use alloy_rlp::Decodable;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(headers: &[Header], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            return headers.serialize(serializer);
        }
        headers
            .iter()
            .map(alloy_rlp::encode)
            .collect::<Vec<_>>()
            .serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<Header>, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            return Vec::<Header>::deserialize(deserializer);
        }
        Vec::<Vec<u8>>::deserialize(deserializer)?
            .into_iter()
            .map(|bytes| Header::decode(&mut bytes.as_slice()).map_err(serde::de::Error::custom))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CANONICAL_PREFLIGHT_SCHEMA_V1, CanonicalPreflightKeyV1, CanonicalShastaPreflightV1,
        chain_rules_fingerprint,
    };
    use alloy_primitives::{Address, B256};
    use raiko2_primitives::{
        ChainSpec, L2BlockRange, ProofType, ShastaCheckpoint,
        chain_spec::{ForkId, TaikoFork},
    };

    fn cache_key() -> CanonicalPreflightKeyV1 {
        CanonicalPreflightKeyV1 {
            schema: CANONICAL_PREFLIGHT_SCHEMA_V1,
            l1_chain_id: 1,
            l2_chain_id: 167_001,
            proposal_id: 42,
            l2_block_range: L2BlockRange {
                start: 100,
                end: 102,
            },
            l1_inclusion_block_number: 77,
            last_anchor_block_number: 99,
            checkpoint: Some(ShastaCheckpoint {
                block_number: 102,
                block_hash: B256::repeat_byte(0x11),
                state_root: B256::repeat_byte(0x22),
            }),
            l1_inclusion_hash: B256::repeat_byte(0x33),
            proposal_event_digest: B256::repeat_byte(0x44),
            chain_rules_fingerprint: B256::repeat_byte(0x55),
        }
    }

    fn chain_specs() -> (ChainSpec, ChainSpec) {
        let mut l1 = ChainSpec {
            chain_id: 32_382,
            name: "l1".to_string(),
            rpc: "https://l1.example.invalid".to_string(),
            ..ChainSpec::default()
        };
        l1.l1_contract
            .insert(ForkId::Taiko(TaikoFork::Shasta), Address::repeat_byte(0x10));

        let l2 = ChainSpec {
            chain_id: 167_001,
            name: "l2".to_string(),
            rpc: "https://l2.example.invalid".to_string(),
            l2_contract: Some(Address::repeat_byte(0x20)),
            checkpoint_store_contract: Some(Address::repeat_byte(0x30)),
            is_taiko: true,
            ..ChainSpec::default()
        };
        (l1, l2)
    }

    #[test]
    fn canonical_key_digest_is_deterministic_and_boundary_sensitive() {
        let key = cache_key();
        assert_eq!(key.digest().expect("digest"), key.digest().expect("digest"));

        let mut changed = key.clone();
        changed.last_anchor_block_number += 1;
        assert_ne!(
            key.digest().expect("digest"),
            changed.digest().expect("digest")
        );

        changed = key.clone();
        changed.checkpoint.as_mut().expect("checkpoint").state_root = B256::repeat_byte(0x99);
        assert_ne!(
            key.digest().expect("digest"),
            changed.digest().expect("digest")
        );
    }

    #[test]
    fn canonical_key_digest_tracks_l1_identity_and_event() {
        let key = cache_key();

        let mut changed = key.clone();
        changed.l1_inclusion_hash = B256::repeat_byte(0x99);
        assert_ne!(
            key.digest().expect("digest"),
            changed.digest().expect("digest")
        );

        changed = key.clone();
        changed.proposal_event_digest = B256::repeat_byte(0x99);
        assert_ne!(
            key.digest().expect("digest"),
            changed.digest().expect("digest")
        );

        changed = key.clone();
        changed.l2_block_range.end += 1;
        assert_ne!(
            key.digest().expect("digest"),
            changed.digest().expect("digest")
        );
    }

    #[test]
    fn rules_fingerprint_ignores_host_presentation_and_verifiers() {
        let (l1, l2) = chain_specs();
        let expected = chain_rules_fingerprint(&l1, &l2).expect("fingerprint");

        let mut changed = l2.clone();
        changed.name = "renamed".to_string();
        changed.rpc = "https://replacement.example.invalid".to_string();
        changed.beacon_rpc = Some("https://beacon.example.invalid".to_string());
        changed
            .verifier_address_forks
            .entry(ForkId::Taiko(TaikoFork::Shasta))
            .or_default()
            .insert(ProofType::Sgx, Some(Address::repeat_byte(0xaa)));

        assert_eq!(
            expected,
            chain_rules_fingerprint(&l1, &changed).expect("fingerprint")
        );
    }

    #[test]
    fn rules_fingerprint_tracks_preflight_semantics() {
        let (l1, l2) = chain_specs();
        let expected = chain_rules_fingerprint(&l1, &l2).expect("fingerprint");

        let mut changed = l2.clone();
        changed.seconds_per_slot += 1;
        assert_ne!(
            expected,
            chain_rules_fingerprint(&l1, &changed).expect("fingerprint")
        );

        let mut changed = l1;
        changed
            .l1_contract
            .insert(ForkId::Taiko(TaikoFork::Unzen), Address::repeat_byte(0xbb));
        assert_ne!(
            expected,
            chain_rules_fingerprint(&changed, &l2).expect("fingerprint")
        );
    }

    #[test]
    fn canonical_core_has_stable_binary_roundtrip() {
        let core = CanonicalShastaPreflightV1::default();
        let bytes = bincode::serialize(&core).expect("encode");
        let decoded: CanonicalShastaPreflightV1 = bincode::deserialize(&bytes).expect("decode");
        assert_eq!(bytes, bincode::serialize(&decoded).expect("re-encode"));
    }
}
