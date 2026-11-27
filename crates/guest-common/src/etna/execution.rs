//! Native Etna execution composed with the authenticated blob/range projection.

use super::{verify_blob_range_statement_projection, L1AnchorProof};
use alethia_reth_block::config::TaikoEvmConfig;
use alethia_reth_chainspec::hardfork::TaikoHardforks;
use alloy_consensus::Header;
use alloy_eips_execution::eip2718::{Decodable2718, Encodable2718};
use alloy_primitives::{B256, U256};
use anyhow::{ensure, Context, Result};
use raiko2_primitives::{ExecutionWitness, WitnessHeader, WitnessStateNode};
use raiko2_protocol::InputDataSource;
use raiko2_protocol_etna::{
    projection::ExecutionProjection,
    range::{RangeBlock, RangeProjectionContext},
    statement::LandStatement,
};
use raiko2_stateless::validate_block_with_witness_resources;
use reth_ethereum_primitives::{Block, BlockBody, TransactionSigned};

/// Existing per-block witness and full ancestor window, with an optional shared state-node pool
/// supplied separately to [`verify_blob_range_statement_execution`].
#[derive(Clone, Copy, Debug)]
pub struct EtnaBlockExecutionWitness<'a> {
    /// Account, storage and code witness consumed by the existing sparse-state executor.
    pub witness: &'a ExecutionWitness,
    /// Contiguous full ancestor headers ending at this block's authenticated parent.
    pub ancestor_headers: &'a [WitnessHeader],
}

/// Authenticates the phase-one Etna projection and strictly executes every matched block.
///
/// The EVM config is selected by the trusted caller from the authenticated deployment's chain
/// and fork schedule. Every supplied block must be Etna-active under that schedule. Original
/// transactions are decoded from the DA-bound EIP2718 payloads; signer recovery, body and receipt
/// validation, witness pre-state reads and computed post-state validation use the existing
/// supplied-block stateless path. No candidate filtering or reconstruction is performed.
///
/// The incoming parent must already be authenticated by the caller. Each witness window is bound
/// to that parent or the preceding successfully verified block, including the exact state root.
/// Shared witness nodes do not supply an independent parent root. Cached witness metadata and
/// transaction hashes or senders are not trusted by this entry point.
///
/// This function retains all authority requirements documented by
/// [`verify_blob_range_statement_projection`]: authenticated deployment and L1 reference, Inbox
/// proofs and queue state, timing/rules, committee and certificate authority, and FI-disabled
/// phase-one context. It binds the committed miss bitmap and lets the EVM consume it; it does
/// not establish its committee/QC semantics. The returned native digest is not an SGX proof or
/// an on-chain landing authorization.
///
/// # Errors
///
/// Returns an error on any projection, DA, L1-history, execution, witness or parent mismatch.
/// No digest is returned until every supplied block has passed strict replay.
#[allow(clippy::too_many_arguments)]
pub fn verify_blob_range_statement_execution(
    source: &InputDataSource,
    blocks: &[RangeBlock<'_, TransactionSigned>],
    context: &RangeProjectionContext,
    statement: &LandStatement<'_>,
    post_block_cursors: &[u64],
    parent_execution_header: &Header,
    l1_anchor_proof: L1AnchorProof<'_>,
    execution_witnesses: &[EtnaBlockExecutionWitness<'_>],
    shared_state_nodes: &[WitnessStateNode],
    config: &TaikoEvmConfig,
) -> Result<B256> {
    ensure!(
        U256::from(config.chain_spec().inner.chain().id()) == context.domain.l2_chain_id,
        "Etna execution config chain differs from the authenticated deployment"
    );
    let (projected, digest) = verify_blob_range_statement_projection(
        source,
        blocks,
        context,
        statement,
        post_block_cursors,
        parent_execution_header,
        l1_anchor_proof,
    )?;
    replay_projected_blocks(
        blocks,
        &projected,
        parent_execution_header,
        execution_witnesses,
        shared_state_nodes,
        config,
    )?;
    Ok(digest)
}

fn decode_committed_transactions(
    projection: &ExecutionProjection<'_>,
) -> Result<Vec<TransactionSigned>> {
    projection
        .transaction_bytes()
        .iter()
        .enumerate()
        .map(|(index, encoded)| {
            let tx = TransactionSigned::decode_2718_exact(encoded)
                .with_context(|| format!("invalid committed Etna transaction {index}"))?;
            ensure!(
                tx.encoded_2718().as_slice() == *encoded,
                "noncanonical committed Etna transaction {index}"
            );
            Ok(tx)
        })
        .collect()
}

fn replay_projected_blocks(
    blocks: &[RangeBlock<'_, TransactionSigned>],
    projected: &[ExecutionProjection<'_>],
    parent_execution_header: &Header,
    execution_witnesses: &[EtnaBlockExecutionWitness<'_>],
    shared_state_nodes: &[WitnessStateNode],
    config: &TaikoEvmConfig,
) -> Result<()> {
    ensure!(
        execution_witnesses.len() == blocks.len(),
        "Etna execution witness count differs from the complete projected range"
    );
    ensure!(
        projected.len() == blocks.len(),
        "Etna execution projection count mismatch"
    );
    let chain_spec = config.chain_spec();
    let mut parent = parent_execution_header;
    for ((block, projection), resource) in blocks.iter().zip(projected).zip(execution_witnesses) {
        let header = block.execution_header;
        ensure!(
            chain_spec.is_etna_active(header.timestamp),
            "Etna is not active in the trusted execution schedule at height {}",
            header.number
        );
        // Rebuild every cached hash/number/timestamp from the actual full header. In particular,
        // an untrusted metadata hash cannot make an arbitrary parent state root authoritative.
        let ancestors = resource
            .ancestor_headers
            .iter()
            .map(|witness_header| {
                witness_header
                    .full_header()
                    .cloned()
                    .map(WitnessHeader::from_header)
                    .context("Etna execution requires full ancestor headers")
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            ancestors.last().and_then(WitnessHeader::full_header) == Some(parent),
            "Etna witness parent differs from the authenticated execution parent at height {}",
            header.number
        );
        ensure!(
            header.parent_hash == parent.hash_slow(),
            "Etna execution parent link mismatch at height {}",
            header.number
        );

        let transactions = decode_committed_transactions(projection)?;
        // Etna consensus requires empty withdrawals and no ommers. The exact transaction root,
        // canonical empty body commitments and receipts are checked by the existing verifier.
        let supplied = Block {
            header: header.clone(),
            body: BlockBody {
                transactions,
                ommers: Vec::new(),
                withdrawals: Some(Default::default()),
            },
        };
        let verified_hash = validate_block_with_witness_resources(
            supplied,
            resource.witness,
            &ancestors,
            shared_state_nodes,
            chain_spec,
            config,
        )
        .with_context(|| format!("strict Etna execution failed at height {}", header.number))?;
        ensure!(
            verified_hash == projection.execution_hash(),
            "Etna verified execution hash differs from the committed projection"
        );
        parent = header;
    }
    Ok(())
}
