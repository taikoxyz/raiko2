use alethia_reth_evm::zk_gas::pricing::{
    FIXED_POINT_SCALE, PricingBreakdown, ProposalPricingInput, RUNTIME_SPEC_SHA256,
    ZkGasCompletionError, ZkGasError, ZkGasModel,
};
use alloy_consensus::BlockHeader as _;
use alloy_primitives::B256;
use raiko2_primitives_shasta::GuestInput;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    mpt,
    proposal_inputs::{PayloadAccumulator, blob_selectors, guest_input_identity},
    reconstruct::{BlockExecutionObservation, QuoteDerivedBlockExecutor},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PricingBreakdownSnapshot {
    pub fixed_scaled: i128,
    pub operations_scaled: i128,
    pub feature_error_scaled: i128,
    pub resources_scaled: i128,
    pub mpt_scaled: i128,
    pub startup_scaled: i128,
    pub transaction_payload_scaled: i128,
    pub blob_source_scaled: i128,
    pub parent_mpt_scaled: i128,
    pub total_scaled: i128,
    pub total_pgu: u128,
}

impl TryFrom<PricingBreakdown> for PricingBreakdownSnapshot {
    type Error = RuntimeQuoteError;

    fn try_from(value: PricingBreakdown) -> Result<Self, Self::Error> {
        Ok(Self {
            fixed_scaled: value.fixed_scaled,
            operations_scaled: value.operations_scaled,
            feature_error_scaled: value.feature_error_scaled,
            resources_scaled: value.resources_scaled,
            mpt_scaled: value.mpt_scaled,
            startup_scaled: value.startup_scaled,
            transaction_payload_scaled: value.transaction_payload_scaled,
            blob_source_scaled: value.blob_source_scaled,
            parent_mpt_scaled: value.parent_mpt_scaled,
            total_scaled: value.total_scaled,
            total_pgu: value
                .total_pgu
                .ok_or(RuntimeQuoteError::Missing("total_pgu"))?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockWorkloadQuote {
    pub block_index: usize,
    pub block_number: u64,
    pub unzen_zkgas: u64,
    pub input_transaction_count: u64,
    pub started_transaction_count: u64,
    pub true_unattempted_transactions: u64,
    pub breakdown: PricingBreakdownSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalWorkloadQuote {
    pub schema_version: u32,
    pub model_id: String,
    pub guest_input_sha256: String,
    pub guest_input_bincode_length: usize,
    pub public_output: B256,
    pub blocks: Vec<BlockWorkloadQuote>,
    pub unzen_zkgas_sum: u128,
    pub block_local_scaled: i128,
    pub block_local_pgu: u128,
    pub sum_of_block_admission_pgu: u128,
    pub proposal_once: PricingBreakdownSnapshot,
    pub proposal_once_pgu: u128,
    pub total_scaled: i128,
    pub total_pgu: u128,
}

#[derive(Debug, Error)]
pub enum RuntimeQuoteError {
    #[error("proposal validation failed: {0}")]
    Validation(String),
    #[error("missing runtime quote input: {0}")]
    Missing(&'static str),
    #[error("runtime quote topology mismatch: {0}")]
    Topology(&'static str),
    #[error("runtime quote arithmetic overflow")]
    ArithmeticOverflow,
    #[error("block {block_index} completion failed: {source}")]
    BlockCompletion {
        block_index: usize,
        #[source]
        source: ZkGasCompletionError,
    },
    #[error("proposal-once pricing failed: {0}")]
    ProposalPricing(#[from] ZkGasError),
}

fn checked_add_u64(left: u64, right: u64) -> Result<u64, RuntimeQuoteError> {
    left.checked_add(right)
        .ok_or(RuntimeQuoteError::ArithmeticOverflow)
}

fn checked_add_i128(left: i128, right: i128) -> Result<i128, RuntimeQuoteError> {
    left.checked_add(right)
        .ok_or(RuntimeQuoteError::ArithmeticOverflow)
}

fn ceil_scaled(value: i128) -> Result<u128, RuntimeQuoteError> {
    if value < 0 || FIXED_POINT_SCALE <= 0 {
        return Err(RuntimeQuoteError::ArithmeticOverflow);
    }
    let quotient = value / FIXED_POINT_SCALE;
    let remainder = value % FIXED_POINT_SCALE;
    u128::try_from(quotient + i128::from(remainder != 0))
        .map_err(|_| RuntimeQuoteError::ArithmeticOverflow)
}

struct ReplayedBlock {
    index: usize,
    number: u64,
    input_transaction_count: u64,
    true_unattempted_transactions: u64,
    mpt: alethia_reth_evm::zk_gas::pricing::MptSelectors,
    execution: BlockExecutionObservation,
}

/// Replays one current-format Shasta `GuestInput` and returns a complete host-only workload quote.
///
/// The production guest path is unchanged. This host utility uses the same proposal validator and
/// block reconstruction, while observing exact block-end resources and sparse-trie work needed to
/// complete Alethia's sealed Shadow reports.
///
/// # Errors
///
/// Returns an error when proposal validation fails, a required observation is missing, the replay
/// topology differs from the supplied input, arithmetic overflows, or sealed pricing completion
/// rejects the observed selectors.
#[allow(clippy::too_many_lines)]
pub fn quote_shasta_proposal(
    input: &GuestInput,
) -> Result<ProposalWorkloadQuote, RuntimeQuoteError> {
    let (guest_input_sha256, guest_input_bincode_length) = guest_input_identity(input)
        .map_err(|error| RuntimeQuoteError::Validation(error.to_string()))?;
    let blob =
        blob_selectors(input).map_err(|error| RuntimeQuoteError::Validation(error.to_string()))?;
    let mut payload = PayloadAccumulator::default();
    let mut blocks = Vec::with_capacity(input.witnesses.len());
    let mut parent_mpt = alethia_reth_evm::zk_gas::pricing::MptSelectors::default();
    let mut parent_read_count = 0_u64;

    let public_output =
        raiko2_guest_common::prove_shasta_proposal_with_reconstructor_and_parent_reader(
            input,
            |request| {
                payload.record_block(request.anchor_tx.inner(), &request.transactions)?;
                let input_transaction_count = request
                    .transactions
                    .len()
                    .checked_add(1)
                    .context("block input transaction count overflows usize")
                    .and_then(|count| {
                        u64::try_from(count).context("block input transaction count exceeds u64")
                    })?;
                let mut executor = QuoteDerivedBlockExecutor::default();
                let (outcome, metrics) =
                    raiko2_stateless::reconstruct_block_from_transactions_with_executor_and_mpt_metrics(
                        request.anchor_tx,
                        request.transactions,
                        request.block_env,
                        request.witness,
                        request.ancestor_headers,
                        request.shared_state_nodes,
                        request.chain_spec,
                        request.evm_config,
                        &mut executor,
                    )
                    .map_err(anyhow::Error::from)?;
                let execution = executor
                    .take()
                    .ok_or_else(|| anyhow::anyhow!("missing completed block pricing observation"))?;
                let started = u64::try_from(execution.started_transaction_count)
                    .context("started transaction count exceeds u64")?;
                if started == 0 || started > input_transaction_count {
                    anyhow::bail!("started transaction count is outside the block input domain");
                }
                blocks.push(ReplayedBlock {
                    index: request.index,
                    number: outcome.filtered_block.header().number(),
                    input_transaction_count,
                    true_unattempted_transactions: input_transaction_count - started,
                    mpt: mpt::selectors(&metrics)
                        .map_err(|error| anyhow::anyhow!(error.to_string()))?,
                    execution,
                });
                Ok(outcome)
            },
            |request| {
                let (value, metrics) = raiko2_stateless::read_parent_storage_with_mpt_metrics(
                    request.address,
                    request.slot,
                    request.witness,
                    request.ancestor_headers,
                    request.shared_state_nodes,
                )
                .map_err(anyhow::Error::from)?;
                parent_mpt = mpt::add(
                    parent_mpt,
                    mpt::selectors(&metrics)
                        .map_err(|error| anyhow::anyhow!(error.to_string()))?,
                )
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                parent_read_count = parent_read_count
                    .checked_add(1)
                    .context("parent MPT read count overflows")?;
                Ok(value)
            },
        )
        .map_err(|error| RuntimeQuoteError::Validation(format!("{error:#}")))?;

    if blocks.is_empty() || blocks.len() != input.witnesses.len() {
        return Err(RuntimeQuoteError::Topology("block_count"));
    }
    // Canonical Shasta validation always reads Anchor._blockState from the parent state before
    // deriving blocks. Zero means the observation hook did not cover the validation path, so a
    // quote without parent MPT work would be incomplete.
    if parent_read_count == 0 {
        return Err(RuntimeQuoteError::Missing("parent_mpt_reads"));
    }
    let payload = payload.finish();
    let mut block_quotes = Vec::with_capacity(blocks.len());
    let mut total_transaction_count = 0_u64;
    let mut total_unattempted = 0_u64;
    let mut unzen_zkgas_sum = 0_u128;
    let mut block_local_scaled = 0_i128;
    let mut sum_of_block_admission_pgu = 0_u128;
    for (position, block) in blocks.into_iter().enumerate() {
        if block.index != position || block.number != input.witnesses[position].block.header.number
        {
            return Err(RuntimeQuoteError::Topology("block_identity"));
        }
        let priced = ZkGasModel
            .complete_block_shadow_report(
                &block.execution.report,
                block.execution.exact_resources,
                block.mpt,
            )
            .map_err(|source| RuntimeQuoteError::BlockCompletion {
                block_index: position,
                source,
            })?;
        let breakdown = PricingBreakdownSnapshot::try_from(priced)?;
        total_transaction_count =
            checked_add_u64(total_transaction_count, block.input_transaction_count)?;
        total_unattempted =
            checked_add_u64(total_unattempted, block.true_unattempted_transactions)?;
        unzen_zkgas_sum = unzen_zkgas_sum
            .checked_add(u128::from(block.execution.finalized_block_zkgas))
            .ok_or(RuntimeQuoteError::ArithmeticOverflow)?;
        block_local_scaled = checked_add_i128(block_local_scaled, breakdown.total_scaled)?;
        sum_of_block_admission_pgu = sum_of_block_admission_pgu
            .checked_add(breakdown.total_pgu)
            .ok_or(RuntimeQuoteError::ArithmeticOverflow)?;
        block_quotes.push(BlockWorkloadQuote {
            block_index: position,
            block_number: block.number,
            unzen_zkgas: block.execution.finalized_block_zkgas,
            input_transaction_count: block.input_transaction_count,
            started_transaction_count: block
                .input_transaction_count
                .checked_sub(block.true_unattempted_transactions)
                .ok_or(RuntimeQuoteError::ArithmeticOverflow)?,
            true_unattempted_transactions: block.true_unattempted_transactions,
            breakdown,
        });
    }
    if payload.total_transaction_count != total_transaction_count
        || payload.nonempty_payload_transactions > total_transaction_count
    {
        return Err(RuntimeQuoteError::Topology("transaction_payload_domain"));
    }

    let proposal_once = ZkGasModel.price_proposal_once(&ProposalPricingInput {
        canonical_transaction_bytes: payload.canonical_transaction_bytes,
        nonempty_payload_transactions: payload.nonempty_payload_transactions,
        true_unattempted_transactions: total_unattempted,
        source_blob_counts: &blob.source_blob_counts,
        proposal_blob_hash_count: blob.proposal_blob_hash_count,
        provider_blob_count: blob.provider_blob_count,
        provider_commitment_count: blob.provider_commitment_count,
        provider_proof_count: blob.provider_proof_count,
        parent_mpt,
    })?;
    let proposal_once = PricingBreakdownSnapshot::try_from(proposal_once)?;
    let total_scaled = checked_add_i128(block_local_scaled, proposal_once.total_scaled)?;
    Ok(ProposalWorkloadQuote {
        schema_version: 1,
        model_id: RUNTIME_SPEC_SHA256.to_owned(),
        guest_input_sha256,
        guest_input_bincode_length,
        public_output,
        blocks: block_quotes,
        unzen_zkgas_sum,
        block_local_scaled,
        block_local_pgu: ceil_scaled(block_local_scaled)?,
        sum_of_block_admission_pgu,
        proposal_once_pgu: proposal_once.total_pgu,
        proposal_once,
        total_scaled,
        total_pgu: ceil_scaled(total_scaled)?,
    })
}

use anyhow::Context as _;

#[cfg(test)]
mod tests {
    use super::*;
    use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
    use std::{fs, path::Path};

    #[test]
    fn checked_in_current_guest_input_completes_quote_with_public_output_parity() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_23077_l2_9051439_9051630.json",
        );
        let input: GuestInput = serde_json::from_str(
            &fs::read_to_string(&fixture)
                .unwrap_or_else(|error| panic!("read {}: {error}", fixture.display())),
        )
        .expect("parse checked-in current GuestInput");
        let expected_output = hash_shasta_subproof_input(&input.proof_carry_data);

        let quote = quote_shasta_proposal(&input).expect("complete proposal workload quote");

        assert_eq!(quote.public_output, expected_output);
        assert_eq!(quote.blocks.len(), input.witnesses.len());
        assert_eq!(
            quote.unzen_zkgas_sum, 0,
            "the checked-in proposal predates mainnet Unzen activation"
        );
        assert!(quote.total_pgu > 0);
        assert!(quote.proposal_once_pgu > 0);
    }
}
