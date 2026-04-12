use alloy::{eips::BlockNumberOrTag, providers::Provider as AlloyProvider};
use futures::{StreamExt, TryStreamExt, stream};
use raiko2_primitives::{ExecutionWitness, RaikoError, RaikoResult};

use super::NetworkProvider;

const WITNESS_CONCURRENCY: usize = 1;

impl NetworkProvider {
    pub(crate) async fn fetch_witnesses(
        &self,
        block_numbers: &[u64],
    ) -> RaikoResult<Vec<ExecutionWitness>> {
        stream::iter(block_numbers.iter().copied())
            .map(|block_number| async move {
                let raw_witness: alloy_rpc_types_debug::ExecutionWitness = self
                    .l2_provider
                    .client()
                    .request(
                        "debug_executionWitness",
                        (BlockNumberOrTag::from(block_number),),
                    )
                    .await
                    .map_err(|e| {
                        RaikoError::RPC(format!(
                            "debug_executionWitness failed for block {block_number}: {e}"
                        ))
                    })?;

                ExecutionWitness::try_from(raw_witness).map_err(|e| {
                    RaikoError::RPC(format!(
                        "failed to decode debug_executionWitness headers for block {block_number}: {e}"
                    ))
                })
            })
            .buffered(WITNESS_CONCURRENCY)
            .try_collect()
            .await
    }
}
