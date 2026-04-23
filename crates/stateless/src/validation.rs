use crate::{
    sparse::SparseState,
    trie::{StatelessTrie, StatelessTrieExt},
    witness_db::{AncestorHashes, WitnessDatabase},
};
use alethia_reth_block::{
    config::{TaikoEvmConfig, TaikoNextBlockEnvAttributes},
    derived_block::{assemble_filtered_block, execute_derived_block},
};
use alethia_reth_chainspec::spec::TaikoChainSpec;
use alethia_reth_consensus::validation::{
    TaikoBeaconConsensus, TaikoBlockReader, validate_anchor_transaction_in_block,
};
use alloy_consensus::{BlockHeader, Header, TrieAccount, transaction::Recovered};
use alloy_primitives::{Address, B256, map::AddressMap};
use raiko2_primitives::{
    ExecutionWitness, StatelessValidationError, WitnessHeader, WitnessStateNode,
};
use reth_consensus::{Consensus, HeaderValidator};
use reth_consensus_common::validation::validate_block_pre_execution;
use reth_ethereum_consensus::validate_block_post_execution;
use reth_ethereum_primitives::{Block, BlockBody, Receipt, TransactionSigned};
use reth_evm::{ConfigureEvm, block::BlockExecutionError, execute::Executor};
use reth_execution_types::BlockExecutionResult;
use reth_primitives_traits::SignedTransaction;
use reth_primitives_traits::{Block as _, RecoveredBlock, SealedHeader};
use reth_trie_common::{HashedPostState, KeccakKeyHasher};
use std::sync::Arc;

/// Performs stateless validation of a block using the provided witness data.
///
/// # Errors
///
/// Returns `StatelessValidationError` when witness data is invalid, consensus checks fail,
/// or the computed post-state root mismatches.
#[inline]
pub fn validate_block(
    block: Block,
    witness: &ExecutionWitness,
    callers: AddressMap<TrieAccount>,
    chain_spec: &Arc<TaikoChainSpec>,
    config: &TaikoEvmConfig,
) -> Result<B256, StatelessValidationError> {
    validate_block_with_ancestor_headers(
        block,
        witness,
        &witness.headers,
        callers,
        chain_spec,
        config,
    )
}

/// Performs stateless validation of a block using an externally supplied ancestor header window.
///
/// This is the proposal-path variant used when a contiguous sequence of blocks shares a single
/// rolling ancestor window rather than embedding the same headers inside each witness.
///
/// # Errors
///
/// Returns `StatelessValidationError` when ancestor headers are invalid, witness data is invalid,
/// consensus checks fail, or the computed post-state root mismatches.
#[inline]
pub fn validate_block_with_ancestor_headers(
    block: Block,
    witness: &ExecutionWitness,
    ancestor_headers: &[WitnessHeader],
    callers: AddressMap<TrieAccount>,
    chain_spec: &Arc<TaikoChainSpec>,
    config: &TaikoEvmConfig,
) -> Result<B256, StatelessValidationError> {
    validate_block_with_witness_resources(
        block,
        witness,
        ancestor_headers,
        &[],
        callers,
        chain_spec,
        config,
    )
}

/// Performs stateless validation of a block using ancestor overrides and a proposal-level shared
/// state node pool.
///
/// # Errors
///
/// Returns `StatelessValidationError` when ancestor headers are invalid, witness data is invalid,
/// consensus checks fail, or the computed post-state root mismatches.
#[inline]
pub fn validate_block_with_witness_resources(
    block: Block,
    witness: &ExecutionWitness,
    ancestor_headers: &[WitnessHeader],
    shared_state_nodes: &[WitnessStateNode],
    callers: AddressMap<TrieAccount>,
    chain_spec: &Arc<TaikoChainSpec>,
    config: &TaikoEvmConfig,
) -> Result<B256, StatelessValidationError> {
    stateless_validation_with_trie::<SparseState>(
        block,
        witness,
        ancestor_headers,
        shared_state_nodes,
        callers,
        chain_spec,
        config,
    )
}

pub(crate) fn decode_recovered_block(
    block: Block,
) -> Result<RecoveredBlock<Block>, StatelessValidationError> {
    block
        .try_into_recovered()
        .map_err(|_| StatelessValidationError::SignerRecovery)
}

pub(crate) fn determine_pre_state_root(
    headers: &[WitnessHeader],
) -> Result<B256, StatelessValidationError> {
    match headers.last() {
        Some(prev_header) => prev_header
            .full_header()
            .map(|header| header.state_root)
            .ok_or(StatelessValidationError::HeaderDeserializationFailed),
        None => Err(StatelessValidationError::MissingAncestorHeader),
    }
}

fn ensure_full_ancestor_headers(
    ancestor_headers: &[WitnessHeader],
) -> Result<(), StatelessValidationError> {
    if ancestor_headers
        .iter()
        .any(|header| header.full_header().is_none())
    {
        return Err(StatelessValidationError::CompactAncestorHeaderUnsupported);
    }

    Ok(())
}

fn sealed_parent_header(
    ancestor_headers: &[WitnessHeader],
) -> Result<SealedHeader, StatelessValidationError> {
    ancestor_headers
        .last()
        .and_then(|header| {
            header
                .full_header()
                .cloned()
                .map(|full| SealedHeader::new(full, header.hash))
        })
        .ok_or(StatelessValidationError::MissingAncestorHeader)
}

fn map_block_execution_error(err: BlockExecutionError) -> StatelessValidationError {
    StatelessValidationError::StatelessExecutionFailed(err.to_string())
}

/// Filtered block execution artifacts returned by txlist-driven reconstruction.
#[derive(Debug)]
pub struct FilteredBlockExecutionOutcome {
    /// The block assembled from transactions that were actually committed.
    pub filtered_block: RecoveredBlock<Block>,
    /// The execution result produced while building the filtered block.
    pub execution_result: BlockExecutionResult<Receipt>,
    /// The hashed post-state after execution.
    pub hashed_state: HashedPostState,
}

fn recovered_signer_or_zero(tx: &TransactionSigned) -> Address {
    tx.recover_signer().unwrap_or(Address::ZERO)
}

fn build_derived_block(
    parent_header: &SealedHeader,
    anchor_tx: Option<Recovered<TransactionSigned>>,
    transactions: Vec<TransactionSigned>,
    block_env: TaikoNextBlockEnvAttributes,
) -> RecoveredBlock<Block> {
    let mut block_transactions =
        Vec::with_capacity(transactions.len() + usize::from(anchor_tx.is_some()));
    let mut senders = Vec::with_capacity(block_transactions.capacity());

    if let Some(anchor_tx) = anchor_tx {
        senders.push(anchor_tx.signer());
        block_transactions.push(anchor_tx.into_inner());
    }

    for tx in transactions {
        senders.push(recovered_signer_or_zero(&tx));
        block_transactions.push(tx);
    }

    let header = Header {
        parent_hash: parent_header.hash(),
        number: parent_header.number + 1,
        timestamp: block_env.timestamp,
        beneficiary: block_env.suggested_fee_recipient,
        gas_limit: block_env.gas_limit,
        base_fee_per_gas: Some(block_env.base_fee_per_gas),
        mix_hash: block_env.prev_randao,
        extra_data: block_env.extra_data,
        ..Default::default()
    };

    RecoveredBlock::new_unhashed(
        Block {
            header,
            body: BlockBody {
                transactions: block_transactions,
                ommers: Default::default(),
                withdrawals: Some(Default::default()),
            },
        },
        senders,
    )
}

/// Reconstruct a candidate block from a transaction sequence using the witness-backed pre-state.
///
/// The optional `anchor_tx` is always executed first and is treated as fatal if it cannot be
/// recovered or executed. All transactions in `transactions` are treated as non-anchor candidates:
/// unrecoverable or invalid transactions are skipped, while committed transactions are recorded in
/// the generated block body.
///
/// # Errors
///
/// Returns an error if the witness pre-state cannot be materialized, the EVM block builder fails,
/// the anchor transaction fails, or the reconstructed block fails consensus/post-state validation.
#[allow(clippy::too_many_arguments)]
pub fn reconstruct_block_from_transactions_with_witness_resources(
    anchor_tx: Option<Recovered<TransactionSigned>>,
    transactions: Vec<TransactionSigned>,
    block_env: TaikoNextBlockEnvAttributes,
    witness: &ExecutionWitness,
    ancestor_headers: &[WitnessHeader],
    shared_state_nodes: &[WitnessStateNode],
    callers: AddressMap<TrieAccount>,
    chain_spec: &Arc<TaikoChainSpec>,
    evm_config: &TaikoEvmConfig,
) -> Result<FilteredBlockExecutionOutcome, StatelessValidationError> {
    let parent_header = sealed_parent_header(ancestor_headers)?;
    let pre_state_root = determine_pre_state_root(ancestor_headers)?;
    let ancestor_hashes = compute_next_block_ancestor_hashes(ancestor_headers)?;
    let derived_block = build_derived_block(&parent_header, anchor_tx, transactions, block_env);

    let (mut trie, bytecode) =
        SparseState::new_with_state_pool(witness, shared_state_nodes, pre_state_root)?;
    trie.append_callers(callers);

    let db = WitnessDatabase::new(&trie, bytecode, ancestor_hashes);
    let execution_outcome = execute_derived_block(evm_config, &parent_header, &derived_block, db)
        .map_err(map_block_execution_error)?;
    let state_root = trie.calculate_state_root(execution_outcome.hashed_state.clone())?;
    let filtered_block = assemble_filtered_block(
        evm_config,
        &parent_header,
        &derived_block,
        execution_outcome.committed_transactions,
        &execution_outcome.execution_result,
        execution_outcome.finalized_block_zk_gas,
        state_root,
    )
    .map_err(map_block_execution_error)?;

    let outcome = FilteredBlockExecutionOutcome {
        filtered_block,
        execution_result: execution_outcome.execution_result,
        hashed_state: execution_outcome.hashed_state,
    };

    validate_block_consensus(chain_spec, &outcome.filtered_block, ancestor_headers)?;
    validate_block_post_execution(
        &outcome.filtered_block,
        chain_spec.as_ref(),
        &outcome.execution_result,
        None,
    )
    .map_err(StatelessValidationError::ConsensusValidationFailed)?;

    Ok(outcome)
}

#[allow(clippy::too_many_arguments)]
fn execute_block<T>(
    current_block: &RecoveredBlock<Block>,
    witness: &ExecutionWitness,
    shared_state_nodes: &[WitnessStateNode],
    callers: AddressMap<TrieAccount>,
    chain_spec: &TaikoChainSpec,
    evm_config: &TaikoEvmConfig,
    ancestor_hashes: AncestorHashes,
    pre_state_root: B256,
) -> Result<B256, StatelessValidationError>
where
    T: StatelessTrieExt,
{
    // First verify that the pre-state reads are correct
    let (mut trie, bytecode) = T::new_with_state_pool(witness, shared_state_nodes, pre_state_root)?;
    trie.append_callers(callers);

    // Create an in-memory database that will use the reads to validate the block
    let db = WitnessDatabase::new(&trie, bytecode, ancestor_hashes);

    // Execute the block
    let executor = evm_config.executor(db);
    let output = executor
        .execute(current_block)
        .map_err(|e| StatelessValidationError::StatelessExecutionFailed(e.to_string()))?;

    // Post validation checks
    validate_block_post_execution(current_block, chain_spec, &output, None)
        .map_err(StatelessValidationError::ConsensusValidationFailed)?;

    validate_anchor_transaction_in_block(current_block, chain_spec)
        .map_err(StatelessValidationError::ConsensusValidationFailed)?;

    // Compute and check the post state root
    let hashed_state = HashedPostState::from_bundle_state::<KeccakKeyHasher>(&output.state.state);
    let state_root = trie.calculate_state_root(hashed_state)?;
    if state_root != current_block.state_root {
        return Err(StatelessValidationError::PostStateRootMismatch {
            got: state_root,
            expected: current_block.state_root,
        });
    }

    // Return block hash
    Ok(current_block.hash())
}

// Performs stateless validation of a block using a custom `StatelessTrie` implementation.
//
// This is a generic version of `stateless_validation` that allows users to provide their own
// implementation of the `StatelessTrie` for custom trie backends or optimizations.
//
// See `stateless_validation` for detailed documentation of the validation process.
fn stateless_validation_with_trie<T>(
    current_block: Block,
    witness: &ExecutionWitness,
    ancestor_headers: &[WitnessHeader],
    shared_state_nodes: &[WitnessStateNode],
    callers: AddressMap<TrieAccount>,
    chain_spec: &Arc<TaikoChainSpec>,
    evm_config: &TaikoEvmConfig,
) -> Result<B256, StatelessValidationError>
where
    T: StatelessTrieExt,
{
    let current_block = decode_recovered_block(current_block)?;

    // Validate block against pre-execution consensus rules
    validate_block_consensus(chain_spec, &current_block, ancestor_headers)?;

    // Check that the ancestor headers form a contiguous chain and are not just random headers.
    let ancestor_hashes = compute_ancestor_hashes(&current_block, ancestor_headers)?;

    // Get the last ancestor header and retrieve its state root.
    //
    // There should be at least one ancestor header, this is because we need the parent header to
    // retrieve the previous state root.
    // The edge case here would be the genesis block, but we do not create proofs for the genesis
    // block.
    let pre_state_root = determine_pre_state_root(ancestor_headers)?;

    execute_block::<T>(
        &current_block,
        witness,
        shared_state_nodes,
        callers,
        chain_spec.as_ref(),
        evm_config,
        ancestor_hashes,
        pre_state_root,
    )
}

#[derive(Debug, Clone)]
struct WitnessTaikoBlockReader {
    timestamps_by_hash: Vec<(B256, u64)>,
}

impl WitnessTaikoBlockReader {
    fn from_headers(headers: &[WitnessHeader]) -> Self {
        let timestamps_by_hash = headers
            .iter()
            .map(|header| (header.hash, header.timestamp))
            .collect();

        Self { timestamps_by_hash }
    }
}

impl TaikoBlockReader for WitnessTaikoBlockReader {
    fn block_timestamp_by_hash(&self, hash: B256) -> Option<u64> {
        self.timestamps_by_hash
            .iter()
            .rev()
            .find_map(|(candidate, timestamp)| (*candidate == hash).then_some(*timestamp))
    }
}

pub(crate) fn validate_block_consensus(
    chain_spec: &Arc<TaikoChainSpec>,
    block: &RecoveredBlock<Block>,
    ancestor_headers: &[WitnessHeader],
) -> Result<(), StatelessValidationError> {
    ensure_full_ancestor_headers(ancestor_headers)?;

    let parent_header = ancestor_headers
        .last()
        .and_then(|header| {
            header
                .full_header()
                .cloned()
                .map(|full| SealedHeader::new(full, header.hash))
        })
        .ok_or(StatelessValidationError::MissingAncestorHeader)?;

    let block_reader = Arc::new(WitnessTaikoBlockReader::from_headers(ancestor_headers));
    let consensus = TaikoBeaconConsensus::new(chain_spec.clone(), block_reader);

    consensus.validate_header(block.sealed_header())?;

    consensus.validate_header_against_parent(block.sealed_header(), &parent_header)?;

    <TaikoBeaconConsensus as Consensus<Block>>::validate_body_against_header(
        &consensus,
        block.body(),
        block.sealed_header(),
    )?;

    validate_block_pre_execution(block, chain_spec.as_ref())?;

    consensus.validate_block_pre_execution(block)?;

    Ok(())
}

pub(crate) fn compute_ancestor_hashes(
    current_block: &RecoveredBlock<Block>,
    ancestor_headers: &[WitnessHeader],
) -> Result<AncestorHashes, StatelessValidationError> {
    compute_ancestor_hashes_for_child(
        current_block.number(),
        current_block.parent_hash(),
        ancestor_headers,
    )
}

fn compute_next_block_ancestor_hashes(
    ancestor_headers: &[WitnessHeader],
) -> Result<AncestorHashes, StatelessValidationError> {
    let parent_header = ancestor_headers
        .last()
        .ok_or(StatelessValidationError::MissingAncestorHeader)?;

    compute_ancestor_hashes_for_child(
        parent_header.number + 1,
        parent_header.hash,
        ancestor_headers,
    )
}

fn compute_ancestor_hashes_for_child(
    mut child_number: u64,
    mut child_parent_hash: B256,
    ancestor_headers: &[WitnessHeader],
) -> Result<AncestorHashes, StatelessValidationError> {
    ensure_full_ancestor_headers(ancestor_headers)?;

    for parent_header in ancestor_headers.iter().rev() {
        if child_parent_hash != parent_header.hash || parent_header.number + 1 != child_number {
            return Err(StatelessValidationError::InvalidAncestorChain);
        }

        child_number = parent_header.number;
        child_parent_hash = parent_header.parent_hash;
    }

    let Some(start_block_number) = ancestor_headers.first().map(|header| header.number) else {
        return Ok(AncestorHashes::default());
    };

    Ok(AncestorHashes::new(
        start_block_number,
        ancestor_headers.iter().map(|header| header.hash).collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{reconstruct_block_from_transactions_with_witness_resources, validate_block};
    use alethia_reth_block::config::TaikoEvmConfig;
    use alethia_reth_block::config::TaikoNextBlockEnvAttributes;
    use alethia_reth_chainspec::TAIKO_DEVNET;
    use alloy_consensus::{
        Header, SignableTransaction, TrieAccount, TxEip1559, constants::KECCAK_EMPTY, proofs,
        transaction::Recovered,
    };
    use alloy_primitives::{Address, Bytes, Signature, TxKind, U256, keccak256};
    use alloy_trie::EMPTY_ROOT_HASH;
    use raiko2_primitives::{
        ExecutionWitness, StatelessValidationError, WitnessHeader, WitnessStateNode,
    };
    use reth_consensus::ConsensusError;
    use reth_ethereum_primitives::{Block, BlockBody, TransactionSigned};
    use reth_primitives_traits::SignedTransaction;
    use risc0_ethereum_trie::Trie;

    fn empty_shanghai_body() -> BlockBody {
        BlockBody {
            withdrawals: Some(Default::default()),
            ..Default::default()
        }
    }

    fn shanghai_header(number: u64, timestamp: u64, parent_hash: alloy_primitives::B256) -> Header {
        let body = empty_shanghai_body();
        let mut header = Header {
            number,
            timestamp,
            parent_hash,
            gas_limit: 30_000_000,
            base_fee_per_gas: Some(1),
            ..Default::default()
        };

        header.transactions_root = proofs::calculate_transaction_root(body.transactions.as_slice());
        header.ommers_hash = body.calculate_ommers_root();
        header.withdrawals_root = body.calculate_withdrawals_root();

        header
    }

    fn witness_from_state_nodes(
        state_nodes: Vec<Bytes>,
        state_root: alloy_primitives::B256,
    ) -> ExecutionWitness {
        let header = Header {
            number: 0,
            state_root,
            ..Default::default()
        };
        ExecutionWitness {
            state: state_nodes
                .into_iter()
                .map(WitnessStateNode::from_bytes)
                .collect(),
            headers: vec![WitnessHeader::from_header(header)],
            ..Default::default()
        }
    }

    #[test]
    fn shasta_equal_timestamp_must_fail_consensus() {
        let chain_spec = TAIKO_DEVNET.clone();
        let evm_config = TaikoEvmConfig::new(chain_spec.clone());

        let parent_header = shanghai_header(0, 100, alloy_primitives::B256::ZERO);
        let parent_hash = parent_header.hash_slow();

        let header = shanghai_header(1, 100, parent_hash);
        let body = empty_shanghai_body();
        let block = Block { header, body };

        let witness = ExecutionWitness {
            headers: vec![WitnessHeader::from_header(parent_header.clone())],
            ..Default::default()
        };

        let callers = Default::default();
        let result = validate_block(block, &witness, callers, &chain_spec, &evm_config);

        // Shasta requires timestamps to strictly increase.
        assert!(matches!(
            result,
            Err(StatelessValidationError::ConsensusValidationFailed(_))
        ));
    }

    #[test]
    fn rejects_mismatched_transaction_root() -> Result<(), Box<dyn std::error::Error>> {
        let chain_spec = TAIKO_DEVNET.clone();
        let evm_config = TaikoEvmConfig::new(chain_spec.clone());

        let parent_header = shanghai_header(0, 100, alloy_primitives::B256::ZERO);
        let parent_hash = parent_header.hash_slow();

        let mut header = shanghai_header(1, 200, parent_hash);
        header.transactions_root = alloy_primitives::B256::ZERO;
        header.base_fee_per_gas = Some(25_000_000);

        let body = empty_shanghai_body();
        let block = Block { header, body };

        let witness = ExecutionWitness {
            headers: vec![WitnessHeader::from_header(parent_header.clone())],
            ..Default::default()
        };

        let callers = Default::default();
        let result = validate_block(block, &witness, callers, &chain_spec, &evm_config);

        match result {
            Err(StatelessValidationError::ConsensusValidationFailed(
                ConsensusError::BodyTransactionRootDiff(_),
            )) => Ok(()),
            other => Err(format!("unexpected result: {other:?}").into()),
        }
    }

    #[test]
    fn rejects_compact_ancestor_headers() {
        let chain_spec = TAIKO_DEVNET.clone();
        let evm_config = TaikoEvmConfig::new(chain_spec.clone());

        let parent_header = shanghai_header(0, 100, alloy_primitives::B256::ZERO);
        let parent_hash = parent_header.hash_slow();

        let header = shanghai_header(1, 200, parent_hash);
        let body = empty_shanghai_body();
        let block = Block { header, body };

        let witness = ExecutionWitness {
            headers: vec![WitnessHeader::from_header(parent_header).into_compact()],
            ..Default::default()
        };

        let result = validate_block(
            block,
            &witness,
            Default::default(),
            &chain_spec,
            &evm_config,
        );

        assert!(matches!(
            result,
            Err(StatelessValidationError::CompactAncestorHeaderUnsupported)
        ));
    }

    #[test]
    fn reconstruct_block_skips_invalid_nonce_transaction() {
        let chain_spec = TAIKO_DEVNET.clone();
        let evm_config = TaikoEvmConfig::new(chain_spec.clone());
        let candidate_tx: TransactionSigned = TxEip1559 {
            chain_id: chain_spec.inner.chain().id(),
            nonce: 2,
            gas_limit: 21_000,
            max_fee_per_gas: 25_000_000,
            max_priority_fee_per_gas: 0,
            to: TxKind::Call(Address::with_last_byte(0x22)),
            value: U256::ZERO,
            access_list: Default::default(),
            input: Bytes::new(),
        }
        .into_signed(Signature::test_signature())
        .into();
        let signer = candidate_tx
            .clone()
            .try_into_recovered()
            .expect("test signature should recover")
            .signer();
        let anchor_tx: TransactionSigned = TxEip1559 {
            chain_id: chain_spec.inner.chain().id(),
            nonce: 0,
            gas_limit: 21_000,
            max_fee_per_gas: 25_000_000,
            max_priority_fee_per_gas: 0,
            to: TxKind::Call(Address::with_last_byte(0x22)),
            value: U256::ZERO,
            access_list: Default::default(),
            input: Bytes::new(),
        }
        .into_signed(Signature::test_signature())
        .into();
        let anchor_tx = Recovered::new_unchecked(anchor_tx, signer);

        let mut trie = Trie::default();
        trie.insert(
            keccak256(signer),
            alloy_rlp::encode(TrieAccount {
                nonce: 0,
                balance: U256::from(1_000_000_000_000_000u64),
                storage_root: EMPTY_ROOT_HASH,
                code_hash: KECCAK_EMPTY,
            }),
        );

        let parent_header = Header {
            number: 0,
            timestamp: 100,
            gas_limit: 30_000_000,
            base_fee_per_gas: Some(25_000_000),
            state_root: trie.hash_slow(),
            ..Default::default()
        };
        let witness = witness_from_state_nodes(trie.rlp_nodes(), parent_header.state_root);

        let outcome = reconstruct_block_from_transactions_with_witness_resources(
            Some(anchor_tx),
            vec![candidate_tx],
            TaikoNextBlockEnvAttributes {
                timestamp: 101,
                suggested_fee_recipient: Address::ZERO,
                prev_randao: alloy_primitives::B256::ZERO,
                gas_limit: 30_000_000,
                extra_data: Bytes::new(),
                base_fee_per_gas: 25_000_000,
            },
            &witness,
            &witness.headers,
            &[],
            Default::default(),
            &chain_spec,
            &evm_config,
        )
        .expect("reconstruction should succeed while skipping invalid nonce tx");

        assert_eq!(outcome.filtered_block.body().transactions().count(), 1);
    }
}
