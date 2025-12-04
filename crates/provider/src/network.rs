use alloy_primitives::{Address, map::AddressMap};
use alloy_rpc_client::RpcClient;
use alloy_rpc_types::{Account, BlockNumberOrTag};
use alloy_trie::TrieAccount;
use raiko2_primitives::{RaizenError, RaizenResult};
use reth_ethereum_primitives::Block as RethBlock;
use reth_stateless::ExecutionWitness;

use crate::Provider;

/// Convert RPC Account to TrieAccount for state trie operations.
fn rpc_account_to_trie_account(account: Account) -> TrieAccount {
    TrieAccount {
        nonce: account.nonce,
        balance: account.balance,
        storage_root: account.storage_root,
        code_hash: account.code_hash,
    }
}

#[derive(Clone)]
pub struct NetworkProvider {
    client: RpcClient,
}

impl NetworkProvider {
    pub fn new(rpc_url: &str) -> RaizenResult<Self> {
        let url = reqwest::Url::parse(rpc_url)
            .map_err(|e| RaizenError::RPC(format!("Invalid RPC URL: {e}")))?;

        Ok(Self {
            client: RpcClient::builder().http(url),
        })
    }
}

impl Provider for NetworkProvider {
    async fn batch_blocks(&self, block_numbers: &[u64]) -> RaizenResult<Vec<RethBlock>> {
        const MAX_BATCH_SIZE: usize = 32;
        let mut blocks = Vec::with_capacity(block_numbers.len());
        for block_numbers in block_numbers.chunks(MAX_BATCH_SIZE) {
            let mut batch = self.client.new_batch();
            let mut requests = Vec::with_capacity(MAX_BATCH_SIZE);
            for block_number in block_numbers {
                requests.push(Box::pin(
                    batch
                        .add_call(
                            "eth_getBlockByNumber",
                            &(BlockNumberOrTag::from(*block_number), true),
                        )
                        .map_err(|_| {
                            RaizenError::RPC(
                                "Failed adding eth_getBlockByNumber call to batch".to_owned(),
                            )
                        })?,
                ));
            }
            batch.send().await.map_err(|e| {
                RaizenError::RPC(format!(
                    "Error sending batch request for block {block_numbers:?}: {e}"
                ))
            })?;
            // Collect the data from the batch
            for request in requests {
                blocks.push(request.await.map_err(|e| {
                    RaizenError::RPC(format!("Error collecting request data: {e}"))
                })?);
            }
        }

        Ok(blocks)
    }

    async fn batch_accounts(
        &self,
        block_numbers: &[u64],
        addresses: &[Vec<Address>],
    ) -> RaizenResult<Vec<AddressMap<TrieAccount>>> {
        const MAX_BATCH_SIZE: usize = 250;
        let mut result = Vec::with_capacity(block_numbers.len());
        for (block_number, addresses) in block_numbers.iter().zip(addresses.iter()) {
            let mut accounts = AddressMap::default();
            for addresses in addresses.chunks(MAX_BATCH_SIZE) {
                let mut batch = self.client.new_batch();
                let mut requests = Vec::with_capacity(MAX_BATCH_SIZE);
                for address in addresses {
                    requests.push((
                        address,
                        Box::pin(
                            batch
                                .add_call(
                                    "eth_getAccount",
                                    &(*address, BlockNumberOrTag::from(*block_number)),
                                )
                                .map_err(|_| {
                                    RaizenError::RPC(
                                        "Failed adding eth_getAccount call to batch".to_owned(),
                                    )
                                })?,
                        ),
                    ));
                }

                batch
                    .send()
                    .await
                    .map_err(|e| RaizenError::RPC(format!("Error sending batch request {e}")))?;

                // Collect the data from the batch
                for (address, request) in requests {
                    let rpc_account: Account = request.await.map_err(|e| {
                        RaizenError::RPC(format!("Error collecting request data: {e}"))
                    })?;
                    accounts.insert(*address, rpc_account_to_trie_account(rpc_account));
                }
            }
            result.push(accounts);
        }

        Ok(result)
    }

    async fn batch_witnesses(&self, block_numbers: &[u64]) -> RaizenResult<Vec<ExecutionWitness>> {
        const MAX_BATCH_SIZE: usize = 32;
        let mut witnesses = Vec::with_capacity(block_numbers.len());
        for block_numbers in block_numbers.chunks(MAX_BATCH_SIZE) {
            let mut batch = self.client.new_batch();
            let mut requests = Vec::with_capacity(MAX_BATCH_SIZE);
            for block_number in block_numbers {
                requests.push(Box::pin(
                    batch
                        .add_call(
                            "debug_executionWitness",
                            &(BlockNumberOrTag::from(*block_number),),
                        )
                        .map_err(|_| {
                            RaizenError::RPC(
                                "Failed adding debug_executionWitness call to batch".to_owned(),
                            )
                        })?,
                ));
            }
            batch.send().await.map_err(|e| {
                RaizenError::RPC(format!(
                    "Error sending batch request for block {block_numbers:?}: {e}"
                ))
            })?;
            // Collect the data from the batch
            for request in requests {
                witnesses.push(request.await.map_err(|e| {
                    RaizenError::RPC(format!("Error collecting request data: {e}"))
                })?);
            }
        }

        Ok(witnesses)
    }
}
