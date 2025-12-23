use crate::{sparse::SparseState, trie::StatelessTrieExt, witness_db::WitnessDatabase};
use alethia_reth_block_core::config::TaikoEvmConfig;
use alethia_reth_chainspec_core::spec::TaikoChainSpec;
use alethia_reth_consensus_core::validation::{
    TaikoBeaconConsensus, TaikoBlockReader, validate_anchor_transaction_in_block,
};
use alloy_consensus::{BlockHeader, Header, TrieAccount};
use alloy_primitives::{B256, map::AddressMap};
use alloy_rlp::Decodable;
use reth_consensus::{Consensus, HeaderValidator};
use reth_consensus_common::validation::validate_block_pre_execution;
use reth_ethereum_consensus::validate_block_post_execution;
use reth_ethereum_primitives::Block;
use reth_evm::{ConfigureEvm, execute::Executor};
use reth_primitives_traits::{Block as _, RecoveredBlock, SealedHeader};
use reth_stateless::{ExecutionWitness, validation::StatelessValidationError};
use reth_trie_common::{HashedPostState, KeccakKeyHasher};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

/// Performs stateless validation of a block using the provided witness data.
#[inline]
pub fn validate_block(
    block: Block,
    witness: ExecutionWitness,
    callers: AddressMap<TrieAccount>,
    chain_spec: Arc<TaikoChainSpec>,
    config: TaikoEvmConfig,
) -> Result<B256, StatelessValidationError> {
    stateless_validation_with_trie::<SparseState>(block, witness, callers, chain_spec, config)
}

// Performs stateless validation of a block using a custom `StatelessTrie` implementation.
//
// This is a generic version of `stateless_validation` that allows users to provide their own
// implementation of the `StatelessTrie` for custom trie backends or optimizations.
//
// See `stateless_validation` for detailed documentation of the validation process.
fn stateless_validation_with_trie<T>(
    current_block: Block,
    witness: ExecutionWitness,
    callers: AddressMap<TrieAccount>,
    chain_spec: Arc<TaikoChainSpec>,
    evm_config: TaikoEvmConfig,
) -> Result<B256, StatelessValidationError>
where
    T: StatelessTrieExt,
{
    let current_block = current_block
        .try_into_recovered()
        .map_err(|_| StatelessValidationError::SignerRecovery)?;

    let mut ancestor_headers: Vec<Header> = witness
        .headers
        .iter()
        .map(|serialized_header| {
            let bytes = serialized_header.as_ref();
            Header::decode(&mut &bytes[..])
                .map_err(|_| StatelessValidationError::HeaderDeserializationFailed)
        })
        .collect::<Result<_, _>>()?;
    // Sort the headers by their block number to ensure that they are in
    // ascending order.
    ancestor_headers.sort_by_key(|header| header.number());

    // Validate block against pre-execution consensus rules
    validate_block_consensus(chain_spec.clone(), &current_block, &ancestor_headers)?;

    // Check that the ancestor headers form a contiguous chain and are not just random headers.
    let ancestor_hashes = compute_ancestor_hashes(&current_block, &ancestor_headers)?;

    // Get the last ancestor header and retrieve its state root.
    //
    // There should be at least one ancestor header, this is because we need the parent header to
    // retrieve the previous state root.
    // The edge case here would be the genesis block, but we do not create proofs for the genesis
    // block.
    let pre_state_root = match ancestor_headers.last() {
        Some(prev_header) => prev_header.state_root,
        None => return Err(StatelessValidationError::MissingAncestorHeader),
    };

    // First verify that the pre-state reads are correct
    let (mut trie, bytecode) = T::new(&witness, pre_state_root)?;
    trie.append_callers(callers);

    // Create an in-memory database that will use the reads to validate the block
    let db = WitnessDatabase::new(&trie, bytecode, ancestor_hashes);

    // Execute the block
    let executor = evm_config.executor(db);
    let output = executor
        .execute(&current_block)
        .map_err(|e| StatelessValidationError::StatelessExecutionFailed(e.to_string()))?;

    // Post validation checks
    validate_block_post_execution(
        &current_block,
        &chain_spec,
        &output.receipts,
        &output.requests,
    )
    .map_err(StatelessValidationError::ConsensusValidationFailed)?;

    validate_anchor_transaction_in_block(&current_block, chain_spec.as_ref())
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
    Ok(current_block.hash_slow())
}

#[derive(Debug, Clone)]
struct WitnessTaikoBlockReader {
    timestamps_by_hash: HashMap<B256, u64>,
}

impl WitnessTaikoBlockReader {
    fn from_headers(headers: &[Header]) -> Self {
        let timestamps_by_hash = headers
            .iter()
            .map(|header| (header.hash_slow(), header.timestamp()))
            .collect();

        Self { timestamps_by_hash }
    }
}

impl TaikoBlockReader for WitnessTaikoBlockReader {
    fn block_timestamp_by_hash(&self, hash: B256) -> Option<u64> {
        self.timestamps_by_hash.get(&hash).copied()
    }
}

fn validate_block_consensus(
    chain_spec: Arc<TaikoChainSpec>,
    block: &RecoveredBlock<Block>,
    ancestor_headers: &[Header],
) -> Result<(), StatelessValidationError> {
    let parent_header = match ancestor_headers.last() {
        Some(header) => header,
        None => return Err(StatelessValidationError::MissingAncestorHeader),
    };

    let parent_header = SealedHeader::seal_slow(parent_header.clone());

    let block_reader = Arc::new(WitnessTaikoBlockReader::from_headers(ancestor_headers));
    let consensus_chain_spec = chain_spec.clone();
    let consensus = TaikoBeaconConsensus::new(consensus_chain_spec, block_reader);

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
    ancestor_headers: &[Header],
) -> Result<BTreeMap<u64, B256>, StatelessValidationError> {
    let mut ancestor_hashes = BTreeMap::new();

    let mut child_header = current_block.header();

    // Next verify that headers supplied are contiguous
    for parent_header in ancestor_headers.iter().rev() {
        let parent_hash = child_header.parent_hash();
        ancestor_hashes.insert(parent_header.number, parent_hash);

        if parent_hash != parent_header.hash_slow() {
            return Err(StatelessValidationError::InvalidAncestorChain); // Blocks must be contiguous
        }

        if parent_header.number + 1 != child_header.number {
            return Err(StatelessValidationError::InvalidAncestorChain); // Header number should be
            // contiguous
        }

        child_header = parent_header
    }

    Ok(ancestor_hashes)
}

#[cfg(test)]
mod tests {
    use super::validate_block;
    use alethia_reth_block_core::config::TaikoEvmConfig;
    use alethia_reth_chainspec_core::TAIKO_DEVNET;
    use alloy_consensus::{Header, proofs};
    use alloy_primitives::Bytes;
    use reth_consensus::ConsensusError;
    use reth_ethereum_primitives::{Block, BlockBody};
    use reth_stateless::{ExecutionWitness, validation::StatelessValidationError};

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
            headers: vec![Bytes::from(alloy_rlp::encode(&parent_header))],
            ..Default::default()
        };

        let callers = Default::default();
        let result = validate_block(block, witness, callers, chain_spec, evm_config);

        // Shasta requires timestamps to strictly increase.
        assert!(matches!(
            result,
            Err(StatelessValidationError::ConsensusValidationFailed(_))
        ));
    }

    #[test]
    fn rejects_mismatched_transaction_root() {
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
            headers: vec![Bytes::from(alloy_rlp::encode(&parent_header))],
            ..Default::default()
        };

        let callers = Default::default();
        let result = validate_block(block, witness, callers, chain_spec, evm_config);

        match result {
            Err(StatelessValidationError::ConsensusValidationFailed(
                ConsensusError::BodyTransactionRootDiff(_),
            )) => {}
            other => panic!("unexpected result: {other:?}"),
        }
    }
}
