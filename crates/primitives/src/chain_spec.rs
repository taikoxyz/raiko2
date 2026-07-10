use crate::proof_type::ProofType;
pub use alethia_reth_chainspec::spec::TaikoChainSpec;
use alethia_reth_chainspec::{TAIKO_DEVNET, TAIKO_HOODI, TAIKO_MAINNET, TAIKO_MASAYA};
use alethia_reth_chainspec::{
    hardfork::TaikoHardfork as AlethiaTaikoHardfork, spec::TaikoDevnetConfigExt,
};
use alloy_hardforks::{EthereumHardfork, ForkCondition as AlethiaForkCondition};
use alloy_primitives::{Address, B256, BlockNumber, ChainId, U256, keccak256, map::HashMap, uint};
use anyhow::{Result, anyhow, bail, ensure};
use reth_chainspec::{ChainSpec as RethChainSpec, HOODI as RETH_HOODI, MAINNET as RETH_MAINNET};
use reth_revm::primitives::hardfork::SpecId;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf, str::FromStr, sync::Arc};

#[cfg(feature = "chain-spec-json")]
const DEFAULT_CHAIN_SPECS: &str = include_str!("../../../config/chain_spec_list_default.json");
#[cfg(not(feature = "chain-spec-json"))]
const DEFAULT_CHAIN_SPECS: &str = "[]";
pub const SHASTA_SIGNAL_SERVICE_CHECKPOINTS_SLOT: u64 = 254;
const SHASTA_TAIKO_L2_ADDRESS_SUFFIX: &str = "10001";
const SHASTA_CHECKPOINT_STORE_ADDRESS_SUFFIX: &str = "5";

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

/// Returns the built-in Alethia Taiko runtime chain spec for a supported Taiko chain ID.
///
/// # Errors
///
/// Returns an error when the chain ID is not one of the built-in Taiko networks.
pub fn builtin_taiko_chain_spec(chain_id: ChainId) -> Result<Arc<TaikoChainSpec>> {
    match chain_id {
        167_000 => Ok(TAIKO_MAINNET.clone()),
        167_001 => Ok(TAIKO_DEVNET.clone()),
        167_011 => Ok(TAIKO_MASAYA.clone()),
        167_013 => Ok(TAIKO_HOODI.clone()),
        other => bail!(
            "unsupported Taiko chain_id={other}; no built-in genesis is available for conversion"
        ),
    }
}

/// Derives the Shasta `TaikoL2` predeploy address from a Taiko chain ID.
///
/// # Errors
///
/// Returns an error if the chain ID cannot fit in the predeploy address format.
pub fn shasta_taiko_l2_address(chain_id: ChainId) -> Result<Address> {
    shasta_predeploy_address(chain_id, SHASTA_TAIKO_L2_ADDRESS_SUFFIX)
}

/// Derives the Shasta `CheckpointStore` predeploy address from a Taiko chain ID.
///
/// # Errors
///
/// Returns an error if the chain ID cannot fit in the predeploy address format.
pub fn shasta_checkpoint_store_address(chain_id: ChainId) -> Result<Address> {
    shasta_predeploy_address(chain_id, SHASTA_CHECKPOINT_STORE_ADDRESS_SUFFIX)
}

fn shasta_predeploy_address(chain_id: ChainId, suffix: &str) -> Result<Address> {
    ensure!(
        chain_id != 0,
        "chain_id must be non-zero to derive Shasta predeploy address"
    );
    let prefix = chain_id.to_string();
    let address_nibbles = 40usize;
    let used_nibbles = prefix.len() + suffix.len();
    ensure!(
        used_nibbles <= address_nibbles,
        "chain_id {chain_id} is too long to derive Shasta predeploy address"
    );

    let address = format!(
        "0x{}{}{}",
        prefix,
        "0".repeat(address_nibbles - used_nibbles),
        suffix
    );
    Address::from_str(&address)
        .map_err(|err| anyhow!("failed to derive Shasta predeploy address: {err}"))
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

    /// Validates host-side chain-spec JSON against compiled-in Taiko chain rules.
    ///
    /// This check is intentionally host-only policy: zk/SGX guests use compiled-in runtime rules
    /// keyed by `chain_id` and must not depend on this JSON.
    ///
    /// # Errors
    ///
    /// Returns an error when the list is empty, when a Taiko chain ID is not supported by
    /// Alethia's built-in specs, or when Shasta predeploy/fork overlays are inconsistent.
    pub fn validate_host_sanity(&self) -> Result<()> {
        ensure!(
            !self.0.is_empty(),
            "chain spec list is empty; host builds must enable chain-spec-json or provide specs"
        );

        let mut taiko_count = 0usize;
        let mut taiko_chain_ids = BTreeMap::<ChainId, String>::new();
        for spec in self.0.values().filter(|spec| spec.is_taiko) {
            taiko_count += 1;
            if let Some(previous) = taiko_chain_ids.insert(spec.chain_id, spec.name.clone()) {
                bail!(
                    "duplicate Taiko chain_id {} in chain specs: {} and {}",
                    spec.chain_id,
                    previous,
                    spec.name
                );
            }

            let builtin_spec = builtin_taiko_chain_spec(spec.chain_id).map_err(|err| {
                anyhow!(
                    "{}: unsupported Taiko chain_id {} in host chain spec: {err}",
                    spec.name,
                    spec.chain_id
                )
            })?;
            let canonical = canonical_taiko_chain_spec(builtin_spec.as_ref());

            ensure!(
                spec.max_spec_id == canonical.max_spec_id,
                "{}: max_spec_id mismatch with built-in Taiko runtime: expected {:?}, got {:?}",
                spec.name,
                canonical.max_spec_id,
                spec.max_spec_id
            );
            ensure!(
                spec.hard_forks == canonical.hard_forks,
                "{}: hard_forks mismatch with built-in Taiko runtime: expected {:?}, got {:?}",
                spec.name,
                canonical.hard_forks,
                spec.hard_forks
            );
            ensure!(
                spec.eip_1559_constants == canonical.eip_1559_constants,
                "{}: eip_1559_constants mismatch with built-in Taiko runtime: expected {:?}, got {:?}",
                spec.name,
                canonical.eip_1559_constants,
                spec.eip_1559_constants
            );

            let expected_l2_contract = shasta_taiko_l2_address(spec.chain_id)?;
            ensure!(
                spec.l2_contract == Some(expected_l2_contract),
                "{}: l2_contract mismatch: expected {expected_l2_contract:?}, got {:?}",
                spec.name,
                spec.l2_contract
            );

            let expected_checkpoint_store = shasta_checkpoint_store_address(spec.chain_id)?;
            ensure!(
                spec.checkpoint_store_contract == Some(expected_checkpoint_store),
                "{}: checkpoint_store_contract mismatch: expected {expected_checkpoint_store:?}, got {:?}",
                spec.name,
                spec.checkpoint_store_contract
            );
        }

        ensure!(
            taiko_count > 0,
            "chain spec list does not contain any Taiko networks"
        );
        Ok(())
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

#[cfg(all(test, feature = "chain-spec-json"))]
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
    let hard_forks = helper
        .hard_forks
        .or_else(|| canonical.as_ref().map(|spec| spec.hard_forks.clone()))
        .ok_or_else(|| missing_chain_spec_field::<E>(&helper.name, "hard_forks"))?;
    let max_spec_id = helper
        .max_spec_id
        .or_else(|| canonical.as_ref().map(|spec| spec.max_spec_id))
        .unwrap_or_else(|| max_spec_id(&hard_forks));
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
    /// Aligns this config-level Taiko fork table with the Alethia runtime chainspec used by
    /// execution, while preserving raiko2 overlay fields such as RPCs, contracts, and verifiers.
    ///
    /// # Errors
    ///
    /// Returns an error when the chain is marked as Taiko but no built-in Alethia runtime
    /// chainspec exists for its chain ID.
    pub fn align_taiko_runtime_forks(&self) -> Result<Self> {
        if !self.is_taiko {
            return Ok(self.clone());
        }

        let runtime_spec = self.to_taiko_chain_spec()?;
        let canonical = canonical_taiko_chain_spec(runtime_spec.as_ref());
        let mut spec = self.clone();
        spec.max_spec_id = canonical.max_spec_id;
        spec.hard_forks = canonical.hard_forks;
        Ok(spec)
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

        let base = builtin_taiko_chain_spec(self.chain_id)?;

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
    #[cfg(feature = "chain-spec-json")]
    use alethia_reth_chainspec::hardfork::{TaikoHardfork, TaikoHardforks as _};
    use alethia_reth_chainspec::{
        TAIKO_DEVNET_GENESIS_HASH, TAIKO_HOODI_GENESIS_HASH, TAIKO_MAINNET_GENESIS_HASH,
        TAIKO_MASAYA_GENESIS_HASH,
    };
    #[cfg(feature = "chain-spec-json")]
    use alloy_primitives::address;

    #[cfg(feature = "chain-spec-json")]
    const MAINNET_UNZEN_TIMESTAMP: u64 = 1_786_021_200;

    #[cfg(feature = "chain-spec-json")]
    const HOODI_UNZEN_TIMESTAMP: u64 = 1_781_787_600;

    #[cfg(feature = "chain-spec-json")]
    fn mainnet_shasta_timestamp() -> u64 {
        match canonical_taiko_fork_condition("taiko_mainnet", TaikoFork::Shasta)
            .expect("taiko mainnet Shasta fork")
        {
            ForkCondition::Timestamp(timestamp) => timestamp,
            condition => panic!("expected mainnet Shasta timestamp fork, got {condition:?}"),
        }
    }

    #[cfg(feature = "chain-spec-json")]
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

    #[cfg(feature = "chain-spec-json")]
    #[test]
    fn shasta_predeploy_addresses_match_configured_taiko_addresses() -> Result<()> {
        let list: Vec<ChainSpec> = serde_json::from_str(DEFAULT_CHAIN_SPECS)?;
        for spec in list.iter().filter(|spec| spec.is_taiko) {
            assert_eq!(
                Some(shasta_taiko_l2_address(spec.chain_id)?),
                spec.l2_contract
            );
            assert_eq!(
                shasta_checkpoint_store_address(spec.chain_id)?,
                spec.checkpoint_store_contract.expect("checkpoint store")
            );
        }
        Ok(())
    }

    #[cfg(feature = "chain-spec-json")]
    #[test]
    fn default_chain_specs_pass_host_sanity_check() -> Result<()> {
        SupportedChainSpecs::default().validate_host_sanity()
    }

    #[cfg(feature = "chain-spec-json")]
    #[test]
    fn host_sanity_check_rejects_mismatched_taiko_l2_predeploy() -> Result<()> {
        let mut specs = SupportedChainSpecs::default();
        let mut spec = specs
            .get_chain_spec_with_chain_id(167_000)
            .ok_or_else(|| anyhow!("missing Taiko mainnet spec"))?;
        spec.l2_contract = Some(Address::ZERO);
        specs.0.insert(spec.name.clone(), spec);

        let err = specs
            .validate_host_sanity()
            .expect_err("mismatched TaikoL2 predeploy must fail host sanity");
        assert!(
            err.to_string().contains("l2_contract"),
            "unexpected error: {err}"
        );
        Ok(())
    }

    #[cfg(feature = "chain-spec-json")]
    #[test]
    fn host_sanity_check_rejects_mismatched_checkpoint_store_predeploy() -> Result<()> {
        let mut specs = SupportedChainSpecs::default();
        let mut spec = specs
            .get_chain_spec_with_chain_id(167_000)
            .ok_or_else(|| anyhow!("missing Taiko mainnet spec"))?;
        spec.checkpoint_store_contract = Some(Address::ZERO);
        specs.0.insert(spec.name.clone(), spec);

        let err = specs
            .validate_host_sanity()
            .expect_err("mismatched CheckpointStore predeploy must fail host sanity");
        assert!(
            err.to_string().contains("checkpoint_store_contract"),
            "unexpected error: {err}"
        );
        Ok(())
    }

    #[cfg(feature = "chain-spec-json")]
    #[test]
    fn host_sanity_check_rejects_taiko_fork_overlay_mismatch() -> Result<()> {
        let mut specs = SupportedChainSpecs::default();
        let mut spec = specs
            .get_chain_spec_with_chain_id(167_013)
            .ok_or_else(|| anyhow!("missing Taiko Hoodi spec"))?;
        spec.hard_forks.insert(
            ForkId::Taiko(TaikoFork::Unzen),
            ForkCondition::Timestamp(HOODI_UNZEN_TIMESTAMP + 1),
        );
        specs.0.insert(spec.name.clone(), spec);

        let err = specs
            .validate_host_sanity()
            .expect_err("mismatched Taiko fork overlay must fail host sanity");
        assert!(
            err.to_string().contains("hard_forks"),
            "unexpected error: {err}"
        );
        Ok(())
    }

    #[cfg(feature = "chain-spec-json")]
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
    fn custom_json_spec_derives_max_spec_id_from_hard_forks() -> Result<()> {
        let raw = r#"[
            {
                "name": "custom_l1",
                "chain_id": 12345,
                "hard_forks": {
                    "FRONTIER": {
                        "Block": 0
                    },
                    "SHANGHAI": {
                        "Timestamp": 0
                    },
                    "CANCUN": {
                        "Timestamp": 0
                    }
                },
                "eip_1559_constants": {
                    "base_fee_change_denominator": "0x8",
                    "base_fee_max_increase_denominator": "0x8",
                    "base_fee_max_decrease_denominator": "0x8",
                    "elasticity_multiplier": "0x2"
                },
                "l1_contract": {},
                "l2_contract": null,
                "rpc": "http://localhost:8545",
                "beacon_rpc": null,
                "verifier_address_forks": {
                    "FRONTIER": {
                        "SGX": null,
                        "SP1": null,
                        "RISC0": null
                    }
                },
                "genesis_time": 0,
                "seconds_per_slot": 12,
                "is_taiko": false
            }
        ]"#;

        let list: Vec<ChainSpec> = serde_json::from_str(raw)?;

        assert_eq!(list[0].max_spec_id, SpecId::CANCUN);
        Ok(())
    }

    #[cfg(feature = "chain-spec-json")]
    #[test]
    fn default_chain_spec_json_omits_derived_max_spec_id() -> Result<()> {
        let raw_list: Vec<Value> = serde_json::from_str(DEFAULT_CHAIN_SPECS)?;
        for spec in raw_list {
            assert!(
                spec.get("max_spec_id").is_none(),
                "{} should not store derived max_spec_id",
                spec["name"].as_str().unwrap_or("<unnamed>")
            );
        }
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

    #[cfg(feature = "chain-spec-json")]
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
            Some(&ForkCondition::Timestamp(MAINNET_UNZEN_TIMESTAMP))
        );
        let taiko = spec.to_taiko_chain_spec()?;
        let unzen = taiko.taiko_fork_activation(TaikoHardfork::Unzen);
        assert!(unzen.active_at_timestamp(MAINNET_UNZEN_TIMESTAMP));
        assert!(!unzen.active_at_timestamp(MAINNET_UNZEN_TIMESTAMP - 1));
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

    #[cfg(feature = "chain-spec-json")]
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

    #[cfg(feature = "chain-spec-json")]
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

    #[cfg(feature = "chain-spec-json")]
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
        assert_eq!(l2_spec.max_spec_id, SpecId::OSAKA);
        assert_eq!(l2_spec.rpc, "https://rpc.internal.taiko.xyz");
        assert_eq!(
            l2_spec.hard_forks.get(&ForkId::Taiko(TaikoFork::Shasta)),
            Some(&ForkCondition::Timestamp(0))
        );
        assert_eq!(
            l2_spec.hard_forks.get(&ForkId::Taiko(TaikoFork::Unzen)),
            Some(&ForkCondition::Timestamp(0))
        );
        assert_eq!(
            l2_spec.get_fork_l1_contract_address_at(0, 0)?,
            address!("83e383dec6E3C2CD167E3bF6aA8c36F0e55Ad910")
        );
        assert_eq!(
            l2_spec.get_fork_verifier_address(0, 0, ProofType::Sgx)?,
            address!("63Ec87f54cCed71B0DC879ce6cEDfA6f3D582670")
        );
        assert_eq!(
            l2_spec.get_fork_verifier_address(0, 0, ProofType::Sp1)?,
            address!("2546D7424F23EE0D1260C414DA3f17E295c187C6")
        );
        assert_eq!(
            l2_spec.get_fork_verifier_address(0, 0, ProofType::Risc0)?,
            address!("3DA89a777B11aABa02B5C92Fab96545D05fd4cc6")
        );
        assert_eq!(
            l2_spec.get_fork_verifier_address(0, 0, ProofType::SgxGeth)?,
            address!("429B4115e773a0Cf0e49c0443685dd290aE426ef")
        );
        Ok(())
    }

    #[cfg(feature = "chain-spec-json")]
    #[test]
    fn taiko_mainnet_uses_shanghai_until_unzen_then_osaka() -> Result<()> {
        let list: Vec<ChainSpec> = serde_json::from_str(DEFAULT_CHAIN_SPECS)?;
        let spec = list
            .into_iter()
            .find(|spec| spec.name == "taiko_mainnet")
            .ok_or_else(|| anyhow!("missing taiko_mainnet spec"))?;

        assert_eq!(
            spec.spec_id(5_412_478, MAINNET_UNZEN_TIMESTAMP - 1),
            Some(SpecId::SHANGHAI)
        );
        assert_eq!(
            spec.spec_id(5_412_478, MAINNET_UNZEN_TIMESTAMP),
            Some(SpecId::OSAKA)
        );
        Ok(())
    }

    #[cfg(feature = "chain-spec-json")]
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
        assert_eq!(
            unzen,
            AlethiaForkCondition::Timestamp(0),
            "Unzen must be active at genesis on internal devnet"
        );
        Ok(())
    }

    #[cfg(feature = "chain-spec-json")]
    #[test]
    fn aligns_taiko_runtime_forks_from_alethia_runtime_spec() -> Result<()> {
        let list: Vec<ChainSpec> = serde_json::from_str(DEFAULT_CHAIN_SPECS)?;
        let mut spec = list
            .into_iter()
            .find(|spec| spec.name == "taiko_dev")
            .ok_or_else(|| anyhow!("missing taiko_dev spec"))?;
        spec.max_spec_id = SpecId::SHANGHAI;
        spec.hard_forks = BTreeMap::from([(
            ForkId::Taiko(TaikoFork::Shasta),
            ForkCondition::Timestamp(0),
        )]);

        let aligned = spec.align_taiko_runtime_forks()?;

        assert_eq!(aligned.max_spec_id, SpecId::OSAKA);
        assert_eq!(
            aligned.hard_forks.get(&ForkId::Taiko(TaikoFork::Shasta)),
            Some(&ForkCondition::Timestamp(0))
        );
        assert_eq!(
            aligned.hard_forks.get(&ForkId::Taiko(TaikoFork::Unzen)),
            Some(&ForkCondition::Timestamp(0))
        );
        assert_eq!(aligned.l1_contract, spec.l1_contract);
        assert_eq!(aligned.verifier_address_forks, spec.verifier_address_forks);
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

    #[cfg(feature = "chain-spec-json")]
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

    #[cfg(feature = "chain-spec-json")]
    #[test]
    fn taiko_hoodi_unzen_uses_dedicated_verifier_addresses() -> Result<()> {
        let list: Vec<ChainSpec> = serde_json::from_str(DEFAULT_CHAIN_SPECS)?;
        let spec = list
            .into_iter()
            .find(|spec| spec.name == "taiko_hoodi")
            .ok_or_else(|| anyhow!("missing taiko_hoodi spec"))?;

        assert_eq!(
            spec.get_fork_verifier_address(0, HOODI_UNZEN_TIMESTAMP - 1, ProofType::Risc0)?,
            address!("fa0e7dAFe9785627df034c123A9B87497EB06b41")
        );
        assert_eq!(
            spec.get_fork_verifier_address(0, HOODI_UNZEN_TIMESTAMP - 1, ProofType::Sp1)?,
            address!("c42Ef1A7A606162e144F696A07A7D3Ad98bF4EE7")
        );
        assert_eq!(
            spec.get_fork_verifier_address(0, HOODI_UNZEN_TIMESTAMP, ProofType::Sgx)?,
            address!("7B6de561E26F5aB65958e5A3a1dCf807Cb91fD02")
        );
        assert_eq!(
            spec.get_fork_verifier_address(0, HOODI_UNZEN_TIMESTAMP, ProofType::SgxGeth)?,
            address!("8cF41Ee873Ca293Dc339006b0069d6337F68CCCA")
        );
        assert_eq!(
            spec.get_fork_verifier_address(0, HOODI_UNZEN_TIMESTAMP, ProofType::Risc0)?,
            address!("8f2007dC3Bf34a1E4A4Ea5303EDC2D8e140934E9")
        );
        assert_eq!(
            spec.get_fork_verifier_address(0, HOODI_UNZEN_TIMESTAMP, ProofType::Sp1)?,
            address!("2a872461C4629D5626Cb6852e50d75Bc7702f0e2")
        );
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

    #[cfg(feature = "chain-spec-json")]
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
