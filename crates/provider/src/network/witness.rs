use alethia_reth_block::config::TaikoEvmConfig;
use alethia_reth_primitives::addresses::{TAIKO_GOLDEN_TOUCH_ADDRESS, get_treasury_address};
use alloy::{
    eips::BlockNumberOrTag,
    primitives::{Address, B256},
    providers::Provider as AlloyProvider,
};
use futures::{StreamExt, stream};
use raiko2_primitives::{
    ChainSpec, ExecutionWitness, RaikoError, RaikoResult, WitnessStateNode,
    chain_spec::SupportedChainSpecs,
};
use std::{sync::Arc, time::Instant};
use tracing::info;

use crate::on_the_spot_witness::execution_witness;

use super::NetworkProvider;

const DEFAULT_WITNESS_BATCH_SIZE: usize = 5;
const DEFAULT_SYSTEM_PROOF_BATCH_SIZE: usize = 64;

#[derive(Debug, Clone, Default)]
struct TaikoSystemProofTargets {
    account_only: Vec<Address>,
    storage_backed: Vec<Address>,
}

#[derive(Debug, Clone)]
struct SystemProofRequest {
    block_idx: usize,
    block_number: u64,
    parent_block_number: u64,
    address: Address,
    storage_keys: Vec<B256>,
}

fn taiko_system_proof_targets(chain_id: u64) -> TaikoSystemProofTargets {
    let mut account_only = Vec::with_capacity(1);
    let mut storage_backed = Vec::with_capacity(2);
    let golden_touch = Address::from(TAIKO_GOLDEN_TOUCH_ADDRESS);
    account_only.push(golden_touch);

    let treasury = get_treasury_address(chain_id);
    if treasury != golden_touch {
        storage_backed.push(treasury);
    }

    if let Some(chain_spec) = SupportedChainSpecs::default().get_chain_spec_with_chain_id(chain_id)
    {
        if let Some(l2_contract) = chain_spec.l2_contract
            && !account_only.contains(&l2_contract)
            && !storage_backed.contains(&l2_contract)
        {
            storage_backed.push(l2_contract);
        }
    }

    TaikoSystemProofTargets {
        account_only,
        storage_backed,
    }
}

fn witness_storage_keys(witness: &ExecutionWitness) -> Vec<B256> {
    let mut keys = witness
        .keys
        .iter()
        .filter(|key| key.len() == B256::len_bytes())
        .map(|key| B256::from_slice(key.as_ref()))
        .collect::<Vec<_>>();
    keys.sort_unstable();
    keys.dedup();
    keys
}

fn should_fallback_to_on_the_spot(err: &RaikoError) -> bool {
    let RaikoError::RPC(message) = err else {
        return false;
    };

    let lower = message.to_ascii_lowercase();
    lower.contains("method not found")
        || lower.contains("method_not_found")
        || lower.contains("-32601")
        || lower.contains("debug_executionwitness")
            && (lower.contains("unsupported") || lower.contains("not available"))
}

fn witness_batch_size() -> usize {
    std::env::var("WITNESS_BATCH_SIZE")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_WITNESS_BATCH_SIZE)
}

fn system_proof_batch_size() -> usize {
    std::env::var("SYSTEM_PROOF_BATCH_SIZE")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_SYSTEM_PROOF_BATCH_SIZE)
}

impl NetworkProvider {
    fn resolved_l2_chain_spec(&self, chain_id: u64) -> Option<ChainSpec> {
        self.l2_chain_spec
            .as_ref()
            .filter(|spec| spec.chain_id == chain_id)
            .cloned()
            .or_else(|| SupportedChainSpecs::default().get_chain_spec_with_chain_id(chain_id))
    }

    async fn fetch_on_the_spot_taiko_witnesses(
        &self,
        chain_id: u64,
        block_numbers: &[u64],
    ) -> RaikoResult<Vec<ExecutionWitness>> {
        let taiko_chain_spec = self
            .resolved_l2_chain_spec(chain_id)
            .ok_or_else(|| {
                RaikoError::RPC(format!(
                    "cannot build on-the-spot witness: unsupported chain_id {chain_id}"
                ))
            })?
            .to_taiko_chain_spec()
            .map_err(|e| {
                RaikoError::RPC(format!(
                    "cannot build on-the-spot witness for chain_id {chain_id}: {e}"
                ))
            })?;
        let evm_config = Arc::new(TaikoEvmConfig::new(taiko_chain_spec));
        let batch_size = witness_batch_size();
        let mut witness_chunk = stream::iter(block_numbers.iter().copied().enumerate())
            .map(|(index, block_number)| {
                let evm_config = Arc::clone(&evm_config);
                async move {
                    let witness = execution_witness(
                        evm_config,
                        &self.l2_provider,
                        block_number.into(),
                    )
                    .await
                    .map_err(|e| {
                        RaikoError::RPC(format!(
                            "on-the-spot witness build failed for block {block_number}: {e:#}"
                        ))
                    })?;
                    Ok::<_, RaikoError>((index, witness))
                }
            })
            .buffer_unordered(batch_size)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<RaikoResult<Vec<_>>>()?;

        witness_chunk.sort_by_key(|(index, _)| *index);
        Ok(witness_chunk
            .into_iter()
            .map(|(_, witness)| witness)
            .collect())
    }

    fn build_system_proof_requests(
        witness_chunk: &[(u64, ExecutionWitness)],
        address: Address,
        include_storage_keys: bool,
    ) -> RaikoResult<Vec<SystemProofRequest>> {
        witness_chunk
            .iter()
            .enumerate()
            .map(|(block_idx, (block_number, witness))| {
                let parent_block_number = block_number.checked_sub(1).ok_or_else(|| {
                    RaikoError::RPC(format!(
                        "cannot fetch system proof for genesis block {block_number}"
                    ))
                })?;
                Ok(SystemProofRequest {
                    block_idx,
                    block_number: *block_number,
                    parent_block_number,
                    address,
                    storage_keys: if include_storage_keys {
                        witness_storage_keys(witness)
                    } else {
                        Vec::new()
                    },
                })
            })
            .collect()
    }

    async fn fetch_system_account_proofs(
        &self,
        requests: &[SystemProofRequest],
    ) -> RaikoResult<Vec<(usize, alloy_rpc_types_eth::EIP1186AccountProofResponse)>> {
        let mut proofs = Vec::with_capacity(requests.len());
        let batch_size = system_proof_batch_size();

        for chunk in requests.chunks(batch_size) {
            let mut batch = self.l2_client.new_batch();
            let mut pending = Vec::with_capacity(chunk.len());

            for request in chunk {
                pending.push((
                    request.block_idx,
                    request.block_number,
                    request.parent_block_number,
                    request.address,
                    Box::pin(
                        batch
                            .add_call::<_, alloy_rpc_types_eth::EIP1186AccountProofResponse>(
                                "eth_getProof",
                                &(
                                    request.address,
                                    request.storage_keys.clone(),
                                    BlockNumberOrTag::from(request.parent_block_number),
                                ),
                            )
                            .map_err(|_| {
                                RaikoError::RPC(
                                    "failed adding system eth_getProof call to batch".to_owned(),
                                )
                            })?,
                    ),
                ));
            }

            batch.send().await.map_err(|e| {
                let blocks = chunk
                    .iter()
                    .map(|request| request.block_number)
                    .collect::<Vec<_>>();
                RaikoError::RPC(format!(
                    "error sending system eth_getProof batch for blocks {blocks:?}: {e}"
                ))
            })?;

            for (block_idx, block_number, parent_block_number, address, request) in pending {
                let proof = request.await.map_err(|e| {
                    RaikoError::RPC(format!(
                        "eth_getProof failed for system address {address} at parent block {parent_block_number} (block {block_number}): {e}"
                    ))
                })?;
                proofs.push((block_idx, proof));
            }
        }

        Ok(proofs)
    }

    async fn supplement_taiko_system_account_proofs(
        &self,
        witness_chunk: &mut [(u64, ExecutionWitness)],
        targets: &TaikoSystemProofTargets,
    ) -> RaikoResult<()> {
        for &address in &targets.account_only {
            let requests = Self::build_system_proof_requests(witness_chunk, address, false)?;
            let mut proofs = self.fetch_system_account_proofs(&requests).await?;
            proofs.sort_by_key(|(block_idx, _)| *block_idx);

            for ((_, witness), (_, proof)) in witness_chunk.iter_mut().zip(proofs.into_iter()) {
                witness.state.extend(
                    proof
                        .account_proof
                        .into_iter()
                        .map(WitnessStateNode::from_bytes),
                );
            }
        }

        for &address in &targets.storage_backed {
            let requests = Self::build_system_proof_requests(witness_chunk, address, true)?;
            let mut proofs = self.fetch_system_account_proofs(&requests).await?;
            proofs.sort_by_key(|(block_idx, _)| *block_idx);

            for ((_, witness), (_, proof)) in witness_chunk.iter_mut().zip(proofs.into_iter()) {
                witness.state.extend(
                    proof
                        .account_proof
                        .into_iter()
                        .map(WitnessStateNode::from_bytes),
                );
                witness.state.extend(
                    proof
                        .storage_proof
                        .into_iter()
                        .flat_map(|storage_proof| storage_proof.proof)
                        .map(WitnessStateNode::from_bytes),
                );
            }
        }

        for (_, witness) in witness_chunk.iter_mut() {
            witness.state =
                ExecutionWitness::canonicalize_state_nodes(std::mem::take(&mut witness.state));
        }

        Ok(())
    }

    async fn fetch_witnesses_via_debug_endpoint(
        &self,
        block_numbers: &[u64],
        targets: &TaikoSystemProofTargets,
    ) -> RaikoResult<Vec<ExecutionWitness>> {
        let started_at = Instant::now();
        let mut witness_chunk = Vec::with_capacity(block_numbers.len());
        let batch_size = witness_batch_size();

        for indexed_chunk in block_numbers
            .iter()
            .copied()
            .enumerate()
            .collect::<Vec<_>>()
            .chunks(batch_size)
        {
            let mut batch = self.l2_client.new_batch();
            let mut pending = Vec::with_capacity(indexed_chunk.len());

            for &(index, block_number) in indexed_chunk {
                pending.push((
                    index,
                    block_number,
                    Box::pin(
                        batch
                            .add_call::<_, alloy_rpc_types_debug::ExecutionWitness>(
                                "debug_executionWitness",
                                &(BlockNumberOrTag::from(block_number),),
                            )
                            .map_err(|_| {
                                RaikoError::RPC(
                                    "failed adding debug_executionWitness call to batch".to_owned(),
                                )
                            })?,
                    ),
                ));
            }

            batch.send().await.map_err(|e| {
                let blocks = indexed_chunk
                    .iter()
                    .map(|(_, block_number)| *block_number)
                    .collect::<Vec<_>>();
                RaikoError::RPC(format!(
                    "error sending debug_executionWitness batch for blocks {blocks:?}: {e}"
                ))
            })?;

            for (index, block_number, request) in pending {
                let raw_witness = request.await.map_err(|e| {
                    RaikoError::RPC(format!(
                        "debug_executionWitness failed for block {block_number}: {e}"
                    ))
                })?;
                let witness = ExecutionWitness::try_from(raw_witness).map_err(|e| {
                    RaikoError::RPC(format!(
                        "failed to decode debug_executionWitness headers for block {block_number}: {e}"
                    ))
                })?;
                witness_chunk.push((index, block_number, witness));
            }
        }

        witness_chunk.sort_by_key(|(index, _, _)| *index);
        let mut witness_chunk = witness_chunk
            .into_iter()
            .map(|(_, block_number, witness)| (block_number, witness))
            .collect::<Vec<_>>();

        if !targets.account_only.is_empty() || !targets.storage_backed.is_empty() {
            self.supplement_taiko_system_account_proofs(&mut witness_chunk, targets)
                .await?;
        }

        info!(
            block_count = block_numbers.len(),
            witness_batch_size = batch_size,
            system_proof_batch_size = system_proof_batch_size(),
            elapsed_ms = started_at.elapsed().as_millis(),
            "fetched proposal-window execution witnesses"
        );
        Ok(witness_chunk
            .into_iter()
            .map(|(_, witness)| witness)
            .collect())
    }

    pub(crate) async fn fetch_witnesses(
        &self,
        block_numbers: &[u64],
    ) -> RaikoResult<Vec<ExecutionWitness>> {
        let started_at = Instant::now();
        let chain_id = self.l2_provider.get_chain_id().await.map_err(|e| {
            RaikoError::RPC(format!("eth_chainId failed while fetching witnesses: {e}"))
        })?;
        let resolved_l2_chain_spec = self.resolved_l2_chain_spec(chain_id);
        let supports_taiko_on_the_spot =
            resolved_l2_chain_spec.as_ref().is_some_and(|chain_spec| {
                chain_spec.is_taiko() && chain_spec.to_taiko_chain_spec().is_ok()
            });

        let system_proof_targets = resolved_l2_chain_spec
            .filter(|chain_spec| chain_spec.is_taiko())
            .map_or_else(TaikoSystemProofTargets::default, |_| {
                taiko_system_proof_targets(chain_id)
            });
        match self
            .fetch_witnesses_via_debug_endpoint(block_numbers, &system_proof_targets)
            .await
        {
            Ok(witnesses) => {
                info!(
                    chain_id,
                    block_count = block_numbers.len(),
                    elapsed_ms = started_at.elapsed().as_millis(),
                    "fetched witnesses via debug_executionWitness"
                );
                Ok(witnesses)
            }
            Err(err) if supports_taiko_on_the_spot && should_fallback_to_on_the_spot(&err) => {
                let witnesses = self
                    .fetch_on_the_spot_taiko_witnesses(chain_id, block_numbers)
                    .await?;
                info!(
                    chain_id,
                    block_count = block_numbers.len(),
                    elapsed_ms = started_at.elapsed().as_millis(),
                    "fetched witnesses via on-the-spot fallback"
                );
                Ok(witnesses)
            }
            Err(err) => Err(err),
        }
    }
}
