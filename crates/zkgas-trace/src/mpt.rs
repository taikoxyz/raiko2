#![allow(clippy::redundant_pub_crate)]

use alethia_reth_evm::zk_gas::pricing::MptSelectors;
use raiko2_stateless::{MptNodeKindCounts, MptWorkloadMetrics};

use crate::RuntimeQuoteError;

fn checked_add(left: u64, right: u64) -> Result<u64, RuntimeQuoteError> {
    left.checked_add(right)
        .ok_or(RuntimeQuoteError::ArithmeticOverflow)
}

fn node_count(nodes: MptNodeKindCounts) -> Result<u64, RuntimeQuoteError> {
    checked_add(
        checked_add(nodes.null, nodes.branch)?,
        checked_add(nodes.extension, nodes.leaf)?,
    )
}

pub(crate) fn selectors(metrics: &MptWorkloadMetrics) -> Result<MptSelectors, RuntimeQuoteError> {
    let mutations = [
        &metrics.account_inserts,
        &metrics.account_updates,
        &metrics.account_deletes,
        &metrics.storage_inserts,
        &metrics.storage_updates,
        &metrics.storage_deletes,
    ];
    let mut path_node_visits = checked_add(
        metrics.account_reads.node_visits,
        metrics.storage_reads.node_visits,
    )?;
    let mut structural_branch_extension_churn_nodes = 0_u64;
    for mutation in mutations {
        path_node_visits = checked_add(path_node_visits, mutation.pre_mutation_node_visits)?;
        structural_branch_extension_churn_nodes = checked_add(
            structural_branch_extension_churn_nodes,
            checked_add(
                checked_add(
                    mutation.created_nodes.branch,
                    mutation.created_nodes.extension,
                )?,
                checked_add(
                    mutation.removed_nodes.branch,
                    mutation.removed_nodes.extension,
                )?,
            )?,
        )?;
    }

    let mut materialization_work_units = 0_u64;
    for materialization in [
        &metrics.state_materializations,
        &metrics.storage_materializations,
    ] {
        materialization_work_units = checked_add(
            materialization_work_units,
            checked_add(
                node_count(materialization.decoded_nodes)?,
                checked_add(
                    materialization.resolved_digest_lookups,
                    materialization.unresolved_digest_lookups,
                )?,
            )?,
        )?;
    }

    Ok(MptSelectors {
        path_node_visits,
        materialization_work_units,
        hydration_hash_blocks: metrics.witness_hydration.hash_input_blocks,
        dirty_encoded_nodes: checked_add(
            metrics.account_hashes.dirty_encoded_nodes,
            metrics.storage_hashes.dirty_encoded_nodes,
        )?,
        dirty_hash_blocks: checked_add(
            metrics.account_hashes.hash_input_blocks,
            metrics.storage_hashes.hash_input_blocks,
        )?,
        structural_branch_extension_churn_nodes,
        dropped_trie_nodes: node_count(metrics.materialized_storage_drop_nodes)?,
    })
}

pub(crate) fn add(
    left: MptSelectors,
    right: MptSelectors,
) -> Result<MptSelectors, RuntimeQuoteError> {
    Ok(MptSelectors {
        path_node_visits: checked_add(left.path_node_visits, right.path_node_visits)?,
        materialization_work_units: checked_add(
            left.materialization_work_units,
            right.materialization_work_units,
        )?,
        hydration_hash_blocks: checked_add(
            left.hydration_hash_blocks,
            right.hydration_hash_blocks,
        )?,
        dirty_encoded_nodes: checked_add(left.dirty_encoded_nodes, right.dirty_encoded_nodes)?,
        dirty_hash_blocks: checked_add(left.dirty_hash_blocks, right.dirty_hash_blocks)?,
        structural_branch_extension_churn_nodes: checked_add(
            left.structural_branch_extension_churn_nodes,
            right.structural_branch_extension_churn_nodes,
        )?,
        dropped_trie_nodes: checked_add(left.dropped_trie_nodes, right.dropped_trie_nodes)?,
    })
}
