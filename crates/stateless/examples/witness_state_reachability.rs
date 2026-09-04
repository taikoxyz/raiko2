#![allow(missing_docs)]

use alethia_reth_block::config::TaikoEvmConfig;
use alloy_primitives::{B256, Bytes, map::B256Map};
use raiko2_primitives::{ExecutionWitness, WitnessHeader, WitnessStateNode};
use raiko2_primitives_shasta::{GuestInput, roll_proposal_ancestor_headers_in_place};
use raiko2_stateless::analyze_block_with_witness_resources;
use risc0_ethereum_trie::Trie;
use std::{env, error::Error, fs, path::PathBuf};

#[derive(Debug, Clone)]
struct Row {
    block_number: u64,
    supplied_nodes: usize,
    state_trie_nodes: usize,
    materialized_nodes: usize,
    storage_trie_count: usize,
    storage_trie_nodes: usize,
}

fn parse_input_path() -> Result<PathBuf, Box<dyn Error>> {
    let path = env::args().nth(1).ok_or(
        "usage: cargo run -p raiko2-stateless --example witness_state_reachability -- <input.json>",
    )?;
    Ok(PathBuf::from(path))
}

fn resolve_state_nodes<'a>(
    witness: &'a ExecutionWitness,
    shared_state_nodes: &'a [WitnessStateNode],
) -> Result<B256Map<&'a Bytes>, Box<dyn Error>> {
    if witness.state_indices.is_empty() {
        return Ok(witness
            .state
            .iter()
            .map(|node| (node.hash, &node.bytes))
            .collect());
    }

    witness
        .state_indices
        .iter()
        .map(|index| {
            let node = shared_state_nodes.get(*index as usize).ok_or_else(|| {
                format!(
                    "shared witness state index {} out of bounds for pool length {}",
                    index,
                    shared_state_nodes.len()
                )
            })?;
            Ok((node.hash, &node.bytes))
        })
        .collect()
}

fn determine_pre_state_root(headers: &[WitnessHeader]) -> Result<B256, Box<dyn Error>> {
    headers
        .last()
        .and_then(WitnessHeader::full_header)
        .map(|header| header.state_root)
        .ok_or_else(|| "missing parent header for pre-state root".into())
}

fn build_rows(input: &GuestInput) -> Result<Vec<Row>, Box<dyn Error>> {
    let shared_state_nodes = input.proposal_state_nodes();
    let mut ancestor_headers = input.initial_proposal_ancestor_headers();
    let mut rows = Vec::with_capacity(input.witnesses.len());
    let chain_spec = input
        .witnesses
        .first()
        .ok_or("guest input must contain at least one witness")?
        .chain_spec
        .to_taiko_chain_spec()?;
    let evm_config = TaikoEvmConfig::new(chain_spec.clone());

    for stateless_input in &input.witnesses {
        let supplied_nodes = if stateless_input.witness.state_indices.is_empty() {
            stateless_input.witness.state.len()
        } else {
            stateless_input.witness.state_indices.len()
        };
        let pre_state_root = determine_pre_state_root(&ancestor_headers)?;
        let rlp_by_digest = resolve_state_nodes(&stateless_input.witness, shared_state_nodes)?;
        let state_trie = Trie::from_prehashed_nodes(pre_state_root, &rlp_by_digest)?;
        let materialized = analyze_block_with_witness_resources(
            stateless_input.block.clone(),
            &stateless_input.witness,
            &ancestor_headers,
            shared_state_nodes,
            &chain_spec,
            &evm_config,
        )?;

        rows.push(Row {
            block_number: stateless_input.block.header.number,
            supplied_nodes,
            state_trie_nodes: state_trie.size(),
            materialized_nodes: materialized.total_materialized_nodes(),
            storage_trie_count: materialized.storage_trie_count,
            storage_trie_nodes: materialized.storage_trie_nodes,
        });

        roll_proposal_ancestor_headers_in_place(
            &mut ancestor_headers,
            &stateless_input.block.header,
            stateless_input.block.header.hash_slow(),
        );
    }

    Ok(rows)
}

fn print_summary(rows: &[Row], proposal_pool_nodes: usize) {
    let block_count = rows.len();
    let total_supplied_nodes = rows.iter().map(|row| row.supplied_nodes).sum::<usize>();
    let total_state_trie_nodes = rows.iter().map(|row| row.state_trie_nodes).sum::<usize>();
    let total_materialized_nodes = rows.iter().map(|row| row.materialized_nodes).sum::<usize>();
    let total_storage_trie_nodes = rows.iter().map(|row| row.storage_trie_nodes).sum::<usize>();
    let total_storage_tries = rows.iter().map(|row| row.storage_trie_count).sum::<usize>();
    let total_unused_nodes = total_supplied_nodes.saturating_sub(total_materialized_nodes);
    let state_trie_ratio = if total_supplied_nodes == 0 {
        1.0
    } else {
        total_state_trie_nodes as f64 / total_supplied_nodes as f64
    };
    let materialized_ratio = if total_supplied_nodes == 0 {
        1.0
    } else {
        total_materialized_nodes as f64 / total_supplied_nodes as f64
    };

    println!("blocks={block_count}");
    println!("proposal_state_pool_nodes={proposal_pool_nodes}");
    println!("total_supplied_state_nodes={total_supplied_nodes}");
    println!("total_state_trie_nodes={total_state_trie_nodes}");
    println!("total_materialized_state_nodes={total_materialized_nodes}");
    println!("total_storage_trie_nodes={total_storage_trie_nodes}");
    println!("total_storage_tries={total_storage_tries}");
    println!("total_unused_state_nodes={total_unused_nodes}");
    println!("state_trie_ratio={state_trie_ratio:.6}");
    println!("materialized_ratio={materialized_ratio:.6}");
    println!(
        "avg_supplied_state_nodes_per_block={:.2}",
        total_supplied_nodes as f64 / block_count as f64
    );
    println!(
        "avg_state_trie_nodes_per_block={:.2}",
        total_state_trie_nodes as f64 / block_count as f64
    );
    println!(
        "avg_materialized_state_nodes_per_block={:.2}",
        total_materialized_nodes as f64 / block_count as f64
    );

    let mut worst_rows = rows.to_vec();
    worst_rows.sort_by(|lhs, rhs| {
        rhs.supplied_nodes
            .saturating_sub(rhs.materialized_nodes)
            .cmp(&lhs.supplied_nodes.saturating_sub(lhs.materialized_nodes))
            .then_with(|| rhs.supplied_nodes.cmp(&lhs.supplied_nodes))
            .then_with(|| lhs.block_number.cmp(&rhs.block_number))
    });
    for row in worst_rows.into_iter().take(10) {
        let unused_nodes = row.supplied_nodes.saturating_sub(row.materialized_nodes);
        println!(
            "block_state_reachability block={} supplied={} state_trie={} materialized={} unused={} storage_tries={} storage_nodes={} materialized_ratio={:.6}",
            row.block_number,
            row.supplied_nodes,
            row.state_trie_nodes,
            row.materialized_nodes,
            unused_nodes,
            row.storage_trie_count,
            row.storage_trie_nodes,
            if row.supplied_nodes == 0 {
                1.0
            } else {
                row.materialized_nodes as f64 / row.supplied_nodes as f64
            }
        );
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let input_path = parse_input_path()?;
    let input_bytes = fs::read_to_string(&input_path)?;
    let input: GuestInput = serde_json::from_str(&input_bytes)?;
    let rows = build_rows(&input)?;

    print_summary(&rows, input.proposal_state_nodes().len());
    Ok(())
}
