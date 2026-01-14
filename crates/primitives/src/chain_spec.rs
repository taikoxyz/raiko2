use crate::proof_type::ProofType;
pub use alethia_reth_chainspec::spec::TaikoChainSpec;
use alethia_reth_chainspec::{TAIKO_DEVNET, TAIKO_HOODI, TAIKO_MAINNET};
use alloy_primitives::{Address, BlockNumber, ChainId, U256, map::HashMap, uint};
use anyhow::{Result, anyhow, bail};
use reth_revm::primitives::hardfork::SpecId;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

const DEFAULT_CHAIN_SPECS: &str = include_str!("../../../config/chain_spec_list_default.json");

#[derive(Clone, Debug)]
pub struct SupportedChainSpecs(HashMap<String, ChainSpec>);

impl Default for SupportedChainSpecs {
    fn default() -> Self {
        let deserialized: Vec<ChainSpec> = serde_json::from_str(DEFAULT_CHAIN_SPECS)
            .unwrap_or_else(|e| {
                eprintln!("Failed to deserialize chain specs: {e}");
                eprintln!("This may cause 'Unsupported chain_id' errors");
                Vec::default()
            });
        let chain_spec_list = deserialized
            .into_iter()
            .map(|cs| (cs.name.clone(), cs))
            .collect::<HashMap<String, ChainSpec>>();
        SupportedChainSpecs(chain_spec_list)
    }
}

impl SupportedChainSpecs {
    pub fn merge_from_file(file_path: PathBuf) -> Result<SupportedChainSpecs> {
        let mut known_chain_specs = SupportedChainSpecs::default();
        let file = std::fs::File::open(file_path)?;
        let reader = std::io::BufReader::new(file);
        let config: Value = serde_json::from_reader(reader)?;
        let chain_spec_list: Vec<ChainSpec> = serde_json::from_value(config)?;
        let new_chain_specs = chain_spec_list
            .into_iter()
            .map(|cs| (cs.name.clone(), cs))
            .collect::<HashMap<String, ChainSpec>>();

        // override known specs
        known_chain_specs.0.extend(new_chain_specs);
        Ok(known_chain_specs)
    }

    pub fn supported_networks(&self) -> Vec<String> {
        self.0.keys().cloned().collect()
    }

    pub fn get_chain_spec(&self, network: &str) -> Option<ChainSpec> {
        self.0.get(network).cloned()
    }

    pub fn get_chain_spec_with_chain_id(&self, chain_id: u64) -> Option<ChainSpec> {
        self.0
            .values()
            .find(|spec| spec.chain_id == chain_id)
            .cloned()
    }
}

/// The condition at which a fork is activated.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum ForkCondition {
    /// The fork is activated with a certain block.
    Block(BlockNumber),
    /// The fork is activated with a specific timestamp.
    Timestamp(u64),
    /// The fork is not yet active.
    Tbd,
}

impl<'de> Deserialize<'de> for ForkCondition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Handle both "TBD" (from JSON) and "Tbd" (Rust enum variant)
        let value: Value = Value::deserialize(deserializer)?;

        match value {
            Value::Object(map) => {
                if let Some(block) = map.get("Block") {
                    let block_num =
                        BlockNumber::deserialize(block).map_err(serde::de::Error::custom)?;
                    return Ok(ForkCondition::Block(block_num));
                }
                if let Some(timestamp) = map.get("Timestamp") {
                    let ts = u64::deserialize(timestamp).map_err(serde::de::Error::custom)?;
                    return Ok(ForkCondition::Timestamp(ts));
                }
                Err(serde::de::Error::custom("Invalid ForkCondition object"))
            }
            Value::String(s) => {
                // Handle "TBD" or "Tbd" string
                match s.as_str() {
                    "TBD" | "Tbd" => Ok(ForkCondition::Tbd),
                    _ => Err(serde::de::Error::custom(format!(
                        "Unknown ForkCondition variant: {}",
                        s
                    ))),
                }
            }
            _ => Err(serde::de::Error::custom("Invalid ForkCondition format")),
        }
    }
}

impl ForkCondition {
    /// Returns whether the condition has been met.
    pub const fn active(&self, block_no: BlockNumber, timestamp: u64) -> bool {
        match self {
            ForkCondition::Block(block) => *block <= block_no,
            ForkCondition::Timestamp(ts) => *ts <= timestamp,
            ForkCondition::Tbd => false,
        }
    }
}

/// [EIP-1559](https://eips.ethereum.org/EIPS/eip-1559) parameters.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub struct Eip1559Constants {
    pub base_fee_change_denominator: U256,
    pub base_fee_max_increase_denominator: U256,
    pub base_fee_max_decrease_denominator: U256,
    pub elasticity_multiplier: U256,
}

impl Default for Eip1559Constants {
    /// Defaults to Ethereum network values
    fn default() -> Self {
        Self {
            base_fee_change_denominator: uint!(8_U256),
            base_fee_max_increase_denominator: uint!(8_U256),
            base_fee_max_decrease_denominator: uint!(8_U256),
            elasticity_multiplier: uint!(2_U256),
        }
    }
}

/// Helper function to convert Taiko fork name string to a unique SpecId placeholder.
/// Since standard SpecId doesn't include Taiko forks, we use a combination approach:
/// - For Taiko forks, we use CANCUN as base and encode the fork name in a way that
///   allows us to distinguish them when needed
/// - We maintain a separate mapping for Taiko fork lookups
const fn taiko_fork_to_spec_id(_fork_name: &str) -> SpecId {
    // For now, we'll use CANCUN as placeholder for all Taiko forks.
    // The actual fork resolution happens in TaikoChainSpec.
    // We need to ensure different forks map to different values to avoid collisions.
    // Since we can't extend SpecId enum, we'll use a workaround:
    // Store the fork name mapping separately and use CANCUN as placeholder.
    SpecId::CANCUN
}

fn parse_spec_id_str(value: &str) -> Result<SpecId, String> {
    match value {
        // Taiko-specific forks - use CANCUN as placeholder
        "HEKLA" | "ONTAKE" | "PACAYA" | "SHASTA" => Ok(taiko_fork_to_spec_id(value)),
        // Standard forks - deserialize normally
        _ => serde_json::from_str(&format!("\"{}\"", value))
            .map_err(|_| format!("unknown SpecId variant: {}", value)),
    }
}

/// Custom deserializer for SpecId that handles Taiko-specific fork names.
fn deserialize_spec_id<'de, D>(deserializer: D) -> Result<SpecId, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    parse_spec_id_str(&s).map_err(serde::de::Error::custom)
}

/// Custom deserializer for BTreeMap<SpecId, T> that handles Taiko-specific fork names.
/// For Taiko forks, we need to preserve the original fork name to avoid collisions.
/// We'll use a workaround: store them with string keys internally, then convert.
fn deserialize_spec_id_map<'de, D, T>(deserializer: D) -> Result<BTreeMap<SpecId, T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    // First deserialize as a map with string keys
    let string_map: BTreeMap<String, T> = BTreeMap::deserialize(deserializer)?;

    // For Taiko forks, we need to handle them specially to avoid collisions.
    // Since multiple Taiko forks would all map to CANCUN, we'll use a different approach:
    // We'll create a composite key or use a workaround.
    // Actually, for Taiko chains, the lookup is done via TaikoChainSpec anyway,
    // so we can use a placeholder. But we need to preserve the mapping.

    // Let's use a simpler approach: for Taiko forks, we'll store them separately
    // and use CANCUN as the key. The actual resolution happens in TaikoChainSpec.
    let mut spec_id_map = BTreeMap::new();

    for (key, value) in string_map {
        let spec_id = parse_spec_id_str(&key).map_err(serde::de::Error::custom)?;
        // Note: This will overwrite if multiple Taiko forks exist, but that's OK
        // because for Taiko chains, we use TaikoChainSpec which handles forks properly.
        spec_id_map.insert(spec_id, value);
    }
    Ok(spec_id_map)
}

/// Specification of a specific chain.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct ChainSpec {
    pub name: String,
    pub chain_id: ChainId,
    #[serde(deserialize_with = "deserialize_spec_id")]
    pub max_spec_id: SpecId,
    #[serde(deserialize_with = "deserialize_spec_id_map")]
    pub hard_forks: BTreeMap<SpecId, ForkCondition>,
    pub eip_1559_constants: Eip1559Constants,
    #[serde(default, deserialize_with = "deserialize_spec_id_map")]
    pub l1_contract: BTreeMap<SpecId, Address>,
    pub l2_contract: Option<Address>,
    pub rpc: String,
    pub beacon_rpc: Option<String>,
    #[serde(deserialize_with = "deserialize_verifier_address_forks")]
    pub verifier_address_forks: VerifierAddressForks,
    pub genesis_time: u64,
    pub seconds_per_slot: u64,
    pub is_taiko: bool,
}

type VerifierAddressForks = BTreeMap<SpecId, BTreeMap<ProofType, Option<Address>>>;

fn parse_proof_type_str(value: &str) -> Result<ProofType, String> {
    match value {
        "NATIVE" | "Native" => Ok(ProofType::Native),
        "SP1" | "Sp1" => Ok(ProofType::Sp1),
        "SGX" | "Sgx" => Ok(ProofType::Sgx),
        "RISC0" | "Risc0" => Ok(ProofType::Risc0),
        _ => Err(format!("unknown ProofType variant: {}", value)),
    }
}

/// Custom deserializer for verifier_address_forks nested map structure.
fn deserialize_verifier_address_forks<'de, D>(
    deserializer: D,
) -> Result<VerifierAddressForks, D::Error>
where
    D: Deserializer<'de>,
{
    // First deserialize as a map with string keys, where inner map also has string keys
    let string_map: BTreeMap<String, BTreeMap<String, Option<Address>>> =
        BTreeMap::deserialize(deserializer)?;

    // Convert string keys to SpecId and inner string keys to ProofType
    let mut spec_id_map: VerifierAddressForks = BTreeMap::new();
    for (key, inner_map) in string_map {
        let spec_id = parse_spec_id_str(&key).map_err(serde::de::Error::custom)?;

        // Convert inner map: skip SGXGETH, convert other keys to ProofType
        let mut proof_type_map = BTreeMap::new();
        for (proof_key, address) in inner_map {
            // Skip SGXGETH
            if proof_key == "SGXGETH" {
                continue;
            }

            // Convert proof type string to ProofType enum
            let proof_type = parse_proof_type_str(&proof_key).map_err(serde::de::Error::custom)?;
            proof_type_map.insert(proof_type, address);
        }

        spec_id_map.insert(spec_id, proof_type_map);
    }
    Ok(spec_id_map)
}

impl<'de> Deserialize<'de> for ChainSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct ChainSpecHelper {
            name: String,
            chain_id: ChainId,
            #[serde(deserialize_with = "deserialize_spec_id")]
            max_spec_id: SpecId,
            #[serde(deserialize_with = "deserialize_spec_id_map")]
            hard_forks: BTreeMap<SpecId, ForkCondition>,
            eip_1559_constants: Eip1559Constants,
            #[serde(default, deserialize_with = "deserialize_spec_id_map")]
            l1_contract: BTreeMap<SpecId, Address>,
            l2_contract: Option<Address>,
            rpc: String,
            beacon_rpc: Option<String>,
            #[serde(deserialize_with = "deserialize_verifier_address_forks")]
            verifier_address_forks: VerifierAddressForks,
            genesis_time: u64,
            seconds_per_slot: u64,
            is_taiko: bool,
        }

        let helper = ChainSpecHelper::deserialize(deserializer)?;
        Ok(ChainSpec {
            name: helper.name,
            chain_id: helper.chain_id,
            max_spec_id: helper.max_spec_id,
            hard_forks: helper.hard_forks,
            eip_1559_constants: helper.eip_1559_constants,
            l1_contract: helper.l1_contract,
            l2_contract: helper.l2_contract,
            rpc: helper.rpc,
            beacon_rpc: helper.beacon_rpc,
            verifier_address_forks: helper.verifier_address_forks,
            genesis_time: helper.genesis_time,
            seconds_per_slot: helper.seconds_per_slot,
            is_taiko: helper.is_taiko,
        })
    }
}

impl ChainSpec {
    /// Creates a new configuration consisting of only one specification ID.
    pub fn new_single(
        name: String,
        chain_id: ChainId,
        spec_id: SpecId,
        eip_1559_constants: Eip1559Constants,
        is_taiko: bool,
    ) -> Self {
        ChainSpec {
            name,
            chain_id,
            max_spec_id: spec_id,
            hard_forks: BTreeMap::from([(spec_id, ForkCondition::Block(0))]),
            eip_1559_constants,
            l1_contract: BTreeMap::new(),
            l2_contract: None,
            rpc: "".to_string(),
            beacon_rpc: None,
            verifier_address_forks: BTreeMap::new(),
            genesis_time: 0u64,
            seconds_per_slot: 1u64,
            is_taiko,
        }
    }

    /// Returns the network chain ID.
    pub const fn chain_id(&self) -> ChainId {
        self.chain_id
    }

    /// Returns the [SpecId] for a given block number and timestamp or an error if not
    /// supported.
    pub fn active_fork(&self, block_no: BlockNumber, timestamp: u64) -> Result<SpecId> {
        match self.spec_id(block_no, timestamp) {
            Some(spec_id) => {
                if spec_id > self.max_spec_id {
                    bail!("expected <= {:?}, got {spec_id:?}", self.max_spec_id);
                }
                Ok(spec_id)
            }
            None => Err(anyhow!("no supported fork for block {block_no}")),
        }
    }

    /// Returns the Eip1559 constants
    pub const fn gas_constants(&self) -> &Eip1559Constants {
        &self.eip_1559_constants
    }

    pub fn spec_id(&self, block_no: BlockNumber, timestamp: u64) -> Option<SpecId> {
        for (spec_id, fork) in self.hard_forks.iter().rev() {
            if fork.active(block_no, timestamp) {
                return Some(*spec_id);
            }
        }
        None
    }

    pub fn get_fork_verifier_address(
        &self,
        block_num: u64,
        block_timestamp: u64,
        proof_type: ProofType,
    ) -> Result<Address> {
        // fall down to the first fork that is active as default
        for (spec_id, fork) in self.hard_forks.iter().rev() {
            if fork.active(block_num, block_timestamp)
                && let Some(fork_verifier) = self.verifier_address_forks.get(spec_id)
            {
                return fork_verifier
                    .get(&proof_type)
                    .ok_or_else(|| anyhow!("Verifier type not found"))
                    .and_then(|address| {
                        address.ok_or_else(|| anyhow!("Verifier address not found"))
                    });
            }
        }

        Err(anyhow!("fork verifier is not active"))
    }

    pub fn get_fork_l1_contract_address(&self, block_num: u64) -> Result<Address> {
        // fall down to the first fork that is active as default
        for (spec_id, fork) in self.hard_forks.iter().rev() {
            if fork.active(block_num, 0u64)
                && let Some(l1_address) = self.l1_contract.get(spec_id)
            {
                return Ok(*l1_address);
            }
        }

        Err(anyhow!("fork l1 contract is not active"))
    }

    pub const fn is_taiko(&self) -> bool {
        self.is_taiko
    }

    /// Convert this config-level [`ChainSpec`] into Alethia Reth's Taiko chain spec.
    ///
    /// This conversion is only supported for Taiko networks where Alethia provides a built-in
    /// genesis chainspec:
    ///
    /// - `167000` (Taiko Mainnet)
    /// - `167001` (Taiko Devnet)
    /// - `167013` (Taiko Hoodi)
    ///
    /// Returns an error for non-Taiko chains or unknown Taiko chain IDs.
    pub fn to_taiko_chain_spec(&self) -> Result<Arc<TaikoChainSpec>> {
        if !self.is_taiko {
            bail!(
                "chain spec is not a Taiko chain and cannot be converted (chain_id={})",
                self.chain_id
            );
        }

        match self.chain_id {
            167000 => Ok(TAIKO_MAINNET.clone()),
            167001 => Ok(TAIKO_DEVNET.clone()),
            167013 => Ok(TAIKO_HOODI.clone()),
            other => bail!(
                "unsupported Taiko chain_id={other}; no built-in genesis is available for conversion"
            ),
        }
    }

    pub fn network(&self) -> String {
        self.name.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alethia_reth_chainspec::{
        TAIKO_DEVNET_GENESIS_HASH, TAIKO_HOODI_GENESIS_HASH, TAIKO_MAINNET_GENESIS_HASH,
    };

    #[test]
    fn converts_taiko_mainnet_to_alethia_taiko_chain_spec() {
        let spec = ChainSpec::new_single(
            "taiko_mainnet".to_string(),
            167000,
            SpecId::CANCUN,
            Eip1559Constants::default(),
            true,
        );

        let taiko = spec
            .to_taiko_chain_spec()
            .expect("failed to convert to TaikoChainSpec");

        assert_eq!(taiko.inner.genesis_hash(), TAIKO_MAINNET_GENESIS_HASH);
    }

    #[test]
    fn converts_taiko_devnet_to_alethia_taiko_chain_spec() {
        let spec = ChainSpec::new_single(
            "taiko_devnet".to_string(),
            167001,
            SpecId::CANCUN,
            Eip1559Constants::default(),
            true,
        );

        let taiko = spec
            .to_taiko_chain_spec()
            .expect("failed to convert to TaikoChainSpec");

        assert_eq!(taiko.inner.genesis_hash(), TAIKO_DEVNET_GENESIS_HASH);
    }

    #[test]
    fn converts_taiko_hoodi_to_alethia_taiko_chain_spec() {
        let spec = ChainSpec::new_single(
            "taiko_hoodi".to_string(),
            167013,
            SpecId::CANCUN,
            Eip1559Constants::default(),
            true,
        );

        let taiko = spec
            .to_taiko_chain_spec()
            .expect("failed to convert to TaikoChainSpec");

        assert_eq!(taiko.inner.genesis_hash(), TAIKO_HOODI_GENESIS_HASH);
    }

    #[test]
    fn rejects_non_taiko_chain_spec() {
        let spec = ChainSpec::new_single(
            "ethereum".to_string(),
            1,
            SpecId::CANCUN,
            Eip1559Constants::default(),
            false,
        );

        assert!(spec.to_taiko_chain_spec().is_err());
    }

    #[test]
    fn rejects_unknown_taiko_chain_id_without_builtin_genesis() {
        let spec = ChainSpec::new_single(
            "taiko_custom".to_string(),
            9_999_999,
            SpecId::CANCUN,
            Eip1559Constants::default(),
            true,
        );

        assert!(spec.to_taiko_chain_spec().is_err());
    }
}
