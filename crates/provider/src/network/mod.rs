use alloy::{
    providers::{DynProvider, Provider as AlloyProvider, ProviderBuilder},
    rpc::client::RpcClient,
};
use alloy_chains::NamedChain;
use alloy_primitives::{Address, map::AddressMap};
use raiko2_primitives::{RaikoError, RaikoResult};
use reth_chainspec::{HOLESKY, HOODI, MAINNET, SEPOLIA};
use reth_ethereum_primitives::Block as RethBlock;
use reth_evm_ethereum::EthEvmConfig;
use reth_stateless::ExecutionWitness;
use std::sync::Arc;
use tokio::sync::OnceCell;

use crate::Provider;

mod accounts;
mod blocks;
mod witness;

pub use witness::WitnessMode;

// For Taiko chains
const TAIKO_CHAIN_IDS: [u64; 4] = [167000, 167001, 167013, 167009];

pub(super) fn is_taiko_chain_id(chain_id: u64) -> bool {
    TAIKO_CHAIN_IDS.contains(&chain_id)
}

#[derive(Clone)]
pub struct NetworkProvider {
    client: RpcClient,
    provider: DynProvider,
    evm_config: Arc<OnceCell<Arc<EthEvmConfig>>>,
    witness_mode: WitnessMode,
    debug_witness_supported: Option<bool>,
}

impl NetworkProvider {
    pub fn new(rpc_url: &str) -> RaikoResult<Self> {
        let url = reqwest::Url::parse(rpc_url)
            .map_err(|e| RaikoError::RPC(format!("Invalid RPC URL: {e}")))?;

        let client = RpcClient::builder().http(url);
        let provider = ProviderBuilder::new()
            .connect_client(client.clone())
            .erased();

        Ok(Self {
            client,
            provider,
            evm_config: Arc::new(OnceCell::new()),
            witness_mode: WitnessMode::default(),
            debug_witness_supported: None,
        })
    }

    pub fn with_evm_config(self, evm_config: Arc<EthEvmConfig>) -> Self {
        let _ = self.evm_config.set(evm_config);
        self
    }

    pub const fn with_witness_mode(mut self, mode: WitnessMode) -> Self {
        self.witness_mode = mode;
        self
    }

    pub const fn with_debug_witness_support(mut self, supported: bool) -> Self {
        self.debug_witness_supported = Some(supported);
        self
    }

    async fn resolve_evm_config(&self) -> RaikoResult<Arc<EthEvmConfig>> {
        if let Some(config) = self.evm_config.get() {
            return Ok(config.clone());
        }

        let chain_id = self
            .provider
            .get_chain_id()
            .await
            .map_err(|e| RaikoError::RPC(format!("eth_chainId failed: {e}")))?;

        // Map Taiko chains to standard Ethereum config for local witness generation
        // Taiko forks (SHASTA, PACAYA, etc.) are mapped to SHANGHAI (pre-Cancun)
        // to avoid EIP-4788 parent beacon block root requirement
        let evm_config = if is_taiko_chain_id(chain_id) {
            tracing::info!(
                "Mapping Taiko chain (chain_id={}) to Ethereum SHANGHAI config for local witness generation",
                chain_id
            );
            // Use HOLESKY config as it supports SHANGHAI, which is a reasonable approximation
            // for Taiko forks like SHASTA/PACAYA (pre-Cancun)
            Arc::new(EthEvmConfig::ethereum(HOLESKY.clone()))
        } else {
            let chain: NamedChain = chain_id
                .try_into()
                .map_err(|e| RaikoError::RPC(format!("Invalid chain_id: {e}")))?;
            match chain {
                NamedChain::Mainnet => Arc::new(EthEvmConfig::ethereum(MAINNET.clone())),
                NamedChain::Holesky => Arc::new(EthEvmConfig::ethereum(HOLESKY.clone())),
                NamedChain::Hoodi => Arc::new(EthEvmConfig::ethereum(HOODI.clone())),
                NamedChain::Sepolia => Arc::new(EthEvmConfig::ethereum(SEPOLIA.clone())),
                _ => return Err(RaikoError::RPC(format!("Unsupported chain: {chain}"))),
            }
        };
        let _ = self.evm_config.set(evm_config.clone());
        Ok(evm_config)
    }
}

#[async_trait::async_trait]
impl Provider for NetworkProvider {
    async fn batch_blocks(&self, block_numbers: &[u64]) -> RaikoResult<Vec<RethBlock>> {
        self.fetch_blocks(block_numbers).await
    }

    async fn batch_accounts(
        &self,
        block_numbers: &[u64],
        addresses: &[Vec<Address>],
    ) -> RaikoResult<Vec<AddressMap<alloy_trie::TrieAccount>>> {
        self.fetch_accounts(block_numbers, addresses).await
    }

    async fn batch_witnesses(&self, block_numbers: &[u64]) -> RaikoResult<Vec<ExecutionWitness>> {
        self.fetch_witnesses(block_numbers).await
    }
}
