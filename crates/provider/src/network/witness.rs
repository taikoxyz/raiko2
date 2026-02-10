use alloy::{eips::BlockNumberOrTag, providers::Provider as AlloyProvider};
use futures::{StreamExt, stream};
use raiko2_primitives::{RaikoError, RaikoResult};
use reth_stateless::ExecutionWitness;
use std::sync::Arc;

use crate::on_the_spot_witness::execution_witness;

// For Taiko chains
use alethia_reth_block::config::TaikoEvmConfig;
use alethia_reth_chainspec::{TAIKO_DEVNET, TAIKO_HOODI, TAIKO_MAINNET};

use super::NetworkProvider;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum WitnessMode {
    #[default]
    Auto,
    ForceRemote,
    ForceLocal,
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

impl NetworkProvider {
    pub(crate) async fn build_local_witnesses(
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
        if super::is_taiko_chain_id(chain_id) {
            let evm_config = match chain_id {
                167_000 => Arc::new(TaikoEvmConfig::new(TAIKO_MAINNET.clone())),
                167_001 => Arc::new(TaikoEvmConfig::new(TAIKO_DEVNET.clone())),
                167_013 => Arc::new(TaikoEvmConfig::new(TAIKO_HOODI.clone())),
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

    pub(crate) async fn fetch_remote_witnesses(
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

    pub(crate) async fn fetch_witnesses(
        &self,
        block_numbers: &[u64],
    ) -> RaikoResult<Vec<ExecutionWitness>> {
        if self.witness_mode == WitnessMode::Auto && self.debug_witness_supported.is_none() {
            return Err(RaikoError::RPC(
                "Witness support not specified; set with_debug_witness_support(true|false)"
                    .to_owned(),
            ));
        }

        match witness_strategy(self.witness_mode, self.debug_witness_supported) {
            WitnessStrategy::RemoteOnly => self.fetch_remote_witnesses(block_numbers).await,
            WitnessStrategy::LocalOnly => self.build_local_then_collect(block_numbers).await,
        }
    }

    async fn build_local_then_collect(
        &self,
        block_numbers: &[u64],
    ) -> RaikoResult<Vec<ExecutionWitness>> {
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

// Reserved for future use if we want to detect unavailable remote witness RPC.
#[allow(dead_code)]
const fn is_missing_debug_witness(_err: &RaikoError) -> bool {
    false
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
