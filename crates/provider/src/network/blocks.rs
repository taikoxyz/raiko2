use alloy::{
    consensus::constants::EMPTY_WITHDRAWALS, eips::BlockNumberOrTag,
    providers::Provider as AlloyProvider,
};
use raiko2_primitives::{RaikoError, RaikoResult};
use reth_ethereum_primitives::Block as RethBlock;

use super::NetworkProvider;

// Workaround for inconsistent upstream block representations:
// Some RPC providers / conversions may set `withdrawals_root` to the canonical
// EMPTY_WITHDRAWALS root while omitting the actual withdrawals list (leaving it
// as `None`). This helper normalizes such blocks by materializing an explicit
// empty withdrawals list so downstream code can rely on a consistent
// representation. Once upstream always returns an explicit empty list when the
// root is EMPTY_WITHDRAWALS, this workaround can be revisited/removed.
fn normalize_withdrawals_for_empty_root(block: &mut RethBlock) {
    if block.body.withdrawals.is_none() && block.header.withdrawals_root == Some(EMPTY_WITHDRAWALS)
    {
        block.body.withdrawals = Some(alloy::eips::eip4895::Withdrawals::new(Vec::new()));
    }
}

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
                        "eth_getBlockByNumber failed for block {}: {e}",
                        block_number
                    ))
                })?
                .ok_or_else(|| RaikoError::RPC(format!("Block {} not found", block_number)))?;

            // Convert alloy BlockResponse to RethBlock
            let mut reth_block: RethBlock = rpc_block.into();
            normalize_withdrawals_for_empty_root(&mut reth_block);

            blocks.push(reth_block);
        }

        Ok(blocks)
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_withdrawals_for_empty_root;
    use alloy::consensus::constants::EMPTY_WITHDRAWALS;
    use reth_ethereum_primitives::Block as RethBlock;

    #[test]
    fn fills_empty_withdrawals_when_root_is_empty() {
        let mut block = RethBlock::default();
        block.header.withdrawals_root = Some(EMPTY_WITHDRAWALS);
        block.body.withdrawals = None;

        normalize_withdrawals_for_empty_root(&mut block);

        assert_eq!(block.body.withdrawals.as_ref().map(|w| w.len()), Some(0));
    }

    #[test]
    fn leaves_withdrawals_missing_when_root_is_absent() {
        let mut block = RethBlock::default();
        block.header.withdrawals_root = None;
        block.body.withdrawals = None;

        normalize_withdrawals_for_empty_root(&mut block);

        assert!(block.body.withdrawals.is_none());
    }
}
