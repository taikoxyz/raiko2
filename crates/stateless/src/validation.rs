use crate::{
    sparse::SparseState,
    trie::StatelessTrieExt,
    witness_db::{AncestorHashes, WitnessDatabase},
};
use alethia_reth_block::config::TaikoEvmConfig;
use alethia_reth_chainspec::spec::TaikoChainSpec;
use alethia_reth_consensus::validation::{
    TaikoBeaconConsensus, TaikoBlockReader, validate_anchor_transaction_in_block,
};
use alloy_consensus::BlockHeader;
use alloy_consensus::TrieAccount;
use alloy_primitives::{B256, map::AddressMap};
use raiko2_primitives::{
    ExecutionWitness, StatelessValidationError, WitnessHeader, WitnessStateNode,
};
use reth_consensus::{Consensus, HeaderValidator};
use reth_consensus_common::validation::validate_block_pre_execution;
use reth_ethereum_consensus::validate_block_post_execution;
use reth_ethereum_primitives::Block;
use reth_evm::{ConfigureEvm, execute::Executor};
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

fn decode_recovered_block(block: Block) -> Result<RecoveredBlock<Block>, StatelessValidationError> {
    block
        .try_into_recovered()
        .map_err(|_| StatelessValidationError::SignerRecovery)
}

fn determine_pre_state_root(headers: &[WitnessHeader]) -> Result<B256, StatelessValidationError> {
    match headers.last() {
        Some(prev_header) => prev_header
            .full_header()
            .map(|header| header.state_root)
            .ok_or(StatelessValidationError::HeaderDeserializationFailed),
        None => Err(StatelessValidationError::MissingAncestorHeader),
    }
}

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
    validate_block_post_execution(
        current_block,
        chain_spec,
        &output.receipts,
        &output.requests,
        None,
    )
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

fn validate_block_consensus(
    chain_spec: &Arc<TaikoChainSpec>,
    block: &RecoveredBlock<Block>,
    ancestor_headers: &[WitnessHeader],
) -> Result<(), StatelessValidationError> {
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

fn compute_ancestor_hashes(
    current_block: &RecoveredBlock<Block>,
    ancestor_headers: &[WitnessHeader],
) -> Result<AncestorHashes, StatelessValidationError> {
    let mut child_number = current_block.number();
    let mut child_parent_hash = current_block.parent_hash();

    // Next verify that headers supplied are contiguous
    for parent_header in ancestor_headers.iter().rev() {
        if child_parent_hash != parent_header.hash || parent_header.number + 1 != child_number {
            return Err(StatelessValidationError::InvalidAncestorChain); // Blocks must be contiguous
        }

        child_number = parent_header.number;
        child_parent_hash = parent_header.parent_hash;
    }

    let Some(start_block_number) = ancestor_headers.first().map(|header| header.number) else {
        return Ok(AncestorHashes::default());
    };
    let hashes = ancestor_headers.iter().map(|header| header.hash).collect();

    Ok(AncestorHashes::new(start_block_number, hashes))
}

#[cfg(test)]
mod tests {
    use super::validate_block;
    use alethia_reth_block::config::TaikoEvmConfig;
    use alethia_reth_chainspec::TAIKO_DEVNET;
    use alloy_consensus::{Header, proofs};
    use raiko2_primitives::{ExecutionWitness, StatelessValidationError, WitnessHeader};
    use reth_consensus::ConsensusError;
    use reth_ethereum_primitives::{Block, BlockBody};

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
}
