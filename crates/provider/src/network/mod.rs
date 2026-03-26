use alloy::{
    providers::{DynProvider, Provider as AlloyProvider, ProviderBuilder},
    rpc::client::RpcClient,
};
use alloy_primitives::{Address, map::AddressMap};
use raiko2_primitives::{ExecutionWitness, RaikoResult};
use reth_ethereum_primitives::Block as RethBlock;

use crate::Provider;
use crate::rpc::{RpcClientConfig, build_rpc_client};

mod accounts;
mod blocks;
mod witness;

#[derive(Clone)]
pub struct NetworkProvider {
    client: RpcClient,
    provider: DynProvider,
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
        let client = build_rpc_client(rpc_url, config)?;
        let provider = ProviderBuilder::new()
            .connect_client(client.clone())
            .erased();

        Ok(Self { client, provider })
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
