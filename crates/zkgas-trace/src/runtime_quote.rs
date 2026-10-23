use alethia_reth_evm::zk_gas::pricing::{
    FIXED_POINT_SCALE, MptSelectors, PricingBreakdown, ProposalPricingInput, RUNTIME_SPEC_SHA256,
    ResourceSelectors, ZkGasCompletionError, ZkGasError, ZkGasKnownBlockBreakdown, ZkGasModel,
    ZkGasShadowReport, ZkGasShadowStatus,
};
use alloy_consensus::BlockHeader as _;
use alloy_primitives::B256;
use raiko2_primitives_shasta::GuestInput;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    mpt,
    proposal_inputs::{
        PayloadAccumulator, ProposalBlobSelectors, ProposalPayloadSelectors, blob_selectors,
        guest_input_identity,
    },
    reconstruct::{BlockExecutionObservation, QuoteDerivedBlockExecutor},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceSelectorsSnapshot {
    pub unique_account_count: u64,
    pub unique_storage_slot_access_count: u64,
    pub unique_storage_slot_write_count: u64,
    pub unique_code_hash_count: u64,
    pub unique_code_bytes: u64,
}

impl From<ResourceSelectors> for ResourceSelectorsSnapshot {
    fn from(value: ResourceSelectors) -> Self {
        Self {
            unique_account_count: value.unique_account_count,
            unique_storage_slot_access_count: value.unique_storage_slot_access_count,
            unique_storage_slot_write_count: value.unique_storage_slot_write_count,
            unique_code_hash_count: value.unique_code_hash_count,
            unique_code_bytes: value.unique_code_bytes,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MptSelectorsSnapshot {
    pub path_node_visits: u64,
    pub materialization_work_units: u64,
    pub hydration_hash_blocks: u64,
    pub dirty_encoded_nodes: u64,
    pub dirty_hash_blocks: u64,
    pub structural_branch_extension_churn_nodes: u64,
    pub dropped_trie_nodes: u64,
}

impl From<MptSelectors> for MptSelectorsSnapshot {
    fn from(value: MptSelectors) -> Self {
        Self {
            path_node_visits: value.path_node_visits,
            materialization_work_units: value.materialization_work_units,
            hydration_hash_blocks: value.hydration_hash_blocks,
            dirty_encoded_nodes: value.dirty_encoded_nodes,
            dirty_hash_blocks: value.dirty_hash_blocks,
            structural_branch_extension_churn_nodes: value.structural_branch_extension_churn_nodes,
            dropped_trie_nodes: value.dropped_trie_nodes,
        }
    }
}

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodePricingBreakdownSnapshot {
    pub fixed_scaled: i128,
    pub operations_scaled: i128,
    pub feature_error_scaled: i128,
    pub evm_local_resources: ResourceSelectorsSnapshot,
    pub evm_local_resources_scaled: i128,
    pub known_subtotal_scaled: i128,
    pub known_subtotal_pgu: u128,
}

impl From<ZkGasKnownBlockBreakdown> for NodePricingBreakdownSnapshot {
    fn from(value: ZkGasKnownBlockBreakdown) -> Self {
        Self {
            fixed_scaled: value.fixed_scaled(),
            operations_scaled: value.operations_scaled(),
            feature_error_scaled: value.feature_error_scaled(),
            evm_local_resources: value.evm_local_resources().into(),
            evm_local_resources_scaled: value.evm_local_resources_scaled(),
            known_subtotal_scaled: value.evm_local_known_subtotal_scaled(),
            known_subtotal_pgu: value.evm_local_known_subtotal_pgu(),
        }
    }
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
    pub node: NodeBlockWorkloadQuote,
    pub prove: ProveBlockWorkloadQuote,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeBlockWorkloadQuote {
    pub unzen_zkgas: u64,
    pub pricing: NodePricingBreakdownSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProveBlockWorkloadQuote {
    pub input_transaction_count: u64,
    pub consumed_candidate_count: u64,
    pub attempted_transaction_count: u64,
    pub true_unattempted_transactions: u64,
    pub exact_resources: ResourceSelectorsSnapshot,
    pub mpt: MptSelectorsSnapshot,
    pub pricing: PricingBreakdownSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeProposalWorkloadQuote {
    pub unzen_zkgas_sum: u128,
    pub block_local_known_scaled: i128,
    pub block_local_known_pgu: u128,
    pub sum_of_block_known_pgu: u128,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalPricingSelectorsSnapshot {
    pub payload: ProposalPayloadSelectors,
    pub true_unattempted_transactions: u64,
    pub blobs: ProposalBlobSelectors,
    pub parent_mpt: MptSelectorsSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProveProposalWorkloadQuote {
    pub selectors: ProposalPricingSelectorsSnapshot,
    pub block_local_scaled: i128,
    pub block_local_pgu: u128,
    pub sum_of_block_local_pgu: u128,
    pub proposal_once: PricingBreakdownSnapshot,
    pub proposal_once_pgu: u128,
    pub total_scaled: i128,
    pub total_pgu: u128,
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
    pub node: NodeProposalWorkloadQuote,
    pub prove: ProveProposalWorkloadQuote,
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

fn true_unattempted_transactions(
    input: u64,
    consumed: u64,
    attempted: u64,
) -> Result<u64, RuntimeQuoteError> {
    if attempted > consumed || consumed > input {
        return Err(RuntimeQuoteError::Topology("transaction_counts"));
    }
    input
        .checked_sub(consumed)
        .ok_or(RuntimeQuoteError::ArithmeticOverflow)
}

#[allow(clippy::too_many_arguments)]
fn observe_discarded_attempts(
    guest_input_sha256: &str,
    block_index: usize,
    block_number: u64,
    input_transaction_count: u64,
    consumed_candidate_count: u64,
    attempted_transaction_count: u64,
    committed_transaction_count: u64,
) -> Result<u64, RuntimeQuoteError> {
    let discarded_attempt_count = attempted_transaction_count
        .checked_sub(committed_transaction_count)
        .ok_or(RuntimeQuoteError::Topology("transaction_counts"))?;
    if discarded_attempt_count > 0 {
        tracing::warn!(
            guest_input_sha256,
            block_index,
            block_number,
            input_transaction_count,
            consumed_candidate_count,
            attempted_transaction_count,
            committed_transaction_count,
            discarded_attempt_count,
            "discarded attempted transactions observed during proposal quote replay"
        );
    }
    Ok(discarded_attempt_count)
}

fn complete_block_pricing(
    block_index: usize,
    committed_report: &ZkGasShadowReport,
    attempted_report: &ZkGasShadowReport,
    exact_resources: ResourceSelectors,
    mpt: MptSelectors,
) -> Result<(NodePricingBreakdownSnapshot, PricingBreakdownSnapshot), RuntimeQuoteError> {
    let committed_known = match committed_report.status() {
        ZkGasShadowStatus::Incomplete { known, .. } => Some(known),
        ZkGasShadowStatus::Diagnostic(_) => None,
    };
    let committed_priced = ZkGasModel
        .complete_block_shadow_report(committed_report, exact_resources, mpt)
        .map_err(|source| RuntimeQuoteError::BlockCompletion {
            block_index,
            source,
        })?;
    let committed_known =
        committed_known.ok_or(RuntimeQuoteError::Missing("node_shadow_breakdown"))?;

    let attempted_known = match attempted_report.status() {
        ZkGasShadowStatus::Incomplete { known, .. } => Some(known),
        ZkGasShadowStatus::Diagnostic(_) => None,
    };
    // Completing with the attempted report's own EVM-local resource subset validates the sealed
    // report without allowing rejected-candidate resources to leak into the exact committed term.
    let attempted_priced = ZkGasModel
        .complete_block_shadow_report(
            attempted_report,
            attempted_known
                .map(ZkGasKnownBlockBreakdown::evm_local_resources)
                .unwrap_or_default(),
            MptSelectors::default(),
        )
        .map_err(|source| RuntimeQuoteError::BlockCompletion {
            block_index,
            source,
        })?;
    attempted_known.ok_or(RuntimeQuoteError::Missing("attempted_shadow_breakdown"))?;
    let total_scaled = checked_add_i128(
        checked_add_i128(
            attempted_priced.fixed_scaled,
            attempted_priced.operations_scaled,
        )?,
        checked_add_i128(
            committed_priced.resources_scaled,
            committed_priced.mpt_scaled,
        )?,
    )?;
    let prove = PricingBreakdownSnapshot {
        fixed_scaled: attempted_priced.fixed_scaled,
        operations_scaled: attempted_priced.operations_scaled,
        feature_error_scaled: attempted_priced.feature_error_scaled,
        resources_scaled: committed_priced.resources_scaled,
        mpt_scaled: committed_priced.mpt_scaled,
        total_scaled,
        total_pgu: ceil_scaled(total_scaled)?,
        ..Default::default()
    };
    Ok((NodePricingBreakdownSnapshot::from(committed_known), prove))
}

struct ReplayedBlock {
    index: usize,
    number: u64,
    input_transaction_count: u64,
    consumed_candidate_count: u64,
    attempted_transaction_count: u64,
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
                let consumed = u64::try_from(execution.consumed_candidate_count)
                    .context("consumed candidate count exceeds u64")?;
                let attempted = execution.attempted_transaction_count;
                let true_unattempted = true_unattempted_transactions(
                    input_transaction_count,
                    consumed,
                    attempted,
                )
                .map_err(anyhow::Error::from)?;
                let block_number = outcome.filtered_block.header().number();
                let committed = u64::try_from(
                    outcome.filtered_block.body().transactions().count(),
                )
                .context("committed transaction count exceeds u64")?;
                observe_discarded_attempts(
                    &guest_input_sha256,
                    request.index,
                    block_number,
                    input_transaction_count,
                    consumed,
                    attempted,
                    committed,
                )
                .map_err(anyhow::Error::from)?;
                blocks.push(ReplayedBlock {
                    index: request.index,
                    number: block_number,
                    input_transaction_count,
                    consumed_candidate_count: consumed,
                    attempted_transaction_count: attempted,
                    true_unattempted_transactions: true_unattempted,
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
    let mut block_local_known_scaled = 0_i128;
    let mut sum_of_block_known_pgu = 0_u128;
    let mut block_local_scaled = 0_i128;
    let mut sum_of_block_local_pgu = 0_u128;
    for (position, block) in blocks.into_iter().enumerate() {
        if block.index != position || block.number != input.witnesses[position].block.header.number
        {
            return Err(RuntimeQuoteError::Topology("block_identity"));
        }
        let (node_pricing, prove_pricing) = complete_block_pricing(
            position,
            &block.execution.committed_report,
            &block.execution.attempted_report,
            block.execution.exact_resources,
            block.mpt,
        )?;
        total_transaction_count =
            checked_add_u64(total_transaction_count, block.input_transaction_count)?;
        total_unattempted =
            checked_add_u64(total_unattempted, block.true_unattempted_transactions)?;
        unzen_zkgas_sum = unzen_zkgas_sum
            .checked_add(u128::from(block.execution.finalized_block_zkgas))
            .ok_or(RuntimeQuoteError::ArithmeticOverflow)?;
        block_local_known_scaled =
            checked_add_i128(block_local_known_scaled, node_pricing.known_subtotal_scaled)?;
        sum_of_block_known_pgu = sum_of_block_known_pgu
            .checked_add(node_pricing.known_subtotal_pgu)
            .ok_or(RuntimeQuoteError::ArithmeticOverflow)?;
        block_local_scaled = checked_add_i128(block_local_scaled, prove_pricing.total_scaled)?;
        sum_of_block_local_pgu = sum_of_block_local_pgu
            .checked_add(prove_pricing.total_pgu)
            .ok_or(RuntimeQuoteError::ArithmeticOverflow)?;
        block_quotes.push(BlockWorkloadQuote {
            block_index: position,
            block_number: block.number,
            node: NodeBlockWorkloadQuote {
                unzen_zkgas: block.execution.finalized_block_zkgas,
                pricing: node_pricing,
            },
            prove: ProveBlockWorkloadQuote {
                input_transaction_count: block.input_transaction_count,
                consumed_candidate_count: block.consumed_candidate_count,
                attempted_transaction_count: block.attempted_transaction_count,
                true_unattempted_transactions: block.true_unattempted_transactions,
                exact_resources: block.execution.exact_resources.into(),
                mpt: block.mpt.into(),
                pricing: prove_pricing,
            },
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
    let proposal_selectors = ProposalPricingSelectorsSnapshot {
        payload,
        true_unattempted_transactions: total_unattempted,
        blobs: blob,
        parent_mpt: parent_mpt.into(),
    };
    let proposal_once = PricingBreakdownSnapshot::try_from(proposal_once)?;
    let total_scaled = checked_add_i128(block_local_scaled, proposal_once.total_scaled)?;
    Ok(ProposalWorkloadQuote {
        schema_version: 1,
        model_id: RUNTIME_SPEC_SHA256.to_owned(),
        guest_input_sha256,
        guest_input_bincode_length,
        public_output,
        blocks: block_quotes,
        node: NodeProposalWorkloadQuote {
            unzen_zkgas_sum,
            block_local_known_scaled,
            block_local_known_pgu: ceil_scaled(block_local_known_scaled)?,
            sum_of_block_known_pgu,
        },
        prove: ProveProposalWorkloadQuote {
            selectors: proposal_selectors,
            block_local_scaled,
            block_local_pgu: ceil_scaled(block_local_scaled)?,
            sum_of_block_local_pgu,
            proposal_once_pgu: proposal_once.total_pgu,
            proposal_once,
            total_scaled,
            total_pgu: ceil_scaled(total_scaled)?,
        },
    })
}

use anyhow::Context as _;

#[cfg(test)]
mod tests {
    use super::*;
    use alethia_reth_evm::zk_gas::pricing::{
        OperationRoute, OperationRoutePricingInput, ZkGasBlockSnapshot, ZkGasShadowDiagnostic,
        ZkGasShadowReport,
    };
    use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
    use std::{
        collections::BTreeMap,
        fmt, fs,
        path::Path,
        sync::{Arc, Mutex},
    };
    use tracing::{Event, Level, Subscriber, field::Visit};
    use tracing_subscriber::{Layer, layer::Context, prelude::*};

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct CapturedEvent {
        level: Level,
        fields: BTreeMap<String, String>,
    }

    #[derive(Clone, Default)]
    struct EventCapture(Arc<Mutex<Vec<CapturedEvent>>>);

    struct FieldCapture<'a>(&'a mut BTreeMap<String, String>);

    impl Visit for FieldCapture<'_> {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
            self.0.insert(field.name().to_owned(), format!("{value:?}"));
        }

        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.0.insert(field.name().to_owned(), value.to_owned());
        }

        fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
            self.0.insert(field.name().to_owned(), value.to_string());
        }
    }

    impl<S> Layer<S> for EventCapture
    where
        S: Subscriber,
    {
        fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
            let mut fields = BTreeMap::new();
            event.record(&mut FieldCapture(&mut fields));
            self.0
                .lock()
                .expect("event capture poisoned")
                .push(CapturedEvent {
                    level: *event.metadata().level(),
                    fields,
                });
        }
    }

    impl EventCapture {
        fn events(&self) -> Vec<CapturedEvent> {
            self.0.lock().expect("event capture poisoned").clone()
        }
    }

    fn assert_initial_json_schema(quote: &ProposalWorkloadQuote) {
        let encoded =
            serde_json::to_vec_pretty(quote).expect("serialize proposal workload quote JSON");
        let json: serde_json::Value =
            serde_json::from_slice(&encoded).expect("parse proposal workload quote JSON");
        assert_eq!(quote.schema_version, 1);
        assert!(json.get("node").is_some());
        assert!(json.get("prove").is_some());
        assert!(json["blocks"][0].get("node").is_some());
        assert!(json["blocks"][0].get("prove").is_some());
        assert!(
            json["blocks"][0]["prove"]
                .get("consumed_candidate_count")
                .is_some()
        );
        assert!(
            json["blocks"][0]["prove"]
                .get("attempted_transaction_count")
                .is_some()
        );
        assert!(
            json["blocks"][0]["prove"]
                .get("started_transaction_count")
                .is_none()
        );
        assert!(json.get("unzen_zkgas_sum").is_none());
        assert!(json["blocks"][0].get("breakdown").is_none());
    }

    fn assert_exact_resources_cover_evm_local(block: &BlockWorkloadQuote) {
        let exact = block.prove.exact_resources;
        let local = block.node.pricing.evm_local_resources;
        assert!(exact.unique_account_count >= local.unique_account_count);
        assert!(exact.unique_storage_slot_access_count >= local.unique_storage_slot_access_count);
        assert!(exact.unique_storage_slot_write_count >= local.unique_storage_slot_write_count);
        assert!(exact.unique_code_hash_count >= local.unique_code_hash_count);
        assert!(exact.unique_code_bytes >= local.unique_code_bytes);
    }

    fn assert_block_formula_closes(block: &BlockWorkloadQuote) {
        assert_eq!(
            block.node.pricing.known_subtotal_scaled,
            block.node.pricing.fixed_scaled
                + block.node.pricing.operations_scaled
                + block.node.pricing.evm_local_resources_scaled,
        );
        assert_eq!(
            block.prove.pricing.total_scaled,
            block.prove.pricing.fixed_scaled
                + block.prove.pricing.operations_scaled
                + block.prove.pricing.resources_scaled
                + block.prove.pricing.mpt_scaled,
        );
        assert_exact_resources_cover_evm_local(block);
    }

    fn assert_proposal_formula_closes(quote: &ProposalWorkloadQuote) {
        assert_eq!(
            quote.prove.proposal_once.total_scaled,
            quote.prove.proposal_once.startup_scaled
                + quote.prove.proposal_once.transaction_payload_scaled
                + quote.prove.proposal_once.blob_source_scaled
                + quote.prove.proposal_once.parent_mpt_scaled,
        );
        assert_eq!(
            quote.prove.total_scaled,
            quote.prove.block_local_scaled + quote.prove.proposal_once.total_scaled,
        );
        assert_eq!(
            quote.prove.selectors.payload.total_transaction_count,
            quote
                .blocks
                .iter()
                .map(|block| block.prove.input_transaction_count)
                .sum::<u64>(),
        );
        assert_eq!(
            quote.prove.selectors.true_unattempted_transactions,
            quote
                .blocks
                .iter()
                .map(|block| block.prove.true_unattempted_transactions)
                .sum::<u64>(),
        );
    }

    #[test]
    fn block_completion_preserves_typed_shadow_diagnostic_and_block_index() {
        let error = complete_block_pricing(
            7,
            &ZkGasShadowReport::collector_unavailable(),
            &ZkGasShadowReport::collector_unavailable(),
            ResourceSelectors::default(),
            MptSelectors::default(),
        )
        .expect_err("collector diagnostic must fail completion");

        assert!(matches!(
            error,
            RuntimeQuoteError::BlockCompletion {
                block_index: 7,
                source: ZkGasCompletionError::ZkGasShadowDiagnostic(
                    ZkGasShadowDiagnostic::CollectorUnavailable
                ),
            }
        ));
    }

    #[test]
    fn block_completion_preserves_attempted_shadow_diagnostic() {
        let error = complete_block_pricing(
            9,
            &ZkGasBlockSnapshot::default().shadow_report(),
            &ZkGasShadowReport::collector_unavailable(),
            ResourceSelectors::default(),
            MptSelectors::default(),
        )
        .expect_err("attempted collector diagnostic must fail completion");

        assert!(matches!(
            error,
            RuntimeQuoteError::BlockCompletion {
                block_index: 9,
                source: ZkGasCompletionError::ZkGasShadowDiagnostic(
                    ZkGasShadowDiagnostic::CollectorUnavailable
                ),
            }
        ));
    }

    #[test]
    fn block_completion_uses_attempted_execution_but_committed_exact_resources() {
        let route = OperationRoute::Opcode(0x00);
        let operation =
            |accepted_absolute_scaled, feature_error_scaled| OperationRoutePricingInput {
                route,
                accepted_absolute_scaled,
                incremental_scaled: 0,
                structural_scaled: 0,
                feature_error_scaled,
            };
        let committed_resources = ResourceSelectors {
            unique_account_count: 1,
            ..Default::default()
        };
        let attempted_resources = ResourceSelectors {
            unique_account_count: 7,
            ..Default::default()
        };
        let committed = ZkGasBlockSnapshot {
            operation_routes: vec![operation(10, 0)],
            resources: committed_resources,
            ..Default::default()
        }
        .shadow_report();
        let attempted = ZkGasBlockSnapshot {
            started_non_anchor_transactions: 1,
            operation_routes: vec![operation(50, 7)],
            resources: attempted_resources,
            ..Default::default()
        }
        .shadow_report();

        let (node, prove) = complete_block_pricing(
            0,
            &committed,
            &attempted,
            committed_resources,
            MptSelectors::default(),
        )
        .expect("complete mixed-scope block pricing");

        assert_eq!(node.operations_scaled, 10);
        assert_eq!(prove.operations_scaled, 57);
        assert_eq!(prove.feature_error_scaled, 7);
        assert_ne!(prove.fixed_scaled, node.fixed_scaled);
        assert_eq!(prove.resources_scaled, node.evm_local_resources_scaled);
        assert_eq!(
            prove.total_scaled,
            prove.fixed_scaled + prove.operations_scaled + prove.resources_scaled
        );
    }

    #[test]
    fn transaction_count_domains_distinguish_consumed_attempted_and_unattempted() {
        assert_eq!(true_unattempted_transactions(5, 4, 3).unwrap(), 1);
        assert_eq!(true_unattempted_transactions(5, 5, 3).unwrap(), 0);
        assert!(matches!(
            true_unattempted_transactions(5, 3, 4),
            Err(RuntimeQuoteError::Topology("transaction_counts"))
        ));
        assert!(matches!(
            true_unattempted_transactions(5, 6, 5),
            Err(RuntimeQuoteError::Topology("transaction_counts"))
        ));
    }

    #[test]
    fn discarded_attempts_emit_one_structured_warning() {
        let capture = EventCapture::default();
        let subscriber = tracing_subscriber::registry().with(capture.clone());

        let discarded = tracing::subscriber::with_default(subscriber, || {
            observe_discarded_attempts("guest-input-sha256", 7, 12_345, 6, 5, 4, 3)
        })
        .expect("attempted count covers committed count");

        assert_eq!(discarded, 1);
        let events = capture.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].level, Level::WARN);
        assert_eq!(
            events[0].fields,
            BTreeMap::from([
                ("attempted_transaction_count".to_owned(), "4".to_owned()),
                ("block_index".to_owned(), "7".to_owned()),
                ("block_number".to_owned(), "12345".to_owned()),
                ("committed_transaction_count".to_owned(), "3".to_owned()),
                ("consumed_candidate_count".to_owned(), "5".to_owned()),
                ("discarded_attempt_count".to_owned(), "1".to_owned()),
                (
                    "guest_input_sha256".to_owned(),
                    "guest-input-sha256".to_owned(),
                ),
                ("input_transaction_count".to_owned(), "6".to_owned()),
                (
                    "message".to_owned(),
                    "discarded attempted transactions observed during proposal quote replay"
                        .to_owned(),
                ),
            ])
        );
    }

    #[test]
    fn zero_discarded_attempts_emit_no_warning() {
        let capture = EventCapture::default();
        let subscriber = tracing_subscriber::registry().with(capture.clone());

        let discarded = tracing::subscriber::with_default(subscriber, || {
            observe_discarded_attempts("guest-input-sha256", 7, 12_345, 6, 5, 4, 4)
        })
        .expect("attempted count equals committed count");

        assert_eq!(discarded, 0);
        assert!(capture.events().is_empty());
    }

    #[test]
    fn committed_count_exceeding_attempted_count_is_a_topology_error() {
        let error = observe_discarded_attempts("guest-input-sha256", 7, 12_345, 6, 5, 3, 4)
            .expect_err("committed count cannot exceed attempted count");

        assert!(matches!(
            error,
            RuntimeQuoteError::Topology("transaction_counts")
        ));
    }

    #[test]
    #[allow(clippy::too_many_lines)] // One assertion table pins the complete fixture quote contract.
    fn checked_in_current_guest_input_completes_quote_with_public_output_parity() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_41455_l2_12556056_12556247.json",
        );
        let input: GuestInput = serde_json::from_str(
            &fs::read_to_string(&fixture)
                .unwrap_or_else(|error| panic!("read {}: {error}", fixture.display())),
        )
        .expect("parse checked-in current GuestInput");
        let expected_output = hash_shasta_subproof_input(&input.proof_carry_data);

        let quote = quote_shasta_proposal(&input).expect("complete proposal workload quote");

        assert_eq!(quote.public_output, expected_output);
        assert_initial_json_schema(&quote);
        assert_eq!(
            quote.model_id,
            "93d04eddda3779393807808859eb36561d35aad0cdd8a8a3960ec3e279a93046"
        );
        assert_eq!(
            quote.guest_input_sha256,
            "10e26ee7c6c05eecf23e3c19c58e9c0c4269a877de83df1f1b70a33f06008506"
        );
        assert_eq!(quote.guest_input_bincode_length, 4_916_804);
        assert_eq!(quote.blocks.len(), 192);
        assert_eq!(
            quote
                .blocks
                .iter()
                .map(|block| block.prove.consumed_candidate_count)
                .sum::<u64>(),
            199
        );
        assert_eq!(
            quote
                .blocks
                .iter()
                .map(|block| block.prove.attempted_transaction_count)
                .sum::<u64>(),
            199
        );
        assert!(quote.blocks.iter().all(|block| {
            block.prove.input_transaction_count == block.prove.consumed_candidate_count
                && block.prove.consumed_candidate_count == block.prove.attempted_transaction_count
        }));
        assert_eq!(
            quote.node.unzen_zkgas_sum, 259_514_910,
            "the checked-in proposal uses mainnet Unzen pricing"
        );
        assert_eq!(
            quote.blocks.iter().fold(
                (0_i128, 0_i128, 0_i128, 0_i128),
                |(fixed, operations, feature, resources), block| (
                    fixed + block.node.pricing.fixed_scaled,
                    operations + block.node.pricing.operations_scaled,
                    feature + block.node.pricing.feature_error_scaled,
                    resources + block.node.pricing.evm_local_resources_scaled,
                ),
            ),
            (
                247_097_387_893_866_428_488_965_859,
                3_160_041_535_744_141_665_900_594_819,
                0,
                35_600_428_000_000_000_000_000_000,
            )
        );
        assert_eq!(
            quote.blocks.iter().fold(
                (0_i128, 0_i128, 0_i128, 0_i128, 0_i128),
                |(fixed, operations, feature, resources, mpt), block| (
                    fixed + block.prove.pricing.fixed_scaled,
                    operations + block.prove.pricing.operations_scaled,
                    feature + block.prove.pricing.feature_error_scaled,
                    resources + block.prove.pricing.resources_scaled,
                    mpt + block.prove.pricing.mpt_scaled,
                ),
            ),
            (
                247_097_387_893_866_428_488_965_859,
                3_160_041_535_744_141_665_900_594_819,
                0,
                35_600_428_000_000_000_000_000_000,
                1_185_183_312_000_000_000_000_000_000,
            )
        );
        assert_eq!(
            quote.node,
            NodeProposalWorkloadQuote {
                unzen_zkgas_sum: 259_514_910,
                block_local_known_scaled: 3_442_739_351_638_008_094_389_560_678,
                block_local_known_pgu: 3_442_739_352,
                sum_of_block_known_pgu: 3_442_739_480,
            }
        );
        assert_eq!(
            quote.prove.selectors,
            ProposalPricingSelectorsSnapshot {
                payload: ProposalPayloadSelectors {
                    total_transaction_count: 199,
                    canonical_transaction_bytes: 48_022,
                    nonempty_payload_transactions: 199,
                },
                true_unattempted_transactions: 0,
                blobs: ProposalBlobSelectors {
                    source_blob_counts: vec![1],
                    proposal_blob_hash_count: 1,
                    provider_blob_count: 1,
                    provider_commitment_count: 1,
                    provider_proof_count: 1,
                },
                parent_mpt: MptSelectorsSnapshot {
                    path_node_visits: 15,
                    materialization_work_units: 1074,
                    hydration_hash_blocks: 328,
                    dirty_encoded_nodes: 0,
                    dirty_hash_blocks: 0,
                    structural_branch_extension_churn_nodes: 0,
                    dropped_trie_nodes: 0,
                },
            }
        );
        assert_eq!(
            quote.prove.proposal_once,
            PricingBreakdownSnapshot {
                fixed_scaled: 0,
                operations_scaled: 0,
                feature_error_scaled: 0,
                resources_scaled: 0,
                mpt_scaled: 0,
                startup_scaled: 154_263_870_735_480_402_119_958_866,
                transaction_payload_scaled: 15_554_048_000_000_000_000_000_000,
                blob_source_scaled: 0,
                parent_mpt_scaled: 6_357_840_000_000_000_000_000_000,
                total_scaled: 176_175_758_735_480_402_119_958_866,
                total_pgu: 176_175_759,
            }
        );
        assert_eq!(
            quote.prove.block_local_scaled,
            4_627_922_663_638_008_094_389_560_678
        );
        assert_eq!(quote.prove.block_local_pgu, 4_627_922_664);
        assert_eq!(quote.prove.sum_of_block_local_pgu, 4_627_922_792);
        assert_eq!(quote.prove.proposal_once_pgu, 176_175_759);
        assert_eq!(
            quote.prove.total_scaled,
            4_804_098_422_373_488_496_509_519_544
        );
        assert_eq!(quote.prove.total_pgu, 4_804_098_423);
        quote.blocks.iter().for_each(assert_block_formula_closes);
        assert_proposal_formula_closes(&quote);
    }

    #[test]
    fn full_quote_still_rejects_an_invalid_blob_proof() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_41455_l2_12556056_12556247.json",
        );
        let mut input: GuestInput = serde_json::from_str(
            &fs::read_to_string(&fixture)
                .unwrap_or_else(|error| panic!("read {}: {error}", fixture.display())),
        )
        .expect("parse checked-in current GuestInput");
        input.taiko.data_sources[0].blob_proofs[0][0] ^= 0x01;

        let error = quote_shasta_proposal(&input).expect_err("canonical validation must reject");
        assert!(
            error
                .to_string()
                .contains("Blob proof-of-equivalence verification failed"),
            "unexpected error: {error:#}"
        );
    }
}
