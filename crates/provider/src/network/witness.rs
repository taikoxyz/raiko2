use alloy::eips::BlockNumberOrTag;
use raiko2_primitives::{ExecutionWitness, RaikoError, RaikoResult};

use super::NetworkProvider;

impl NetworkProvider {
    pub(crate) async fn fetch_witnesses(
        &self,
        block_numbers: &[u64],
    ) -> RaikoResult<Vec<ExecutionWitness>> {
        const MAX_BATCH_SIZE: usize = 32;
        let mut witnesses = Vec::with_capacity(block_numbers.len());
        for block_numbers in block_numbers.chunks(MAX_BATCH_SIZE) {
            let mut batch = self.l2_client.new_batch();
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
