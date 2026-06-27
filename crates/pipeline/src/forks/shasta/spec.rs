use super::checkpoint_verify::verify_guest_input_checkpoint_against_l2_rpc;
use super::manifest::ShastaManifestBuilder;
use crate::{PipelineKey, PipelineSpec, Preflight, ProverBackend, Validation};
use alethia_reth_block::config::TaikoEvmConfig;
use alethia_reth_consensus::validation::ANCHOR_V3_V4_GAS_LIMIT;
use alethia_reth_primitives::addresses::TAIKO_GOLDEN_TOUCH_ADDRESS;
use alloy_consensus::{
    Header,
    transaction::{SignerRecoverable, Transaction as _},
};
use alloy_primitives::{Address, B256, Bytes, U256, keccak256};
use alloy_rlp::{Encodable, Header as RlpHeader};
use alloy_sol_types::{SolCall, sol};
use futures::{StreamExt, future::try_join, stream};
use raiko2_primitives::{
    ChainSpec, ExecutionWitness, PreflightRpcClientConfig, ProofContext, ProofType, RaikoError,
    RaikoResult, StatelessInput, SupportedChainSpecs, WitnessStateNode,
    chain_spec::{ForkCondition, ForkId, GuestInputAbi, TaikoFork},
};
use raiko2_primitives_shasta::{
    GuestInput, roll_proposal_ancestor_headers_in_place, should_bypass_stalled_anchor_linkage,
    validate_anchor_progression,
};
use raiko2_protocol_shasta::shasta::{
    ParentBlockContext, ProposalMetadata, ShastaEventData,
    constants::{DERIVATION_SOURCE_MAX_BLOCKS, UNZEN_DERIVATION_SOURCE_MAX_BLOCKS},
    decode_proposal_id_from_extra_data,
    manifest::BlockManifest,
    prepare_source_manifest_with_max_blocks,
};
use raiko2_provider::{
    AccountProofWitnessNodes, AccountStateMaps, ParentStorageProofRequest, Provider,
    RpcClientConfig,
};
use raiko2_stateless::validate_block_with_witness_resources;
use std::{
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    time::{Duration, Instant},
};
use tracing::{info, warn};

sol! {
    #[derive(Debug)]
    struct AnchorV4Checkpoint {
        uint48 blockNumber;
        bytes32 blockHash;
        bytes32 stateRoot;
    }

    function anchorV4(AnchorV4Checkpoint _checkpoint) external;
}

const DEFAULT_PREFLIGHT_CHUNK_SIZE: usize = 8;
const DEFAULT_PREFLIGHT_CHUNK_CONCURRENCY: usize = 6;
const TAIKO_MAINNET_CHAIN_ID: u64 = 167_000;
const SIGNAL_SERVICE_CHECKPOINTS_SLOT: u64 = 254;
#[cfg(not(test))]
const PREFLIGHT_RETRY_INITIAL_DELAY: Duration = Duration::from_secs(1);
#[cfg(not(test))]
const PREFLIGHT_RETRY_MAX_DELAY: Duration = Duration::from_secs(30);

/// Shasta hardfork specification.
#[derive(Debug)]
pub struct ShastaSpec<Pr, Bk, Pv> {
    prover: Pr,
    backend: Bk,
    provider: Pv,
    manifest_builder: ShastaManifestBuilder,
    pipeline_key: PipelineKey,
}

impl<Pr, Bk, Pv> ShastaSpec<Pr, Bk, Pv> {
    /// Create a Shasta spec with the default manifest builder.
    pub const fn new(pipeline_key: PipelineKey, prover: Pr, backend: Bk, provider: Pv) -> Self {
        Self {
            prover,
            backend,
            provider,
            manifest_builder: ShastaManifestBuilder::new(),
            pipeline_key,
        }
    }

    /// Create a Shasta spec using the provided manifest builder.
    pub const fn with_manifest_builder(
        manifest_builder: ShastaManifestBuilder,
        pipeline_key: PipelineKey,
        prover: Pr,
        backend: Bk,
        provider: Pv,
    ) -> Self {
        Self {
            prover,
            backend,
            provider,
            manifest_builder,
            pipeline_key,
        }
    }
}

#[async_trait::async_trait]
impl<Pr, Bk, Pv> Preflight for ShastaSpec<Pr, Bk, Pv>
where
    Pr: Send + Sync,
    Bk: ProverBackend,
    Pv: Provider,
{
    type Input = GuestInput;

    async fn preflight<P: Provider>(
        &self,
        ctx: &ProofContext,
        provider: &P,
    ) -> RaikoResult<GuestInput> {
        let preflight_started_at = Instant::now();
        let proof_type = proof_type_from_context(ctx);
        let chain_spec = chain_spec_from_context(ctx)?;
        let (block_numbers, expected_proposal_id, proposal_event) =
            resolve_preflight_block_range_and_proposal_event(ctx, provider, &chain_spec).await?;
        let blocks = fetch_preflight_blocks(provider, &block_numbers).await?;
        let manifest =
            build_preflight_manifest(ctx, provider, &chain_spec, &blocks, proposal_event).await?;
        let parent_storage_proofs =
            stalled_anchor_parent_storage_requests_for_blocks(&chain_spec, &manifest, &blocks)?;
        let parent_block =
            fetch_preflight_parent_block_for_tx_lists(provider, &manifest, &blocks).await?;
        let tx_lists =
            derive_preflight_tx_lists(&chain_spec, &manifest, parent_block.as_ref(), &blocks)?;
        info!(
            proposal_id = ctx.request.proposal_id,
            block_count = block_numbers.len(),
            tx_list_count = tx_lists.as_ref().map(Vec::len).unwrap_or_default(),
            "derived shasta tx-list witness inputs"
        );
        let witnesses = fetch_preflight_witnesses(
            provider,
            &chain_spec,
            ctx.request.proposal_id,
            &blocks,
            tx_lists.as_deref(),
            &parent_storage_proofs,
        )
        .await?;
        info!(
            proposal_id = ctx.request.proposal_id,
            witness_count = witnesses.len(),
            first_witness_block = witnesses.first().map(|witness| witness.block.header.number),
            last_witness_block = witnesses.last().map(|witness| witness.block.header.number),
            "shasta tx-list witnesses ready"
        );
        validate_block_range(&witnesses, expected_proposal_id)?;
        let input = build_preflight_guest_input(manifest, witnesses, proof_type)?;
        info!(
            proposal_id = ctx.request.proposal_id,
            witness_count = input.witnesses.len(),
            proposal_ancestor_headers = input.proposal_ancestor_headers.len(),
            proposal_state_nodes = input.proposal_state_nodes.len(),
            "shasta guest input ready after tx-list preflight"
        );
        if let Some(verify_rpc) = ctx.preflight.verify_checkpoint_l2_rpc.as_deref() {
            let rpc_client_config = ctx
                .preflight
                .rpc_client_config
                .as_ref()
                .map(preflight_rpc_client_config)
                .unwrap_or_default();
            verify_guest_input_checkpoint_against_l2_rpc(&input, verify_rpc, &rpc_client_config)
                .await?;
        }

        info!(
            proposal_id = ctx.request.proposal_id,
            block_count = block_numbers.len(),
            elapsed_ms = preflight_started_at.elapsed().as_millis(),
            "completed shasta preflight"
        );
        Ok(input)
    }
}

const fn preflight_rpc_client_config(config: &PreflightRpcClientConfig) -> RpcClientConfig {
    RpcClientConfig {
        timeout_ms: config.timeout_ms,
        concurrency_limit: config.concurrency_limit,
        retry: raiko2_provider::RpcRetryConfig {
            max_attempts: config.retry.max_attempts,
            initial_backoff_ms: config.retry.initial_backoff_ms,
            compute_units_per_second: config.retry.compute_units_per_second,
        },
    }
}

async fn resolve_preflight_block_range_and_proposal_event<P: Provider>(
    ctx: &ProofContext,
    provider: &P,
    chain_spec: &ChainSpec,
) -> RaikoResult<(Vec<u64>, u64, ShastaEventData)> {
    let (block_numbers, expected_proposal_id) = extract_block_range(ctx, chain_spec)?;
    let first_block_no = block_numbers.first().copied().ok_or_else(|| {
        RaikoError::InvalidRequestConfig("request l2_block_range is empty".to_string())
    })?;
    let proposal_block = fetch_preflight_proposal_block(provider, first_block_no).await?;
    let proposal_event = resolve_shasta_proposal_event(
        ctx,
        provider,
        chain_spec,
        std::slice::from_ref(&proposal_block),
    )
    .await?;
    validate_derivation_source_block_limit(
        block_numbers.len(),
        first_block_no,
        proposal_event.proposal.timestamp.to::<u64>(),
        chain_spec,
    )?;
    Ok((block_numbers, expected_proposal_id, proposal_event))
}

async fn fetch_preflight_witnesses<P: Provider>(
    provider: &P,
    chain_spec: &ChainSpec,
    proposal_id: u64,
    blocks: &[reth_ethereum_primitives::Block],
    tx_lists: Option<&[Bytes]>,
    parent_storage_proofs: &[ParentStorageProofRequest],
) -> RaikoResult<Vec<StatelessInput>> {
    let block_numbers = blocks
        .iter()
        .map(|block| block.header.number)
        .collect::<Vec<_>>();
    let chunk_size = preflight_chunk_size();
    let chunk_concurrency = preflight_chunk_concurrency();
    info!(
        proposal_id,
        block_count = block_numbers.len(),
        chunk_size,
        chunk_concurrency,
        "starting shasta preflight"
    );
    if let Some(tx_lists) = tx_lists
        && tx_lists.len() != blocks.len()
    {
        return Err(RaikoError::Preflight(format!(
            "tx list count ({}) does not match block count ({})",
            tx_lists.len(),
            blocks.len()
        )));
    }
    let chunked_inputs = (0..blocks.len())
        .step_by(chunk_size)
        .enumerate()
        .map(|(chunk_index, start)| (chunk_index, start, (start + chunk_size).min(blocks.len())))
        .collect::<Vec<_>>();
    let chunk_count = chunked_inputs.len();
    let mut chunk_results: Vec<(usize, Vec<StatelessInput>)> =
        stream::iter(chunked_inputs.into_iter())
            .map(|(chunk_index, start, end)| {
                let chain_spec = chain_spec.clone();
                let chunk_blocks = &blocks[start..end];
                let chunk_tx_lists = tx_lists.map(|tx_lists| &tx_lists[start..end]);
                let chunk_parent_storage_proofs = if start == 0 {
                    parent_storage_proofs
                } else {
                    &[]
                };
                async move {
                    let chunk_block_numbers = chunk_blocks
                        .iter()
                        .map(|block| block.header.number)
                        .collect::<Vec<_>>();
                    let operation = preflight_chunk_operation(
                        proposal_id,
                        chunk_index,
                        chunk_count,
                        &chunk_block_numbers,
                        chunk_tx_lists.is_some(),
                    );
                    retry_shasta_preflight_operation(&operation, || {
                        let chain_spec = chain_spec.clone();
                        async {
                            fetch_preflight_chunk(
                                provider,
                                proposal_id,
                                chunk_index,
                                chunk_count,
                                chunk_blocks,
                                chunk_tx_lists,
                                chunk_parent_storage_proofs,
                                chain_spec,
                            )
                            .await
                        }
                    })
                    .await
                }
            })
            .buffer_unordered(chunk_concurrency)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<RaikoResult<Vec<_>>>()?;
    chunk_results.sort_by_key(|(chunk_index, _)| *chunk_index);
    Ok(chunk_results
        .into_iter()
        .flat_map(|(_, chunk)| chunk)
        .collect())
}

async fn fetch_preflight_blocks<P: Provider>(
    provider: &P,
    block_numbers: &[u64],
) -> RaikoResult<Vec<reth_ethereum_primitives::Block>> {
    retry_shasta_preflight_operation("fetch shasta preflight blocks", || async {
        let blocks = provider.batch_blocks(block_numbers).await?;
        validate_fetched_block_numbers(block_numbers, &blocks)?;
        Ok(blocks)
    })
    .await
}

async fn build_preflight_manifest<P: Provider>(
    ctx: &ProofContext,
    provider: &P,
    chain_spec: &ChainSpec,
    blocks: &[reth_ethereum_primitives::Block],
    proposal_event: ShastaEventData,
) -> RaikoResult<raiko2_protocol_shasta::TaikoManifest> {
    let mut manifest =
        ShastaManifestBuilder::taiko_manifest_with_event(ctx, blocks, proposal_event)?;
    hydrate_shasta_l1_headers(provider, chain_spec.chain_id, blocks, &mut manifest).await?;
    if manifest.data_sources.is_empty() && !manifest.proposal_event.proposal.sources.is_empty() {
        let l1_chain_spec = l1_chain_spec_from_context(ctx)?;
        manifest.data_sources =
            retry_shasta_preflight_operation("fetch shasta data sources", || async {
                provider
                    .shasta_data_sources(
                        &l1_chain_spec,
                        &manifest.proposal_event,
                        manifest.blob_proof_type,
                    )
                    .await
            })
            .await?;
    }
    Ok(manifest)
}

fn build_preflight_guest_input(
    manifest: raiko2_protocol_shasta::TaikoManifest,
    witnesses: Vec<StatelessInput>,
    proof_type: ProofType,
) -> RaikoResult<GuestInput> {
    let mut input = GuestInput {
        taiko: manifest,
        witnesses,
        proof_carry_data: raiko2_protocol_shasta::shasta::ProofCarryData::default(),
        proposal_ancestor_headers: Vec::new(),
        proposal_state_nodes: Vec::new(),
    };
    input.compact_proposal_witness_data();
    input.proof_carry_data = raiko2_primitives_shasta::build_proof_carry_data(&input, proof_type)?;
    Ok(input)
}

fn preflight_chunk_size() -> usize {
    std::env::var("PREFLIGHT_CHUNK_SIZE")
        .or_else(|_| std::env::var("PREFETCH_CHUNK_SIZE"))
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_PREFLIGHT_CHUNK_SIZE)
}

fn preflight_chunk_concurrency() -> usize {
    std::env::var("PREFLIGHT_CHUNK_CONCURRENCY")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_PREFLIGHT_CHUNK_CONCURRENCY)
}

fn preflight_chunk_operation(
    proposal_id: u64,
    chunk_index: usize,
    chunk_count: usize,
    block_numbers: &[u64],
    tx_list_witness: bool,
) -> String {
    let block_range = match (block_numbers.first(), block_numbers.last()) {
        (Some(first), Some(last)) => format!("{first}..{last}"),
        _ => "<empty>".to_string(),
    };
    format!(
        "shasta preflight chunk {chunk_index} proposal_id={proposal_id} chunk_count={chunk_count} blocks={block_range} block_count={} tx_list_witness={tx_list_witness}",
        block_numbers.len()
    )
}

const fn preflight_retry_initial_delay() -> Duration {
    #[cfg(test)]
    {
        Duration::from_millis(1)
    }
    #[cfg(not(test))]
    {
        PREFLIGHT_RETRY_INITIAL_DELAY
    }
}

const fn preflight_retry_max_delay() -> Duration {
    #[cfg(test)]
    {
        Duration::from_millis(2)
    }
    #[cfg(not(test))]
    {
        PREFLIGHT_RETRY_MAX_DELAY
    }
}

async fn retry_shasta_preflight_operation<T, F, Fut>(operation: &str, mut run: F) -> RaikoResult<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = RaikoResult<T>>,
{
    let mut attempt = 1_u64;
    let mut delay = preflight_retry_initial_delay();
    let max_delay = preflight_retry_max_delay();

    loop {
        match run().await {
            Ok(value) => {
                if attempt > 1 {
                    info!(
                        operation,
                        attempts = attempt,
                        "shasta preflight operation succeeded after retry"
                    );
                }
                return Ok(value);
            }
            Err(err) if retryable_shasta_preflight_error(&err) => {
                warn!(
                    operation,
                    attempt,
                    delay_ms = delay.as_millis(),
                    error = %err,
                    "retrying shasta preflight operation"
                );
                tokio::time::sleep(delay).await;
                attempt = attempt.saturating_add(1);
                delay = delay.saturating_mul(2).min(max_delay);
            }
            Err(err) => return Err(err),
        }
    }
}

const fn retryable_shasta_preflight_error(err: &RaikoError) -> bool {
    matches!(
        err,
        RaikoError::RPC(_)
            | RaikoError::RpcWithContext { .. }
            | RaikoError::Provider(_)
            | RaikoError::Io(_)
            | RaikoError::IoWithPath { .. }
    )
}

const fn preflight_uses_canonical_witness_for_tx_lists(chain_spec: &ChainSpec) -> bool {
    chain_spec.chain_id == TAIKO_MAINNET_CHAIN_ID
}

fn derived_tx_list_signers(tx_list: &Bytes) -> RaikoResult<Vec<Address>> {
    let transactions: Vec<reth_ethereum_primitives::TransactionSigned> =
        alloy_rlp::decode_exact(tx_list.as_ref()).map_err(|err| {
            RaikoError::Preflight(format!(
                "failed decode derived tx list for signer proofs: {err}"
            ))
        })?;

    Ok(transactions
        .iter()
        .skip(1)
        .filter_map(|tx| tx.recover_signer().ok())
        .collect())
}

fn collect_preflight_account_targets(tx_list: Option<&Bytes>) -> RaikoResult<Vec<Address>> {
    let mut targets = vec![Address::from(TAIKO_GOLDEN_TOUCH_ADDRESS)];
    if let Some(tx_list) = tx_list {
        targets.extend(derived_tx_list_signers(tx_list)?);
    }
    targets.sort_unstable();
    targets.dedup();
    Ok(targets)
}

async fn fetch_preflight_accounts<P: Provider>(
    provider: &P,
    block_numbers: &[u64],
    account_targets: &[Vec<Address>],
    use_canonical_witness: bool,
) -> RaikoResult<(AccountStateMaps, AccountProofWitnessNodes, u128)> {
    let started_at = Instant::now();
    let (accounts, account_witness_nodes) = if use_canonical_witness {
        provider
            .batch_accounts_with_proof_witnesses(block_numbers, account_targets)
            .await?
    } else {
        let accounts = provider
            .batch_accounts(block_numbers, account_targets)
            .await?;
        (accounts, vec![Vec::new(); block_numbers.len()])
    };
    Ok((
        accounts,
        account_witness_nodes,
        started_at.elapsed().as_millis(),
    ))
}

fn merge_account_witness_nodes(
    witnesses: &mut [ExecutionWitness],
    account_witness_nodes: Vec<Vec<WitnessStateNode>>,
) {
    for (witness, nodes) in witnesses.iter_mut().zip(account_witness_nodes) {
        if nodes.is_empty() {
            continue;
        }
        witness.state.extend(nodes);
        witness.state =
            ExecutionWitness::canonicalize_state_nodes(std::mem::take(&mut witness.state));
    }
}

async fn fetch_preflight_chunk<P: Provider>(
    provider: &P,
    proposal_id: u64,
    chunk_index: usize,
    chunk_count: usize,
    blocks: &[reth_ethereum_primitives::Block],
    tx_lists: Option<&[Bytes]>,
    parent_storage_proofs: &[ParentStorageProofRequest],
    chain_spec: ChainSpec,
) -> RaikoResult<(usize, Vec<StatelessInput>)> {
    let block_numbers = blocks
        .iter()
        .map(|block| block.header.number)
        .collect::<Vec<_>>();
    let chunk_started_at = Instant::now();
    info!(
        proposal_id,
        chunk_index,
        chunk_count,
        first_block = block_numbers.first().copied(),
        last_block = block_numbers.last().copied(),
        block_count = block_numbers.len(),
        tx_list_witness = tx_lists.is_some(),
        parent_storage_proof_count = parent_storage_proofs.len(),
        "starting shasta preflight chunk"
    );
    if let Some(tx_lists) = tx_lists
        && tx_lists.len() != blocks.len()
    {
        return Err(RaikoError::Preflight(format!(
            "tx-list witness count ({}) does not match block count ({})",
            tx_lists.len(),
            blocks.len()
        )));
    }
    let use_canonical_witness =
        tx_lists.is_some() && preflight_uses_canonical_witness_for_tx_lists(&chain_spec);
    let witnesses = async {
        let started_at = Instant::now();
        let witnesses = if let Some(tx_lists) = tx_lists
            && !use_canonical_witness
        {
            if parent_storage_proofs.is_empty() {
                provider
                    .batch_witnesses_with_tx_lists(&block_numbers, tx_lists)
                    .await
            } else {
                provider
                    .batch_witnesses_with_tx_lists_and_parent_storage_proofs(
                        &block_numbers,
                        tx_lists,
                        parent_storage_proofs,
                    )
                    .await
            }
        } else if parent_storage_proofs.is_empty() {
            provider.batch_witnesses(&block_numbers).await
        } else {
            provider
                .batch_witnesses_with_parent_storage_proofs(&block_numbers, parent_storage_proofs)
                .await
        }?;
        Ok::<_, RaikoError>((witnesses, started_at.elapsed().as_millis()))
    };
    let account_targets = blocks
        .iter()
        .enumerate()
        .map(|(index, _)| {
            collect_preflight_account_targets(tx_lists.map(|tx_lists| &tx_lists[index]))
        })
        .collect::<RaikoResult<Vec<_>>>()?;
    let accounts = fetch_preflight_accounts(
        provider,
        &block_numbers,
        &account_targets,
        use_canonical_witness,
    );
    let (
        (mut witnesses, witnesses_elapsed_ms),
        (accounts, account_witness_nodes, accounts_elapsed_ms),
    ) = try_join(witnesses, accounts).await?;

    if blocks.len() != witnesses.len()
        || blocks.len() != accounts.len()
        || blocks.len() != account_witness_nodes.len()
    {
        return Err(RaikoError::InvalidRequestConfig(
            "Provider returned mismatched input lengths".to_string(),
        ));
    }

    if use_canonical_witness {
        merge_account_witness_nodes(&mut witnesses, account_witness_nodes);
    }

    let witnesses = blocks
        .iter()
        .cloned()
        .zip(witnesses)
        .zip(accounts)
        .map(|((block, witness), accounts)| StatelessInput {
            block,
            chain_spec: chain_spec.clone(),
            witness,
            accounts,
        })
        .collect::<Vec<_>>();

    info!(
        proposal_id,
        chunk_index,
        chunk_count,
        first_block = block_numbers.first().copied(),
        last_block = block_numbers.last().copied(),
        block_count = block_numbers.len(),
        tx_list_witness = tx_lists.is_some(),
        witnesses_elapsed_ms,
        accounts_elapsed_ms,
        total_elapsed_ms = chunk_started_at.elapsed().as_millis(),
        "completed shasta preflight chunk"
    );
    Ok((chunk_index, witnesses))
}

fn derive_preflight_tx_lists(
    chain_spec: &ChainSpec,
    manifest: &raiko2_protocol_shasta::TaikoManifest,
    parent_block: Option<&reth_ethereum_primitives::Block>,
    blocks: &[reth_ethereum_primitives::Block],
) -> RaikoResult<Option<Vec<Bytes>>> {
    let sources = &manifest.proposal_event.proposal.sources;
    if sources.is_empty() {
        return Ok(None);
    }
    if sources.len() != manifest.data_sources.len() {
        return Err(RaikoError::Preflight(format!(
            "data source count ({}) does not match proposal source count ({})",
            manifest.data_sources.len(),
            sources.len()
        )));
    }

    let first_block = blocks.first().ok_or_else(|| {
        RaikoError::Preflight("cannot derive Shasta tx lists without blocks".to_string())
    })?;
    let proposal_timestamp = manifest.proposal_event.proposal.timestamp.to::<u64>();
    let max_blocks = derivation_source_max_blocks_for_chain_spec_at(
        chain_spec,
        first_block.header.number,
        proposal_timestamp,
    );
    let fork_timestamp = shasta_fork_timestamp_for_chain_spec(chain_spec)?;
    let mut parent = preflight_parent_context(
        parent_block.ok_or_else(|| {
            RaikoError::Preflight("cannot derive Shasta tx lists without parent block".to_string())
        })?,
        manifest,
    );
    let meta = ProposalMetadata {
        proposal_timestamp,
        origin_block_number: manifest
            .proposal_event
            .proposal
            .originBlockNumber
            .to::<u64>(),
        proposer: manifest.proposal_event.proposal.proposer,
        chain_id: chain_spec.chain_id,
    };

    let mut manifest_blocks = Vec::new();
    for (source_index, source) in sources.iter().enumerate() {
        let source_manifest = prepare_source_manifest_with_max_blocks(
            source,
            manifest.data_sources.get(source_index),
            parent,
            meta,
            fork_timestamp,
            max_blocks,
        )
        .map_err(|err| {
            RaikoError::Preflight(format!(
                "failed to decode Shasta tx-list source {source_index}: {err}"
            ))
        })?;
        for block in source_manifest.blocks {
            parent = ParentBlockContext {
                timestamp: block.timestamp,
                gas_limit: block.gas_limit.saturating_add(ANCHOR_V3_V4_GAS_LIMIT),
                block_number: parent.block_number + 1,
                anchor_block_number: block.anchor_block_number,
            };
            manifest_blocks.push(block);
        }
    }

    if manifest_blocks.len() != blocks.len() {
        return Err(RaikoError::Preflight(format!(
            "derived tx-list block count ({}) does not match fetched block count ({})",
            manifest_blocks.len(),
            blocks.len()
        )));
    }

    blocks
        .iter()
        .zip(manifest_blocks.iter())
        .map(|(block, manifest_block)| encode_replay_tx_list(block, manifest_block))
        .collect::<RaikoResult<Vec<_>>>()
        .map(Some)
}

async fn fetch_preflight_parent_block_for_tx_lists<P: Provider>(
    provider: &P,
    manifest: &raiko2_protocol_shasta::TaikoManifest,
    blocks: &[reth_ethereum_primitives::Block],
) -> RaikoResult<Option<reth_ethereum_primitives::Block>> {
    if manifest.proposal_event.proposal.sources.is_empty() {
        return Ok(None);
    }
    let first_block = blocks.first().ok_or_else(|| {
        RaikoError::Preflight("cannot fetch Shasta tx-list parent without blocks".to_string())
    })?;
    let parent_block_number = first_block.header.number.checked_sub(1).ok_or_else(|| {
        RaikoError::Preflight("cannot derive Shasta tx-list parent for block 0".to_string())
    })?;
    let parent_blocks =
        retry_shasta_preflight_operation("fetch shasta tx-list parent block", || async {
            let requested = [parent_block_number];
            let parent_blocks = provider.batch_blocks(&requested).await?;
            validate_fetched_block_numbers(&requested, &parent_blocks)?;
            Ok(parent_blocks)
        })
        .await?;
    parent_blocks.into_iter().next().map_or_else(
        || {
            Err(RaikoError::Preflight(format!(
                "provider returned no Shasta tx-list parent block {parent_block_number}"
            )))
        },
        |block| Ok(Some(block)),
    )
}

fn shasta_fork_timestamp_for_chain_spec(chain_spec: &ChainSpec) -> RaikoResult<u64> {
    match chain_spec.hard_forks.get(&ForkId::Taiko(TaikoFork::Shasta)) {
        Some(ForkCondition::Timestamp(timestamp)) => Ok(*timestamp),
        Some(other) => Err(RaikoError::InvalidRequestConfig(format!(
            "unsupported Shasta fork condition for chain {}: {other:?}",
            chain_spec.name
        ))),
        None => Ok(0),
    }
}

fn preflight_parent_context(
    parent_block: &reth_ethereum_primitives::Block,
    manifest: &raiko2_protocol_shasta::TaikoManifest,
) -> ParentBlockContext {
    ParentBlockContext {
        timestamp: parent_block.header.timestamp,
        gas_limit: parent_block.header.gas_limit,
        block_number: parent_block.header.number,
        anchor_block_number: manifest
            .prover_data
            .last_anchor_block_number
            .unwrap_or_default(),
    }
}

fn encode_replay_tx_list(
    block: &reth_ethereum_primitives::Block,
    manifest_block: &BlockManifest,
) -> RaikoResult<Bytes> {
    let anchor_tx = block.body.transactions().next().ok_or_else(|| {
        RaikoError::Preflight(format!(
            "cannot build tx-list witness input: block {} has no anchor transaction",
            block.header.number
        ))
    })?;
    let mut encoded_txs = Vec::with_capacity(manifest_block.transactions.len() + 1);
    encoded_txs.push(encode_tx_for_rlp_list(anchor_tx));
    encoded_txs.extend(
        manifest_block
            .transactions
            .iter()
            .map(encode_tx_for_rlp_list),
    );

    let payload_length = encoded_txs.iter().map(Vec::len).sum();
    let mut out = Vec::with_capacity(payload_length);
    RlpHeader {
        list: true,
        payload_length,
    }
    .encode(&mut out);
    for tx in encoded_txs {
        out.extend_from_slice(&tx);
    }
    Ok(out.into())
}

fn encode_tx_for_rlp_list(tx: &impl Encodable) -> Vec<u8> {
    let mut out = Vec::with_capacity(tx.length());
    tx.encode(&mut out);
    out
}

async fn fetch_preflight_proposal_block<P: Provider>(
    provider: &P,
    block_number: u64,
) -> RaikoResult<reth_ethereum_primitives::Block> {
    retry_shasta_preflight_operation("fetch shasta proposal block", || async {
        let requested = [block_number];
        let mut blocks = provider.batch_blocks(&requested).await?;
        validate_fetched_block_numbers(&requested, &blocks)?;
        blocks.pop().ok_or_else(|| {
            RaikoError::Preflight(format!(
                "provider returned no block for requested proposal block {block_number}"
            ))
        })
    })
    .await
}

fn validate_fetched_block_numbers(
    expected_block_numbers: &[u64],
    blocks: &[reth_ethereum_primitives::Block],
) -> RaikoResult<()> {
    if blocks.len() != expected_block_numbers.len() {
        return Err(RaikoError::InvalidRequestConfig(format!(
            "provider returned {} blocks for {} requested block numbers",
            blocks.len(),
            expected_block_numbers.len()
        )));
    }
    for (index, (expected, block)) in expected_block_numbers.iter().zip(blocks).enumerate() {
        if block.header.number != *expected {
            return Err(RaikoError::Preflight(format!(
                "provider returned block {} for requested block {} at chunk index {index}",
                block.header.number, expected
            )));
        }
    }
    Ok(())
}

fn chain_spec_from_context(ctx: &ProofContext) -> RaikoResult<ChainSpec> {
    let guest_input_abi = guest_input_abi_from_context(ctx)?;
    let chain_spec = SupportedChainSpecs::default()
        .get_chain_spec_with_chain_id(ctx.request.l2_chain_id)
        .unwrap_or_else(|| ChainSpec {
            name: "unknown".to_string(),
            chain_id: ctx.request.l2_chain_id,
            ..Default::default()
        });
    Ok(chain_spec.project_for_guest_input_abi(guest_input_abi))
}

fn guest_input_abi_from_context(ctx: &ProofContext) -> RaikoResult<GuestInputAbi> {
    let Some(value) = ctx.config.get("guest_input_abi") else {
        return Ok(GuestInputAbi::default());
    };
    if value.is_null() {
        return Ok(GuestInputAbi::default());
    }
    serde_json::from_value(value.clone()).map_err(|err| {
        RaikoError::InvalidRequestConfig(format!("invalid prover.guest_input_abi: {err}"))
    })
}

fn l1_chain_spec_from_context(ctx: &ProofContext) -> RaikoResult<ChainSpec> {
    if let Some(chain_spec) = &ctx.preflight.resolved_l1_chain_spec {
        if chain_spec.chain_id != ctx.request.l1_chain_id {
            return Err(RaikoError::InvalidRequestConfig(format!(
                "preflight l1 chain spec chain_id {} does not match request l1_chain_id {}",
                chain_spec.chain_id, ctx.request.l1_chain_id
            )));
        }
        return Ok(chain_spec.clone());
    }

    SupportedChainSpecs::default()
        .get_chain_spec_with_chain_id(ctx.request.l1_chain_id)
        .ok_or_else(|| {
            RaikoError::InvalidRequestConfig(format!(
                "unsupported l1_chain_id {} for Shasta preflight",
                ctx.request.l1_chain_id
            ))
        })
}

async fn resolve_shasta_proposal_event<P: Provider>(
    ctx: &ProofContext,
    provider: &P,
    chain_spec: &ChainSpec,
    blocks: &[reth_ethereum_primitives::Block],
) -> RaikoResult<ShastaEventData> {
    let l1_inclusion_block_number = ctx
        .request
        .shasta
        .map(|shasta| shasta.l1_inclusion_block_number)
        .ok_or_else(|| {
            RaikoError::InvalidRequestConfig(
                "request.shasta.l1_inclusion_block_number is required for Shasta preflight"
                    .to_string(),
            )
        })?;
    let proposal_block = blocks.first().ok_or_else(|| {
        RaikoError::Preflight("cannot resolve Shasta proposal event without blocks".to_string())
    })?;
    let l1_contract = chain_spec
        .get_fork_l1_contract_address_at(
            proposal_block.header.number,
            proposal_block.header.timestamp,
        )
        .map_err(|e| {
            RaikoError::InvalidRequestConfig(format!(
                "failed to resolve Shasta L1 contract address for block {} timestamp {}: {e}",
                proposal_block.header.number, proposal_block.header.timestamp
            ))
        })?;
    retry_shasta_preflight_operation("fetch shasta proposal event", || async {
        provider
            .shasta_proposal_event(
                l1_contract,
                l1_inclusion_block_number,
                ctx.request.proposal_id,
            )
            .await
    })
    .await
}

const fn proof_type_from_context(ctx: &ProofContext) -> ProofType {
    ctx.request.proof_type
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct AnchorCheckpoint {
    block_number: u64,
    block_hash: B256,
    state_root: B256,
}

fn decode_anchor_checkpoint(
    block: &reth_ethereum_primitives::Block,
) -> RaikoResult<AnchorCheckpoint> {
    let anchor_tx = block.body.transactions().next().ok_or_else(|| {
        RaikoError::Preflight(format!(
            "missing anchor transaction in block {}",
            block.header.number
        ))
    })?;
    let input = anchor_tx.input();
    if !input.starts_with(&anchorV4Call::SELECTOR) {
        return Err(RaikoError::Preflight(format!(
            "block {} first transaction is not anchorV4",
            block.header.number
        )));
    }

    let decoded = anchorV4Call::abi_decode(input).map_err(|err| {
        RaikoError::Preflight(format!(
            "failed to decode anchorV4 calldata for block {}: {err}",
            block.header.number
        ))
    })?;

    Ok(AnchorCheckpoint {
        block_number: decoded._checkpoint.blockNumber.to::<u64>(),
        block_hash: decoded._checkpoint.blockHash,
        state_root: decoded._checkpoint.stateRoot,
    })
}

fn signal_service_checkpoint_entry_slot(block_number: u64) -> B256 {
    let mut encoded = [0u8; 64];
    encoded[..32].copy_from_slice(&U256::from(block_number).to_be_bytes::<32>());
    encoded[32..].copy_from_slice(&U256::from(SIGNAL_SERVICE_CHECKPOINTS_SLOT).to_be_bytes::<32>());
    keccak256(encoded)
}

fn signal_service_checkpoint_storage_keys(block_number: u64) -> Vec<B256> {
    let entry_slot = signal_service_checkpoint_entry_slot(block_number);
    let state_root_slot = U256::from_be_slice(entry_slot.as_slice()).wrapping_add(U256::from(1));
    vec![entry_slot, B256::from(state_root_slot.to_be_bytes::<32>())]
}

fn stalled_anchor_parent_storage_requests(
    chain_spec: &ChainSpec,
    parent_anchor_block_number: u64,
) -> RaikoResult<Vec<ParentStorageProofRequest>> {
    let signal_service = chain_spec.l2_signal_service.ok_or_else(|| {
        RaikoError::InvalidRequestConfig(
            "chain_spec.l2_signal_service is required for stalled anchor checkpoint proof"
                .to_string(),
        )
    })?;

    Ok(vec![ParentStorageProofRequest {
        block_index: 0,
        address: signal_service,
        storage_keys: signal_service_checkpoint_storage_keys(parent_anchor_block_number),
    }])
}

fn stalled_anchor_parent_storage_requests_for_blocks(
    chain_spec: &ChainSpec,
    manifest: &raiko2_protocol_shasta::TaikoManifest,
    blocks: &[reth_ethereum_primitives::Block],
) -> RaikoResult<Vec<ParentStorageProofRequest>> {
    let anchor_checkpoints = blocks
        .iter()
        .map(decode_anchor_checkpoint)
        .collect::<RaikoResult<Vec<_>>>()?;
    let anchor_block_numbers = anchor_checkpoints
        .iter()
        .map(|checkpoint| checkpoint.block_number)
        .collect::<Vec<_>>();
    let last_anchor_block_number = manifest
        .prover_data
        .last_anchor_block_number
        .unwrap_or_default();
    let origin_block_number = manifest
        .proposal_event
        .proposal
        .originBlockNumber
        .to::<u64>();

    if should_bypass_stalled_anchor_linkage(
        &anchor_block_numbers,
        last_anchor_block_number,
        origin_block_number,
        chain_spec.chain_id,
    ) {
        return stalled_anchor_parent_storage_requests(chain_spec, last_anchor_block_number);
    }

    Ok(Vec::new())
}

async fn hydrate_shasta_l1_headers<P: Provider>(
    provider: &P,
    chain_id: u64,
    blocks: &[reth_ethereum_primitives::Block],
    manifest: &mut raiko2_protocol_shasta::TaikoManifest,
) -> RaikoResult<()> {
    let proposal = &manifest.proposal_event.proposal;
    let origin_block_number = proposal.originBlockNumber.to::<u64>();
    if proposal.originBlockHash == B256::ZERO {
        return Err(RaikoError::InvalidRequestConfig(
            "shasta_proposal_event.proposal.originBlockHash is required".to_string(),
        ));
    }

    let anchor_checkpoints = blocks
        .iter()
        .map(decode_anchor_checkpoint)
        .collect::<RaikoResult<Vec<_>>>()?;
    let anchor_block_numbers = anchor_checkpoints
        .iter()
        .map(|checkpoint| checkpoint.block_number)
        .collect::<Vec<_>>();
    let last_anchor_block_number = manifest
        .prover_data
        .last_anchor_block_number
        .unwrap_or_default();
    if should_bypass_stalled_anchor_linkage(
        &anchor_block_numbers,
        last_anchor_block_number,
        origin_block_number,
        chain_id,
    ) {
        let l1_headers =
            retry_shasta_preflight_operation("fetch shasta origin l1 header", || async {
                provider.batch_l1_headers(&[origin_block_number]).await
            })
            .await?;
        let origin_header = l1_headers.into_iter().next().ok_or_else(|| {
            RaikoError::Preflight("provider returned no Shasta origin L1 header".to_string())
        })?;
        if origin_header.number != origin_block_number {
            return Err(RaikoError::Preflight(format!(
                "proposal origin block number mismatch: expected {origin_block_number}, got {}",
                origin_header.number
            )));
        }
        if origin_header.hash_slow() != proposal.originBlockHash {
            return Err(RaikoError::Preflight(format!(
                "proposal origin block hash mismatch: expected {:?}, got {:?}",
                proposal.originBlockHash,
                origin_header.hash_slow()
            )));
        }

        manifest.l1_header = origin_header;
        manifest.l1_ancestor_headers.clear();
        return Ok(());
    }

    validate_anchor_progression(
        &anchor_block_numbers,
        last_anchor_block_number,
        origin_block_number,
        chain_id,
    )
    .map_err(RaikoError::Preflight)?;
    let min_anchor_block_number = anchor_block_numbers.iter().copied().min().ok_or_else(|| {
        RaikoError::Preflight("cannot derive Shasta anchor checkpoints".to_string())
    })?;
    let first_required_header_block_number = min_anchor_block_number.max(last_anchor_block_number);
    let l1_block_numbers =
        (first_required_header_block_number..=origin_block_number).collect::<Vec<_>>();
    let l1_headers = retry_shasta_preflight_operation("fetch shasta l1 headers", || async {
        provider.batch_l1_headers(&l1_block_numbers).await
    })
    .await?;
    if l1_headers.len() != l1_block_numbers.len() {
        return Err(RaikoError::Preflight(format!(
            "provider returned {} L1 headers for {} requested block numbers",
            l1_headers.len(),
            l1_block_numbers.len()
        )));
    }

    validate_l1_headers(
        &l1_headers,
        &l1_block_numbers,
        &anchor_checkpoints,
        proposal.originBlockHash,
    )?;
    let origin_header = l1_headers
        .last()
        .cloned()
        .ok_or_else(|| RaikoError::Preflight("missing Shasta origin L1 header".to_string()))?;
    if origin_header.number != origin_block_number {
        return Err(RaikoError::Preflight(format!(
            "proposal origin block number mismatch: expected {origin_block_number}, got {}",
            origin_header.number
        )));
    }

    manifest.l1_header = origin_header;
    manifest.l1_ancestor_headers = l1_headers;
    Ok(())
}

fn validate_l1_headers(
    headers: &[Header],
    expected_numbers: &[u64],
    anchor_checkpoints: &[AnchorCheckpoint],
    expected_origin_hash: B256,
) -> RaikoResult<()> {
    if headers.is_empty() {
        return Err(RaikoError::Preflight(
            "no L1 headers returned for Shasta linkage".to_string(),
        ));
    }

    let mut checkpoint_index = 0usize;
    let mut previous_hash = None;
    let mut last_hash = B256::ZERO;

    for (index, (header, expected_number)) in headers.iter().zip(expected_numbers).enumerate() {
        if header.number != *expected_number {
            return Err(RaikoError::Preflight(format!(
                "L1 header {index} number mismatch: expected {expected_number}, got {}",
                header.number
            )));
        }
        let header_hash = header.hash_slow();
        if let Some(previous_hash) = previous_hash
            && header.parent_hash != previous_hash
        {
            return Err(RaikoError::Preflight(format!(
                "L1 header chain broken at {} -> {}",
                expected_numbers[index - 1],
                header.number
            )));
        }

        loop {
            let Some(checkpoint) = anchor_checkpoints.get(checkpoint_index) else {
                break;
            };
            if checkpoint.block_number != header.number {
                break;
            }

            if checkpoint.block_hash != header_hash || checkpoint.state_root != header.state_root {
                return Err(RaikoError::Preflight(format!(
                    "anchor checkpoint ({}, {:?}, {:?}) not found in fetched L1 header chain",
                    checkpoint.block_number, checkpoint.block_hash, checkpoint.state_root
                )));
            }
            checkpoint_index += 1;
        }

        previous_hash = Some(header_hash);
        last_hash = header_hash;
    }

    if let Some(checkpoint) = anchor_checkpoints.get(checkpoint_index) {
        return Err(RaikoError::Preflight(format!(
            "anchor checkpoint ({}, {:?}, {:?}) not found in fetched L1 header chain",
            checkpoint.block_number, checkpoint.block_hash, checkpoint.state_root
        )));
    }

    if last_hash != expected_origin_hash {
        return Err(RaikoError::Preflight(format!(
            "proposal origin block hash mismatch: expected {expected_origin_hash:?}, got {last_hash:?}"
        )));
    }

    Ok(())
}

fn derivation_source_max_blocks_for_chain_spec_at(
    chain_spec: &ChainSpec,
    block_no: u64,
    proposal_timestamp: u64,
) -> usize {
    if chain_spec
        .hard_forks
        .get(&ForkId::Taiko(TaikoFork::Unzen))
        .is_some_and(|fork| fork.active(block_no, proposal_timestamp))
    {
        UNZEN_DERIVATION_SOURCE_MAX_BLOCKS
    } else {
        DERIVATION_SOURCE_MAX_BLOCKS
    }
}

fn validate_derivation_source_block_limit(
    block_count: usize,
    first_block_no: u64,
    proposal_timestamp: u64,
    chain_spec: &ChainSpec,
) -> RaikoResult<()> {
    let max_blocks = derivation_source_max_blocks_for_chain_spec_at(
        chain_spec,
        first_block_no,
        proposal_timestamp,
    );
    if block_count > max_blocks {
        return Err(RaikoError::InvalidRequestConfig(format!(
            "request l2_block_range contains {block_count} blocks, max {max_blocks}"
        )));
    }
    Ok(())
}

fn possible_derivation_source_max_blocks_for_chain_spec(chain_spec: &ChainSpec) -> usize {
    if matches!(
        chain_spec.hard_forks.get(&ForkId::Taiko(TaikoFork::Unzen)),
        Some(ForkCondition::Block(_) | ForkCondition::Timestamp(_))
    ) {
        UNZEN_DERIVATION_SOURCE_MAX_BLOCKS
    } else {
        DERIVATION_SOURCE_MAX_BLOCKS
    }
}

fn extract_block_range(ctx: &ProofContext, chain_spec: &ChainSpec) -> RaikoResult<(Vec<u64>, u64)> {
    if let Some(range) = ctx.request.l2_block_range {
        if !range.is_valid() {
            return Err(RaikoError::InvalidRequestConfig(
                "request l2_block_range.start must be <= end".into(),
            ));
        }
        let block_count = range
            .end
            .checked_sub(range.start)
            .and_then(|count| count.checked_add(1))
            .ok_or_else(|| {
                RaikoError::InvalidRequestConfig(
                    "request l2_block_range contains too many blocks".into(),
                )
            })?;
        let max_blocks = u64::try_from(possible_derivation_source_max_blocks_for_chain_spec(
            chain_spec,
        ))
        .expect("derivation source max blocks fits u64");
        if block_count > max_blocks {
            return Err(RaikoError::InvalidRequestConfig(format!(
                "request l2_block_range contains {block_count} blocks, max {max_blocks}"
            )));
        }
        return Ok(((range.start..=range.end).collect(), ctx.request.proposal_id));
    }

    Err(RaikoError::InvalidRequestConfig(
        "request l2_block_range is required for Shasta preflight".into(),
    ))
}

fn validate_block_range(
    witnesses: &[StatelessInput],
    expected_proposal_id: u64,
) -> RaikoResult<()> {
    if witnesses.is_empty() {
        return Err(RaikoError::Preflight(
            "GuestInput has no witnesses to validate".to_string(),
        ));
    }
    for (idx, w) in witnesses.iter().enumerate() {
        let extradata = &w.block.header.extra_data;
        let pid = decode_proposal_id_from_extra_data(extradata).ok_or_else(|| {
            RaikoError::Preflight(format!(
                "witness {idx} extradata too short ({})",
                extradata.len()
            ))
        })?;
        if pid != expected_proposal_id {
            return Err(RaikoError::Preflight(format!(
                "witness {idx} proposal_id mismatch: expected {expected_proposal_id}, got {pid}"
            )));
        }
    }

    // Blocks must be contiguous.
    for pair in witnesses.windows(2).enumerate() {
        let (idx, window) = pair;
        let prev = window[0].block.header.number;
        let next = window[1].block.header.number;
        if next != prev + 1 {
            return Err(RaikoError::Preflight(format!(
                "witness blocks not contiguous at index {idx}: {prev} -> {next}"
            )));
        }
    }

    Ok(())
}

/// Validates a Shasta guest input with the same stateless checks used by preflight.
///
/// # Errors
///
/// Returns an error when the input is empty, has inconsistent witness chain specs, is missing
/// proposal ancestor headers, or any block fails stateless validation.
pub fn validate_shasta_guest_input(input: &GuestInput) -> RaikoResult<()> {
    let Some(first_input) = input.witnesses.first() else {
        return Err(RaikoError::Preflight(
            "GuestInput has no witnesses to validate".to_string(),
        ));
    };

    let chain_spec = &first_input.chain_spec;
    let taiko_chain_spec = chain_spec
        .to_taiko_chain_spec()
        .map_err(|e| RaikoError::Preflight(e.to_string()))?;
    let evm_config = TaikoEvmConfig::new(taiko_chain_spec.clone());
    let mut ancestor_headers = input.initial_proposal_ancestor_headers();
    if ancestor_headers.is_empty() {
        return Err(RaikoError::Preflight(
            "GuestInput is missing proposal ancestor headers".to_string(),
        ));
    }

    for (index, stateless_input) in input.witnesses.iter().enumerate() {
        if stateless_input.chain_spec.chain_id != chain_spec.chain_id {
            return Err(RaikoError::Preflight(format!(
                "witness {index} chain_id mismatch: expected {}, got {}",
                chain_spec.chain_id, stateless_input.chain_spec.chain_id
            )));
        }

        if stateless_input.chain_spec.is_taiko != chain_spec.is_taiko {
            return Err(RaikoError::Preflight(format!(
                "witness {index} is_taiko mismatch: expected {}, got {}",
                chain_spec.is_taiko, stateless_input.chain_spec.is_taiko
            )));
        }

        let block_number = stateless_input.block.header.number;
        let validated_hash = catch_unwind(AssertUnwindSafe(|| {
            validate_block_with_witness_resources(
                stateless_input.block.clone(),
                &stateless_input.witness,
                &ancestor_headers,
                input.proposal_state_nodes(),
                stateless_input.accounts.clone(),
                &taiko_chain_spec,
                &evm_config,
            )
        }))
        .map_err(|panic| {
            let reason = if let Some(message) = panic.downcast_ref::<&str>() {
                (*message).to_string()
            } else if let Some(message) = panic.downcast_ref::<String>() {
                message.clone()
            } else {
                "unknown panic".to_string()
            };
            RaikoError::stateless_validation_detailed(
                format!(
                    "validation panicked at witness index {index}, block {block_number}: {reason}"
                ),
                Some(block_number),
            )
        })??;
        roll_proposal_ancestor_headers_in_place(
            &mut ancestor_headers,
            &stateless_input.block.header,
            validated_hash,
        );
    }

    Ok(())
}

impl<Pr, Bk, Pv> Validation for ShastaSpec<Pr, Bk, Pv>
where
    Pr: Send + Sync,
    Bk: ProverBackend,
    Pv: Provider,
{
    type Input = GuestInput;

    fn validate(&self, _ctx: &ProofContext, input: &GuestInput) -> RaikoResult<()> {
        validate_shasta_guest_input(input)
    }
}

impl<Pr, Bk, Pv> PipelineSpec for ShastaSpec<Pr, Bk, Pv>
where
    Pr: Send + Sync,
    Bk: ProverBackend,
    Pv: Provider,
{
    type GuestInput = GuestInput;
    type Preflight = Self;
    type Validation = Self;
    type ManifestBuilder = ShastaManifestBuilder;
    type Prover = Pr;
    type Backend = Bk;
    type Provider = Pv;

    fn pipeline_key(&self) -> PipelineKey {
        self.pipeline_key
    }

    fn prover(&self) -> &Self::Prover {
        &self.prover
    }

    fn backend(&self) -> &Self::Backend {
        &self.backend
    }

    fn provider(&self) -> &Self::Provider {
        &self.provider
    }

    fn preflight(&self) -> &Self::Preflight {
        self
    }

    fn validation(&self) -> &Self::Validation {
        self
    }

    fn manifest_builder(&self) -> &Self::ManifestBuilder {
        &self.manifest_builder
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AnchorV4Checkpoint, Preflight, ShastaSpec, TAIKO_GOLDEN_TOUCH_ADDRESS, anchorV4Call,
    };
    use alethia_reth_chainspec::{
        TAIKO_MAINNET,
        hardfork::{TaikoHardfork as AlethiaTaikoHardfork, TaikoHardforks as _},
    };
    use alloy_consensus::{Header, SignableTransaction, TxEip1559};
    use alloy_eips::eip4844::BYTES_PER_BLOB;
    use alloy_hardforks::ForkCondition as AlethiaForkCondition;
    use alloy_primitives::{Address, B256, Bytes, Signature, TxKind, U256, map::AddressMap};
    use alloy_sol_types::SolCall;
    use alloy_trie::TrieAccount;
    use raiko2_primitives::{
        ChainSpec, ExecutionWitness, L2BlockRange, ProofContext, ProofRequest, ProofType,
        ProverConfig, RaikoError, RaikoResult, ShastaRequest, SupportedChainSpecs,
        WitnessStateNode,
        chain_spec::{ForkCondition, ForkId, TaikoFork},
    };
    use raiko2_protocol::{BlobProofType, InputDataSource};
    use raiko2_protocol_shasta::shasta::{
        BlobSlice, DerivationSource, ShastaEventData,
        manifest::{BlockManifest, DerivationSourceManifest},
    };
    use raiko2_provider::{ParentStorageProofRequest, Provider};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use crate::{NativeBackend, PipelineKey};

    #[derive(Clone)]
    struct TestProvider {
        block: reth_ethereum_primitives::Block,
        parent_block: reth_ethereum_primitives::Block,
        proposal_event: ShastaEventData,
        l1_headers: Vec<Header>,
        data_sources: Vec<InputDataSource>,
        witness_failures: Arc<AtomicUsize>,
        witness_calls: Arc<AtomicUsize>,
        tx_list_witness_calls: Arc<AtomicUsize>,
        tx_list_witness_inputs: Arc<Mutex<Vec<Bytes>>>,
        parent_storage_inputs: Arc<Mutex<Vec<ParentStorageProofRequest>>>,
        account_inputs: Arc<Mutex<Vec<Vec<Address>>>>,
        account_witness_nodes: Arc<Mutex<Vec<Vec<WitnessStateNode>>>>,
    }

    #[async_trait::async_trait]
    impl Provider for TestProvider {
        async fn batch_blocks(
            &self,
            blocks: &[u64],
        ) -> RaikoResult<Vec<reth_ethereum_primitives::Block>> {
            Ok(blocks
                .iter()
                .map(|block_number| {
                    if *block_number == self.parent_block.header.number {
                        self.parent_block.clone()
                    } else {
                        self.block.clone()
                    }
                })
                .collect())
        }

        async fn batch_accounts(
            &self,
            _blocks: &[u64],
            accounts: &[Vec<Address>],
        ) -> RaikoResult<Vec<AddressMap<TrieAccount>>> {
            *self.account_inputs.lock().expect("account inputs lock") = accounts.to_vec();
            Ok(vec![AddressMap::default(); accounts.len()])
        }

        async fn batch_accounts_with_proof_witnesses(
            &self,
            _blocks: &[u64],
            accounts: &[Vec<Address>],
        ) -> RaikoResult<(Vec<AddressMap<TrieAccount>>, Vec<Vec<WitnessStateNode>>)> {
            *self.account_inputs.lock().expect("account inputs lock") = accounts.to_vec();
            let nodes = self
                .account_witness_nodes
                .lock()
                .expect("account witness nodes lock")
                .clone();
            let mut nodes = nodes;
            nodes.resize_with(accounts.len(), Vec::new);
            nodes.truncate(accounts.len());
            Ok((vec![AddressMap::default(); accounts.len()], nodes))
        }

        async fn batch_witnesses(&self, blocks: &[u64]) -> RaikoResult<Vec<ExecutionWitness>> {
            self.witness_calls.fetch_add(1, Ordering::SeqCst);
            if self
                .witness_failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    (remaining > 0).then(|| remaining - 1)
                })
                .is_ok()
            {
                return Err(RaikoError::RPC("transient witness rpc error".to_string()));
            }
            Ok(blocks.iter().map(|_| ExecutionWitness::default()).collect())
        }

        async fn batch_witnesses_with_parent_storage_proofs(
            &self,
            blocks: &[u64],
            parent_storage_proofs: &[ParentStorageProofRequest],
        ) -> RaikoResult<Vec<ExecutionWitness>> {
            self.parent_storage_inputs
                .lock()
                .expect("parent storage inputs lock")
                .extend_from_slice(parent_storage_proofs);
            self.batch_witnesses(blocks).await
        }

        async fn batch_witnesses_with_tx_lists(
            &self,
            _blocks: &[u64],
            tx_lists: &[Bytes],
        ) -> RaikoResult<Vec<ExecutionWitness>> {
            self.tx_list_witness_calls.fetch_add(1, Ordering::SeqCst);
            *self
                .tx_list_witness_inputs
                .lock()
                .expect("tx list witness inputs lock") = tx_lists.to_vec();
            Ok(tx_lists
                .iter()
                .map(|_| ExecutionWitness::default())
                .collect())
        }

        async fn batch_witnesses_with_tx_lists_and_parent_storage_proofs(
            &self,
            _blocks: &[u64],
            tx_lists: &[Bytes],
            parent_storage_proofs: &[ParentStorageProofRequest],
        ) -> RaikoResult<Vec<ExecutionWitness>> {
            self.tx_list_witness_calls.fetch_add(1, Ordering::SeqCst);
            *self
                .tx_list_witness_inputs
                .lock()
                .expect("tx list witness inputs lock") = tx_lists.to_vec();
            self.parent_storage_inputs
                .lock()
                .expect("parent storage inputs lock")
                .extend_from_slice(parent_storage_proofs);
            Ok(tx_lists
                .iter()
                .map(|_| ExecutionWitness::default())
                .collect())
        }

        async fn batch_l1_headers(&self, blocks: &[u64]) -> RaikoResult<Vec<Header>> {
            blocks
                .iter()
                .map(|block_number| {
                    self.l1_headers
                        .iter()
                        .find(|header| header.number == *block_number)
                        .cloned()
                        .ok_or_else(|| {
                            RaikoError::RPC(format!("missing L1 header for block {block_number}"))
                        })
                })
                .collect()
        }

        async fn shasta_proposal_event(
            &self,
            _l1_contract: Address,
            _l1_inclusion_block_number: u64,
            _proposal_id: u64,
        ) -> RaikoResult<ShastaEventData> {
            Ok(self.proposal_event.clone())
        }

        async fn shasta_data_sources(
            &self,
            _l1_chain_spec: &raiko2_primitives::ChainSpec,
            _proposal_event: &ShastaEventData,
            _blob_proof_type: BlobProofType,
        ) -> RaikoResult<Vec<InputDataSource>> {
            Ok(self.data_sources.clone())
        }
    }

    fn shasta_extra_data(proposal_id: u64) -> Bytes {
        let proposal_id_bytes = proposal_id.to_be_bytes();
        vec![
            0,
            proposal_id_bytes[2],
            proposal_id_bytes[3],
            proposal_id_bytes[4],
            proposal_id_bytes[5],
            proposal_id_bytes[6],
            proposal_id_bytes[7],
        ]
        .into()
    }

    fn anchor_tx(checkpoint: &AnchorV4Checkpoint) -> reth_ethereum_primitives::TransactionSigned {
        TxEip1559 {
            chain_id: 167_013,
            nonce: 0,
            gas_limit: 1_000_000,
            max_fee_per_gas: 1,
            max_priority_fee_per_gas: 0,
            to: TxKind::Call(Address::ZERO),
            value: U256::ZERO,
            access_list: Vec::default().into(),
            input: anchorV4Call {
                _checkpoint: checkpoint.clone(),
            }
            .abi_encode()
            .into(),
        }
        .into_signed(Signature::test_signature())
        .into()
    }

    fn sample_derived_tx() -> reth_ethereum_primitives::TransactionSigned {
        TxEip1559 {
            chain_id: 167_013,
            nonce: 1,
            gas_limit: 21_000,
            max_fee_per_gas: 1,
            max_priority_fee_per_gas: 0,
            to: TxKind::Call(Address::from([0x44; 20])),
            value: U256::from(1),
            access_list: Vec::default().into(),
            input: Bytes::new(),
        }
        .into_signed(Signature::test_signature())
        .into()
    }

    fn sample_unrecoverable_tx() -> reth_ethereum_primitives::TransactionSigned {
        TxEip1559 {
            chain_id: 167_013,
            nonce: 1,
            gas_limit: 21_000,
            max_fee_per_gas: 1,
            max_priority_fee_per_gas: 0,
            to: TxKind::Call(Address::from([0x55; 20])),
            value: U256::ZERO,
            access_list: Vec::default().into(),
            input: Bytes::new(),
        }
        .into_signed(Signature::new(U256::ZERO, U256::ZERO, false))
        .into()
    }

    fn sample_l1_header(number: u64, state_root: B256) -> Header {
        Header {
            number,
            parent_hash: B256::from([0xAA; 32]),
            state_root,
            timestamp: 777,
            ..Default::default()
        }
    }

    fn sample_block(
        proposal_id: u64,
        anchor_block_number: u64,
        anchor_block_hash: B256,
        anchor_state_root: B256,
    ) -> reth_ethereum_primitives::Block {
        let mut block = reth_ethereum_primitives::Block::default();
        block.header.number = 1;
        block.header.timestamp = u64::MAX / 2;
        block.header.extra_data = shasta_extra_data(proposal_id);
        block.body.transactions.push(anchor_tx(&AnchorV4Checkpoint {
            blockNumber: anchor_block_number.try_into().expect("fits in uint48"),
            blockHash: anchor_block_hash,
            stateRoot: anchor_state_root,
        }));
        block
    }

    fn sample_context(
        proposal_id: u64,
        l1_inclusion_block_number: u64,
        last_anchor_block_number: u64,
    ) -> ProofContext {
        let request = ProofRequest {
            l1_chain_id: 560_048,
            l2_chain_id: 167_013,
            proposal_id,
            l2_block_range: Some(L2BlockRange { start: 1, end: 1 }),
            shasta: Some(ShastaRequest {
                l1_inclusion_block_number,
                last_anchor_block_number,
                checkpoint: None,
            }),
            proof_type: ProofType::Native,
            ..Default::default()
        };
        ProofContext::new(request, ProverConfig::default())
    }

    fn alethia_mainnet_shasta_timestamp() -> u64 {
        match TAIKO_MAINNET.taiko_fork_activation(AlethiaTaikoHardfork::Shasta) {
            AlethiaForkCondition::Timestamp(timestamp) => timestamp,
            condition => panic!("expected mainnet Shasta timestamp fork, got {condition:?}"),
        }
    }

    #[test]
    fn chain_spec_from_context_defaults_to_current_guest_input_abi() {
        let mut ctx = sample_context(42, 11, 9);
        ctx.request.l2_chain_id = 167_000;
        let spec = super::chain_spec_from_context(&ctx).expect("chain spec");

        assert_eq!(
            spec.hard_forks.get(&ForkId::Taiko(TaikoFork::Shasta)),
            Some(&ForkCondition::Timestamp(alethia_mainnet_shasta_timestamp()))
        );
        assert!(
            spec.verifier_address_forks
                .values()
                .any(|verifiers| verifiers.contains_key(&ProofType::SgxGeth))
        );
    }

    #[test]
    fn chain_spec_from_context_projects_v0_1_0_guest_input_abi() {
        let mut ctx = sample_context(42, 11, 9);
        ctx.request.l2_chain_id = 167_000;
        ctx.config = serde_json::json!({ "guest_input_abi": "v0_1_0" });

        let spec = super::chain_spec_from_context(&ctx).expect("chain spec");

        assert_eq!(
            spec.hard_forks.get(&ForkId::Taiko(TaikoFork::Shasta)),
            Some(&ForkCondition::Timestamp(alethia_mainnet_shasta_timestamp()))
        );
        assert!(
            spec.verifier_address_forks
                .values()
                .all(|verifiers| !verifiers.contains_key(&ProofType::SgxGeth))
        );
    }

    #[test]
    fn chain_spec_from_context_rejects_invalid_guest_input_abi() {
        let mut ctx = sample_context(42, 11, 9);
        ctx.config = serde_json::json!({ "guest_input_abi": "legacy" });

        let err = super::chain_spec_from_context(&ctx).expect_err("invalid abi");

        assert!(err.to_string().contains("invalid prover.guest_input_abi"));
    }

    #[test]
    fn l1_chain_spec_from_context_uses_preflight_override() {
        let mut ctx = sample_context(42, 11, 9);
        let mut l1_spec = SupportedChainSpecs::default()
            .get_chain_spec_with_chain_id(ctx.request.l1_chain_id)
            .expect("known l1 chain");
        l1_spec.beacon_rpc = Some("https://override.example/beacon".to_string());
        ctx.preflight.resolved_l1_chain_spec = Some(l1_spec.clone());

        let resolved = super::l1_chain_spec_from_context(&ctx).expect("l1 chain spec");

        assert_eq!(resolved.chain_id, l1_spec.chain_id);
        assert_eq!(
            resolved.beacon_rpc.as_deref(),
            Some("https://override.example/beacon")
        );
    }

    #[test]
    fn l1_chain_spec_from_context_rejects_mismatched_preflight_override() {
        let mut ctx = sample_context(42, 11, 9);
        let mut l1_spec = SupportedChainSpecs::default()
            .get_chain_spec_with_chain_id(ctx.request.l1_chain_id)
            .expect("known l1 chain");
        l1_spec.chain_id += 1;
        ctx.preflight.resolved_l1_chain_spec = Some(l1_spec);

        let err = super::l1_chain_spec_from_context(&ctx).expect_err("chain id mismatch");

        assert!(matches!(err, RaikoError::InvalidRequestConfig(_)));
    }

    fn sample_provider() -> TestProvider {
        let origin_header = sample_l1_header(10, B256::from([0x66; 32]));
        let block = sample_block(42, 10, origin_header.hash_slow(), origin_header.state_root);
        let mut parent_block = reth_ethereum_primitives::Block::default();
        parent_block.header.number = block.header.number.saturating_sub(1);
        parent_block.header.timestamp = block.header.timestamp.saturating_sub(1);
        parent_block.header.gas_limit = block.header.gas_limit;
        let mut proposal_event = ShastaEventData::default();
        proposal_event.proposal.id = 42u64.try_into().expect("fits in uint48");
        proposal_event.proposal.proposer = Address::from([0x11; 20]);
        proposal_event.proposal.parentProposalHash = B256::from([0x22; 32]);
        proposal_event.proposal.originBlockNumber = 10u64.try_into().expect("fits in uint48");
        proposal_event.proposal.originBlockHash = origin_header.hash_slow();
        proposal_event.proposal.timestamp = 777u64.try_into().expect("fits in uint48");

        TestProvider {
            block,
            parent_block,
            proposal_event,
            l1_headers: vec![origin_header],
            data_sources: Vec::new(),
            witness_failures: Arc::new(AtomicUsize::new(0)),
            witness_calls: Arc::new(AtomicUsize::new(0)),
            tx_list_witness_calls: Arc::new(AtomicUsize::new(0)),
            tx_list_witness_inputs: Arc::new(Mutex::new(Vec::new())),
            parent_storage_inputs: Arc::new(Mutex::new(Vec::new())),
            account_inputs: Arc::new(Mutex::new(Vec::new())),
            account_witness_nodes: Arc::new(Mutex::new(vec![vec![WitnessStateNode::from_bytes(
                Bytes::from_static(&[0xc1, 0x80]),
            )]])),
        }
    }

    fn add_inline_shasta_source(provider: &mut TestProvider) {
        provider.proposal_event.proposal.sources = vec![DerivationSource {
            isForcedInclusion: false,
            blobSlice: BlobSlice {
                blobHashes: Vec::new(),
                offset: 0usize.try_into().expect("fits in uint24"),
                timestamp: 0u64.try_into().expect("fits in uint48"),
            },
        }];
        let manifest = DerivationSourceManifest {
            blocks: vec![BlockManifest {
                timestamp: provider.block.header.timestamp,
                coinbase: provider.block.header.beneficiary,
                anchor_block_number: 10,
                gas_limit: provider.block.header.gas_limit,
                transactions: Vec::new(),
            }],
        };
        provider.data_sources = vec![InputDataSource {
            tx_data_from_calldata: manifest.encode_and_compress().expect("encode manifest"),
            is_forced_inclusion: false,
            ..Default::default()
        }];
    }

    fn add_invalid_inline_shasta_source_with_transaction(provider: &mut TestProvider) {
        let tx = sample_derived_tx();
        add_inline_shasta_source_with_transactions(provider, vec![tx]);
    }

    fn add_inline_shasta_source_with_transactions(
        provider: &mut TestProvider,
        transactions: Vec<reth_ethereum_primitives::TransactionSigned>,
    ) {
        provider.proposal_event.proposal.sources = vec![DerivationSource {
            isForcedInclusion: false,
            blobSlice: BlobSlice {
                blobHashes: Vec::new(),
                offset: 0usize.try_into().expect("fits in uint24"),
                timestamp: 0u64.try_into().expect("fits in uint48"),
            },
        }];
        let manifest = DerivationSourceManifest {
            blocks: vec![BlockManifest {
                timestamp: 0,
                coinbase: Address::ZERO,
                anchor_block_number: 0,
                gas_limit: 0,
                transactions: transactions
                    .into_iter()
                    .map(|tx| {
                        alloy_rlp::decode_exact(alloy_rlp::encode(tx))
                            .expect("transaction decodes manifest transaction")
                    })
                    .collect(),
            }],
        };
        provider.data_sources = vec![InputDataSource {
            tx_data_from_calldata: manifest.encode_and_compress().expect("encode manifest"),
            is_forced_inclusion: false,
            ..Default::default()
        }];
    }

    #[tokio::test]
    async fn preflight_fetches_canonical_shasta_proposal_event() {
        let provider = sample_provider();
        let ctx = sample_context(42, 11, 9);
        let spec = ShastaSpec::new(
            PipelineKey::ShastaNative,
            (),
            NativeBackend,
            provider.clone(),
        );

        let input = spec.preflight(&ctx, &provider).await.expect("preflight");

        assert_eq!(input.taiko.proposal_event.proposal.id.to::<u64>(), 42);
        assert_eq!(input.taiko.l1_header.number, 10);
        assert_eq!(
            input.taiko.proposal_event.proposal.originBlockHash,
            input.taiko.l1_header.hash_slow()
        );
        assert_eq!(input.taiko.l1_ancestor_headers.len(), 1);
        assert_eq!(input.taiko.prover_data.last_anchor_block_number, Some(9));
        assert_eq!(
            input.taiko.blob_proof_type,
            BlobProofType::ProofOfEquivalence
        );
        assert_eq!(
            input.witnesses[0].chain_spec,
            SupportedChainSpecs::default()
                .get_chain_spec_with_chain_id(167_013)
                .expect("supported chain")
        );
    }

    #[tokio::test]
    async fn preflight_uses_tx_list_witnesses_for_shasta_sources() {
        let mut provider = sample_provider();
        add_inline_shasta_source(&mut provider);
        let ctx = sample_context(42, 11, 9);
        let spec = ShastaSpec::new(
            PipelineKey::ShastaNative,
            (),
            NativeBackend,
            provider.clone(),
        );

        let _input = spec.preflight(&ctx, &provider).await.expect("preflight");

        assert_eq!(provider.witness_calls.load(Ordering::SeqCst), 0);
        assert_eq!(provider.tx_list_witness_calls.load(Ordering::SeqCst), 1);
        let tx_lists = provider
            .tx_list_witness_inputs
            .lock()
            .expect("tx list witness inputs lock");
        assert_eq!(tx_lists.len(), 1);
        assert!(!tx_lists[0].is_empty());
    }

    #[tokio::test]
    async fn preflight_defaults_invalid_manifest_before_tx_list_witness() {
        let mut provider = sample_provider();
        add_invalid_inline_shasta_source_with_transaction(&mut provider);
        let ctx = sample_context(42, 11, 9);
        let spec = ShastaSpec::new(
            PipelineKey::ShastaNative,
            (),
            NativeBackend,
            provider.clone(),
        );

        let _input = spec.preflight(&ctx, &provider).await.expect("preflight");

        let tx_lists = provider
            .tx_list_witness_inputs
            .lock()
            .expect("tx list witness inputs lock");
        assert_eq!(tx_lists.len(), 1);
        let expected_anchor_only = super::encode_replay_tx_list(
            &provider.block,
            &BlockManifest {
                timestamp: provider.block.header.timestamp,
                coinbase: provider.block.header.beneficiary,
                anchor_block_number: 10,
                gas_limit: provider.block.header.gas_limit,
                transactions: Vec::new(),
            },
        )
        .expect("encode anchor-only tx list");
        assert_eq!(tx_lists[0], expected_anchor_only);
    }

    #[tokio::test]
    async fn preflight_fetches_only_anchor_account_state() {
        let mut provider = sample_provider();
        add_inline_shasta_source(&mut provider);
        let ctx = sample_context(42, 11, 9);
        let spec = ShastaSpec::new(
            PipelineKey::ShastaNative,
            (),
            NativeBackend,
            provider.clone(),
        );

        let _input = spec.preflight(&ctx, &provider).await.expect("preflight");

        let account_inputs = provider.account_inputs.lock().expect("account inputs lock");
        assert_eq!(
            &*account_inputs,
            &[vec![Address::from(TAIKO_GOLDEN_TOUCH_ADDRESS)]]
        );
    }

    #[tokio::test]
    async fn preflight_fetches_derived_tx_signer_account_state() {
        let provider = sample_provider();
        let tx = sample_derived_tx();
        let manifest_tx = alloy_rlp::decode_exact(alloy_rlp::encode(tx))
            .expect("transaction decodes manifest transaction");
        let tx_lists = vec![
            super::encode_replay_tx_list(
                &provider.block,
                &BlockManifest {
                    timestamp: provider.block.header.timestamp,
                    coinbase: provider.block.header.beneficiary,
                    anchor_block_number: 10,
                    gas_limit: provider.block.header.gas_limit,
                    transactions: vec![manifest_tx],
                },
            )
            .expect("encode tx list"),
        ];
        let signers =
            super::derived_tx_list_signers(&tx_lists[0]).expect("recover derived tx list signers");
        assert!(!signers.is_empty());
        let chain_spec = ChainSpec {
            name: "taiko_hoodi".to_string(),
            chain_id: 167_013,
            ..Default::default()
        };

        let (_chunk_index, witnesses) = super::fetch_preflight_chunk(
            &provider,
            42,
            0,
            1,
            std::slice::from_ref(&provider.block),
            Some(&tx_lists),
            &[],
            chain_spec,
        )
        .await
        .expect("fetch preflight chunk");

        assert_eq!(witnesses.len(), 1);
        assert_eq!(provider.tx_list_witness_calls.load(Ordering::SeqCst), 1);
        let account_inputs = provider.account_inputs.lock().expect("account inputs lock");
        assert_eq!(account_inputs.len(), 1);
        assert!(account_inputs[0].contains(&Address::from(TAIKO_GOLDEN_TOUCH_ADDRESS)));
        for signer in signers {
            assert!(account_inputs[0].contains(&signer));
        }
    }

    #[test]
    fn derived_tx_list_signers_skip_unrecoverable_transactions() {
        let provider = sample_provider();
        let manifest_tx = alloy_rlp::decode_exact(alloy_rlp::encode(sample_unrecoverable_tx()))
            .expect("transaction decodes manifest transaction");
        let tx_list = super::encode_replay_tx_list(
            &provider.block,
            &BlockManifest {
                timestamp: provider.block.header.timestamp,
                coinbase: provider.block.header.beneficiary,
                anchor_block_number: 10,
                gas_limit: provider.block.header.gas_limit,
                transactions: vec![manifest_tx],
            },
        )
        .expect("encode tx list");

        let signers =
            super::derived_tx_list_signers(&tx_list).expect("decode derived tx list signers");

        assert!(signers.is_empty());
    }

    #[tokio::test]
    async fn mainnet_tx_list_preflight_uses_canonical_witness() {
        let provider = sample_provider();
        let tx_lists = vec![
            super::encode_replay_tx_list(
                &provider.block,
                &BlockManifest {
                    timestamp: provider.block.header.timestamp,
                    coinbase: provider.block.header.beneficiary,
                    anchor_block_number: 10,
                    gas_limit: provider.block.header.gas_limit,
                    transactions: Vec::new(),
                },
            )
            .expect("encode tx list"),
        ];
        let chain_spec = ChainSpec {
            name: "taiko_mainnet".to_string(),
            chain_id: super::TAIKO_MAINNET_CHAIN_ID,
            ..Default::default()
        };

        let (_chunk_index, witnesses) = super::fetch_preflight_chunk(
            &provider,
            42,
            0,
            1,
            std::slice::from_ref(&provider.block),
            Some(&tx_lists),
            &[],
            chain_spec,
        )
        .await
        .expect("fetch preflight chunk");

        assert_eq!(witnesses.len(), 1);
        assert_eq!(provider.witness_calls.load(Ordering::SeqCst), 1);
        assert_eq!(provider.tx_list_witness_calls.load(Ordering::SeqCst), 0);
        assert_eq!(witnesses[0].witness.state.len(), 1);
    }

    #[tokio::test]
    async fn preflight_rejects_tx_list_count_mismatch() {
        let provider = sample_provider();
        let tx_lists = Vec::new();
        let chain_spec = ChainSpec {
            name: "taiko_hoodi".to_string(),
            chain_id: 167_013,
            ..Default::default()
        };

        let err = super::fetch_preflight_chunk(
            &provider,
            42,
            0,
            1,
            std::slice::from_ref(&provider.block),
            Some(&tx_lists),
            &[],
            chain_spec,
        )
        .await
        .expect_err("mismatched tx-list count should be rejected");

        assert!(
            err.to_string()
                .contains("tx-list witness count (0) does not match block count (1)")
        );
    }

    #[tokio::test]
    async fn preflight_rejects_block_number_mismatch_from_provider() {
        let provider = sample_provider();
        let mut ctx = sample_context(42, 11, 9);
        ctx.request.l2_block_range = Some(L2BlockRange { start: 2, end: 2 });
        let spec = ShastaSpec::new(
            PipelineKey::ShastaNative,
            (),
            NativeBackend,
            provider.clone(),
        );

        let err = spec
            .preflight(&ctx, &provider)
            .await
            .expect_err("provider block number mismatch should be rejected");

        assert!(err.to_string().contains("provider returned block 1"));
        assert!(err.to_string().contains("requested block 2"));
    }

    #[tokio::test]
    async fn preflight_retries_transient_witness_rpc_errors() {
        let provider = sample_provider();
        provider.witness_failures.store(1, Ordering::SeqCst);
        let ctx = sample_context(42, 11, 9);
        let spec = ShastaSpec::new(
            PipelineKey::ShastaNative,
            (),
            NativeBackend,
            provider.clone(),
        );

        let input = spec.preflight(&ctx, &provider).await.expect("preflight");

        assert_eq!(input.witnesses.len(), 1);
        assert_eq!(provider.witness_calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn preflight_chunk_operation_includes_proposal_and_block_range() {
        let operation = super::preflight_chunk_operation(2156, 19, 48, &[10834703, 10834704], true);

        assert_eq!(
            operation,
            "shasta preflight chunk 19 proposal_id=2156 chunk_count=48 blocks=10834703..10834704 block_count=2 tx_list_witness=true"
        );
    }

    #[test]
    fn extract_block_range_requires_explicit_l2_block_range() {
        let mut ctx = sample_context(42, 11, 9);
        ctx.request.l2_block_range = None;

        let chain_spec = super::chain_spec_from_context(&ctx).expect("chain spec");
        let err = super::extract_block_range(&ctx, &chain_spec).expect_err("missing range");

        assert!(
            err.to_string()
                .contains("request l2_block_range is required for Shasta preflight")
        );
    }

    #[test]
    fn extract_block_range_accepts_protocol_max_blocks() {
        let mut ctx = sample_context(42, 11, 9);
        ctx.request.l2_block_range = Some(L2BlockRange {
            start: 1,
            end: u64::try_from(super::DERIVATION_SOURCE_MAX_BLOCKS).expect("fits u64"),
        });

        let chain_spec = super::chain_spec_from_context(&ctx).expect("chain spec");
        let (blocks, proposal_id) =
            super::extract_block_range(&ctx, &chain_spec).expect("valid range");

        assert_eq!(proposal_id, 42);
        assert_eq!(blocks.len(), super::DERIVATION_SOURCE_MAX_BLOCKS);
    }

    #[test]
    fn extract_block_range_accepts_unzen_max_blocks_for_configured_environment() {
        let mut ctx = sample_context(42, 11, 9);
        ctx.request.l2_chain_id = 167_001;
        ctx.request.l2_block_range = Some(L2BlockRange {
            start: 1,
            end: u64::try_from(super::UNZEN_DERIVATION_SOURCE_MAX_BLOCKS).expect("fits u64"),
        });

        let chain_spec = super::chain_spec_from_context(&ctx).expect("chain spec");
        let (blocks, proposal_id) =
            super::extract_block_range(&ctx, &chain_spec).expect("valid range");

        assert_eq!(proposal_id, 42);
        assert_eq!(blocks.len(), super::UNZEN_DERIVATION_SOURCE_MAX_BLOCKS);
    }

    #[test]
    fn extract_block_range_rejects_unzen_range_when_environment_has_no_activation() {
        let mut ctx = sample_context(42, 11, 9);
        ctx.request.l2_chain_id = 167_000;
        let max_blocks = u64::try_from(super::DERIVATION_SOURCE_MAX_BLOCKS).expect("fits u64");
        ctx.request.l2_block_range = Some(L2BlockRange {
            start: 1,
            end: max_blocks + 1,
        });
        let chain_spec = super::chain_spec_from_context(&ctx).expect("chain spec");

        let err = super::extract_block_range(&ctx, &chain_spec).expect_err("oversized range");

        assert!(err.to_string().contains("contains 193 blocks, max 192"));
    }

    #[test]
    fn extract_block_range_rejects_more_than_unzen_max_blocks() {
        let mut ctx = sample_context(42, 11, 9);
        ctx.request.l2_chain_id = 167_001;
        let max_blocks =
            u64::try_from(super::UNZEN_DERIVATION_SOURCE_MAX_BLOCKS).expect("fits u64");
        ctx.request.l2_block_range = Some(L2BlockRange {
            start: 1,
            end: max_blocks + 1,
        });
        let chain_spec = super::chain_spec_from_context(&ctx).expect("chain spec");

        let err = super::extract_block_range(&ctx, &chain_spec).expect_err("oversized range");

        assert!(err.to_string().contains("contains 769 blocks, max 768"));
    }

    #[test]
    fn derivation_source_limit_uses_environment_hardfork_activation() {
        let mut ctx = sample_context(42, 11, 9);
        ctx.request.l2_chain_id = 167_000;
        let mainnet = super::chain_spec_from_context(&ctx).expect("chain spec");
        assert_eq!(
            super::derivation_source_max_blocks_for_chain_spec_at(&mainnet, 1, u64::MAX),
            super::DERIVATION_SOURCE_MAX_BLOCKS
        );

        ctx.request.l2_chain_id = 167_013;
        let hoodi = super::chain_spec_from_context(&ctx).expect("chain spec");
        let Some(ForkCondition::Timestamp(hoodi_unzen_timestamp)) =
            hoodi.hard_forks.get(&ForkId::Taiko(TaikoFork::Unzen))
        else {
            panic!("taiko_hoodi should configure a timestamp-based Unzen fork");
        };
        assert_eq!(
            super::derivation_source_max_blocks_for_chain_spec_at(
                &hoodi,
                1,
                hoodi_unzen_timestamp.saturating_sub(1),
            ),
            super::DERIVATION_SOURCE_MAX_BLOCKS
        );
        assert_eq!(
            super::derivation_source_max_blocks_for_chain_spec_at(
                &hoodi,
                1,
                *hoodi_unzen_timestamp
            ),
            super::UNZEN_DERIVATION_SOURCE_MAX_BLOCKS
        );

        ctx.request.l2_chain_id = 167_001;
        let devnet = super::chain_spec_from_context(&ctx).expect("chain spec");
        let Some(ForkCondition::Timestamp(devnet_unzen_timestamp)) =
            devnet.hard_forks.get(&ForkId::Taiko(TaikoFork::Unzen))
        else {
            panic!("taiko_dev should configure a timestamp-based Unzen fork");
        };
        assert_eq!(
            super::derivation_source_max_blocks_for_chain_spec_at(
                &devnet,
                1,
                *devnet_unzen_timestamp,
            ),
            super::UNZEN_DERIVATION_SOURCE_MAX_BLOCKS
        );

        ctx.request.l2_chain_id = 167_011;
        let masaya = super::chain_spec_from_context(&ctx).expect("chain spec");
        assert_eq!(
            masaya.hard_forks.get(&ForkId::Taiko(TaikoFork::Unzen)),
            Some(&ForkCondition::Timestamp(0))
        );
        assert_eq!(
            super::derivation_source_max_blocks_for_chain_spec_at(&masaya, 1, 0),
            super::UNZEN_DERIVATION_SOURCE_MAX_BLOCKS
        );
    }

    #[test]
    fn validate_derivation_source_block_limit_rejects_inactive_unzen_environment() {
        let mut ctx = sample_context(42, 11, 9);
        ctx.request.l2_chain_id = 167_000;
        let chain_spec = super::chain_spec_from_context(&ctx).expect("chain spec");

        let err = super::validate_derivation_source_block_limit(
            super::DERIVATION_SOURCE_MAX_BLOCKS + 1,
            1,
            u64::MAX,
            &chain_spec,
        )
        .expect_err("inactive unzen environment should reject");

        assert!(err.to_string().contains("contains 193 blocks, max 192"));
    }

    #[tokio::test]
    async fn preflight_rejects_pre_unzen_range_before_witness_fetch() {
        let provider = sample_provider();
        let mut ctx = sample_context(42, 11, 9);
        ctx.request.l2_chain_id = 167_013;
        let max_blocks = u64::try_from(super::DERIVATION_SOURCE_MAX_BLOCKS).expect("fits u64");
        ctx.request.l2_block_range = Some(L2BlockRange {
            start: 1,
            end: max_blocks + 1,
        });
        let spec = ShastaSpec::new(
            PipelineKey::ShastaNative,
            (),
            NativeBackend,
            provider.clone(),
        );

        let err = spec
            .preflight(&ctx, &provider)
            .await
            .expect_err("pre-Unzen oversized range should fail before witness fetch");

        assert!(err.to_string().contains("contains 193 blocks, max 192"));
        assert_eq!(provider.witness_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn preflight_rejects_kzg_versioned_hash_hint_for_sp1() {
        let provider = sample_provider();
        let mut ctx = sample_context(42, 11, 9);
        ctx.request.proof_type = ProofType::Sp1;
        ctx.request.blob_proof_type = Some("kzg_versioned_hash".to_string());
        let spec = ShastaSpec::new(
            PipelineKey::ShastaNative,
            (),
            NativeBackend,
            provider.clone(),
        );

        let err = spec
            .preflight(&ctx, &provider)
            .await
            .expect_err("kzg_versioned_hash should be rejected");
        assert!(err.to_string().contains("invalid blob_proof_type"));
    }

    #[tokio::test]
    async fn preflight_accepts_repeated_last_anchor_with_l1_headers() {
        let provider = sample_provider();
        let ctx = sample_context(42, 11, 10);
        let spec = ShastaSpec::new(
            PipelineKey::ShastaNative,
            (),
            NativeBackend,
            provider.clone(),
        );

        let input = spec.preflight(&ctx, &provider).await.expect("preflight");

        assert_eq!(input.taiko.l1_ancestor_headers.len(), 1);
        assert_eq!(input.taiko.l1_header.number, 10);
    }

    #[test]
    fn stalled_anchor_parent_storage_request_targets_signal_service_checkpoint() {
        let signal_service = Address::from([0x5a; 20]);
        let chain_spec = ChainSpec {
            l2_signal_service: Some(signal_service),
            ..Default::default()
        };
        let parent_anchor = 123_456u64;

        let requests = super::stalled_anchor_parent_storage_requests(&chain_spec, parent_anchor)
            .expect("stalled anchor parent storage request");

        assert_eq!(
            requests,
            vec![ParentStorageProofRequest {
                block_index: 0,
                address: signal_service,
                storage_keys: super::signal_service_checkpoint_storage_keys(parent_anchor),
            }]
        );
    }

    #[tokio::test]
    async fn preflight_bypasses_stalled_anchor_linkage() {
        let mut provider = sample_provider();
        let origin_header = sample_l1_header(200, B256::from([0x77; 32]));
        provider.block = sample_block(42, 10, B256::from([0x88; 32]), B256::from([0x99; 32]));
        provider.proposal_event.proposal.originBlockNumber =
            origin_header.number.try_into().expect("fits in uint48");
        provider.proposal_event.proposal.originBlockHash = origin_header.hash_slow();
        provider.l1_headers = vec![origin_header.clone()];
        let ctx = sample_context(42, 201, 10);
        let spec = ShastaSpec::new(
            PipelineKey::ShastaNative,
            (),
            NativeBackend,
            provider.clone(),
        );

        let input = spec.preflight(&ctx, &provider).await.expect("preflight");

        assert!(input.taiko.l1_ancestor_headers.is_empty());
        assert_eq!(input.taiko.l1_header.number, origin_header.number);
    }

    #[tokio::test]
    async fn preflight_stalled_anchor_requests_parent_checkpoint_storage_proof() {
        let mut provider = sample_provider();
        let origin_header = sample_l1_header(200, B256::from([0x77; 32]));
        provider.block = sample_block(42, 10, B256::from([0x88; 32]), B256::from([0x99; 32]));
        provider.proposal_event.proposal.originBlockNumber =
            origin_header.number.try_into().expect("fits in uint48");
        provider.proposal_event.proposal.originBlockHash = origin_header.hash_slow();
        provider.l1_headers = vec![origin_header];
        let ctx = sample_context(42, 201, 10);
        let chain_spec = SupportedChainSpecs::default()
            .get_chain_spec_with_chain_id(ctx.request.l2_chain_id)
            .expect("supported chain");
        let signal_service = chain_spec
            .l2_signal_service
            .expect("supported chain defines SignalService");
        let spec = ShastaSpec::new(
            PipelineKey::ShastaNative,
            (),
            NativeBackend,
            provider.clone(),
        );

        let _input = spec.preflight(&ctx, &provider).await.expect("preflight");

        let parent_storage_inputs = provider
            .parent_storage_inputs
            .lock()
            .expect("parent storage inputs lock");
        assert_eq!(
            &*parent_storage_inputs,
            &[ParentStorageProofRequest {
                block_index: 0,
                address: signal_service,
                storage_keys: super::signal_service_checkpoint_storage_keys(10),
            }]
        );
    }

    #[tokio::test]
    async fn preflight_hydrates_canonical_shasta_data_sources() {
        let mut provider = sample_provider();
        let blob_bytes = vec![0; BYTES_PER_BLOB];
        provider.proposal_event.proposal.sources = vec![DerivationSource {
            isForcedInclusion: false,
            blobSlice: BlobSlice {
                blobHashes: vec![B256::from([0x44; 32])],
                offset: 0u32.try_into().expect("fits in uint24"),
                timestamp: 777u64.try_into().expect("fits in uint48"),
            },
        }];
        provider.data_sources = vec![InputDataSource {
            tx_data_from_calldata: Vec::new(),
            tx_data_from_blob: vec![blob_bytes.clone()],
            blob_commitments: vec![vec![4; 48]],
            blob_proofs: vec![vec![5; 48]],
            is_forced_inclusion: false,
        }];
        let ctx = sample_context(42, 11, 9);
        let spec = ShastaSpec::new(
            PipelineKey::ShastaNative,
            (),
            NativeBackend,
            provider.clone(),
        );

        let input = spec.preflight(&ctx, &provider).await.expect("preflight");

        assert_eq!(input.taiko.data_sources.len(), 1);
        assert_eq!(
            input.taiko.data_sources[0].tx_data_from_blob,
            vec![blob_bytes]
        );
        assert_eq!(
            input.taiko.data_sources[0].blob_commitments,
            vec![vec![4; 48]]
        );
        assert_eq!(input.taiko.data_sources[0].blob_proofs, vec![vec![5; 48]]);
    }
}
