use crate::proof_type::ProofType;
pub use alethia_reth_chainspec::spec::TaikoChainSpec;
use alethia_reth_chainspec::{TAIKO_DEVNET, TAIKO_HOODI, TAIKO_MAINNET, TAIKO_MASAYA};
use alethia_reth_chainspec::{
    hardfork::TaikoHardfork as AlethiaTaikoHardfork, spec::TaikoDevnetConfigExt,
};
use alloy_hardforks::{EthereumHardfork, ForkCondition as AlethiaForkCondition};
use alloy_primitives::{
    Address, B256, BlockNumber, ChainId, U256, address, keccak256, map::HashMap, uint,
};
use anyhow::{Result, anyhow, bail};
use reth_chainspec::{ChainSpec as RethChainSpec, HOODI as RETH_HOODI, MAINNET as RETH_MAINNET};
use reth_revm::primitives::hardfork::SpecId;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

const DEFAULT_CHAIN_SPECS: &str = include_str!("../../../config/chain_spec_list_default.json");
pub const SHASTA_SIGNAL_SERVICE_CHECKPOINTS_SLOT: u64 = 254;

#[must_use]
pub fn shasta_checkpoint_storage_slots(block_number: u64) -> (U256, U256) {
    let mut encoded = [0u8; 64];
    encoded[..32].copy_from_slice(&U256::from(block_number).to_be_bytes::<32>());
    encoded[32..]
        .copy_from_slice(&U256::from(SHASTA_SIGNAL_SERVICE_CHECKPOINTS_SLOT).to_be_bytes::<32>());

    let block_hash_slot = U256::from_be_slice(keccak256(encoded).as_slice());
    let state_root_slot = block_hash_slot + U256::from(1);
    (block_hash_slot, state_root_slot)
}

#[must_use]
pub fn storage_slot_key(slot: U256) -> B256 {
    B256::from(slot.to_be_bytes::<32>())
}

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
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub fn merge_from_file(file_path: PathBuf) -> Result<SupportedChainSpecs> {
        let mut known_chain_specs = SupportedChainSpecs::default();
        let config = std::fs::read(file_path)?;
        let chain_spec_list: Vec<ChainSpec> = serde_json::from_slice(&config)?;
        let new_chain_specs = chain_spec_list
            .into_iter()
            .map(|cs| (cs.name.clone(), cs))
            .collect::<HashMap<String, ChainSpec>>();

        // override known specs
        known_chain_specs.0.extend(new_chain_specs);
        Ok(known_chain_specs)
    }

    #[must_use]
    pub fn supported_networks(&self) -> Vec<String> {
        self.0.keys().cloned().collect()
    }

    #[must_use]
    pub fn get_chain_spec(&self, network: &str) -> Option<ChainSpec> {
        self.0.get(network).cloned()
    }

    #[must_use]
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
        // IMPORTANT:
        // - For binary formats (bincode in SP1 stdin), deserialize using the standard enum
        //   representation to stay symmetric with the derived `Serialize`.
        // - For human-readable formats (JSON), keep backward-compatible parsing of the legacy
        //   object/string forms.
        if !deserializer.is_human_readable() {
            #[derive(Deserialize)]
            enum StdForkCondition {
                Block(BlockNumber),
                Timestamp(u64),
                Tbd,
            }
            let v = StdForkCondition::deserialize(deserializer)?;
            return Ok(match v {
                StdForkCondition::Block(n) => ForkCondition::Block(n),
                StdForkCondition::Timestamp(ts) => ForkCondition::Timestamp(ts),
                StdForkCondition::Tbd => ForkCondition::Tbd,
            });
        }

        // Human-readable (JSON): handle both "TBD" (from JSON) and "Tbd" (Rust enum variant)
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
            Value::String(s) => match s.as_str() {
                "TBD" | "Tbd" => Ok(ForkCondition::Tbd),
                _ => Err(serde::de::Error::custom(format!(
                    "Unknown ForkCondition variant: {s}"
                ))),
            },
            _ => Err(serde::de::Error::custom("Invalid ForkCondition format")),
        }
    }
}

impl ForkCondition {
    /// Returns whether the condition has been met.
    #[must_use]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TaikoFork {
    Hekla,
    Ontake,
    Pacaya,
    Shasta,
    Unzen,
}

impl TaikoFork {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Hekla => "HEKLA",
            Self::Ontake => "ONTAKE",
            Self::Pacaya => "PACAYA",
            Self::Shasta => "SHASTA",
            Self::Unzen => "UNZEN",
        }
    }

    fn from_str(value: &str) -> Result<Self, String> {
        match value {
            "HEKLA" => Ok(Self::Hekla),
            "ONTAKE" => Ok(Self::Ontake),
            "PACAYA" => Ok(Self::Pacaya),
            "SHASTA" => Ok(Self::Shasta),
            "UNZEN" => Ok(Self::Unzen),
            _ => Err(format!("unknown Taiko fork: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ForkId {
    Standard(SpecId),
    Taiko(TaikoFork),
}

impl ForkId {
    const fn as_spec_id(self) -> SpecId {
        match self {
            Self::Standard(spec_id) => spec_id,
            Self::Taiko(TaikoFork::Unzen) => SpecId::OSAKA,
            Self::Taiko(_) => SpecId::SHANGHAI,
        }
    }

    fn from_str(value: &str) -> Result<Self, String> {
        if let Ok(fork) = TaikoFork::from_str(value) {
            return Ok(Self::Taiko(fork));
        }
        serde_json::from_str(&format!("\"{value}\""))
            .map(Self::Standard)
            .map_err(|_| format!("unknown SpecId variant: {value}"))
    }
}

impl Serialize for ForkId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if !serializer.is_human_readable() {
            #[derive(Serialize)]
            enum BinaryForkId {
                Standard(SpecId),
                Taiko(TaikoFork),
            }

            return match self {
                Self::Standard(spec_id) => BinaryForkId::Standard(*spec_id).serialize(serializer),
                Self::Taiko(fork) => BinaryForkId::Taiko(*fork).serialize(serializer),
            };
        }

        match self {
            Self::Standard(spec_id) => spec_id.serialize(serializer),
            Self::Taiko(fork) => serializer.serialize_str(fork.as_str()),
        }
    }
}

impl<'de> Deserialize<'de> for ForkId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if !deserializer.is_human_readable() {
            #[derive(Deserialize)]
            enum BinaryForkId {
                Standard(SpecId),
                Taiko(TaikoFork),
            }

            return match BinaryForkId::deserialize(deserializer)? {
                BinaryForkId::Standard(spec_id) => Ok(Self::Standard(spec_id)),
                BinaryForkId::Taiko(fork) => Ok(Self::Taiko(fork)),
            };
        }

        let s = String::deserialize(deserializer)?;
        Self::from_str(&s).map_err(serde::de::Error::custom)
    }
}

fn parse_spec_id_str(value: &str) -> Result<SpecId, String> {
    ForkId::from_str(value).map(ForkId::as_spec_id)
}

fn deserialize_optional_spec_id<'de, D>(deserializer: D) -> Result<Option<SpecId>, D::Error>
where
    D: Deserializer<'de>,
{
    if !deserializer.is_human_readable() {
        return Option::<SpecId>::deserialize(deserializer);
    }

    let Some(value) = Option::<String>::deserialize(deserializer)? else {
        return Ok(None);
    };

    parse_spec_id_str(&value)
        .map(Some)
        .map_err(serde::de::Error::custom)
}

/// Custom deserializer for `SpecId` that handles Taiko-specific fork names.
fn deserialize_spec_id<'de, D>(deserializer: D) -> Result<SpecId, D::Error>
where
    D: Deserializer<'de>,
{
    // For binary formats (bincode), deserialize the enum directly so it stays symmetric
    // with `Serialize` (and works across host/guest).
    if !deserializer.is_human_readable() {
        return SpecId::deserialize(deserializer);
    }

    // For JSON, accept fork names as strings (including Taiko-specific names).
    let s = String::deserialize(deserializer)?;
    parse_spec_id_str(&s).map_err(serde::de::Error::custom)
}

/// Custom deserializer for `BTreeMap<ForkId, T>` that preserves Taiko fork identity.
fn deserialize_fork_id_map<'de, D, T>(deserializer: D) -> Result<BTreeMap<ForkId, T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    // For binary formats (bincode), deserialize the map directly so it stays symmetric with
    // `Serialize`. The string-key workaround is only for human-readable JSON inputs.
    if !deserializer.is_human_readable() {
        return BTreeMap::<ForkId, T>::deserialize(deserializer);
    }

    // First deserialize as a map with string keys
    let string_map: BTreeMap<String, T> = BTreeMap::deserialize(deserializer)?;
    let mut spec_id_map = BTreeMap::new();

    for (key, value) in string_map {
        let spec_id = ForkId::from_str(&key).map_err(serde::de::Error::custom)?;
        spec_id_map.insert(spec_id, value);
    }
    Ok(spec_id_map)
}

fn deserialize_optional_fork_id_map<'de, D, T>(
    deserializer: D,
) -> Result<Option<BTreeMap<ForkId, T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    if !deserializer.is_human_readable() {
        return Option::<BTreeMap<ForkId, T>>::deserialize(deserializer);
    }

    let Some(string_map) = Option::<BTreeMap<String, T>>::deserialize(deserializer)? else {
        return Ok(None);
    };

    let mut spec_id_map = BTreeMap::new();
    for (key, value) in string_map {
        let spec_id = ForkId::from_str(&key).map_err(serde::de::Error::custom)?;
        spec_id_map.insert(spec_id, value);
    }

    Ok(Some(spec_id_map))
}

/// Specification of a specific chain.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct ChainSpec {
    pub name: String,
    pub chain_id: ChainId,
    pub max_spec_id: SpecId,
    pub hard_forks: BTreeMap<ForkId, ForkCondition>,
    pub eip_1559_constants: Eip1559Constants,
    pub l1_contract: BTreeMap<ForkId, Address>,
    pub l2_contract: Option<Address>,
    pub checkpoint_store_contract: Option<Address>,
    pub rpc: String,
    pub beacon_rpc: Option<String>,
    pub verifier_address_forks: VerifierAddressForks,
    pub genesis_time: u64,
    pub seconds_per_slot: u64,
    pub is_taiko: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuestInputAbi {
    #[default]
    Current,
    V0_1_0,
}

type VerifierAddressForks = BTreeMap<ForkId, BTreeMap<ProofType, Option<Address>>>;

#[derive(Deserialize)]
struct BinaryChainSpecHelper {
    name: String,
    chain_id: ChainId,
    #[serde(deserialize_with = "deserialize_spec_id")]
    max_spec_id: SpecId,
    #[serde(deserialize_with = "deserialize_fork_id_map")]
    hard_forks: BTreeMap<ForkId, ForkCondition>,
    eip_1559_constants: Eip1559Constants,
    #[serde(default, deserialize_with = "deserialize_fork_id_map")]
    l1_contract: BTreeMap<ForkId, Address>,
    l2_contract: Option<Address>,
    #[serde(default)]
    checkpoint_store_contract: Option<Address>,
    rpc: String,
    beacon_rpc: Option<String>,
    #[serde(deserialize_with = "deserialize_verifier_address_forks")]
    verifier_address_forks: VerifierAddressForks,
    genesis_time: u64,
    seconds_per_slot: u64,
    is_taiko: bool,
}

impl From<BinaryChainSpecHelper> for ChainSpec {
    fn from(helper: BinaryChainSpecHelper) -> Self {
        Self {
            name: helper.name,
            chain_id: helper.chain_id,
            max_spec_id: helper.max_spec_id,
            hard_forks: helper.hard_forks,
            eip_1559_constants: helper.eip_1559_constants,
            l1_contract: helper.l1_contract,
            l2_contract: helper.l2_contract,
            checkpoint_store_contract: helper.checkpoint_store_contract,
            rpc: helper.rpc,
            beacon_rpc: helper.beacon_rpc,
            verifier_address_forks: helper.verifier_address_forks,
            genesis_time: helper.genesis_time,
            seconds_per_slot: helper.seconds_per_slot,
            is_taiko: helper.is_taiko,
        }
    }
}

#[derive(Deserialize)]
struct JsonChainSpecHelper {
    name: String,
    #[serde(default)]
    chain_id: Option<ChainId>,
    #[serde(default, deserialize_with = "deserialize_optional_spec_id")]
    max_spec_id: Option<SpecId>,
    #[serde(default, deserialize_with = "deserialize_optional_fork_id_map")]
    hard_forks: Option<BTreeMap<ForkId, ForkCondition>>,
    #[serde(default)]
    eip_1559_constants: Option<Eip1559Constants>,
    #[serde(default, deserialize_with = "deserialize_fork_id_map")]
    l1_contract: BTreeMap<ForkId, Address>,
    l2_contract: Option<Address>,
    #[serde(default)]
    checkpoint_store_contract: Option<Address>,
    rpc: String,
    beacon_rpc: Option<String>,
    #[serde(deserialize_with = "deserialize_verifier_address_forks")]
    verifier_address_forks: VerifierAddressForks,
    genesis_time: u64,
    seconds_per_slot: u64,
    #[serde(default)]
    is_taiko: Option<bool>,
}

#[derive(Debug, Clone)]
struct CanonicalChainSpec {
    chain_id: ChainId,
    max_spec_id: SpecId,
    hard_forks: BTreeMap<ForkId, ForkCondition>,
    eip_1559_constants: Eip1559Constants,
    is_taiko: bool,
}

const ETHEREUM_EXECUTION_FORKS: &[(EthereumHardfork, SpecId)] = &[
    (EthereumHardfork::Frontier, SpecId::FRONTIER),
    (EthereumHardfork::Homestead, SpecId::HOMESTEAD),
    (EthereumHardfork::Dao, SpecId::DAO_FORK),
    (EthereumHardfork::Tangerine, SpecId::TANGERINE),
    (EthereumHardfork::SpuriousDragon, SpecId::SPURIOUS_DRAGON),
    (EthereumHardfork::Byzantium, SpecId::BYZANTIUM),
    (EthereumHardfork::Constantinople, SpecId::CONSTANTINOPLE),
    (EthereumHardfork::Petersburg, SpecId::PETERSBURG),
    (EthereumHardfork::Istanbul, SpecId::ISTANBUL),
    (EthereumHardfork::MuirGlacier, SpecId::MUIR_GLACIER),
    (EthereumHardfork::Berlin, SpecId::BERLIN),
    (EthereumHardfork::London, SpecId::LONDON),
    (EthereumHardfork::ArrowGlacier, SpecId::ARROW_GLACIER),
    (EthereumHardfork::GrayGlacier, SpecId::GRAY_GLACIER),
    (EthereumHardfork::Paris, SpecId::MERGE),
    (EthereumHardfork::Shanghai, SpecId::SHANGHAI),
    (EthereumHardfork::Cancun, SpecId::CANCUN),
    (EthereumHardfork::Prague, SpecId::PRAGUE),
    (EthereumHardfork::Osaka, SpecId::OSAKA),
    (EthereumHardfork::Amsterdam, SpecId::AMSTERDAM),
];

const TAIKO_EXECUTION_FORKS: &[(AlethiaTaikoHardfork, TaikoFork)] = &[
    (AlethiaTaikoHardfork::Ontake, TaikoFork::Ontake),
    (AlethiaTaikoHardfork::Pacaya, TaikoFork::Pacaya),
    (AlethiaTaikoHardfork::Shasta, TaikoFork::Shasta),
    (AlethiaTaikoHardfork::Unzen, TaikoFork::Unzen),
];

fn canonical_chain_spec(name: &str) -> Option<CanonicalChainSpec> {
    match name {
        "ethereum" => Some(canonical_l1_chain_spec(RETH_MAINNET.as_ref())),
        "hoodi" => Some(canonical_l1_chain_spec(RETH_HOODI.as_ref())),
        "taiko_mainnet" => Some(canonical_taiko_chain_spec(TAIKO_MAINNET.as_ref())),
        "taiko_dev" => Some(canonical_taiko_chain_spec(TAIKO_DEVNET.as_ref())),
        "taiko_masaya" => Some(canonical_taiko_chain_spec(TAIKO_MASAYA.as_ref())),
        "taiko_hoodi" => Some(canonical_taiko_chain_spec(TAIKO_HOODI.as_ref())),
        _ => None,
    }
}

fn canonical_taiko_fork_condition(name: &str, fork: TaikoFork) -> Option<ForkCondition> {
    canonical_chain_spec(name)?
        .hard_forks
        .get(&ForkId::Taiko(fork))
        .cloned()
}

fn canonical_l1_chain_spec(spec: &RethChainSpec) -> CanonicalChainSpec {
    let hard_forks = ETHEREUM_EXECUTION_FORKS
        .iter()
        .filter_map(|(fork, spec_id)| {
            let condition = spec.inner_hardfork_condition(*fork)?;
            Some((ForkId::Standard(*spec_id), condition))
        })
        .collect::<BTreeMap<_, _>>();

    CanonicalChainSpec {
        chain_id: spec.chain.id(),
        max_spec_id: max_spec_id(&hard_forks),
        hard_forks,
        eip_1559_constants: Eip1559Constants::default(),
        is_taiko: false,
    }
}

fn canonical_taiko_chain_spec(spec: &TaikoChainSpec) -> CanonicalChainSpec {
    let hard_forks = TAIKO_EXECUTION_FORKS
        .iter()
        .map(|(alethia_fork, taiko_fork)| {
            (
                ForkId::Taiko(*taiko_fork),
                from_alethia_fork_condition(spec.inner.hardforks.fork(*alethia_fork)),
            )
        })
        .collect::<BTreeMap<_, _>>();

    CanonicalChainSpec {
        chain_id: spec.inner.chain.id(),
        max_spec_id: max_spec_id(&hard_forks),
        hard_forks,
        eip_1559_constants: Eip1559Constants::default(),
        is_taiko: true,
    }
}

trait RethChainSpecExt {
    fn inner_hardfork_condition(&self, fork: EthereumHardfork) -> Option<ForkCondition>;
}

impl RethChainSpecExt for RethChainSpec {
    fn inner_hardfork_condition(&self, fork: EthereumHardfork) -> Option<ForkCondition> {
        let condition = self.hardforks.get(fork)?;
        Some(from_alethia_fork_condition(condition))
    }
}

fn from_alethia_fork_condition(condition: AlethiaForkCondition) -> ForkCondition {
    match condition {
        AlethiaForkCondition::Block(block) => ForkCondition::Block(block),
        AlethiaForkCondition::TTD {
            activation_block_number,
            fork_block,
            ..
        } => ForkCondition::Block(fork_block.unwrap_or(activation_block_number)),
        AlethiaForkCondition::Timestamp(timestamp) => ForkCondition::Timestamp(timestamp),
        AlethiaForkCondition::Never => ForkCondition::Tbd,
    }
}

fn max_spec_id(hard_forks: &BTreeMap<ForkId, ForkCondition>) -> SpecId {
    hard_forks
        .keys()
        .map(|fork_id| fork_id.as_spec_id())
        .max()
        .unwrap_or(SpecId::FRONTIER)
}

fn chain_spec_from_json_helper<E>(helper: JsonChainSpecHelper) -> Result<ChainSpec, E>
where
    E: serde::de::Error,
{
    let canonical = canonical_chain_spec(&helper.name);
    let chain_id = helper
        .chain_id
        .or_else(|| canonical.as_ref().map(|spec| spec.chain_id))
        .ok_or_else(|| missing_chain_spec_field::<E>(&helper.name, "chain_id"))?;
    let max_spec_id = helper
        .max_spec_id
        .or_else(|| canonical.as_ref().map(|spec| spec.max_spec_id))
        .ok_or_else(|| missing_chain_spec_field::<E>(&helper.name, "max_spec_id"))?;
    let hard_forks = helper
        .hard_forks
        .or_else(|| canonical.as_ref().map(|spec| spec.hard_forks.clone()))
        .ok_or_else(|| missing_chain_spec_field::<E>(&helper.name, "hard_forks"))?;
    let eip_1559_constants = helper
        .eip_1559_constants
        .or_else(|| canonical.as_ref().map(|spec| spec.eip_1559_constants))
        .ok_or_else(|| missing_chain_spec_field::<E>(&helper.name, "eip_1559_constants"))?;
    let is_taiko = helper
        .is_taiko
        .or_else(|| canonical.as_ref().map(|spec| spec.is_taiko))
        .ok_or_else(|| missing_chain_spec_field::<E>(&helper.name, "is_taiko"))?;

    Ok(ChainSpec {
        name: helper.name,
        chain_id,
        max_spec_id,
        hard_forks,
        eip_1559_constants,
        l1_contract: helper.l1_contract,
        l2_contract: helper.l2_contract,
        checkpoint_store_contract: helper.checkpoint_store_contract,
        rpc: helper.rpc,
        beacon_rpc: helper.beacon_rpc,
        verifier_address_forks: helper.verifier_address_forks,
        genesis_time: helper.genesis_time,
        seconds_per_slot: helper.seconds_per_slot,
        is_taiko,
    })
}

fn missing_chain_spec_field<E>(name: &str, field: &str) -> E
where
    E: serde::de::Error,
{
    serde::de::Error::custom(format!("missing {field} for unknown chain spec {name}"))
}

fn v0_1_0_guest_input_hard_forks(name: &str) -> Option<(SpecId, BTreeMap<ForkId, ForkCondition>)> {
    let hard_forks = match name {
        "ethereum" => BTreeMap::from([
            (ForkId::Standard(SpecId::FRONTIER), ForkCondition::Block(0)),
            (
                ForkId::Standard(SpecId::MERGE),
                ForkCondition::Block(15_537_394),
            ),
            (
                ForkId::Standard(SpecId::SHANGHAI),
                ForkCondition::Block(17_034_870),
            ),
            (
                ForkId::Standard(SpecId::CANCUN),
                ForkCondition::Timestamp(1_710_338_135),
            ),
        ]),
        "hoodi" => BTreeMap::from([
            (ForkId::Standard(SpecId::FRONTIER), ForkCondition::Block(0)),
            (
                ForkId::Standard(SpecId::SHANGHAI),
                ForkCondition::Timestamp(1_696_000_704),
            ),
            (
                ForkId::Standard(SpecId::CANCUN),
                ForkCondition::Timestamp(1_707_305_664),
            ),
        ]),
        "taiko_mainnet" => BTreeMap::from([(
            ForkId::Taiko(TaikoFork::Shasta),
            canonical_taiko_fork_condition("taiko_mainnet", TaikoFork::Shasta)?,
        )]),
        "taiko_dev" => BTreeMap::from([
            (ForkId::Taiko(TaikoFork::Unzen), ForkCondition::Timestamp(0)),
            (
                ForkId::Standard(SpecId::CANCUN),
                ForkCondition::Timestamp(0),
            ),
        ]),
        "taiko_masaya" => BTreeMap::from([
            (ForkId::Taiko(TaikoFork::Hekla), ForkCondition::Block(0)),
            (ForkId::Taiko(TaikoFork::Ontake), ForkCondition::Block(0)),
            (ForkId::Taiko(TaikoFork::Pacaya), ForkCondition::Block(0)),
            (
                ForkId::Taiko(TaikoFork::Shasta),
                ForkCondition::Timestamp(0),
            ),
            (
                ForkId::Taiko(TaikoFork::Unzen),
                ForkCondition::Timestamp(1_778_158_800),
            ),
            (ForkId::Standard(SpecId::CANCUN), ForkCondition::Tbd),
        ]),
        "taiko_hoodi" => BTreeMap::from([
            (ForkId::Taiko(TaikoFork::Hekla), ForkCondition::Block(0)),
            (ForkId::Taiko(TaikoFork::Ontake), ForkCondition::Block(0)),
            (ForkId::Taiko(TaikoFork::Pacaya), ForkCondition::Block(0)),
            (
                ForkId::Taiko(TaikoFork::Shasta),
                ForkCondition::Timestamp(1_770_296_400),
            ),
            (ForkId::Taiko(TaikoFork::Unzen), ForkCondition::Tbd),
            (ForkId::Standard(SpecId::CANCUN), ForkCondition::Tbd),
        ]),
        _ => return None,
    };

    let max_spec_id = match name {
        "taiko_mainnet" => SpecId::CANCUN,
        _ => max_spec_id(&hard_forks),
    };

    Some((max_spec_id, hard_forks))
}

fn v0_1_0_guest_input_l1_contracts(name: &str) -> Option<BTreeMap<ForkId, Address>> {
    match name {
        "taiko_mainnet" => Some(BTreeMap::from([(
            ForkId::Taiko(TaikoFork::Shasta),
            address!("6f21C543a4aF5189eBdb0723827577e1EF57ef1f"),
        )])),
        "taiko_dev" => Some(BTreeMap::from([(
            ForkId::Standard(SpecId::CANCUN),
            address!("83e383dec6E3C2CD167E3bF6aA8c36F0e55Ad910"),
        )])),
        "taiko_masaya" => Some(BTreeMap::from([(
            ForkId::Taiko(TaikoFork::Shasta),
            address!("3477f9e8a890c2286c5e62150ad6593eef4590b9"),
        )])),
        "taiko_hoodi" => Some(BTreeMap::from([
            (
                ForkId::Taiko(TaikoFork::Pacaya),
                address!("f6eA848c7d7aC83de84db45Ae28EAbf377fe0eF9"),
            ),
            (
                ForkId::Taiko(TaikoFork::Shasta),
                address!("eF4bB7A442Bd68150A3aa61A6a097B86b91700BF"),
            ),
        ])),
        _ => None,
    }
}

fn v0_1_0_guest_input_verifier_address_forks(name: &str) -> Option<VerifierAddressForks> {
    match name {
        "taiko_mainnet" => Some(BTreeMap::from([(
            ForkId::Taiko(TaikoFork::Shasta),
            verifiers([
                (
                    ProofType::Sgx,
                    Some(address!("a1018Ba2e22139076f91dA2A856B2CAB22d968F6")),
                ),
                (
                    ProofType::Sp1,
                    Some(address!("73A0Db393ef87ce781ac7957bE10D6628432100F")),
                ),
                (
                    ProofType::Risc0,
                    Some(address!("059dAF31F571da48Ab4e74Ae12F64f907681Cd8b")),
                ),
            ]),
        )])),
        "taiko_dev" => Some(BTreeMap::from([(
            ForkId::Standard(SpecId::CANCUN),
            verifiers([
                (
                    ProofType::Sgx,
                    Some(address!("936d8dCd9B731D3fe146BF3E1520e9d790A3a67d")),
                ),
                (
                    ProofType::Sp1,
                    Some(address!("9d351f6e72e3095f24dd854c9b8ca69f99a2c538")),
                ),
                (
                    ProofType::Risc0,
                    Some(address!("fe3edf3e778a647c0955f2a5f79565e272e6afdb")),
                ),
            ]),
        )])),
        "taiko_masaya" => Some(BTreeMap::from([(
            ForkId::Taiko(TaikoFork::Shasta),
            same_verifier(address!("2c47Bf9b02B6Cbe6A73244F38271d36c99D9c815")),
        )])),
        "taiko_hoodi" => Some(BTreeMap::from([
            (
                ForkId::Taiko(TaikoFork::Pacaya),
                verifiers([
                    (
                        ProofType::Sgx,
                        Some(address!("d46c13B67396cD1e74Bb40e298fbABeA7DC01f11")),
                    ),
                    (
                        ProofType::Sp1,
                        Some(address!("3B3bb4A1Cb8B1A0D65F96a5A93415375C039Eda3")),
                    ),
                    (
                        ProofType::Risc0,
                        Some(address!("bf285Dd2FD56BF4893D207Fba4c738D1029edFfd")),
                    ),
                ]),
            ),
            (
                ForkId::Taiko(TaikoFork::Shasta),
                verifiers([
                    (
                        ProofType::Sgx,
                        Some(address!("40CcAFC1C2D14bdD70984b221F2b49af5e7C6114")),
                    ),
                    (
                        ProofType::Risc0,
                        Some(address!("fa0e7dAFe9785627df034c123A9B87497EB06b41")),
                    ),
                    (
                        ProofType::Sp1,
                        Some(address!("c42Ef1A7A606162e144F696A07A7D3Ad98bF4EE7")),
                    ),
                ]),
            ),
        ])),
        _ => None,
    }
}

fn verifiers<const N: usize>(
    entries: [(ProofType, Option<Address>); N],
) -> BTreeMap<ProofType, Option<Address>> {
    BTreeMap::from(entries)
}

fn same_verifier(address: Address) -> BTreeMap<ProofType, Option<Address>> {
    verifiers([
        (ProofType::Sgx, Some(address)),
        (ProofType::Sp1, Some(address)),
        (ProofType::Risc0, Some(address)),
    ])
}

fn parse_proof_type_str(value: &str) -> Result<ProofType, String> {
    match value {
        "NATIVE" | "Native" => Ok(ProofType::Native),
        "SP1" | "Sp1" => Ok(ProofType::Sp1),
        "SGX" | "Sgx" => Ok(ProofType::Sgx),
        "SGXGETH" | "SgxGeth" => Ok(ProofType::SgxGeth),
        "RISC0" | "Risc0" => Ok(ProofType::Risc0),
        _ => Err(format!("unknown ProofType variant: {value}")),
    }
}

/// Custom deserializer for `verifier_address_forks` nested map structure.
fn deserialize_verifier_address_forks<'de, D>(
    deserializer: D,
) -> Result<VerifierAddressForks, D::Error>
where
    D: Deserializer<'de>,
{
    // For binary formats (bincode), deserialize the nested map directly so it stays symmetric with
    // `Serialize`. The string-key conversion is only for human-readable JSON inputs.
    if !deserializer.is_human_readable() {
        return VerifierAddressForks::deserialize(deserializer);
    }

    // First deserialize as a map with string keys, where inner map also has string keys
    let string_map: BTreeMap<String, BTreeMap<String, Option<Address>>> =
        BTreeMap::deserialize(deserializer)?;

    // Convert string keys to ForkId and inner string keys to ProofType
    let mut spec_id_map: VerifierAddressForks = BTreeMap::new();
    for (key, inner_map) in string_map {
        let spec_id = ForkId::from_str(&key).map_err(serde::de::Error::custom)?;

        // Convert inner map string proof types to the canonical enum.
        let mut proof_type_map = BTreeMap::new();
        for (proof_key, address) in inner_map {
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
        if !deserializer.is_human_readable() {
            let helper = BinaryChainSpecHelper::deserialize(deserializer)?;
            return Ok(helper.into());
        }

        let helper = JsonChainSpecHelper::deserialize(deserializer)?;
        chain_spec_from_json_helper(helper)
    }
}

impl ChainSpec {
    #[must_use]
    pub fn project_for_guest_input_abi(&self, abi: GuestInputAbi) -> Self {
        let mut spec = self.clone();
        match abi {
            GuestInputAbi::Current => {}
            GuestInputAbi::V0_1_0 => spec.apply_v0_1_0_guest_input_compat(),
        }
        spec
    }

    fn apply_v0_1_0_guest_input_compat(&mut self) {
        if let Some((max_spec_id, hard_forks)) = v0_1_0_guest_input_hard_forks(&self.name) {
            self.max_spec_id = max_spec_id;
            self.hard_forks = hard_forks;
        }
        if let Some(l1_contract) = v0_1_0_guest_input_l1_contracts(&self.name) {
            self.l1_contract = l1_contract;
        }
        if let Some(verifier_address_forks) = v0_1_0_guest_input_verifier_address_forks(&self.name)
        {
            self.verifier_address_forks = verifier_address_forks;
        }

        self.remove_fork_verifier_proof_type(ProofType::SgxGeth);
    }

    /// Removes a verifier proof type from every configured fork.
    pub fn remove_fork_verifier_proof_type(&mut self, proof_type: ProofType) {
        for fork_verifier in self.verifier_address_forks.values_mut() {
            fork_verifier.remove(&proof_type);
        }
    }

    /// Creates a new configuration consisting of only one specification ID.
    #[must_use]
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
            hard_forks: BTreeMap::from([(ForkId::Standard(spec_id), ForkCondition::Block(0))]),
            eip_1559_constants,
            l1_contract: BTreeMap::new(),
            l2_contract: None,
            checkpoint_store_contract: None,
            rpc: String::new(),
            beacon_rpc: None,
            verifier_address_forks: BTreeMap::new(),
            genesis_time: 0u64,
            seconds_per_slot: 1u64,
            is_taiko,
        }
    }

    /// Returns the network chain ID.
    #[must_use]
    pub const fn chain_id(&self) -> ChainId {
        self.chain_id
    }

    /// Returns the [`SpecId`] for a given block number and timestamp or an error if not
    /// supported.
    ///
    /// # Errors
    ///
    /// Returns an error if no active fork matches or if the spec exceeds `max_spec_id`.
    pub fn active_fork(&self, block_no: BlockNumber, timestamp: u64) -> Result<SpecId> {
        match self.active_fork_id(block_no, timestamp) {
            Some(fork_id) => {
                let spec_id = fork_id.as_spec_id();
                if spec_id > self.max_spec_id {
                    bail!("expected <= {:?}, got {spec_id:?}", self.max_spec_id);
                }
                Ok(spec_id)
            }
            None => Err(anyhow!("no supported fork for block {block_no}")),
        }
    }

    /// Returns the Eip1559 constants
    #[must_use]
    pub const fn gas_constants(&self) -> &Eip1559Constants {
        &self.eip_1559_constants
    }

    #[must_use]
    pub fn spec_id(&self, block_no: BlockNumber, timestamp: u64) -> Option<SpecId> {
        self.active_fork_id(block_no, timestamp)
            .map(ForkId::as_spec_id)
    }

    #[must_use]
    fn active_fork_id(&self, block_no: BlockNumber, timestamp: u64) -> Option<ForkId> {
        for (fork_id, fork) in self.hard_forks.iter().rev() {
            if fork.active(block_no, timestamp) {
                return Some(*fork_id);
            }
        }
        None
    }

    fn active_configured_fork_value<'a, T>(
        &self,
        map: &'a BTreeMap<ForkId, T>,
        block_no: BlockNumber,
        timestamp: u64,
    ) -> Option<(ForkId, &'a T)> {
        let active_fork_id = self.active_fork_id(block_no, timestamp)?;
        map.iter().rev().find_map(|(fork_id, value)| {
            if *fork_id <= active_fork_id
                && self
                    .hard_forks
                    .get(fork_id)
                    .is_some_and(|fork| fork.active(block_no, timestamp))
            {
                Some((*fork_id, value))
            } else {
                None
            }
        })
    }

    /// # Errors
    ///
    /// Returns an error if no active fork or verifier address is configured.
    pub fn get_fork_verifier_address(
        &self,
        block_num: u64,
        block_timestamp: u64,
        proof_type: ProofType,
    ) -> Result<Address> {
        if let Some((_fork_id, fork_verifier)) = self.active_configured_fork_value(
            &self.verifier_address_forks,
            block_num,
            block_timestamp,
        ) {
            return fork_verifier
                .get(&proof_type)
                .ok_or_else(|| anyhow!("Verifier type not found"))
                .and_then(|address| address.ok_or_else(|| anyhow!("Verifier address not found")));
        }

        Err(anyhow!("fork verifier is not active"))
    }

    /// # Errors
    ///
    /// Returns an error if no active fork has an L1 contract address configured.
    pub fn get_fork_l1_contract_address(&self, block_num: u64) -> Result<Address> {
        self.get_fork_l1_contract_address_at(block_num, 0)
    }

    /// # Errors
    ///
    /// Returns an error if no active fork has an L1 contract address configured for the provided
    /// block number and timestamp.
    pub fn get_fork_l1_contract_address_at(
        &self,
        block_num: u64,
        block_timestamp: u64,
    ) -> Result<Address> {
        if let Some((_fork_id, l1_address)) =
            self.active_configured_fork_value(&self.l1_contract, block_num, block_timestamp)
        {
            return Ok(*l1_address);
        }

        Err(anyhow!("fork l1 contract is not active"))
    }

    #[must_use]
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
    /// - `167011` (Taiko Masaya)
    /// - `167013` (Taiko Hoodi)
    ///
    /// Returns an error for non-Taiko chains or unknown Taiko chain IDs.
    ///
    /// # Errors
    ///
    /// Returns an error when the chain is not Taiko or not supported.
    pub fn to_taiko_chain_spec(&self) -> Result<Arc<TaikoChainSpec>> {
        if !self.is_taiko {
            bail!(
                "chain spec is not a Taiko chain and cannot be converted (chain_id={})",
                self.chain_id
            );
        }

        let base = match self.chain_id {
            167_000 => TAIKO_MAINNET.clone(),
            167_001 => TAIKO_DEVNET.clone(),
            167_011 => TAIKO_MASAYA.clone(),
            167_013 => TAIKO_HOODI.clone(),
            other => bail!(
                "unsupported Taiko chain_id={other}; no built-in genesis is available for conversion"
            ),
        };

        let mut spec = self.base_taiko_chain_spec_with_configured_devnet_unzen(base.as_ref())?;
        self.apply_configured_taiko_forks(&mut spec);
        Ok(Arc::new(spec))
    }

    #[must_use]
    pub fn network(&self) -> String {
        self.name.clone()
    }

    fn base_taiko_chain_spec_with_configured_devnet_unzen(
        &self,
        base: &TaikoChainSpec,
    ) -> Result<TaikoChainSpec> {
        if self.chain_id != 167_001 {
            return Ok(base.clone());
        }

        let Some(unzen) = self.hard_forks.get(&ForkId::Taiko(TaikoFork::Unzen)) else {
            return Ok(base.clone());
        };

        match unzen {
            ForkCondition::Timestamp(timestamp) => Ok(base
                .clone_with_devnet_unzen_timestamp(*timestamp)
                .unwrap_or_else(|| base.clone())),
            other => bail!(
                "unsupported devnet Unzen fork condition for chain {}: {other:?}",
                self.name
            ),
        }
    }

    fn apply_configured_taiko_forks(&self, spec: &mut TaikoChainSpec) {
        for (fork_id, condition) in &self.hard_forks {
            let ForkId::Taiko(fork) = fork_id else {
                continue;
            };
            let condition = alethia_fork_condition(condition);
            let Some(alethia_fork) = alethia_taiko_fork(*fork) else {
                continue;
            };
            spec.inner.hardforks.insert(alethia_fork, condition);
            if *fork == TaikoFork::Unzen {
                apply_unzen_eth_forks(spec, condition);
            }
        }
    }
}

const fn alethia_taiko_fork(fork: TaikoFork) -> Option<AlethiaTaikoHardfork> {
    match fork {
        TaikoFork::Hekla => None,
        TaikoFork::Ontake => Some(AlethiaTaikoHardfork::Ontake),
        TaikoFork::Pacaya => Some(AlethiaTaikoHardfork::Pacaya),
        TaikoFork::Shasta => Some(AlethiaTaikoHardfork::Shasta),
        TaikoFork::Unzen => Some(AlethiaTaikoHardfork::Unzen),
    }
}

const fn alethia_fork_condition(condition: &ForkCondition) -> AlethiaForkCondition {
    match condition {
        ForkCondition::Block(block) => AlethiaForkCondition::Block(*block),
        ForkCondition::Timestamp(timestamp) => AlethiaForkCondition::Timestamp(*timestamp),
        ForkCondition::Tbd => AlethiaForkCondition::Never,
    }
}

fn apply_unzen_eth_forks(spec: &mut TaikoChainSpec, condition: AlethiaForkCondition) {
    spec.inner
        .hardforks
        .insert(EthereumHardfork::Cancun, condition);
    spec.inner
        .hardforks
        .insert(EthereumHardfork::Prague, condition);
    spec.inner
        .hardforks
        .insert(EthereumHardfork::Osaka, condition);
}

#[cfg(test)]
mod tests {
    use super::*;
    use alethia_reth_chainspec::{
        TAIKO_DEVNET_GENESIS_HASH, TAIKO_HOODI_GENESIS_HASH, TAIKO_MAINNET_GENESIS_HASH,
        TAIKO_MASAYA_GENESIS_HASH,
        hardfork::{TaikoHardfork, TaikoHardforks as _},
    };
    use alloy_primitives::address;

    const HOODI_UNZEN_TIMESTAMP: u64 = 1_781_787_600;

    fn mainnet_shasta_timestamp() -> u64 {
        match canonical_taiko_fork_condition("taiko_mainnet", TaikoFork::Shasta)
            .expect("taiko mainnet Shasta fork")
        {
            ForkCondition::Timestamp(timestamp) => timestamp,
            condition => panic!("expected mainnet Shasta timestamp fork, got {condition:?}"),
        }
    }

    #[test]
    fn chain_spec_json_to_bincode_roundtrip_default_list() -> Result<()> {
        // Parse the shipped default config list (JSON), then ensure the resulting ChainSpec is
        // bincode roundtrip-safe. This is the core host<->guest compatibility invariant.
        let list: Vec<ChainSpec> = serde_json::from_str(DEFAULT_CHAIN_SPECS)
            .map_err(|e| anyhow!("parse default chain spec list JSON: {e}"))?;
        assert!(
            !list.is_empty(),
            "default chain spec list should not be empty"
        );

        // Pick a deterministic entry (first) to avoid relying on a specific name existing.
        let spec = &list[0];

        let bytes =
            bincode::serialize(spec).map_err(|e| anyhow!("bincode serialize ChainSpec: {e}"))?;
        let decoded: ChainSpec = bincode::deserialize(&bytes)
            .map_err(|e| anyhow!("bincode deserialize ChainSpec: {e}"))?;

        assert_eq!(
            &decoded, spec,
            "ChainSpec changed after bincode roundtrip (host/guest mismatch risk)"
        );
        Ok(())
    }

    #[test]
    fn l1_specs_hydrate_reth_execution_forks() -> Result<()> {
        let list: Vec<ChainSpec> = serde_json::from_str(DEFAULT_CHAIN_SPECS)?;
        let ethereum = list
            .iter()
            .find(|spec| spec.name == "ethereum")
            .ok_or_else(|| anyhow!("missing ethereum spec"))?;
        let hoodi = list
            .iter()
            .find(|spec| spec.name == "hoodi")
            .ok_or_else(|| anyhow!("missing hoodi spec"))?;

        assert_eq!(ethereum.chain_id, RETH_MAINNET.chain.id());
        assert_eq!(ethereum.max_spec_id, SpecId::OSAKA);
        assert_eq!(
            ethereum.hard_forks.get(&ForkId::Standard(SpecId::PRAGUE)),
            Some(&ForkCondition::Timestamp(1_746_612_311))
        );
        assert_eq!(
            ethereum.hard_forks.get(&ForkId::Standard(SpecId::OSAKA)),
            Some(&ForkCondition::Timestamp(1_764_798_551))
        );

        assert_eq!(hoodi.chain_id, RETH_HOODI.chain.id());
        assert_eq!(hoodi.max_spec_id, SpecId::OSAKA);
        assert_eq!(
            hoodi.hard_forks.get(&ForkId::Standard(SpecId::PRAGUE)),
            Some(&ForkCondition::Timestamp(1_742_999_832))
        );
        assert_eq!(
            hoodi.hard_forks.get(&ForkId::Standard(SpecId::OSAKA)),
            Some(&ForkCondition::Timestamp(1_761_677_592))
        );
        Ok(())
    }

    #[test]
    fn converts_taiko_mainnet_to_alethia_taiko_chain_spec() -> Result<()> {
        let spec = ChainSpec::new_single(
            "taiko_mainnet".to_string(),
            167_000,
            SpecId::CANCUN,
            Eip1559Constants::default(),
            true,
        );

        let taiko = spec.to_taiko_chain_spec()?;

        assert_eq!(taiko.inner.genesis_hash(), TAIKO_MAINNET_GENESIS_HASH);
        Ok(())
    }

    #[test]
    fn taiko_mainnet_hydrates_alethia_hardforks_from_overlay() -> Result<()> {
        let raw_list: Vec<Value> = serde_json::from_str(DEFAULT_CHAIN_SPECS)?;
        let raw_spec = raw_list
            .iter()
            .find(|spec| spec.get("name") == Some(&Value::String("taiko_mainnet".to_string())))
            .ok_or_else(|| anyhow!("missing raw taiko_mainnet spec"))?;
        assert!(raw_spec.get("chain_id").is_none());
        assert!(raw_spec.get("max_spec_id").is_none());
        assert!(raw_spec.get("hard_forks").is_none());
        assert!(raw_spec.get("eip_1559_constants").is_none());
        assert!(raw_spec.get("is_taiko").is_none());

        let list: Vec<ChainSpec> = serde_json::from_str(DEFAULT_CHAIN_SPECS)?;
        let spec = list
            .into_iter()
            .find(|spec| spec.name == "taiko_mainnet")
            .ok_or_else(|| anyhow!("missing taiko_mainnet spec"))?;

        assert_eq!(spec.chain_id, 167_000);
        assert_eq!(spec.max_spec_id, SpecId::OSAKA);
        assert!(spec.is_taiko);
        assert_eq!(spec.hard_forks.len(), 4);
        assert_eq!(
            spec.hard_forks.get(&ForkId::Taiko(TaikoFork::Ontake)),
            Some(&ForkCondition::Block(538_304))
        );
        assert_eq!(
            spec.hard_forks.get(&ForkId::Taiko(TaikoFork::Pacaya)),
            Some(&ForkCondition::Block(1_166_000))
        );
        assert_eq!(
            spec.hard_forks.get(&ForkId::Taiko(TaikoFork::Shasta)),
            Some(&ForkCondition::Timestamp(mainnet_shasta_timestamp()))
        );
        assert_eq!(
            spec.hard_forks.get(&ForkId::Taiko(TaikoFork::Unzen)),
            Some(&ForkCondition::Tbd)
        );
        assert_eq!(spec.l1_contract.len(), 1);
        let err = spec
            .get_fork_l1_contract_address_at(5_412_478, mainnet_shasta_timestamp() - 1)
            .expect_err("mainnet config should only activate at Shasta");
        assert!(err.to_string().contains("fork l1 contract is not active"));
        assert_eq!(
            spec.get_fork_verifier_address(5_412_478, mainnet_shasta_timestamp(), ProofType::Sgx)?,
            address!("a1018Ba2e22139076f91dA2A856B2CAB22d968F6")
        );
        assert_eq!(
            spec.get_fork_verifier_address(
                5_412_478,
                mainnet_shasta_timestamp(),
                ProofType::Risc0
            )?,
            address!("059dAF31F571da48Ab4e74Ae12F64f907681Cd8b")
        );
        assert_eq!(
            spec.get_fork_verifier_address(5_412_478, mainnet_shasta_timestamp(), ProofType::Sp1)?,
            address!("73A0Db393ef87ce781ac7957bE10D6628432100F")
        );
        Ok(())
    }

    #[test]
    fn taiko_mainnet_raw_spec_keeps_sgx_geth_shasta_verifier() -> Result<()> {
        let list: Vec<Value> = serde_json::from_str(DEFAULT_CHAIN_SPECS)?;
        let spec = list
            .iter()
            .find(|spec| spec["name"] == "taiko_mainnet")
            .ok_or_else(|| anyhow!("missing taiko_mainnet spec"))?;

        assert_eq!(
            spec["verifier_address_forks"]["SHASTA"]["SGXGETH"].as_str(),
            Some("0x08568Df252ecf37D6C3eFD24f6ca3688118697F1")
        );
        Ok(())
    }

    #[test]
    fn v0_1_0_guest_input_projection_uses_mainnet_branch_shasta_spec() -> Result<()> {
        let list: Vec<ChainSpec> = serde_json::from_str(DEFAULT_CHAIN_SPECS)?;
        let spec = list
            .into_iter()
            .find(|spec| spec.name == "taiko_mainnet")
            .ok_or_else(|| anyhow!("missing taiko_mainnet spec"))?;

        assert!(
            spec.verifier_address_forks
                .values()
                .any(|verifiers| verifiers.contains_key(&ProofType::SgxGeth))
        );
        assert_eq!(
            spec.hard_forks.get(&ForkId::Taiko(TaikoFork::Shasta)),
            Some(&ForkCondition::Timestamp(mainnet_shasta_timestamp()))
        );

        let projected = spec.project_for_guest_input_abi(GuestInputAbi::V0_1_0);

        assert!(
            projected
                .verifier_address_forks
                .values()
                .all(|verifiers| !verifiers.contains_key(&ProofType::SgxGeth))
        );
        assert_eq!(projected.max_spec_id, SpecId::CANCUN);
        assert_eq!(projected.hard_forks.len(), 1);
        assert_eq!(
            projected.hard_forks.get(&ForkId::Taiko(TaikoFork::Shasta)),
            Some(&ForkCondition::Timestamp(mainnet_shasta_timestamp()))
        );
        assert_eq!(projected.l1_contract.len(), 1);
        assert_eq!(
            projected.l1_contract.get(&ForkId::Taiko(TaikoFork::Shasta)),
            Some(&address!("6f21C543a4aF5189eBdb0723827577e1EF57ef1f"))
        );
        assert_eq!(
            projected.get_fork_verifier_address(
                5_412_478,
                mainnet_shasta_timestamp(),
                ProofType::Sgx,
            )?,
            address!("a1018Ba2e22139076f91dA2A856B2CAB22d968F6")
        );
        assert_eq!(
            projected.get_fork_verifier_address(
                5_412_478,
                mainnet_shasta_timestamp(),
                ProofType::Risc0,
            )?,
            address!("059dAF31F571da48Ab4e74Ae12F64f907681Cd8b")
        );
        assert_eq!(
            projected.get_fork_verifier_address(
                5_412_478,
                mainnet_shasta_timestamp(),
                ProofType::Sp1,
            )?,
            address!("73A0Db393ef87ce781ac7957bE10D6628432100F")
        );

        let bytes = bincode::serialize(&projected)
            .map_err(|e| anyhow!("bincode serialize projected ChainSpec: {e}"))?;
        let decoded: ChainSpec = bincode::deserialize(&bytes)
            .map_err(|e| anyhow!("bincode deserialize projected ChainSpec: {e}"))?;
        assert_eq!(decoded, projected);
        Ok(())
    }

    #[test]
    fn taiko_masaya_unzen_inherits_shasta_contracts() -> Result<()> {
        let list: Vec<ChainSpec> = serde_json::from_str(DEFAULT_CHAIN_SPECS)?;
        let spec = list
            .into_iter()
            .find(|spec| spec.name == "taiko_masaya")
            .ok_or_else(|| anyhow!("missing taiko_masaya spec"))?;
        let unzen_timestamp = 1_778_158_800;

        assert_eq!(spec.spec_id(0, unzen_timestamp), Some(SpecId::OSAKA));
        assert_eq!(
            spec.get_fork_l1_contract_address_at(0, unzen_timestamp)?,
            address!("3477f9e8a890c2286c5e62150ad6593eef4590b9")
        );
        assert_eq!(
            spec.get_fork_verifier_address(0, unzen_timestamp, ProofType::Risc0)?,
            address!("2c47Bf9b02B6Cbe6A73244F38271d36c99D9c815")
        );
        assert_eq!(
            spec.get_fork_verifier_address(5_412_478, 1_775_988_339, ProofType::SgxGeth)?,
            address!("2c47Bf9b02B6Cbe6A73244F38271d36c99D9c815")
        );
        Ok(())
    }

    #[test]
    fn converts_taiko_devnet_to_alethia_taiko_chain_spec() -> Result<()> {
        let spec = ChainSpec::new_single(
            "taiko_devnet".to_string(),
            167_001,
            SpecId::CANCUN,
            Eip1559Constants::default(),
            true,
        );

        let taiko = spec.to_taiko_chain_spec()?;

        assert_eq!(taiko.inner.genesis_hash(), TAIKO_DEVNET_GENESIS_HASH);
        Ok(())
    }

    #[test]
    fn taiko_dev_default_spec_matches_sanitized_shasta_devnet() -> Result<()> {
        let list: Vec<ChainSpec> = serde_json::from_str(DEFAULT_CHAIN_SPECS)?;
        let l1_spec = list
            .iter()
            .find(|spec| spec.name == "taiko_dev_l1")
            .ok_or_else(|| anyhow!("missing taiko_dev_l1 spec"))?;
        let l2_spec = list
            .iter()
            .find(|spec| spec.name == "taiko_dev")
            .ok_or_else(|| anyhow!("missing taiko_dev spec"))?;
        assert_eq!(l1_spec.chain_id, 32_382);
        assert_eq!(l1_spec.rpc, "https://l1rpc.internal.taiko.xyz");
        assert_eq!(
            l1_spec.beacon_rpc.as_deref(),
            Some("https://l1beacon.internal.taiko.xyz")
        );
        assert_eq!(l1_spec.genesis_time, 1_782_879_840);
        assert_eq!(l1_spec.seconds_per_slot, 12);
        assert!(!l1_spec.is_taiko);
        assert_eq!(l2_spec.chain_id, 167_001);
        assert_eq!(l2_spec.rpc, "https://rpc.internal.taiko.xyz");
        assert_eq!(
            l2_spec.hard_forks.get(&ForkId::Taiko(TaikoFork::Shasta)),
            Some(&ForkCondition::Timestamp(0))
        );
        assert!(
            !l2_spec
                .hard_forks
                .contains_key(&ForkId::Taiko(TaikoFork::Unzen))
        );
        assert_eq!(
            l2_spec.get_fork_l1_contract_address_at(0, 0)?,
            address!("83e383dec6E3C2CD167E3bF6aA8c36F0e55Ad910")
        );
        assert_eq!(
            l2_spec.get_fork_verifier_address(0, 0, ProofType::Sgx)?,
            address!("936d8dCd9B731D3fe146BF3E1520e9d790A3a67d")
        );
        assert_eq!(
            l2_spec.get_fork_verifier_address(0, 0, ProofType::SgxGeth)?,
            address!("118CB49c7e184D502AFaB1FA1E70b5fBc71Bb998")
        );
        Ok(())
    }

    #[test]
    fn taiko_shasta_helper_maps_to_shanghai_until_unzen() -> Result<()> {
        let list: Vec<ChainSpec> = serde_json::from_str(DEFAULT_CHAIN_SPECS)?;
        let spec = list
            .into_iter()
            .find(|spec| spec.name == "taiko_mainnet")
            .ok_or_else(|| anyhow!("missing taiko_mainnet spec"))?;

        assert_eq!(
            spec.spec_id(5_412_478, mainnet_shasta_timestamp()),
            Some(SpecId::SHANGHAI)
        );
        Ok(())
    }

    #[test]
    fn taiko_devnet_to_alethia_chain_spec_enables_shasta_at_genesis() -> Result<()> {
        let list: Vec<ChainSpec> = serde_json::from_str(DEFAULT_CHAIN_SPECS)?;
        let spec = list
            .into_iter()
            .find(|spec| spec.name == "taiko_dev")
            .ok_or_else(|| anyhow!("missing taiko_dev spec"))?;

        let taiko = spec.to_taiko_chain_spec()?;
        let shasta = taiko.taiko_fork_activation(TaikoHardfork::Shasta);
        let unzen = taiko.taiko_fork_activation(TaikoHardfork::Unzen);

        assert!(
            shasta.active_at_timestamp(0),
            "Shasta must be active at genesis on internal devnet"
        );
        assert!(
            unzen.active_at_timestamp(u64::MAX),
            "Devnet must not force inherited Unzen to Never"
        );
        Ok(())
    }

    #[test]
    fn converts_taiko_hoodi_to_alethia_taiko_chain_spec() -> Result<()> {
        let spec = ChainSpec::new_single(
            "taiko_hoodi".to_string(),
            167_013,
            SpecId::CANCUN,
            Eip1559Constants::default(),
            true,
        );

        let taiko = spec.to_taiko_chain_spec()?;

        assert_eq!(taiko.inner.genesis_hash(), TAIKO_HOODI_GENESIS_HASH);
        Ok(())
    }

    #[test]
    fn taiko_hoodi_default_spec_sets_unzen_timestamp() -> Result<()> {
        let list: Vec<ChainSpec> = serde_json::from_str(DEFAULT_CHAIN_SPECS)?;
        let spec = list
            .into_iter()
            .find(|spec| spec.name == "taiko_hoodi")
            .ok_or_else(|| anyhow!("missing taiko_hoodi spec"))?;

        assert_eq!(
            spec.hard_forks.get(&ForkId::Taiko(TaikoFork::Unzen)),
            Some(&ForkCondition::Timestamp(HOODI_UNZEN_TIMESTAMP))
        );

        let taiko = spec.to_taiko_chain_spec()?;
        let unzen = taiko.taiko_fork_activation(TaikoHardfork::Unzen);

        assert!(unzen.active_at_timestamp(HOODI_UNZEN_TIMESTAMP));
        assert!(!unzen.active_at_timestamp(HOODI_UNZEN_TIMESTAMP - 1));
        Ok(())
    }

    #[test]
    fn converts_taiko_masaya_to_alethia_taiko_chain_spec() -> Result<()> {
        let spec = ChainSpec::new_single(
            "taiko_masaya".to_string(),
            167_011,
            SpecId::CANCUN,
            Eip1559Constants::default(),
            true,
        );

        let taiko = spec.to_taiko_chain_spec()?;

        assert_eq!(taiko.inner.genesis_hash(), TAIKO_MASAYA_GENESIS_HASH);
        Ok(())
    }

    #[test]
    fn taiko_masaya_default_spec_uses_verified_shasta_inbox() -> Result<()> {
        let list: Vec<ChainSpec> = serde_json::from_str(DEFAULT_CHAIN_SPECS)?;
        let spec = list
            .into_iter()
            .find(|spec| spec.name == "taiko_masaya")
            .ok_or_else(|| anyhow!("missing taiko_masaya spec"))?;

        assert_eq!(spec.chain_id, 167_011);
        assert_eq!(
            spec.get_fork_l1_contract_address_at(0, 0)?,
            address!("3477f9e8a890c2286c5e62150ad6593eef4590b9")
        );
        assert_eq!(
            spec.l2_contract,
            Some(address!("1670110000000000000000000000000000010001"))
        );
        assert_eq!(
            spec.get_fork_verifier_address(0, 0, ProofType::Sgx)?,
            address!("2c47Bf9b02B6Cbe6A73244F38271d36c99D9c815")
        );
        Ok(())
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
