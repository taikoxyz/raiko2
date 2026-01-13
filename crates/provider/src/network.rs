use alloy::{
    eips::BlockNumberOrTag,
    providers::{DynProvider, Provider as AlloyProvider, ProviderBuilder},
    rlp::decode_exact,
    rpc::client::RpcClient,
};
use alloy_chains::NamedChain;
use alloy_primitives::{Address, keccak256, map::AddressMap};
use alloy_rpc_types_eth::EIP1186AccountProofResponse;
use alloy_trie::TrieAccount;
use futures::{StreamExt, stream};
use raiko2_primitives::{RaikoError, RaikoResult};
use reth_chainspec::{HOLESKY, HOODI, MAINNET, SEPOLIA};
use reth_ethereum_primitives::Block as RethBlock;
use reth_evm_ethereum::EthEvmConfig;
use reth_stateless::ExecutionWitness;
use risc0_ethereum_trie::Trie;
use std::sync::Arc;
use tokio::sync::OnceCell;

use crate::{Provider, on_the_spot_witness::execution_witness};

// For Taiko chains
use alethia_reth_block::config::TaikoEvmConfig;
use alethia_reth_chainspec::{TAIKO_DEVNET, TAIKO_HOODI, TAIKO_MAINNET};

const TAIKO_CHAIN_IDS: [u64; 4] = [167000, 167001, 167013, 167009];

fn is_taiko_chain_id(chain_id: u64) -> bool {
    TAIKO_CHAIN_IDS.contains(&chain_id)
}

/// Decode account from EIP-1186 proof response
fn decode_account_from_proof(
    proof_response: &EIP1186AccountProofResponse,
) -> Result<TrieAccount, RaikoError> {
    let trie = Trie::from_rlp(&proof_response.account_proof)
        .map_err(|e| RaikoError::RPC(format!("Failed to decode account proof: {e}")))?;
    match trie.get(keccak256(proof_response.address)) {
        None => {
            // Account doesn't exist - return zero account
            Ok(TrieAccount {
                nonce: 0,
                balance: alloy_primitives::U256::ZERO,
                storage_root: alloy_primitives::B256::ZERO,
                code_hash: alloy_primitives::B256::ZERO,
            })
        }
        Some(rlp) => {
            let account: TrieAccount = decode_exact(rlp)
                .map_err(|e| RaikoError::RPC(format!("Failed to decode account RLP: {e}")))?;
            Ok(account)
        }
    }
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

    async fn build_local_witnesses(
        &self,
        requests: &[(usize, u64)],
    ) -> RaikoResult<Vec<(usize, ExecutionWitness)>> {
        const LOCAL_CONCURRENCY_LIMIT: usize = 4;
        let chain_id = self
            .provider
            .get_chain_id()
            .await
            .map_err(|e| RaikoError::RPC(format!("eth_chainId failed: {e}")))?;
        let provider = self.provider.clone();

        // For Taiko chains, use TaikoEvmConfig directly (like in on_the_spot_witness tests)
        if is_taiko_chain_id(chain_id) {
            let evm_config = match chain_id {
                167000 => Arc::new(TaikoEvmConfig::new(TAIKO_MAINNET.clone())),
                167001 => Arc::new(TaikoEvmConfig::new(TAIKO_DEVNET.clone())),
                167013 => Arc::new(TaikoEvmConfig::new(TAIKO_HOODI.clone())),
                _ => {
                    return Err(RaikoError::RPC(format!(
                        "Unsupported Taiko chain_id: {chain_id}"
                    )));
                }
            };

            let results = stream::iter(requests.iter().copied().map(|(index, block_number)| {
                let provider = provider.clone();
                let evm_config = evm_config.clone();
                async move {
                    let witness = execution_witness(
                        evm_config.clone(),
                        &provider,
                        BlockNumberOrTag::from(block_number),
                    )
                    .await
                    .map_err(|e| RaikoError::RPC(format!("execution_witness failed: {e:#}")))?;
                    Ok::<(usize, ExecutionWitness), RaikoError>((index, witness))
                }
            }))
            .buffer_unordered(LOCAL_CONCURRENCY_LIMIT)
            .collect::<Vec<_>>()
            .await;

            let mut witnesses = Vec::with_capacity(requests.len());
            for result in results {
                witnesses.push(result?);
            }
            return Ok(witnesses);
        }

        // For non-Taiko chains, use EthEvmConfig
        let evm_config = self.resolve_evm_config().await?;
        let results = stream::iter(requests.iter().copied().map(|(index, block_number)| {
            let provider = provider.clone();
            let evm_config = evm_config.clone();
            async move {
                let witness = execution_witness(
                    evm_config.clone(),
                    &provider,
                    BlockNumberOrTag::from(block_number),
                )
                .await
                .map_err(|e| RaikoError::RPC(format!("execution_witness failed: {e:#}")))?;
                Ok::<(usize, ExecutionWitness), RaikoError>((index, witness))
            }
        }))
        .buffer_unordered(LOCAL_CONCURRENCY_LIMIT)
        .collect::<Vec<_>>()
        .await;

        let mut witnesses = Vec::with_capacity(requests.len());
        for result in results {
            witnesses.push(result?);
        }

        Ok(witnesses)
    }

    async fn fetch_remote_witnesses(
        &self,
        block_numbers: &[u64],
    ) -> RaikoResult<Vec<ExecutionWitness>> {
        const MAX_BATCH_SIZE: usize = 32;
        let mut witnesses = Vec::with_capacity(block_numbers.len());
        for block_numbers in block_numbers.chunks(MAX_BATCH_SIZE) {
            let mut batch = self.client.new_batch();
            let mut requests = Vec::with_capacity(block_numbers.len());
            for block_number in block_numbers {
                requests.push(Box::pin(
                    batch
                        .add_call(
                            "debug_executionWitness",
                            &(BlockNumberOrTag::from(*block_number),),
                        )
                        .map_err(|_| {
                            RaikoError::RPC(
                                "Failed adding debug_executionWitness call to batch".to_owned(),
                            )
                        })?,
                ));
            }
            batch.send().await.map_err(|e| {
                RaikoError::RPC(format!(
                    "Error sending batch request for block {block_numbers:?}: {e}"
                ))
            })?;
            // Collect the data from the batch
            for request in requests {
                witnesses.push(
                    request.await.map_err(|e| {
                        RaikoError::RPC(format!("Error collecting request data: {e}"))
                    })?,
                );
            }
        }

        Ok(witnesses)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WitnessMode {
    Auto,
    ForceRemote,
    ForceLocal,
}

impl Default for WitnessMode {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WitnessStrategy {
    RemoteOnly,
    LocalOnly,
}

fn witness_strategy(mode: WitnessMode, support: Option<bool>) -> WitnessStrategy {
    match mode {
        WitnessMode::ForceRemote => WitnessStrategy::RemoteOnly,
        WitnessMode::ForceLocal => WitnessStrategy::LocalOnly,
        WitnessMode::Auto => {
            if support == Some(true) {
                WitnessStrategy::RemoteOnly
            } else {
                WitnessStrategy::LocalOnly
            }
        }
    }
}

#[async_trait::async_trait]
impl Provider for NetworkProvider {
    async fn batch_blocks(&self, block_numbers: &[u64]) -> RaikoResult<Vec<RethBlock>> {
        // Use provider.get_block().full() to get complete blocks with header
        let mut blocks = Vec::with_capacity(block_numbers.len());
        for block_number in block_numbers {
            let rpc_block = self
                .provider
                .get_block(BlockNumberOrTag::from(*block_number).into())
                .full()
                .await
                .map_err(|e| {
                    RaikoError::RPC(format!(
                        "eth_getBlockByNumber failed for block {}: {e}",
                        block_number
                    ))
                })?
                .ok_or_else(|| RaikoError::RPC(format!("Block {} not found", block_number)))?;

            // Convert alloy BlockResponse to RethBlock
            let reth_block: RethBlock = rpc_block.into();

            blocks.push(reth_block);
        }

        Ok(blocks)
    }

    async fn batch_accounts(
        &self,
        block_numbers: &[u64],
        addresses: &[Vec<Address>],
    ) -> RaikoResult<Vec<AddressMap<TrieAccount>>> {
        let mut result = Vec::with_capacity(block_numbers.len());
        for (block_number, addresses) in block_numbers.iter().zip(addresses.iter()) {
            let mut accounts = AddressMap::default();
            let block_id = BlockNumberOrTag::from(*block_number);
            // Get block hash once for all addresses in this block
            let block = self
                .provider
                .get_block(block_id.into())
                .await
                .map_err(|e| {
                    RaikoError::RPC(format!(
                        "eth_getBlockByNumber failed for block {block_number}: {e}"
                    ))
                })?
                .ok_or_else(|| RaikoError::RPC(format!("Block {block_number} not found")))?;
            let block_hash = block.header.hash_slow();

            for address in addresses {
                // Use eth_getProof to get account information (standard Ethereum RPC method)
                let proof = self
                    .provider
                    .get_proof(*address, vec![])
                    .hash(block_hash)
                    .await
                    .map_err(|e| {
                        RaikoError::RPC(format!(
                            "eth_getProof failed for address {address} at block {block_number}: {e}"
                        ))
                    })?;
                let account = decode_account_from_proof(&proof)?;
                accounts.insert(*address, account);
            }
            result.push(accounts);
        }

        Ok(result)
    }

    async fn batch_witnesses(&self, block_numbers: &[u64]) -> RaikoResult<Vec<ExecutionWitness>> {
        if self.witness_mode == WitnessMode::Auto && self.debug_witness_supported.is_none() {
            return Err(RaikoError::RPC(
                "Witness support not specified; set with_debug_witness_support(true|false)"
                    .to_owned(),
            ));
        }

        match witness_strategy(self.witness_mode, self.debug_witness_supported) {
            WitnessStrategy::RemoteOnly => self.fetch_remote_witnesses(block_numbers).await,
            WitnessStrategy::LocalOnly => {
                let requests: Vec<_> = block_numbers
                    .iter()
                    .enumerate()
                    .map(|(index, block_number)| (index, *block_number))
                    .collect();
                let mut results = vec![None; block_numbers.len()];
                for (index, witness) in self.build_local_witnesses(&requests).await? {
                    results[index] = Some(witness);
                }
                let mut witnesses = Vec::with_capacity(results.len());
                for (index, witness) in results.into_iter().enumerate() {
                    witnesses.push(witness.ok_or_else(|| {
                        RaikoError::RPC(format!("Missing execution witness at index {index}"))
                    })?);
                }
                Ok(witnesses)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{WitnessMode, WitnessStrategy, witness_strategy};

    #[test]
    fn witness_strategy_prefers_known_support() {
        assert_eq!(
            witness_strategy(WitnessMode::Auto, Some(true)),
            WitnessStrategy::RemoteOnly
        );
        assert_eq!(
            witness_strategy(WitnessMode::Auto, Some(false)),
            WitnessStrategy::LocalOnly
        );
    }
}
