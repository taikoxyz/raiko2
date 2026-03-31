use alloy::{
    consensus::Header,
    providers::{DynProvider, Provider as AlloyProvider, ProviderBuilder},
    rpc::client::RpcClient,
};
use alloy_primitives::{Address, map::AddressMap};
use raiko2_primitives::{ExecutionWitness, RaikoResult};
use raiko2_protocol_shasta::shasta::ShastaEventData;
use reth_ethereum_primitives::Block as RethBlock;

use crate::Provider;
use crate::rpc::{RpcClientConfig, build_rpc_client};

mod accounts;
mod blocks;
mod headers;
mod witness;

#[derive(Clone)]
pub struct NetworkProvider {
    _l1_client: RpcClient,
    l1_provider: DynProvider,
    l2_client: RpcClient,
    l2_provider: DynProvider,
}

impl NetworkProvider {
    /// # Errors
    ///
    /// Returns an error if the RPC URL is invalid.
    pub fn new(rpc_url: &str) -> RaikoResult<Self> {
        Self::new_with_config(rpc_url, &RpcClientConfig::default())
    }

    /// # Errors
    ///
    /// Returns an error if the RPC URL is invalid or the client cannot be constructed.
    pub fn new_with_config(rpc_url: &str, config: &RpcClientConfig) -> RaikoResult<Self> {
        Self::new_pair_with_config(rpc_url, rpc_url, config)
    }

    /// # Errors
    ///
    /// Returns an error if either RPC URL is invalid.
    pub fn new_pair(l1_rpc_url: &str, l2_rpc_url: &str) -> RaikoResult<Self> {
        Self::new_pair_with_config(l1_rpc_url, l2_rpc_url, &RpcClientConfig::default())
    }

    /// # Errors
    ///
    /// Returns an error if either RPC URL is invalid or a client cannot be constructed.
    pub fn new_pair_with_config(
        l1_rpc_url: &str,
        l2_rpc_url: &str,
        config: &RpcClientConfig,
    ) -> RaikoResult<Self> {
        let l1_client = build_rpc_client(l1_rpc_url, config)?;
        let l1_provider = ProviderBuilder::new()
            .connect_client(l1_client.clone())
            .erased();
        let l2_client = build_rpc_client(l2_rpc_url, config)?;
        let l2_provider = ProviderBuilder::new()
            .connect_client(l2_client.clone())
            .erased();

        Ok(Self {
            _l1_client: l1_client,
            l1_provider,
            l2_client,
            l2_provider,
        })
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

    async fn batch_l1_headers(&self, block_numbers: &[u64]) -> RaikoResult<Vec<Header>> {
        self.fetch_l1_headers(block_numbers).await
    }

    async fn shasta_proposal_event(
        &self,
        l1_contract: Address,
        l1_inclusion_block_number: u64,
        proposal_id: u64,
    ) -> RaikoResult<ShastaEventData> {
        self.fetch_shasta_proposal_event(l1_contract, l1_inclusion_block_number, proposal_id)
            .await
    }
}
