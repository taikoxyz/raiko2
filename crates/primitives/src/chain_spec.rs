use crate::proof_type::ProofType;
use alethia_reth_chainspec_lite::{TAIKO_DEVNET, TAIKO_HOODI, TAIKO_MAINNET, spec::TaikoChainSpec};
use alloy_primitives::{Address, BlockNumber, ChainId, U256, map::HashMap, uint};
use anyhow::{Result, anyhow, bail};
use reth_revm::primitives::hardfork::SpecId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

const DEFAULT_CHAIN_SPECS: &str = include_str!("../../../config/chain_spec_list_default.json");

#[derive(Clone, Debug)]
pub struct SupportedChainSpecs(HashMap<String, ChainSpec>);

impl Default for SupportedChainSpecs {
    fn default() -> Self {
        let deserialized: Vec<ChainSpec> =
            serde_json::from_str(DEFAULT_CHAIN_SPECS).unwrap_or_default();
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ForkCondition {
    /// The fork is activated with a certain block.
    Block(BlockNumber),
    /// The fork is activated with a specific timestamp.
    Timestamp(u64),
    /// The fork is not yet active.
    Tbd,
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

/// Specification of a specific chain.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ChainSpec {
    pub name: String,
    pub chain_id: ChainId,
    pub max_spec_id: SpecId,
    pub hard_forks: BTreeMap<SpecId, ForkCondition>,
    pub eip_1559_constants: Eip1559Constants,
    pub l1_contract: BTreeMap<SpecId, Address>,
    pub l2_contract: Option<Address>,
    pub rpc: String,
    pub beacon_rpc: Option<String>,
    pub verifier_address_forks: BTreeMap<SpecId, BTreeMap<ProofType, Option<Address>>>,
    pub genesis_time: u64,
    pub seconds_per_slot: u64,
    pub is_taiko: bool,
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
    use alethia_reth_chainspec_lite::{
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

        assert_eq!(taiko.genesis_hash(), TAIKO_MAINNET_GENESIS_HASH);
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

        assert_eq!(taiko.genesis_hash(), TAIKO_DEVNET_GENESIS_HASH);
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

        assert_eq!(taiko.genesis_hash(), TAIKO_HOODI_GENESIS_HASH);
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
