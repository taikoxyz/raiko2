use alloy_consensus::BlockHeader;
use alloy_rpc_types_eth::Header as AlloyHeader;
use raiko2_primitives::{RaikoError, RaikoResult};
use raiko2_primitives_shasta::GuestInput;
use raiko2_provider::{RpcClientConfig, fetch_l2_headers};
use std::collections::HashMap;
use tracing::info;
use url::Url;

/// Compare preflight carry-data checkpoint fields against blocks fetched from an external L2 RPC.
///
/// # Errors
///
/// Returns an error when witness blocks are missing, the RPC response is incomplete, or any
/// checkpoint or parent-block field disagrees with the preflight guest input.
pub fn compare_guest_input_checkpoint_against_l2_blocks(
    input: &GuestInput,
    blocks: &[AlloyHeader],
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
        .map(|block| (block.number(), block))
        .collect::<HashMap<u64, &AlloyHeader>>();
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
    if first_block_number == 0
        && (rpc_first.parent_hash() != alloy_primitives::B256::ZERO
            || carry.parent_block_hash != alloy_primitives::B256::ZERO)
    {
        return Err(RaikoError::Preflight(format!(
            "external L2 RPC genesis parent block hash must be zero: rpc={:#x}, preflight={:#x}",
            rpc_first.parent_hash(),
            carry.parent_block_hash
        )));
    }
    if rpc_first.parent_hash() != carry.parent_block_hash {
        return Err(RaikoError::Preflight(format!(
            "external L2 RPC parent block hash mismatch at block {first_block_number}: rpc={:#x}, preflight={:#x}",
            rpc_first.parent_hash(),
            carry.parent_block_hash
        )));
    }
    // The check above compares an RPC-controlled field against the carry value, which an RPC can
    // simply echo. Bind the start boundary to a header preimage as well: recompute the parent
    // block's hash from its returned header content and require it to equal the carry value. For a
    // block 0 start, the canonical zero parent hash is enforced above instead.
    if let Some(parent_block_number) = first_block_number.checked_sub(1) {
        let rpc_parent = blocks_by_number.get(&parent_block_number).ok_or_else(|| {
            RaikoError::Preflight(format!(
                "external L2 RPC did not return proposal parent block {parent_block_number}"
            ))
        })?;
        let rpc_parent_hash = rpc_parent.inner.hash_slow();
        if rpc_parent_hash != carry.parent_block_hash {
            return Err(RaikoError::Preflight(format!(
                "external L2 RPC parent block hash preimage mismatch at block {parent_block_number}: rpc={rpc_parent_hash:#x}, preflight={:#x}",
                carry.parent_block_hash
            )));
        }
    }

    let expected_checkpoint = &carry.checkpoint;
    let expected_block_number = expected_checkpoint.blockNumber.to::<u64>();
    if rpc_last.number() != expected_block_number {
        return Err(RaikoError::Preflight(format!(
            "external L2 RPC checkpoint block number mismatch: rpc={}, preflight={expected_block_number}",
            rpc_last.number()
        )));
    }
    // Recompute the hash from the returned header content instead of trusting the RPC-reported
    // `hash` field, so a spoofed field cannot force a spurious pass.
    let rpc_last_hash = rpc_last.inner.hash_slow();
    if rpc_last_hash != expected_checkpoint.blockHash {
        return Err(RaikoError::Preflight(format!(
            "external L2 RPC checkpoint block hash mismatch at block {expected_block_number}: rpc={rpc_last_hash:#x}, preflight={:#x}",
            expected_checkpoint.blockHash
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
    let l2_rpc_endpoint = redact_l2_rpc_url(l2_rpc_url);
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

    let mut boundary_block_numbers = Vec::with_capacity(3);
    if let Some(parent_block_number) = first_block_number.checked_sub(1) {
        boundary_block_numbers.push(parent_block_number);
    }
    boundary_block_numbers.push(first_block_number);
    boundary_block_numbers.push(last_block_number);
    let blocks = fetch_l2_headers(l2_rpc_url, &boundary_block_numbers, rpc_config)
        .await
    .map_err(|err| {
        RaikoError::Preflight(format!(
            "failed to fetch proposal boundary blocks from external L2 RPC endpoint {l2_rpc_endpoint}: {err}"
        ))
    })?;
    compare_guest_input_checkpoint_against_l2_blocks(input, &blocks)?;
    info!(
        l2_rpc_endpoint,
        first_block_number,
        last_block_number,
        "verified preflight checkpoint against external L2 RPC"
    );
    Ok(())
}

fn redact_l2_rpc_url(l2_rpc_url: &str) -> String {
    match Url::parse(l2_rpc_url) {
        Ok(url) => match (url.host_str(), url.port()) {
            (Some(host), Some(port)) => format!("{}://{host}:{port}", url.scheme()),
            (Some(host), None) => format!("{}://{host}", url.scheme()),
            (None, _) => "<redacted-invalid-host>".to_string(),
        },
        Err(_) => "<redacted-invalid-url>".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::Header;
    use alloy_primitives::B256;
    use raiko2_primitives::{ChainSpec, ExecutionWitness, StatelessInput};
    use raiko2_protocol_shasta::TaikoManifest;
    use raiko2_protocol_shasta::shasta::{Checkpoint, ProofCarryData, TransitionInputData};

    /// Header whose recomputed hash the happy-path fixtures use as the carry parent hash, so the
    /// start-boundary preimage check can pass against it.
    fn sample_parent_header() -> Header {
        Header {
            number: 9,
            state_root: B256::from([0x33; 32]),
            ..Default::default()
        }
    }

    fn sample_guest_input(first_parent_hash: B256) -> GuestInput {
        let first = StatelessInput {
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
            accounts: std::collections::HashMap::default(),
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

    fn block_from_witness(witness: &StatelessInput) -> AlloyHeader {
        AlloyHeader::new(witness.block.header.clone())
    }

    #[test]
    fn compare_accepts_matching_boundary_blocks() {
        let parent = sample_parent_header();
        let input = sample_guest_input(parent.hash_slow());
        let blocks = vec![
            AlloyHeader::new(parent),
            block_from_witness(&input.witnesses[0]),
            block_from_witness(&input.witnesses[1]),
        ];

        compare_guest_input_checkpoint_against_l2_blocks(&input, &blocks).expect("match");
    }

    #[test]
    fn compare_rejects_checkpoint_hash_mismatch() {
        let parent = sample_parent_header();
        let input = sample_guest_input(parent.hash_slow());
        let mut mismatched_last = block_from_witness(&input.witnesses[1]);
        // Forge the header CONTENT (keeping the block number correct) so the recomputed hash
        // diverges from the preflight checkpoint hash.
        mismatched_last.inner.state_root = B256::from([0x99; 32]);
        let blocks = vec![
            AlloyHeader::new(parent),
            block_from_witness(&input.witnesses[0]),
            mismatched_last,
        ];

        let err = compare_guest_input_checkpoint_against_l2_blocks(&input, &blocks)
            .expect_err("hash mismatch");
        assert!(err.to_string().contains("checkpoint block hash mismatch"));
    }

    #[test]
    fn compare_ignores_forged_reported_hash_field() {
        let parent = sample_parent_header();
        let input = sample_guest_input(parent.hash_slow());
        let mut forged_last = block_from_witness(&input.witnesses[1]);
        // Only the RPC-reported `hash` field is forged; the header content stays correct. The
        // comparison must still pass because the hash is recomputed from the content, i.e. the
        // reported field is not load-bearing.
        forged_last.hash = B256::from([0x99; 32]);
        let blocks = vec![
            AlloyHeader::new(parent),
            block_from_witness(&input.witnesses[0]),
            forged_last,
        ];

        compare_guest_input_checkpoint_against_l2_blocks(&input, &blocks)
            .expect("forged reported hash field must not affect the content-derived comparison");
    }

    #[test]
    fn compare_rejects_parent_block_hash_mismatch() {
        let parent_hash = B256::from([0xAA; 32]);
        let input = sample_guest_input(parent_hash);
        let mut mismatched_first = block_from_witness(&input.witnesses[0]);
        mismatched_first.parent_hash = B256::from([0x99; 32]);
        let blocks = vec![mismatched_first, block_from_witness(&input.witnesses[1])];

        let err = compare_guest_input_checkpoint_against_l2_blocks(&input, &blocks)
            .expect_err("parent mismatch");
        assert!(err.to_string().contains("parent block hash mismatch"));
    }

    #[test]
    fn compare_rejects_forged_parent_preimage() {
        // The carry parent hash has no known preimage; a malicious RPC echoes it in the first
        // header's parentHash field (which the fixture already does) and returns a synthetic
        // parent block, but cannot make that parent's content hash to the carry value.
        let input = sample_guest_input(B256::from([0xAA; 32]));
        let synthetic_parent = AlloyHeader::new(sample_parent_header());
        let blocks = vec![
            synthetic_parent,
            block_from_witness(&input.witnesses[0]),
            block_from_witness(&input.witnesses[1]),
        ];

        let err = compare_guest_input_checkpoint_against_l2_blocks(&input, &blocks)
            .expect_err("forged parent preimage");
        assert!(
            err.to_string()
                .contains("parent block hash preimage mismatch")
        );
    }

    #[test]
    fn compare_rejects_missing_parent_block() {
        let parent = sample_parent_header();
        let input = sample_guest_input(parent.hash_slow());
        let blocks = vec![
            block_from_witness(&input.witnesses[0]),
            block_from_witness(&input.witnesses[1]),
        ];

        let err = compare_guest_input_checkpoint_against_l2_blocks(&input, &blocks)
            .expect_err("missing parent");
        assert!(
            err.to_string()
                .contains("did not return proposal parent block 9")
        );
    }

    #[test]
    fn compare_accepts_genesis_start_without_parent_block() {
        // A proposal starting at block 0 has no parent block to fetch; its canonical zero parent
        // hash is accepted without underflowing the block number.
        let mut input = sample_guest_input(B256::ZERO);
        input.witnesses[0].block.header.number = 0;
        let blocks = vec![
            block_from_witness(&input.witnesses[0]),
            block_from_witness(&input.witnesses[1]),
        ];

        compare_guest_input_checkpoint_against_l2_blocks(&input, &blocks)
            .expect("genesis start must not require a parent block");
    }

    #[test]
    fn compare_rejects_forged_genesis_parent_hash() {
        let forged_parent_hash = B256::from([0xAA; 32]);
        let mut input = sample_guest_input(forged_parent_hash);
        input.witnesses[0].block.header.number = 0;
        let blocks = vec![
            block_from_witness(&input.witnesses[0]),
            block_from_witness(&input.witnesses[1]),
        ];

        let err = compare_guest_input_checkpoint_against_l2_blocks(&input, &blocks)
            .expect_err("genesis parent hash must be canonical");
        assert!(err.to_string().contains("genesis parent block hash"));
    }
}
