use alloy::{eips::BlockNumberOrTag, providers::Provider as AlloyProvider};
use raiko2_primitives::{RaikoError, RaikoResult};
use reth_ethereum_primitives::Block as RethBlock;

use super::NetworkProvider;

impl NetworkProvider {
    pub(crate) async fn fetch_blocks(&self, block_numbers: &[u64]) -> RaikoResult<Vec<RethBlock>> {
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
                        "eth_getBlockByNumber failed for block {block_number}: {e}"
                    ))
                })?
                .ok_or_else(|| RaikoError::RPC(format!("Block {block_number} not found")))?;

            // Convert alloy BlockResponse to RethBlock
            let reth_block: RethBlock = rpc_block.into();

            blocks.push(reth_block);
        }

        Ok(blocks)
    }
}
