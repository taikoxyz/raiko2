// Copyright 2025 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use alloy::consensus::BlockHeader;
use alloy::{
    consensus::Transaction,
    eips::BlockNumberOrTag,
    network::{BlockResponse, Network, primitives::HeaderResponse},
    primitives::Bytes,
    providers::Provider,
};
use anyhow::{Context, Result};
use reth_evm::{ConfigureEvm, execute::Executor};
use reth_primitives_traits::{Block, BlockBody, NodePrimitives};
use reth_stateless::ExecutionWitness;
use std::collections::HashSet;
use tracing::{Span, debug};

pub mod db;
pub mod rpc;
pub mod trie;

pub use db::{PreflightDb, ProviderConfig, ProviderDb};
pub use rpc::{DebugApi, StorageRangeQueryResponse, StorageRangeQueryResponseEntry};
pub use trie::{handle_modified_account, handle_new_account, handle_removed_account};

pub async fn execution_witness<E, P, N>(
    evm_config: E,
    provider: &P,
    block_id: BlockNumberOrTag,
) -> Result<ExecutionWitness>
where
    E: ConfigureEvm + 'static,
    P: Provider<N> + Clone + Send + Sync + 'static,
    N: Network,
    <E::Primitives as NodePrimitives>::Block: TryFrom<<N as Network>::BlockResponse>,
    <<E::Primitives as NodePrimitives>::Block as TryFrom<<N as Network>::BlockResponse>>::Error:
        std::error::Error + Send + Sync + 'static,
    <E::Primitives as NodePrimitives>::BlockHeader: TryFrom<<N as Network>::HeaderResponse>,
    <<E::Primitives as NodePrimitives>::BlockHeader as TryFrom<<N as Network>::HeaderResponse>>::Error:
        std::error::Error + Send + Sync + 'static,
{
    debug!(%block_id, "Fetching block data");
    let rpc_block = provider
        .get_block(block_id.into())
        .full()
        .await
        .context("eth_getBlock failed")?
        .with_context(|| format!("Block {block_id} not found"))?;
    let block_hash = rpc_block.header().hash();
    let parent_hash = rpc_block.header().parent_hash();

    let block: <E::Primitives as NodePrimitives>::Block = rpc_block.try_into()?;
    let recovered_block = block.try_into_recovered()?;

    let mut db = db::PreflightDb::new(db::ProviderDb::new(
        provider.clone(),
        db::ProviderConfig::default(),
        parent_hash,
    ));

    debug!(%block_hash, "Preprocessing transactions with access lists");
    for tx in recovered_block.body().transactions() {
        if let Some(access_list) = tx.access_list() {
            db.add_access_list(access_list).await?;
        }
    }

    debug!(%block_hash, "Executing block on dedicated thread");
    let current_span = Span::current();

    let (execution_result, db) = tokio::task::spawn_blocking(move || {
        current_span.in_scope(|| {
            let executor = evm_config.executor(db);
            let mut database_capture: Option<Box<db::PreflightDb<db::ProviderDb<N, P>>>> = None;
            let outcome = executor.execute_with_state_closure(&recovered_block, |state| {
                database_capture = Some(Box::new(state.database.clone()));
            });
            (outcome, database_capture)
        })
    })
    .await?;
    let execution_outcome = execution_result?;
    let mut db = db.unwrap();

    debug!("Building pre-state proofs");
    let (mut state_trie, mut storage_tries) = db.state_proof().await?;
    let ancestors = db
        .ancestor_proof(parent_hash)
        .await
        .context("failed to find ancestors")?;

    debug!("Building post-state proofs");
    for (addr, account) in execution_outcome.state.state {
        match (account.original_info.is_some(), account.info.is_some()) {
            (false, true) => {
                trie::handle_new_account(provider, block_hash, addr, &mut state_trie).await?
            }
            (true, false) => {
                trie::handle_removed_account(provider, block_hash, addr, &mut state_trie).await?
            }
            (true, true) => {
                let storage = storage_tries.get_mut(&addr).unwrap();
                trie::handle_modified_account(
                    provider,
                    block_hash,
                    addr,
                    &account.storage,
                    storage,
                )
                .await?;
            }
            _ => {}
        }
    }

    // 5. Assemble the Execution Witness
    let mut state: HashSet<Bytes> = HashSet::new();
    state.extend(state_trie.rlp_nodes());
    for storage_trie in storage_tries.values() {
        state.extend(storage_trie.rlp_nodes());
    }

    let mut headers = Vec::new();
    for header in ancestors {
        let header: <E::Primitives as NodePrimitives>::BlockHeader = header.try_into()?;
        headers.push(alloy::rlp::encode(header).into());
    }

    debug!("Preflight check completed successfully");

    Ok(ExecutionWitness {
        state: state.into_iter().collect(),
        codes: db.contracts().values().cloned().collect(),
        keys: vec![],
        headers,
    })
}

#[cfg(test)]
mod tests {
    use super::execution_witness;
    use alethia_reth_block::config::TaikoEvmConfig;
    use alethia_reth_chainspec::{TAIKO_DEVNET, TAIKO_HOODI, TAIKO_MAINNET};
    use alloy::{
        eips::BlockNumberOrTag,
        providers::{Provider as AlloyProvider, ProviderBuilder},
        rpc::client::RpcClient,
    };
    use alloy_chains::NamedChain;
    use reth_chainspec::{HOLESKY, HOODI, MAINNET, SEPOLIA};
    use reth_evm_ethereum::EthEvmConfig;
    use std::sync::Arc;

    #[tokio::test]
    async fn execution_witness_from_env_rpc() {
        let rpc_url = std::env::var("ON_THE_SPOT_WITNESS_RPC_URL").ok();
        let Some(rpc_url) = rpc_url else {
            return;
        };

        let url = reqwest::Url::parse(&rpc_url).expect("Invalid ON_THE_SPOT_WITNESS_RPC_URL");
        let client = RpcClient::builder().http(url);
        let provider = ProviderBuilder::new().connect_client(client);

        let chain_id = provider.get_chain_id().await.expect("eth_chainId failed");
        let block_id = std::env::var("ON_THE_SPOT_WITNESS_BLOCK")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(BlockNumberOrTag::from)
            .unwrap_or(BlockNumberOrTag::Latest);

        let witness = match chain_id {
            167000 => {
                let evm_config = Arc::new(TaikoEvmConfig::new(TAIKO_MAINNET.clone()));
                execution_witness(evm_config, &provider, block_id)
                    .await
                    .expect("execution_witness failed")
            }
            167001 => {
                let evm_config = Arc::new(TaikoEvmConfig::new(TAIKO_DEVNET.clone()));
                execution_witness(evm_config, &provider, block_id)
                    .await
                    .expect("execution_witness failed")
            }
            167013 => {
                let evm_config = Arc::new(TaikoEvmConfig::new(TAIKO_HOODI.clone()));
                execution_witness(evm_config, &provider, block_id)
                    .await
                    .expect("execution_witness failed")
            }
            _ => {
                let chain: NamedChain = chain_id.try_into().expect("Invalid chain_id");
                let evm_config = match chain {
                    NamedChain::Mainnet => Arc::new(EthEvmConfig::ethereum(MAINNET.clone())),
                    NamedChain::Holesky => Arc::new(EthEvmConfig::ethereum(HOLESKY.clone())),
                    NamedChain::Hoodi => Arc::new(EthEvmConfig::ethereum(HOODI.clone())),
                    NamedChain::Sepolia => Arc::new(EthEvmConfig::ethereum(SEPOLIA.clone())),
                    _ => panic!("Unsupported chain: {chain}"),
                };
                execution_witness(evm_config, &provider, block_id)
                    .await
                    .expect("execution_witness failed")
            }
        };

        assert!(
            !witness.state.is_empty(),
            "witness state should not be empty"
        );
        assert!(
            !witness.headers.is_empty(),
            "witness headers should not be empty"
        );
    }
}
