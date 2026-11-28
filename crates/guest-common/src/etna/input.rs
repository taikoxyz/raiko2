//! Owned native adapters; caller authentication and proof framing remain separate.

use super::{
    verify_blob_range_statement_consensus, verify_blob_range_statement_execution,
    ConsensusParameters, ConsensusWitness, EtnaBlockExecutionWitness, L1CommitteeWitness,
    ParentConsensusWitness,
};
use alethia_reth_block::config::TaikoEvmConfig;
use alloy_eips_execution::eip2718::{Decodable2718, Encodable2718};
use alloy_primitives::B256;
use anyhow::{ensure, Context, Result};
use etna_core::{
    comet::{
        codec::{decode_canonical_block, decode_canonical_commit},
        validators::Validators,
    },
    l1_header::L1HeaderView,
    params::MAX_SET_MEMBERS,
};
use raiko2_primitives::{ExecutionWitness, WitnessStateNode};
use raiko2_protocol::InputDataSource;
use raiko2_protocol_etna::{
    input::{ConsensusRangeInput, RangeInput},
    l1_history::{L1HistoryLimits, L1HistoryProof},
    range::{RangeBlock, RangeProjectionContext},
};
use reth_ethereum_primitives::TransactionSigned;

/// Borrows an owned request into the existing DA, SSZ, range and strict EVM component.
///
/// This lower-level component preserves the obligations documented by
/// [`verify_blob_range_statement_execution`]. Use [`verify_range_input_consensus`] to include
/// state-derived committees, exact original native bodies, required QCs and miss bitmaps.
/// Neither entry authenticates caller policy merely because it was deserialized.
///
/// # Errors
/// Returns an error for caller budgets, malformed/noncanonical transactions or a component failure.
#[allow(clippy::too_many_arguments)]
pub fn verify_range_input_execution(
    input: &RangeInput,
    source: &InputDataSource,
    context: &RangeProjectionContext,
    execution_witnesses: &[EtnaBlockExecutionWitness<'_>],
    shared_state_nodes: &[WitnessStateNode],
    limits: L1HistoryLimits,
    config: &TaikoEvmConfig,
) -> Result<B256> {
    with_decoded_range(
        input,
        context,
        execution_witnesses.len(),
        limits,
        |blocks, proof| {
            verify_blob_range_statement_execution(
                source,
                blocks,
                context,
                &input.statement.as_statement(),
                &input.post_block_cursors,
                &input.parent_execution_header,
                proof,
                execution_witnesses,
                shared_state_nodes,
                config,
            )
        },
    )
}

/// Borrows owned inputs into the actual state-authenticated native consensus composer.
///
/// `input` holds only untrusted original bytes, keys and existing state-proof resources. The
/// caller separately authenticates the deployment/range/checkpoint/reference context, selects
/// execution configuration and supplies decode/history budgets and remaining phase-one policy.
/// All cryptographic storage-root, signer, body, LastCommit and QC checks are performed by
/// [`verify_blob_range_statement_consensus`]; this adapter never accepts staking words or a
/// host-approved schedule as authority. Existing block witnesses/shared nodes are reused directly.
///
/// Encoded bytes must be capped before deserialization. Count and per-native-object byte budgets
/// are checked before decoded view allocation. Canonical native and EIP2718 decoding rejects
/// malformed or normalized encodings. No vote collection, acquisition, signer, or retry is added.
/// The returned digest remains a native component result with the composer's documented limits.
///
/// # Errors
/// Returns an error for malformed/canonicality/count/budget violations, invalid parent set
/// metadata, or any failure in the existing state-authenticated native consensus composer.
#[allow(clippy::too_many_arguments)]
pub fn verify_range_input_consensus(
    input: &ConsensusRangeInput<ExecutionWitness>,
    source: &InputDataSource,
    context: &RangeProjectionContext,
    execution_witnesses: &[EtnaBlockExecutionWitness<'_>],
    shared_state_nodes: &[WitnessStateNode],
    limits: L1HistoryLimits,
    config: &TaikoEvmConfig,
    parameters: &ConsensusParameters,
) -> Result<B256> {
    validate_range_counts(&input.range, context, execution_witnesses.len())?;
    let consensus = &input.consensus;
    ensure!(
        consensus.original_blocks.len() == input.range.blocks.len(),
        "Etna owned original block count mismatch"
    );
    ensure!(
        consensus.required_commits.len() <= input.range.blocks.len(),
        "Etna owned standalone commit count exceeds range"
    );
    ensure!(
        consensus.anchors.len() <= input.range.blocks.len().saturating_add(1),
        "Etna owned committee anchor count exceeds range"
    );
    ensure!(
        consensus
            .anchors
            .windows(2)
            .all(|pair| pair[0].anchor_number < pair[1].anchor_number),
        "Etna owned committee anchors are not strictly increasing"
    );
    ensure!(
        consensus.parent.is_some() == (context.parent_height > context.origin_height),
        "Etna owned parent consensus bootstrap presence mismatch"
    );
    // Check every native byte budget before decoding any native object. These decodes retain
    // upstream native types, signatures and timestamps; the composer authenticates the results.
    ensure!(
        consensus
            .original_blocks
            .iter()
            .all(|raw| !raw.is_empty() && raw.len() <= parameters.max_block_bytes),
        "Etna owned original block exceeds trusted byte limit"
    );
    ensure!(
        consensus
            .required_commits
            .iter()
            .all(|raw| !raw.is_empty() && raw.len() <= parameters.max_commit_bytes),
        "Etna owned commit exceeds trusted byte limit"
    );
    for raw in &consensus.original_blocks {
        decode_canonical_block(raw)?;
    }
    for raw in &consensus.required_commits {
        decode_canonical_commit(raw)?;
    }
    let parent_sets = consensus
        .parent
        .as_ref()
        .map(|parent| -> Result<_> {
            ensure!(
                (1..=MAX_SET_MEMBERS).contains(&parent.active.len())
                    && (1..=MAX_SET_MEMBERS).contains(&parent.next.len()),
                "Etna owned parent validator count exceeds protocol bounds"
            );
            L1HeaderView::from_raw(&parent.anchor_header)
                .context("invalid owned Etna parent anchor header")?;
            Ok((
                Validators::from_members(&parent.active)?,
                Validators::from_members(&parent.next)?,
            ))
        })
        .transpose()?;
    let parent =
        consensus
            .parent
            .as_ref()
            .zip(parent_sets.as_ref())
            .map(|(parent, (active, next))| ParentConsensusWitness {
                header: &parent.header,
                active,
                next,
                anchor_header: &parent.anchor_header,
            });
    let anchors: Vec<_> = consensus
        .anchors
        .iter()
        .map(|anchor| L1CommitteeWitness {
            anchor_number: anchor.anchor_number,
            witness: &anchor.witness,
            fallback_keys: &anchor.fallback_keys,
        })
        .collect();
    let witness = ConsensusWitness {
        original_blocks: &consensus.original_blocks,
        required_commits: &consensus.required_commits,
        origin_fallback_keys: &consensus.origin_fallback_keys,
        reference_state: &consensus.reference_state,
        anchors: &anchors,
        parent,
    };
    with_decoded_range(
        &input.range,
        context,
        execution_witnesses.len(),
        limits,
        |blocks, proof| {
            verify_blob_range_statement_consensus(
                source,
                blocks,
                context,
                &input.range.statement.as_statement(),
                &input.range.post_block_cursors,
                &input.range.parent_execution_header,
                proof,
                execution_witnesses,
                shared_state_nodes,
                config,
                parameters,
                witness,
            )
        },
    )
}

fn validate_range_counts(
    input: &RangeInput,
    context: &RangeProjectionContext,
    witnesses: usize,
) -> Result<()> {
    ensure!(
        input.blocks.len() <= context.max_blocks,
        "Etna owned range exceeds the trusted block limit"
    );
    let expected = context
        .last_height
        .checked_sub(context.parent_height)
        .filter(|count| *count != 0)
        .context("invalid Etna owned range endpoints")?;
    ensure!(
        u64::try_from(input.blocks.len()).ok() == Some(expected),
        "Etna owned range block count mismatch"
    );
    ensure!(
        input.post_block_cursors.len() == input.blocks.len(),
        "Etna owned range cursor count mismatch"
    );
    ensure!(
        witnesses == input.blocks.len(),
        "Etna owned range execution witness count mismatch"
    );
    for block in &input.blocks {
        let bytes = block
            .execution_transactions
            .iter()
            .try_fold(0usize, |total, tx| total.checked_add(tx.len()))
            .context("Etna owned transaction byte count overflow")?;
        ensure!(
            bytes <= context.max_envelope_bytes,
            "Etna owned transactions exceed the trusted envelope limit"
        );
    }
    Ok(())
}

fn with_decoded_range<T>(
    input: &RangeInput,
    context: &RangeProjectionContext,
    witness_count: usize,
    limits: L1HistoryLimits,
    callback: impl for<'a, 'b> FnOnce(
        &[RangeBlock<'a, TransactionSigned>],
        L1HistoryProof<'b>,
    ) -> Result<T>,
) -> Result<T> {
    validate_range_counts(input, context, witness_count)?;
    input.l1_history.with_proof(limits, |proof| {
        let transactions = input
            .blocks
            .iter()
            .enumerate()
            .map(|(block_index, block)| {
                block
                    .execution_transactions
                    .iter()
                    .enumerate()
                    .map(|(tx_index, encoded)| {
                        let tx =
                            TransactionSigned::decode_2718_exact(encoded).with_context(|| {
                                format!("invalid owned Etna transaction {block_index}:{tx_index}")
                            })?;
                        ensure!(
                            tx.encoded_2718() == *encoded,
                            "noncanonical owned Etna transaction {block_index}:{tx_index}"
                        );
                        Ok(tx)
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .collect::<Result<Vec<_>>>()?;
        let blocks: Vec<_> = input
            .blocks
            .iter()
            .enumerate()
            .map(|(index, block)| RangeBlock {
                comet_header: &block.comet_header,
                comet_transactions: &block.comet_transactions,
                execution_header: &block.execution_header,
                execution_transactions: &transactions[index],
            })
            .collect();
        callback(&blocks, proof)
    })?
}
