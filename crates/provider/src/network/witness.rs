use alethia_reth_block::{
    config::{TaikoEvmConfig, TaikoNextBlockEnvAttributes},
    derived_block::{DerivedBlockExecutionOutcome, execute_derived_block},
};
use alethia_reth_primitives::addresses::{TAIKO_GOLDEN_TOUCH_ADDRESS, get_treasury_address};
use alloy::{
    consensus::{
        Header,
        proofs::{calculate_ommers_root, calculate_transaction_root, calculate_withdrawals_root},
        transaction::{Recovered, SignerRecoverable},
    },
    eips::BlockNumberOrTag,
    primitives::{Address, B256, Bytes},
    providers::Provider as AlloyProvider,
};
use anyhow::Context;
use futures::{StreamExt, stream};
use raiko2_primitives::{
    ChainSpec, ExecutionWitness, RaikoError, RaikoResult, StatelessInput, WitnessHeader,
    WitnessStateNode, chain_spec::SupportedChainSpecs,
};
use raiko2_protocol_shasta::shasta::manifest::BlockManifest;
use reth_ethereum_primitives::{Block as RethBlock, BlockBody as RethBlockBody, TransactionSigned};
use reth_evm::{ConfigureEvm, execute::Executor};
use reth_primitives_traits::{RecoveredBlock, SealedHeader};
use serde::{Deserialize, Deserializer};
use std::{collections::HashSet, sync::Arc, time::Instant};
use tracing::{debug, info};

use crate::on_the_spot_witness::{PreflightDb, ProviderConfig, ProviderDb, execution_witness};

use super::{GethL2Provider, GethLocalWitnessL2Provider, RethL2Provider, RpcL2Provider};

const DEFAULT_WITNESS_BATCH_SIZE: usize = 2;
const DEFAULT_SYSTEM_PROOF_BATCH_SIZE: usize = 64;

type CandidateDb = PreflightDb<ProviderDb<alloy::network::Ethereum, alloy::providers::DynProvider>>;

#[derive(Debug, Default)]
struct CandidateSupplement {
    ancestor_headers: Vec<WitnessHeader>,
}

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

#[derive(Debug, Deserialize)]
struct GethExecutionWitness {
    #[serde(default, deserialize_with = "deserialize_null_default")]
    headers: Vec<Header>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    codes: Vec<Bytes>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    state: Vec<Bytes>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    keys: Vec<Bytes>,
}

fn deserialize_null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Option::<T>::deserialize(deserializer).map(Option::unwrap_or_default)
}

impl TryFrom<GethExecutionWitness> for ExecutionWitness {
    type Error = alloy::rlp::Error;

    fn try_from(value: GethExecutionWitness) -> Result<Self, Self::Error> {
        let headers = value
            .headers
            .iter()
            .map(WitnessHeader::from_encoded_header)
            .collect::<Result<_, _>>()?;
        let state = value
            .state
            .into_iter()
            .map(WitnessStateNode::from_bytes)
            .collect();

        Ok(Self {
            state: Self::canonicalize_state_nodes(state),
            state_indices: Vec::new(),
            codes: value.codes,
            keys: value.keys,
            headers: Self::canonicalize_headers(headers),
        })
    }
}

fn taiko_system_proof_targets(chain_id: u64) -> TaikoSystemProofTargets {
    let mut account_only = Vec::with_capacity(2);
    let mut storage_backed = Vec::with_capacity(1);
    let golden_touch = Address::from(TAIKO_GOLDEN_TOUCH_ADDRESS);
    account_only.push(golden_touch);

    let treasury = get_treasury_address(chain_id);
    if treasury != golden_touch && !account_only.contains(&treasury) {
        account_only.push(treasury);
    }

    // `l2_contract` remains downstream Raiko chain-spec metadata. The built-in Taiko specs
    // currently map it to the same `...10001` address as treasury, but this fallback stays here
    // for custom chain specs or future configurations where it diverges again.
    if let Some(chain_spec) = SupportedChainSpecs::default().get_chain_spec_with_chain_id(chain_id)
        && let Some(l2_contract) = chain_spec.l2_contract
        && !account_only.contains(&l2_contract)
        && !storage_backed.contains(&l2_contract)
    {
        storage_backed.push(l2_contract);
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

fn witness_has_account_preimage(witness: &ExecutionWitness, address: Address) -> bool {
    witness
        .keys
        .iter()
        .any(|key| key.len() == Address::len_bytes() && key.as_ref() == address.as_slice())
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

fn decode_manifest_transactions(
    expected_block: &BlockManifest,
) -> RaikoResult<Vec<TransactionSigned>> {
    expected_block
        .transactions
        .iter()
        .map(|transaction| {
            alloy_rlp::decode_exact(alloy_rlp::encode(transaction)).map_err(|err| {
                RaikoError::Provider(format!(
                    "failed to decode Shasta manifest transaction: {err}"
                ))
            })
        })
        .collect()
}

fn recovered_manifest_transactions(
    transactions: Vec<TransactionSigned>,
) -> Vec<Recovered<TransactionSigned>> {
    transactions
        .into_iter()
        .map(|transaction| {
            let signer = transaction.recover_signer().unwrap_or(Address::ZERO);
            Recovered::new_unchecked(transaction, signer)
        })
        .collect()
}

fn parent_header_from_witness(input: &StatelessInput) -> RaikoResult<SealedHeader> {
    input
        .witness
        .headers
        .last()
        .and_then(|header| {
            header
                .full_header()
                .cloned()
                .map(|full_header| SealedHeader::new(full_header, header.hash))
        })
        .ok_or_else(|| {
            RaikoError::Provider(format!(
                "missing parent header while supplementing Shasta witness for block {}",
                input.block.header.number
            ))
        })
}

fn canonical_anchor_tx(input: &StatelessInput) -> RaikoResult<Recovered<TransactionSigned>> {
    let anchor_tx = input
        .block
        .body
        .transactions()
        .next()
        .cloned()
        .ok_or_else(|| {
            RaikoError::Provider(format!(
                "missing anchor transaction while supplementing Shasta witness for block {}",
                input.block.header.number
            ))
        })?;
    reth_primitives_traits::SignedTransaction::try_into_recovered(anchor_tx).map_err(|_| {
        RaikoError::Provider(format!(
            "failed to recover anchor transaction for block {}",
            input.block.header.number
        ))
    })
}

fn candidate_block_env(input: &StatelessInput) -> RaikoResult<TaikoNextBlockEnvAttributes> {
    Ok(TaikoNextBlockEnvAttributes {
        timestamp: input.block.header.timestamp,
        suggested_fee_recipient: input.block.header.beneficiary,
        prev_randao: input.block.header.mix_hash,
        gas_limit: input.block.header.gas_limit,
        extra_data: input.block.header.extra_data.clone(),
        base_fee_per_gas: input.block.header.base_fee_per_gas.ok_or_else(|| {
            RaikoError::Provider(format!(
                "missing base fee while supplementing Shasta witness for block {}",
                input.block.header.number
            ))
        })?,
    })
}

fn build_candidate_block(
    parent_header: &SealedHeader,
    anchor_tx: Recovered<TransactionSigned>,
    transactions: Vec<TransactionSigned>,
    block_env: TaikoNextBlockEnvAttributes,
) -> RecoveredBlock<RethBlock> {
    let mut block_transactions = Vec::with_capacity(transactions.len() + 1);
    let mut senders = Vec::with_capacity(block_transactions.capacity());

    senders.push(anchor_tx.signer());
    block_transactions.push(anchor_tx.into_inner());

    for recovered in recovered_manifest_transactions(transactions) {
        senders.push(recovered.signer());
        block_transactions.push(recovered.into_inner());
    }

    let body = RethBlockBody {
        transactions: block_transactions,
        ommers: Vec::default(),
        withdrawals: Some(Vec::default().into()),
    };
    let header = Header {
        parent_hash: parent_header.hash(),
        number: parent_header.number + 1,
        timestamp: block_env.timestamp,
        beneficiary: block_env.suggested_fee_recipient,
        gas_limit: block_env.gas_limit,
        base_fee_per_gas: Some(block_env.base_fee_per_gas),
        mix_hash: block_env.prev_randao,
        extra_data: block_env.extra_data,
        transactions_root: calculate_transaction_root(body.transactions.as_slice()),
        ommers_hash: calculate_ommers_root(body.ommers.as_slice()),
        withdrawals_root: body
            .withdrawals
            .as_ref()
            .map(|withdrawals| calculate_withdrawals_root(withdrawals.as_slice())),
        ..Default::default()
    };

    RecoveredBlock::new_unhashed(RethBlock { header, body }, senders)
}

fn transaction_sequences_match(
    left: &[Recovered<TransactionSigned>],
    right: &[TransactionSigned],
) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| alloy_rlp::encode(left.inner()) == alloy_rlp::encode(right))
}

async fn merge_candidate_witness(
    witness: &mut ExecutionWitness,
    mut candidate: CandidateDb,
    parent_hash: B256,
) -> RaikoResult<CandidateSupplement> {
    let ancestor_headers = if candidate.has_block_hash_accesses() {
        candidate
            .ancestor_proof(parent_hash)
            .await
            .map_err(|err| {
                RaikoError::Provider(format!("failed to build candidate ancestor proof: {err:#}"))
            })?
            .into_iter()
            .map(|header| WitnessHeader::from_header(header.into()))
            .collect()
    } else {
        Vec::new()
    };

    let (state_trie, storage_tries) = candidate.state_proof().await.map_err(|err| {
        RaikoError::Provider(format!("failed to build candidate state proof: {err:#}"))
    })?;

    let mut state = std::mem::take(&mut witness.state);
    state.extend(
        state_trie
            .rlp_nodes()
            .into_iter()
            .map(WitnessStateNode::from_bytes),
    );
    for storage_trie in storage_tries.values() {
        state.extend(
            storage_trie
                .rlp_nodes()
                .into_iter()
                .map(WitnessStateNode::from_bytes),
        );
    }
    witness.state = ExecutionWitness::canonicalize_state_nodes(state);

    let mut codes = std::mem::take(&mut witness.codes);
    codes.extend(candidate.contracts().values().cloned());
    let mut seen = HashSet::new();
    codes.retain(|code| seen.insert(alloy::primitives::keccak256(code.as_ref())));
    witness.codes = codes;

    Ok(CandidateSupplement { ancestor_headers })
}

fn merge_witness_headers(
    witness: &mut ExecutionWitness,
    headers: impl IntoIterator<Item = WitnessHeader>,
) -> RaikoResult<usize> {
    let mut added = 0usize;
    let mut witness_headers = std::mem::take(&mut witness.headers);

    for header in headers {
        match witness_headers
            .iter_mut()
            .find(|existing| existing.number == header.number)
        {
            Some(existing) if existing.hash != header.hash => {
                return Err(RaikoError::Provider(format!(
                    "conflicting ancestor header for block {} while supplementing Shasta witness",
                    header.number
                )));
            }
            Some(existing) => {
                if existing.full_header().is_none() && header.full_header().is_some() {
                    *existing = header;
                }
            }
            None => {
                witness_headers.push(header);
                added += 1;
            }
        }
    }

    witness.headers = ExecutionWitness::canonicalize_headers(witness_headers);
    Ok(added)
}

fn new_candidate_db(provider: alloy::providers::DynProvider, parent_hash: B256) -> CandidateDb {
    PreflightDb::new(ProviderDb::new(
        provider,
        ProviderConfig::default(),
        parent_hash,
    ))
}

async fn execute_candidate_block(
    evm_config: Arc<TaikoEvmConfig>,
    provider: alloy::providers::DynProvider,
    parent_header: SealedHeader,
    derived_block: RecoveredBlock<RethBlock>,
    block_number: u64,
) -> RaikoResult<DerivedBlockExecutionOutcome> {
    let parent_hash = parent_header.hash();
    let db = new_candidate_db(provider, parent_hash);
    tokio::task::spawn_blocking(move || {
        execute_derived_block(evm_config.as_ref(), &parent_header, &derived_block, db)
    })
    .await
    .map_err(|err| {
        RaikoError::Provider(format!(
            "candidate execution join failed for block {block_number}: {err}",
        ))
    })?
    .map_err(|err| {
        RaikoError::Provider(format!(
            "candidate execution failed for block {block_number}: {err}",
        ))
    })
}

async fn replay_candidate_db(
    evm_config: Arc<TaikoEvmConfig>,
    provider: alloy::providers::DynProvider,
    parent_hash: B256,
    derived_block: RecoveredBlock<RethBlock>,
    block_number: u64,
) -> RaikoResult<CandidateDb> {
    let db = new_candidate_db(provider, parent_hash);
    tokio::task::spawn_blocking(move || {
        let executor = evm_config.executor(db);
        let mut database_capture = None;
        let result = executor.execute_with_state_closure(&derived_block, |state| {
            database_capture = Some(state.database.clone());
        });
        result.map(|_| database_capture)
    })
    .await
    .map_err(|err| {
        RaikoError::Provider(format!(
            "candidate witness replay join failed for block {block_number}: {err}",
        ))
    })?
    .map_err(|err| {
        RaikoError::Provider(format!(
            "candidate witness replay failed for block {block_number}: {err}",
        ))
    })?
    .context("candidate witness replay did not capture preflight db")
    .map_err(|err| {
        RaikoError::Provider(format!(
            "candidate witness replay failed for block {block_number}: {err}",
        ))
    })
}

async fn supplement_shasta_candidate_witness(
    provider: alloy::providers::DynProvider,
    evm_config: Arc<TaikoEvmConfig>,
    expected_block: &BlockManifest,
    input: &mut StatelessInput,
    chain_id: u64,
) -> RaikoResult<Option<CandidateSupplement>> {
    let candidate_tx_count = expected_block.transactions.len().saturating_add(1);
    let canonical_transactions = input.block.body.transactions().cloned().collect::<Vec<_>>();
    let canonical_tx_count = canonical_transactions.len();
    if candidate_tx_count < canonical_tx_count {
        return Err(RaikoError::Provider(format!(
            "Shasta manifest has fewer candidate transactions ({candidate_tx_count}) than canonical block {} ({canonical_tx_count})",
            input.block.header.number
        )));
    }
    if candidate_tx_count == canonical_tx_count {
        return Ok(None);
    }

    let parent_header = parent_header_from_witness(input)?;
    let parent_hash = parent_header.hash();
    let anchor_tx = canonical_anchor_tx(input)?;
    let block_env = candidate_block_env(input)?;
    let transactions = decode_manifest_transactions(expected_block)?;
    let derived_block = build_candidate_block(&parent_header, anchor_tx, transactions, block_env);
    let block_number = input.block.header.number;

    let outcome = execute_candidate_block(
        Arc::clone(&evm_config),
        provider.clone(),
        parent_header,
        derived_block.clone(),
        block_number,
    )
    .await?;
    if !transaction_sequences_match(&outcome.committed_transactions, &canonical_transactions) {
        return Err(RaikoError::Provider(format!(
            "candidate execution for block {block_number} did not reproduce canonical transactions: committed={}, canonical={canonical_tx_count}",
            outcome.committed_transactions.len(),
        )));
    }

    let candidate_db = replay_candidate_db(
        evm_config,
        provider,
        parent_hash,
        derived_block,
        block_number,
    )
    .await?;
    let supplement = merge_candidate_witness(&mut input.witness, candidate_db, parent_hash).await?;
    debug!(
        chain_id,
        block_number,
        candidate_tx_count,
        canonical_tx_count,
        "supplemented Shasta candidate witness"
    );
    Ok(Some(supplement))
}

impl RpcL2Provider {
    fn resolved_l2_chain_spec(&self, chain_id: u64) -> Option<ChainSpec> {
        self.chain_spec
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
                        &self.witness_provider,
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

    pub(super) async fn supplement_shasta_candidate_witnesses(
        &self,
        expected_blocks: &[BlockManifest],
        inputs: &mut [StatelessInput],
    ) -> RaikoResult<()> {
        if expected_blocks.len() != inputs.len() {
            return Err(RaikoError::Provider(format!(
                "expected block count ({}) does not match witness count ({})",
                expected_blocks.len(),
                inputs.len()
            )));
        }

        let chain_id = self.witness_provider.get_chain_id().await.map_err(|e| {
            RaikoError::RPC(format!(
                "eth_chainId failed while supplementing Shasta candidate witnesses: {e}"
            ))
        })?;
        let taiko_chain_spec = self
            .resolved_l2_chain_spec(chain_id)
            .filter(ChainSpec::is_taiko)
            .ok_or_else(|| {
                RaikoError::Provider(format!(
                    "cannot supplement Shasta candidate witnesses for unsupported chain_id {chain_id}"
                ))
            })?
            .to_taiko_chain_spec()
            .map_err(|e| {
                RaikoError::Provider(format!(
                    "cannot build Taiko EVM config for chain_id {chain_id}: {e}"
                ))
            })?;
        let evm_config = Arc::new(TaikoEvmConfig::new(taiko_chain_spec));
        let started_at = Instant::now();
        let mut supplemented_blocks = 0usize;
        let mut supplemented_ancestor_headers = 0usize;
        let first_block_number = inputs.first().map(|input| input.block.header.number);
        let mut initial_ancestor_headers = Vec::new();

        for (expected_block, input) in expected_blocks.iter().zip(inputs.iter_mut()) {
            let Some(supplement) = supplement_shasta_candidate_witness(
                self.witness_provider.clone(),
                Arc::clone(&evm_config),
                expected_block,
                input,
                chain_id,
            )
            .await?
            else {
                continue;
            };

            if let Some(first_block_number) = first_block_number {
                initial_ancestor_headers.extend(
                    supplement
                        .ancestor_headers
                        .into_iter()
                        .filter(|header| header.number < first_block_number),
                );
            }
            supplemented_blocks += 1;
        }

        if let Some(first_input) = inputs.first_mut() {
            supplemented_ancestor_headers =
                merge_witness_headers(&mut first_input.witness, initial_ancestor_headers)?;
        }

        if supplemented_blocks > 0 {
            info!(
                chain_id,
                block_count = inputs.len(),
                supplemented_blocks,
                supplemented_ancestor_headers,
                elapsed_ms = started_at.elapsed().as_millis(),
                "supplemented Shasta source-only candidate witnesses"
            );
        }
        Ok(())
    }

    fn build_system_proof_requests(
        witness_chunk: &[(u64, ExecutionWitness)],
        block_indices: &[usize],
        address: Address,
        include_storage_keys: bool,
    ) -> RaikoResult<Vec<SystemProofRequest>> {
        block_indices
            .iter()
            .map(|&block_idx| {
                let (block_number, witness) = witness_chunk.get(block_idx).ok_or_else(|| {
                    RaikoError::RPC(format!(
                        "missing witness chunk entry while building system proof request for block index {block_idx}"
                    ))
                })?;
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

    fn missing_account_proof_block_indices(
        witness_chunk: &[(u64, ExecutionWitness)],
        address: Address,
    ) -> Vec<usize> {
        witness_chunk
            .iter()
            .enumerate()
            .filter_map(|(block_idx, (_, witness))| {
                (!witness_has_account_preimage(witness, address)).then_some(block_idx)
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
            let mut batch = self.witness_client.new_batch();
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
            let missing_block_indices =
                Self::missing_account_proof_block_indices(witness_chunk, address);
            if missing_block_indices.is_empty() {
                continue;
            }

            let requests = Self::build_system_proof_requests(
                witness_chunk,
                &missing_block_indices,
                address,
                false,
            )?;
            let mut proofs = self.fetch_system_account_proofs(&requests).await?;
            proofs.sort_by_key(|(block_idx, _)| *block_idx);

            for (block_idx, proof) in proofs {
                let (_, witness) = &mut witness_chunk[block_idx];
                witness.state.extend(
                    proof
                        .account_proof
                        .into_iter()
                        .map(WitnessStateNode::from_bytes),
                );
            }
        }

        for &address in &targets.storage_backed {
            let block_indices = (0..witness_chunk.len()).collect::<Vec<_>>();
            let requests =
                Self::build_system_proof_requests(witness_chunk, &block_indices, address, true)?;
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
}

impl RethL2Provider {
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
            let mut batch = self.rpc.witness_client.new_batch();
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
            self.rpc
                .supplement_taiko_system_account_proofs(&mut witness_chunk, targets)
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

    pub(super) async fn fetch_witnesses(
        &self,
        block_numbers: &[u64],
    ) -> RaikoResult<Vec<ExecutionWitness>> {
        let started_at = Instant::now();
        let chain_id = self
            .rpc
            .witness_provider
            .get_chain_id()
            .await
            .map_err(|e| {
                RaikoError::RPC(format!("eth_chainId failed while fetching witnesses: {e}"))
            })?;
        let resolved_l2_chain_spec = self.rpc.resolved_l2_chain_spec(chain_id);
        let supports_taiko_on_the_spot =
            resolved_l2_chain_spec.as_ref().is_some_and(|chain_spec| {
                chain_spec.is_taiko() && chain_spec.to_taiko_chain_spec().is_ok()
            });

        let system_proof_targets = resolved_l2_chain_spec
            .filter(ChainSpec::is_taiko)
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
                    .rpc
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

impl GethL2Provider {
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
            let mut batch = self.rpc.witness_client.new_batch();
            let mut pending = Vec::with_capacity(indexed_chunk.len());

            for &(index, block_number) in indexed_chunk {
                pending.push((
                    index,
                    block_number,
                    Box::pin(
                        batch
                            .add_call::<_, GethExecutionWitness>(
                                "debug_executionWitness",
                                &(BlockNumberOrTag::from(block_number),),
                            )
                            .map_err(|_| {
                                RaikoError::RPC(
                                    "failed adding geth debug_executionWitness call to batch"
                                        .to_owned(),
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
                    "error sending geth debug_executionWitness batch for blocks {blocks:?}: {e}"
                ))
            })?;

            for (index, block_number, request) in pending {
                let raw_witness = request.await.map_err(|e| {
                    RaikoError::RPC(format!(
                        "geth debug_executionWitness failed for block {block_number}: {e}"
                    ))
                })?;
                let witness = ExecutionWitness::try_from(raw_witness).map_err(|e| {
                    RaikoError::RPC(format!(
                        "failed to decode geth debug_executionWitness headers for block {block_number}: {e}"
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
            self.rpc
                .supplement_taiko_system_account_proofs(&mut witness_chunk, targets)
                .await?;
        }

        info!(
            block_count = block_numbers.len(),
            witness_batch_size = batch_size,
            system_proof_batch_size = system_proof_batch_size(),
            elapsed_ms = started_at.elapsed().as_millis(),
            "fetched proposal-window geth execution witnesses"
        );
        Ok(witness_chunk
            .into_iter()
            .map(|(_, witness)| witness)
            .collect())
    }

    pub(super) async fn fetch_witnesses(
        &self,
        block_numbers: &[u64],
    ) -> RaikoResult<Vec<ExecutionWitness>> {
        let started_at = Instant::now();
        let chain_id = self
            .rpc
            .witness_provider
            .get_chain_id()
            .await
            .map_err(|e| {
                RaikoError::RPC(format!(
                    "eth_chainId failed while fetching geth witnesses: {e}"
                ))
            })?;
        let system_proof_targets = self
            .rpc
            .resolved_l2_chain_spec(chain_id)
            .filter(ChainSpec::is_taiko)
            .map_or_else(TaikoSystemProofTargets::default, |_| {
                taiko_system_proof_targets(chain_id)
            });
        let witnesses = self
            .fetch_witnesses_via_debug_endpoint(block_numbers, &system_proof_targets)
            .await?;
        info!(
            chain_id,
            block_count = block_numbers.len(),
            elapsed_ms = started_at.elapsed().as_millis(),
            "fetched witnesses via geth debug_executionWitness"
        );
        Ok(witnesses)
    }
}

impl GethLocalWitnessL2Provider {
    pub(super) async fn fetch_witnesses(
        &self,
        block_numbers: &[u64],
    ) -> RaikoResult<Vec<ExecutionWitness>> {
        let started_at = Instant::now();
        let chain_id = self
            .rpc
            .witness_provider
            .get_chain_id()
            .await
            .map_err(|e| {
                RaikoError::RPC(format!(
                    "eth_chainId failed while building geth local witnesses: {e}"
                ))
            })?;
        let witnesses = self
            .rpc
            .fetch_on_the_spot_taiko_witnesses(chain_id, block_numbers)
            .await?;
        info!(
            chain_id,
            block_count = block_numbers.len(),
            elapsed_ms = started_at.elapsed().as_millis(),
            "fetched witnesses via geth local on-the-spot provider"
        );
        Ok(witnesses)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::{B256, Bytes};

    fn sample_header(number: u64) -> Header {
        Header {
            parent_hash: B256::repeat_byte(0x11),
            number,
            timestamp: 1_700_000_000 + number,
            extra_data: Bytes::from_static(b"raiko2"),
            ..Default::default()
        }
    }

    #[test]
    fn geth_execution_witness_accepts_header_objects_and_null_keys() {
        let header = sample_header(7);
        let first = Bytes::from_static(b"node-a");
        let second = Bytes::from_static(b"node-b");
        let json = serde_json::json!({
            "headers": [header],
            "codes": [Bytes::from_static(b"code")],
            "state": [second.clone(), first.clone(), first.clone()],
            "keys": null,
        });

        let raw: GethExecutionWitness =
            serde_json::from_value(json).expect("deserialize geth witness");
        let witness = ExecutionWitness::try_from(raw).expect("convert geth witness");

        assert_eq!(witness.headers.len(), 1);
        assert_eq!(witness.headers[0].number, 7);
        assert!(witness.headers[0].full_header().is_some());
        assert_eq!(witness.codes, vec![Bytes::from_static(b"code")]);
        assert!(witness.keys.is_empty());
        assert_eq!(witness.state.len(), 2);
        let mut hashes = witness
            .state
            .iter()
            .map(|node| node.hash)
            .collect::<Vec<_>>();
        hashes.sort_unstable();
        let mut expected = vec![
            alloy::primitives::keccak256(&first),
            alloy::primitives::keccak256(&second),
        ];
        expected.sort_unstable();
        assert_eq!(hashes, expected);
    }

    #[test]
    fn merge_witness_headers_preserves_full_headers_without_duplicates() {
        let full_header = sample_header(7);
        let compact_header = WitnessHeader::from_compact_header(&full_header);
        let mut witness = ExecutionWitness {
            headers: vec![compact_header],
            ..Default::default()
        };

        let added = merge_witness_headers(
            &mut witness,
            vec![
                WitnessHeader::from_header(full_header),
                WitnessHeader::from_header(sample_header(6)),
            ],
        )
        .expect("merge headers");

        assert_eq!(added, 1);
        assert_eq!(
            witness
                .headers
                .iter()
                .map(|header| header.number)
                .collect::<Vec<_>>(),
            vec![6, 7]
        );
        assert!(
            witness
                .headers
                .iter()
                .find(|header| header.number == 7)
                .expect("header 7")
                .full_header()
                .is_some()
        );
    }

    #[test]
    fn merge_witness_headers_rejects_conflicting_hashes() {
        let mut witness = ExecutionWitness {
            headers: vec![WitnessHeader::from_header(sample_header(7))],
            ..Default::default()
        };
        let mut conflicting_header = sample_header(7);
        conflicting_header.parent_hash = B256::repeat_byte(0x22);

        let err = merge_witness_headers(
            &mut witness,
            vec![WitnessHeader::from_header(conflicting_header)],
        )
        .expect_err("conflicting header should fail");

        assert!(err.to_string().contains("conflicting ancestor header"));
    }
}
