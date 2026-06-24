use alloy::{
    consensus::Header,
    providers::{DynProvider, Provider as AlloyProvider, ProviderBuilder},
    rpc::client::RpcClient,
};
use alloy_primitives::{Address, Bytes, map::AddressMap};
use alloy_rpc_types_eth::Header as AlloyRpcHeader;
use raiko2_primitives::{ChainSpec, ExecutionWitness, RaikoError, RaikoResult, WitnessStateNode};
use raiko2_protocol::{BlobProofType, InputDataSource};
use raiko2_protocol_shasta::shasta::ShastaEventData;
use reth_ethereum_primitives::Block as RethBlock;
use serde::{Deserialize, Serialize};
use std::{fmt, sync::Arc, time::Duration};

use crate::Provider;
use crate::rpc::{RpcClientConfig, build_rpc_client};

mod accounts;
mod blobs;
mod blocks;
mod headers;
mod witness;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum L2ProviderKind {
    #[default]
    Reth,
    Geth,
    GethLocalWitness,
}

impl fmt::Display for L2ProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reth => f.write_str("reth"),
            Self::Geth => f.write_str("geth"),
            Self::GethLocalWitness => f.write_str("geth_local_witness"),
        }
    }
}

#[async_trait::async_trait]
pub(crate) trait L2Provider: Send + Sync {
    async fn batch_blocks(&self, block_numbers: &[u64]) -> RaikoResult<Vec<RethBlock>>;

    async fn batch_accounts(
        &self,
        block_numbers: &[u64],
        addresses: &[Vec<Address>],
    ) -> RaikoResult<Vec<AddressMap<alloy_trie::TrieAccount>>>;

    async fn batch_accounts_with_proof_witnesses(
        &self,
        block_numbers: &[u64],
        addresses: &[Vec<Address>],
    ) -> RaikoResult<(
        Vec<AddressMap<alloy_trie::TrieAccount>>,
        Vec<Vec<WitnessStateNode>>,
    )> {
        let accounts = self.batch_accounts(block_numbers, addresses).await?;
        Ok((accounts, vec![Vec::new(); block_numbers.len()]))
    }

    async fn batch_witnesses(&self, block_numbers: &[u64]) -> RaikoResult<Vec<ExecutionWitness>>;

    async fn batch_witnesses_with_tx_lists(
        &self,
        _block_numbers: &[u64],
        _tx_lists: &[Bytes],
    ) -> RaikoResult<Vec<ExecutionWitness>> {
        Err(RaikoError::FeatureNotSupportedError(
            "L2 provider does not support tx-list execution witnesses".to_string(),
        ))
    }
}

#[derive(Clone)]
pub(crate) struct RpcL2Provider {
    client: RpcClient,
    witness_client: RpcClient,
    witness_provider: DynProvider,
    chain_spec: Option<ChainSpec>,
}

impl RpcL2Provider {
    fn new(
        l2_rpc_url: &str,
        l2_chain_spec: Option<ChainSpec>,
        l2_witness_rpc_url: Option<&str>,
        config: &RpcClientConfig,
    ) -> RaikoResult<Self> {
        let l2_client = build_rpc_client(l2_rpc_url, config)?;
        let l2_witness_client = build_rpc_client(l2_witness_rpc_url.unwrap_or(l2_rpc_url), config)?;
        let l2_witness_provider = ProviderBuilder::new()
            .connect_client(l2_witness_client.clone())
            .erased();

        Ok(Self {
            client: l2_client,
            witness_client: l2_witness_client,
            witness_provider: l2_witness_provider,
            chain_spec: l2_chain_spec,
        })
    }
}

#[derive(Clone)]
pub(crate) struct RethL2Provider {
    rpc: RpcL2Provider,
}

#[derive(Clone)]
pub(crate) struct GethL2Provider {
    rpc: RpcL2Provider,
}

#[derive(Clone)]
pub(crate) struct GethLocalWitnessL2Provider {
    rpc: RpcL2Provider,
}

impl RethL2Provider {
    const fn new(rpc: RpcL2Provider) -> Self {
        Self { rpc }
    }
}

impl GethL2Provider {
    const fn new(rpc: RpcL2Provider) -> Self {
        Self { rpc }
    }
}

impl GethLocalWitnessL2Provider {
    const fn new(rpc: RpcL2Provider) -> Self {
        Self { rpc }
    }
}

#[async_trait::async_trait]
impl L2Provider for RethL2Provider {
    async fn batch_blocks(&self, block_numbers: &[u64]) -> RaikoResult<Vec<RethBlock>> {
        self.rpc.fetch_blocks(block_numbers).await
    }

    async fn batch_accounts(
        &self,
        block_numbers: &[u64],
        addresses: &[Vec<Address>],
    ) -> RaikoResult<Vec<AddressMap<alloy_trie::TrieAccount>>> {
        self.rpc.fetch_accounts(block_numbers, addresses).await
    }

    async fn batch_accounts_with_proof_witnesses(
        &self,
        block_numbers: &[u64],
        addresses: &[Vec<Address>],
    ) -> RaikoResult<(
        Vec<AddressMap<alloy_trie::TrieAccount>>,
        Vec<Vec<WitnessStateNode>>,
    )> {
        self.rpc
            .fetch_accounts_with_proof_witnesses(block_numbers, addresses)
            .await
    }

    async fn batch_witnesses(&self, block_numbers: &[u64]) -> RaikoResult<Vec<ExecutionWitness>> {
        self.fetch_witnesses(block_numbers).await
    }

    async fn batch_witnesses_with_tx_lists(
        &self,
        block_numbers: &[u64],
        tx_lists: &[Bytes],
    ) -> RaikoResult<Vec<ExecutionWitness>> {
        self.fetch_witnesses_with_tx_lists(block_numbers, tx_lists)
            .await
    }
}

#[async_trait::async_trait]
impl L2Provider for GethL2Provider {
    async fn batch_blocks(&self, block_numbers: &[u64]) -> RaikoResult<Vec<RethBlock>> {
        self.rpc.fetch_blocks(block_numbers).await
    }

    async fn batch_accounts(
        &self,
        block_numbers: &[u64],
        addresses: &[Vec<Address>],
    ) -> RaikoResult<Vec<AddressMap<alloy_trie::TrieAccount>>> {
        self.rpc.fetch_accounts(block_numbers, addresses).await
    }

    async fn batch_accounts_with_proof_witnesses(
        &self,
        block_numbers: &[u64],
        addresses: &[Vec<Address>],
    ) -> RaikoResult<(
        Vec<AddressMap<alloy_trie::TrieAccount>>,
        Vec<Vec<WitnessStateNode>>,
    )> {
        self.rpc
            .fetch_accounts_with_proof_witnesses(block_numbers, addresses)
            .await
    }

    async fn batch_witnesses(&self, block_numbers: &[u64]) -> RaikoResult<Vec<ExecutionWitness>> {
        self.fetch_witnesses(block_numbers).await
    }
}

#[async_trait::async_trait]
impl L2Provider for GethLocalWitnessL2Provider {
    async fn batch_blocks(&self, block_numbers: &[u64]) -> RaikoResult<Vec<RethBlock>> {
        self.rpc.fetch_blocks(block_numbers).await
    }

    async fn batch_accounts(
        &self,
        block_numbers: &[u64],
        addresses: &[Vec<Address>],
    ) -> RaikoResult<Vec<AddressMap<alloy_trie::TrieAccount>>> {
        self.rpc.fetch_accounts(block_numbers, addresses).await
    }

    async fn batch_accounts_with_proof_witnesses(
        &self,
        block_numbers: &[u64],
        addresses: &[Vec<Address>],
    ) -> RaikoResult<(
        Vec<AddressMap<alloy_trie::TrieAccount>>,
        Vec<Vec<WitnessStateNode>>,
    )> {
        self.rpc
            .fetch_accounts_with_proof_witnesses(block_numbers, addresses)
            .await
    }

    async fn batch_witnesses(&self, block_numbers: &[u64]) -> RaikoResult<Vec<ExecutionWitness>> {
        self.fetch_witnesses(block_numbers).await
    }
}

#[derive(Clone)]
pub struct NetworkProvider {
    _l1_client: RpcClient,
    l1_provider: DynProvider,
    l2_provider: Arc<dyn L2Provider>,
    http_client: reqwest::Client,
    _l1_chain_spec: Option<ChainSpec>,
    _l2_chain_spec: Option<ChainSpec>,
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
        Self::new_pair_with_chain_specs_and_config(rpc_url, rpc_url, None, None, None, config)
    }

    /// # Errors
    ///
    /// Returns an error if either RPC URL is invalid.
    pub fn new_pair(l1_rpc_url: &str, l2_rpc_url: &str) -> RaikoResult<Self> {
        Self::new_pair_with_chain_specs_and_config(
            l1_rpc_url,
            l2_rpc_url,
            None,
            None,
            None,
            &RpcClientConfig::default(),
        )
    }

    /// # Errors
    ///
    /// Returns an error if either RPC URL is invalid or a client cannot be constructed.
    pub fn new_pair_with_config(
        l1_rpc_url: &str,
        l2_rpc_url: &str,
        config: &RpcClientConfig,
    ) -> RaikoResult<Self> {
        Self::new_pair_with_chain_specs_and_config(l1_rpc_url, l2_rpc_url, None, None, None, config)
    }

    /// # Errors
    ///
    /// Returns an error if either RPC URL is invalid or a client cannot be constructed.
    pub fn new_pair_with_chain_specs_and_config(
        l1_rpc_url: &str,
        l2_rpc_url: &str,
        l1_chain_spec: Option<ChainSpec>,
        l2_chain_spec: Option<ChainSpec>,
        l2_witness_rpc_url: Option<&str>,
        config: &RpcClientConfig,
    ) -> RaikoResult<Self> {
        Self::new_pair_with_l2_provider_kind_and_chain_specs_and_config(
            l1_rpc_url,
            l2_rpc_url,
            L2ProviderKind::default(),
            l1_chain_spec,
            l2_chain_spec,
            l2_witness_rpc_url,
            config,
        )
    }

    /// # Errors
    ///
    /// Returns an error if either RPC URL is invalid or a client cannot be constructed.
    pub fn new_pair_with_l2_provider_kind_and_chain_specs_and_config(
        l1_rpc_url: &str,
        l2_rpc_url: &str,
        l2_provider_kind: L2ProviderKind,
        l1_chain_spec: Option<ChainSpec>,
        l2_chain_spec: Option<ChainSpec>,
        l2_witness_rpc_url: Option<&str>,
        config: &RpcClientConfig,
    ) -> RaikoResult<Self> {
        let l1_client = build_rpc_client(l1_rpc_url, config)?;
        let l1_provider = ProviderBuilder::new()
            .connect_client(l1_client.clone())
            .erased();
        let l2_rpc = RpcL2Provider::new(
            l2_rpc_url,
            l2_chain_spec.clone(),
            l2_witness_rpc_url,
            config,
        )?;
        let l2_provider: Arc<dyn L2Provider> = match l2_provider_kind {
            L2ProviderKind::Reth => Arc::new(RethL2Provider::new(l2_rpc)),
            L2ProviderKind::Geth => Arc::new(GethL2Provider::new(l2_rpc)),
            L2ProviderKind::GethLocalWitness => Arc::new(GethLocalWitnessL2Provider::new(l2_rpc)),
        };
        let mut http_client_builder = reqwest::Client::builder();
        if config.timeout_ms > 0 {
            http_client_builder =
                http_client_builder.timeout(Duration::from_millis(config.timeout_ms));
        }
        let http_client = http_client_builder.build().map_err(|e| {
            raiko2_primitives::RaikoError::RPC(format!("failed to build HTTP client: {e}"))
        })?;

        Ok(Self {
            _l1_client: l1_client,
            l1_provider,
            l2_provider,
            http_client,
            _l1_chain_spec: l1_chain_spec,
            _l2_chain_spec: l2_chain_spec,
        })
    }
}

/// Fetch L2 blocks from a standalone RPC endpoint.
///
/// Used for optional cross-node checkpoint verification during preflight.
///
/// # Errors
///
/// Returns an error if the RPC client cannot be constructed or any requested block cannot be
/// fetched from the target endpoint.
pub async fn fetch_l2_blocks(
    l2_rpc_url: &str,
    block_numbers: &[u64],
    config: &RpcClientConfig,
) -> RaikoResult<Vec<RethBlock>> {
    let l2_provider = RpcL2Provider::new(l2_rpc_url, None, None, config)?;
    l2_provider.fetch_blocks(block_numbers).await
}

/// Fetch L2 headers from a standalone RPC endpoint.
///
/// Used for optional cross-node checkpoint verification during preflight.
///
/// # Errors
///
/// Returns an error if the RPC client cannot be constructed or any requested header cannot be
/// fetched from the target endpoint.
pub async fn fetch_l2_headers(
    l2_rpc_url: &str,
    block_numbers: &[u64],
    config: &RpcClientConfig,
) -> RaikoResult<Vec<AlloyRpcHeader>> {
    let l2_provider = RpcL2Provider::new(l2_rpc_url, None, None, config)?;
    l2_provider.fetch_headers(block_numbers).await
}

#[async_trait::async_trait]
impl Provider for NetworkProvider {
    async fn batch_blocks(&self, block_numbers: &[u64]) -> RaikoResult<Vec<RethBlock>> {
        self.l2_provider.batch_blocks(block_numbers).await
    }

    async fn batch_accounts(
        &self,
        block_numbers: &[u64],
        addresses: &[Vec<Address>],
    ) -> RaikoResult<Vec<AddressMap<alloy_trie::TrieAccount>>> {
        self.l2_provider
            .batch_accounts(block_numbers, addresses)
            .await
    }

    async fn batch_accounts_with_proof_witnesses(
        &self,
        block_numbers: &[u64],
        addresses: &[Vec<Address>],
    ) -> RaikoResult<(
        Vec<AddressMap<alloy_trie::TrieAccount>>,
        Vec<Vec<WitnessStateNode>>,
    )> {
        self.l2_provider
            .batch_accounts_with_proof_witnesses(block_numbers, addresses)
            .await
    }

    async fn batch_witnesses(&self, block_numbers: &[u64]) -> RaikoResult<Vec<ExecutionWitness>> {
        self.l2_provider.batch_witnesses(block_numbers).await
    }

    async fn batch_witnesses_with_tx_lists(
        &self,
        block_numbers: &[u64],
        tx_lists: &[Bytes],
    ) -> RaikoResult<Vec<ExecutionWitness>> {
        self.l2_provider
            .batch_witnesses_with_tx_lists(block_numbers, tx_lists)
            .await
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

    async fn shasta_data_sources(
        &self,
        l1_chain_spec: &raiko2_primitives::ChainSpec,
        proposal_event: &ShastaEventData,
        blob_proof_type: BlobProofType,
    ) -> RaikoResult<Vec<InputDataSource>> {
        self.fetch_shasta_data_sources(l1_chain_spec, proposal_event, blob_proof_type)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct UnsupportedTxListL2Provider;

    #[async_trait::async_trait]
    impl L2Provider for UnsupportedTxListL2Provider {
        async fn batch_blocks(&self, _block_numbers: &[u64]) -> RaikoResult<Vec<RethBlock>> {
            Ok(Vec::new())
        }

        async fn batch_accounts(
            &self,
            _block_numbers: &[u64],
            _addresses: &[Vec<Address>],
        ) -> RaikoResult<Vec<AddressMap<alloy_trie::TrieAccount>>> {
            Ok(Vec::new())
        }

        async fn batch_witnesses(
            &self,
            _block_numbers: &[u64],
        ) -> RaikoResult<Vec<ExecutionWitness>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn unsupported_l2_provider_tx_list_witnesses_fail_fast() {
        let provider = UnsupportedTxListL2Provider;

        let err = provider
            .batch_witnesses_with_tx_lists(&[1], &[Bytes::from_static(&[0xc0])])
            .await
            .expect_err("default tx-list witness support must fail fast");

        assert!(
            err.to_string()
                .contains("L2 provider does not support tx-list execution witnesses"),
            "{err}"
        );
    }
}
