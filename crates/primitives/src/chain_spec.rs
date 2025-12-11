use crate::proof_type::ProofType;
use alloy_primitives::{Address, BlockNumber, ChainId, U256, uint};
use anyhow::{Result, anyhow, bail};
use reth::revm::primitives::hardfork::SpecId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    pub fn active(&self, block_no: BlockNumber, timestamp: u64) -> bool {
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
    pub fn chain_id(&self) -> ChainId {
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
    pub fn gas_constants(&self) -> &Eip1559Constants {
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

    pub fn is_taiko(&self) -> bool {
        self.is_taiko
    }

    pub fn network(&self) -> String {
        self.name.clone()
    }
}
