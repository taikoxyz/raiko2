use raiko2_primitives::{RaikoError, RaikoResult};
use raiko2_primitives_shasta::GuestInput;
use raiko2_provider::{RpcClientConfig, fetch_l2_blocks};
use reth_ethereum_primitives::Block as RethBlock;
use std::collections::HashMap;
use tracing::info;

/// Compare preflight carry-data checkpoint fields against blocks fetched from an external L2 RPC.
///
/// # Errors
///
/// Returns an error when witness blocks are missing, the RPC response is incomplete, or any
/// checkpoint or parent-block field disagrees with the preflight guest input.
pub fn compare_guest_input_checkpoint_against_l2_blocks(
    input: &GuestInput,
    blocks: &[RethBlock],
) -> RaikoResult<()> {
    let first_witness = input.witnesses.first().ok_or_else(|| {
        RaikoError::Preflight(
            "cannot verify checkpoint against L2 RPC without witnesses".to_string(),
        )
    })?;
    let last_witness = input.witnesses.last().ok_or_else(|| {
        RaikoError::Preflight(
            "cannot verify checkpoint against L2 RPC without witnesses".to_string(),
        )
    })?;
    let first_block_number = first_witness.block.header.number;
    let last_block_number = last_witness.block.header.number;

    let blocks_by_number = blocks
        .iter()
        .map(|block| (block.header.number, block))
        .collect::<HashMap<_, _>>();
    let rpc_first = blocks_by_number.get(&first_block_number).ok_or_else(|| {
        RaikoError::Preflight(format!(
            "external L2 RPC did not return proposal start block {first_block_number}"
        ))
    })?;
    let rpc_last = blocks_by_number.get(&last_block_number).ok_or_else(|| {
        RaikoError::Preflight(format!(
            "external L2 RPC did not return proposal end block {last_block_number}"
        ))
    })?;

    let carry = &input.proof_carry_data.transition_input;
    if rpc_first.header.parent_hash != carry.parent_block_hash {
        return Err(RaikoError::Preflight(format!(
            "external L2 RPC parent block hash mismatch at block {first_block_number}: rpc={:#x}, preflight={:#x}",
            rpc_first.header.parent_hash, carry.parent_block_hash
        )));
    }

    let expected_checkpoint = &carry.checkpoint;
    let expected_block_number = expected_checkpoint.blockNumber.to::<u64>();
    if rpc_last.header.number != expected_block_number {
        return Err(RaikoError::Preflight(format!(
            "external L2 RPC checkpoint block number mismatch: rpc={}, preflight={expected_block_number}",
            rpc_last.header.number
        )));
    }
    let rpc_last_hash = rpc_last.header.hash_slow();
    if rpc_last_hash != expected_checkpoint.blockHash {
        return Err(RaikoError::Preflight(format!(
            "external L2 RPC checkpoint block hash mismatch at block {expected_block_number}: rpc={rpc_last_hash:#x}, preflight={:#x}",
            expected_checkpoint.blockHash
        )));
    }
    if rpc_last.header.state_root != expected_checkpoint.stateRoot {
        return Err(RaikoError::Preflight(format!(
            "external L2 RPC checkpoint state root mismatch at block {expected_block_number}: rpc={:#x}, preflight={:#x}",
            rpc_last.header.state_root, expected_checkpoint.stateRoot
        )));
    }

    Ok(())
}

/// Fetch proposal boundary blocks from an external L2 RPC and compare them with preflight output.
///
/// # Errors
///
/// Returns an error when the RPC request fails or checkpoint/parent-block fields disagree.
pub async fn verify_guest_input_checkpoint_against_l2_rpc(
    input: &GuestInput,
    l2_rpc_url: &str,
    rpc_config: &RpcClientConfig,
) -> RaikoResult<()> {
    let first_block_number = input
        .witnesses
        .first()
        .ok_or_else(|| {
            RaikoError::Preflight(
                "cannot verify checkpoint against L2 RPC without witnesses".to_string(),
            )
        })?
        .block
        .header
        .number;
    let last_block_number = input
        .witnesses
        .last()
        .ok_or_else(|| {
            RaikoError::Preflight(
                "cannot verify checkpoint against L2 RPC without witnesses".to_string(),
            )
        })?
        .block
        .header
        .number;

    let blocks = fetch_l2_blocks(
        l2_rpc_url,
        &[first_block_number, last_block_number],
        rpc_config,
    )
    .await
    .map_err(|err| {
        RaikoError::Preflight(format!(
            "failed to fetch proposal boundary blocks from external L2 RPC {l2_rpc_url}: {err}"
        ))
    })?;
    compare_guest_input_checkpoint_against_l2_blocks(input, &blocks)?;
    info!(
        l2_rpc_url,
        first_block_number,
        last_block_number,
        "verified preflight checkpoint against external L2 RPC"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::Header;
    use alloy_primitives::B256;
    use raiko2_primitives::{ChainSpec, ExecutionWitness, StatelessInput};
    use raiko2_protocol_shasta::TaikoManifest;
    use raiko2_protocol_shasta::shasta::{Checkpoint, ProofCarryData, TransitionInputData};

    fn sample_guest_input(first_parent_hash: B256) -> GuestInput {
        let mut first = StatelessInput {
            block: reth_ethereum_primitives::Block {
                header: Header {
                    number: 10,
                    parent_hash: first_parent_hash,
                    state_root: B256::from([0x11; 32]),
                    ..Default::default()
                },
                ..Default::default()
            },
            witness: ExecutionWitness::default(),
            accounts: Default::default(),
            chain_spec: ChainSpec {
                chain_id: 167_013,
                is_taiko: true,
                ..Default::default()
            },
        };
        first.block.header.hash_slow();

        let mut last = first.clone();
        last.block.header.number = 12;
        last.block.header.parent_hash = first.block.header.hash_slow();
        last.block.header.state_root = B256::from([0x22; 32]);
        last.block.header.hash_slow();

        GuestInput {
            taiko: TaikoManifest::default(),
            witnesses: vec![first, last.clone()],
            proof_carry_data: ProofCarryData {
                transition_input: TransitionInputData {
                    parent_block_hash: first_parent_hash,
                    checkpoint: Checkpoint {
                        blockNumber: 12u64.try_into().expect("fits in uint48"),
                        blockHash: last.block.header.hash_slow(),
                        stateRoot: last.block.header.state_root,
                    },
                    ..Default::default()
                },
                ..Default::default()
            },
            proposal_ancestor_headers: Vec::new(),
            proposal_state_nodes: Vec::new(),
        }
    }

    fn block_from_witness(witness: &StatelessInput) -> RethBlock {
        witness.block.clone()
    }

    #[test]
    fn compare_accepts_matching_boundary_blocks() {
        let parent_hash = B256::from([0xAA; 32]);
        let input = sample_guest_input(parent_hash);
        let blocks = vec![
            block_from_witness(&input.witnesses[0]),
            block_from_witness(&input.witnesses[1]),
        ];

        compare_guest_input_checkpoint_against_l2_blocks(&input, &blocks).expect("match");
    }

    #[test]
    fn compare_rejects_checkpoint_hash_mismatch() {
        let parent_hash = B256::from([0xAA; 32]);
        let input = sample_guest_input(parent_hash);
        let mut mismatched_last = block_from_witness(&input.witnesses[1]);
        mismatched_last.header.state_root = B256::from([0x99; 32]);
        let blocks = vec![block_from_witness(&input.witnesses[0]), mismatched_last];

        let err = compare_guest_input_checkpoint_against_l2_blocks(&input, &blocks)
            .expect_err("hash mismatch");
        assert!(err.to_string().contains("checkpoint block hash mismatch"));
    }

    #[test]
    fn compare_rejects_parent_block_hash_mismatch() {
        let parent_hash = B256::from([0xAA; 32]);
        let input = sample_guest_input(parent_hash);
        let mut mismatched_first = block_from_witness(&input.witnesses[0]);
        mismatched_first.header.parent_hash = B256::from([0x99; 32]);
        let blocks = vec![mismatched_first, block_from_witness(&input.witnesses[1])];

        let err = compare_guest_input_checkpoint_against_l2_blocks(&input, &blocks)
            .expect_err("parent mismatch");
        assert!(err.to_string().contains("parent block hash mismatch"));
    }
}
