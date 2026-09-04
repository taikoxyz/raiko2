use crate::{
    sparse::SparseState,
    trie::StatelessTrie,
    validation::{
        compute_ancestor_hashes, decode_recovered_block, determine_pre_state_root,
        validate_block_consensus,
    },
    witness_db::WitnessDatabase,
};
use alethia_reth_block::config::TaikoEvmConfig;
use alethia_reth_chainspec::spec::TaikoChainSpec;
use alloy_primitives::B256;
use raiko2_primitives::{
    ExecutionWitness, StatelessValidationError, WitnessHeader, WitnessStateNode,
};
use reth_ethereum_primitives::Block;
use reth_evm::{ConfigureEvm, execute::Executor};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WitnessMaterializationStats {
    pub supplied_state_nodes: usize,
    pub pre_state_root: B256,
    pub state_trie_nodes: usize,
    pub storage_trie_count: usize,
    pub storage_trie_nodes: usize,
    pub materialized_state_nodes: Vec<WitnessStateNode>,
}

impl WitnessMaterializationStats {
    #[must_use]
    pub const fn total_materialized_nodes(&self) -> usize {
        self.materialized_state_nodes.len()
    }

    #[must_use]
    pub const fn unused_supplied_nodes(&self) -> usize {
        self.supplied_state_nodes
            .saturating_sub(self.total_materialized_nodes())
    }
}

/// Analyze witness materialization costs for a single block validation run.
///
/// # Errors
///
/// Returns `StatelessValidationError` when block decoding, consensus validation, witness
/// materialization, or stateless execution fails.
pub fn analyze_block_with_witness_resources(
    block: Block,
    witness: &ExecutionWitness,
    ancestor_headers: &[WitnessHeader],
    shared_state_nodes: &[WitnessStateNode],
    chain_spec: &Arc<TaikoChainSpec>,
    evm_config: &TaikoEvmConfig,
) -> Result<WitnessMaterializationStats, StatelessValidationError> {
    let current_block = decode_recovered_block(block)?;
    validate_block_consensus(chain_spec, &current_block, ancestor_headers)?;
    let ancestor_hashes = compute_ancestor_hashes(&current_block, ancestor_headers)?;
    let pre_state_root = determine_pre_state_root(ancestor_headers)?;

    let supplied_state_nodes = if witness.state_indices.is_empty() {
        witness.state.len()
    } else {
        witness.state_indices.len()
    };

    let (trie, bytecode) =
        SparseState::new_with_state_pool(witness, shared_state_nodes, pre_state_root)?;
    let db = WitnessDatabase::new(&trie, bytecode, ancestor_hashes);
    let executor = evm_config.executor(db);
    let _output = executor
        .execute(&current_block)
        .map_err(|err| StatelessValidationError::StatelessExecutionFailed(err.to_string()))?;

    Ok(WitnessMaterializationStats {
        supplied_state_nodes,
        pre_state_root,
        state_trie_nodes: trie.state_node_count(),
        storage_trie_count: trie.storage_trie_count(),
        storage_trie_nodes: trie.storage_node_count(),
        materialized_state_nodes: trie.materialized_witness_state_nodes(),
    })
}
