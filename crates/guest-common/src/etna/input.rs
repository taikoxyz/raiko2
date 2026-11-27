//! Native owned-input adapter; no runtime route, signer or proof framing.

use super::{verify_blob_range_statement_execution, EtnaBlockExecutionWitness};
use alethia_reth_block::config::TaikoEvmConfig;
use alloy_eips_execution::eip2718::{Decodable2718, Encodable2718};
use alloy_primitives::B256;
use anyhow::{ensure, Context, Result};
use raiko2_primitives::WitnessStateNode;
use raiko2_protocol::InputDataSource;
use raiko2_protocol_etna::{
    input::RangeInput,
    l1_history::L1HistoryLimits,
    range::{RangeBlock, RangeProjectionContext},
};
use reth_ethereum_primitives::TransactionSigned;

/// Borrows an owned request into the existing DA, SSZ, range and strict EVM verifier.
///
/// The caller caps encoded bytes before deserialization and supplies authenticated range/domain
/// context, L1-reference authority, history limits and execution configuration separately. This
/// adapter retains every requirement of [`verify_blob_range_statement_execution`], including
/// authenticated phase-one FI-disabled profile/queue state, origin, Inbox proofs, timing/rules
/// and committee/QC authority. Supplied statement fields are requests, not caller authority.
/// Existing per-block witnesses and shared sparse-state nodes are passed directly to the current
/// executor. No host approval flag, alternate execution path or resource policy is deserialized.
///
/// Every original EIP2718 byte string is decoded exactly and checked for canonical re-encoding.
/// Complete list counts remain visible to the existing verifier; no zip truncates supplied data.
/// The result is a native component digest, not a complete SGX proof or landing authorization.
///
/// # Errors
/// Returns an error for exceeded caller budgets, malformed/noncanonical transactions, or any
/// failure of the existing complete-range DA/SSZ/projection/execution component.
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
    // Bound derived view/transaction-vector allocations before building them. The existing
    // verifier still validates the exact authenticated endpoints and every supplied list count.
    ensure!(
        input.blocks.len() <= context.max_blocks,
        "Etna owned range exceeds the trusted block limit"
    );
    let expected = context
        .last_height
        .checked_sub(context.parent_height)
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
        execution_witnesses.len() == input.blocks.len(),
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
                execution_transactions: transactions[index].as_slice(),
            })
            .collect();
        verify_blob_range_statement_execution(
            source,
            &blocks,
            context,
            &input.statement.as_statement(),
            &input.post_block_cursors,
            &input.parent_execution_header,
            proof,
            execution_witnesses,
            shared_state_nodes,
            config,
        )
    })?
}
