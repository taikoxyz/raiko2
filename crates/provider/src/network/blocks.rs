use alloy::eips::BlockNumberOrTag;
use alloy_rpc_types_eth::Block as AlloyBlock;
use raiko2_primitives::{RaikoError, RaikoResult};
use reth_ethereum_primitives::Block as RethBlock;

use super::RpcL2Provider;

const BLOCK_BATCH_SIZE: usize = 32;

impl RpcL2Provider {
    pub(super) async fn fetch_blocks(&self, block_numbers: &[u64]) -> RaikoResult<Vec<RethBlock>> {
        let mut blocks = Vec::with_capacity(block_numbers.len());
        for chunk in block_numbers.chunks(BLOCK_BATCH_SIZE) {
            let mut batch = self.client.new_batch();
            let mut requests = Vec::with_capacity(chunk.len());
            for &block_number in chunk {
                requests.push((
                    block_number,
                    Box::pin(
                        batch
                            .add_call::<_, Option<AlloyBlock>>(
                                "eth_getBlockByNumber",
                                &(BlockNumberOrTag::from(block_number), true),
                            )
                            .map_err(|_| {
                                RaikoError::RPC(
                                    "failed adding eth_getBlockByNumber call to batch".to_owned(),
                                )
                            })?,
                    ),
                ));
            }

            batch.send().await.map_err(|e| {
                RaikoError::RPC(format!(
                    "error sending eth_getBlockByNumber batch for blocks {chunk:?}: {e}"
                ))
            })?;

            for (block_number, request) in requests {
                let rpc_block = request.await.map_err(|e| {
                    RaikoError::RPC(format!(
                        "error collecting eth_getBlockByNumber for block {block_number}: {e}"
                    ))
                })?;
                let rpc_block = rpc_block
                    .ok_or_else(|| RaikoError::RPC(format!("block {block_number} not found")))?;
                blocks.push(rpc_block.into());
            }
        }

        Ok(blocks)
    }
}
