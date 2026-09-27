//! Traced derived-block and proposal reconstruction.

use alethia_reth_block::{
    config::{MissingBaseFee, TaikoEvmConfig, TaikoNextBlockEnvAttributes},
    derived_block::DerivedBlockExecutionOutcome,
    executor::TaikoBlockExecutor,
};
use alloy_consensus::{
    TxReceipt,
    transaction::{SignerRecoverable, Transaction, TxHashRef},
};
use alloy_evm::EvmFactory;
use alloy_primitives::{B256, TxKind};
use reth_ethereum_primitives::{Block, Receipt, TransactionSigned};
use reth_evm::{ConfigureEvm, block::BlockExecutionError};
use reth_execution_types::BlockExecutionResult;
use reth_primitives_traits::{RecoveredBlock, SealedHeader};
use reth_revm::{
    Database, State,
    db::states::{BundleState, bundle_state::BundleRetention},
};
use reth_trie_common::{HashedPostState, KeccakKeyHasher};
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use sha2::{Digest, Sha256};

use crate::{
    OperationTrace, TraceInspector, TraceSink,
    transactions::{
        RecoveredTransactionOccurrence, TracingTransactions, TransactionTrace,
        classify_started_occurrences_with_statuses, is_native_value_transfer,
    },
};

pub const OPERATION_TRACE_SCHEMA_VERSION: u32 = 2;

fn deserialize_operation_trace_schema_version<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let version = u32::deserialize(deserializer)?;
    if version != OPERATION_TRACE_SCHEMA_VERSION {
        return Err(D::Error::custom(format_args!(
            "unsupported operation trace schema version {version}; expected {OPERATION_TRACE_SCHEMA_VERSION}"
        )));
    }
    Ok(version)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionParityRecord {
    pub committed_transaction_hashes: Vec<B256>,
    pub execution_result: BlockExecutionResult<Receipt>,
    pub hashed_state: HashedPostState,
    pub assembled_block: Block,
    pub assembled_senders: Vec<alloy_primitives::Address>,
    pub state_root: B256,
    pub finalized_block_zkgas: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlockTrace {
    pub block_index: usize,
    pub block_number: u64,
    pub input_transaction_count: usize,
    pub started_transaction_count: usize,
    pub committed_transaction_hashes: Vec<B256>,
    pub attempted_transaction_hashes: Vec<B256>,
    pub unattempted_transaction_hashes: Vec<B256>,
    pub native_value_transfer_count: usize,
    pub finalized_block_zkgas: u64,
    pub transactions: Vec<TransactionTrace>,
    pub operations: Vec<OperationTrace>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PartialBlockTrace {
    pub block_index: usize,
    pub block_number: u64,
    pub started_transaction_count: usize,
    pub operations: Vec<OperationTrace>,
}

impl BlockTrace {
    fn into_partial(self) -> PartialBlockTrace {
        PartialBlockTrace {
            block_index: self.block_index,
            block_number: self.block_number,
            started_transaction_count: self.started_transaction_count,
            operations: self.operations,
        }
    }
}

#[derive(Debug)]
pub struct TracingDerivedBlockExecutor {
    block_index: usize,
    manifest_indices: Vec<usize>,
    sink: TraceSink,
    completed: Option<BlockTrace>,
    partial: Option<PartialBlockTrace>,
}

impl TracingDerivedBlockExecutor {
    #[must_use]
    pub fn new(block_index: usize) -> Self {
        Self::with_manifest_indices(block_index, Vec::new())
    }

    #[must_use]
    pub fn with_manifest_indices(block_index: usize, manifest_indices: Vec<usize>) -> Self {
        Self {
            block_index,
            manifest_indices,
            sink: TraceSink::default(),
            completed: None,
            partial: None,
        }
    }

    pub const fn take_completed(&mut self) -> Option<BlockTrace> {
        self.completed.take()
    }

    pub const fn take_partial(&mut self) -> Option<PartialBlockTrace> {
        self.partial.take()
    }

    fn take_failure_diagnostics(&mut self) -> Option<PartialBlockTrace> {
        self.partial
            .take()
            .or_else(|| self.completed.take().map(BlockTrace::into_partial))
    }
}

fn attributes_from_derived_block(
    derived_block: &RecoveredBlock<Block>,
) -> Result<TaikoNextBlockEnvAttributes, BlockExecutionError> {
    let header = derived_block.header();
    let base_fee_per_gas = header.base_fee_per_gas.ok_or_else(|| {
        BlockExecutionError::other(MissingBaseFee {
            block_number: header.number,
        })
    })?;
    Ok(TaikoNextBlockEnvAttributes {
        timestamp: header.timestamp,
        suggested_fee_recipient: header.beneficiary,
        prev_randao: header.mix_hash,
        gas_limit: header.gas_limit,
        extra_data: header.extra_data.clone(),
        base_fee_per_gas,
        parent_beacon_block_root: header.parent_beacon_block_root,
    })
}

impl raiko2_stateless::DerivedBlockExecutor for TracingDerivedBlockExecutor {
    #[allow(clippy::too_many_lines)]
    fn execute<DB: Database + std::fmt::Debug>(
        &mut self,
        evm_config: &TaikoEvmConfig,
        parent_header: &SealedHeader,
        derived_block: &RecoveredBlock<Block>,
        db: DB,
    ) -> Result<DerivedBlockExecutionOutcome, BlockExecutionError> {
        let mut state = State::builder()
            .with_database(db)
            .with_bundle_update()
            .build();
        let attributes = attributes_from_derived_block(derived_block)?;
        let evm_env = evm_config
            .next_evm_env(parent_header, &attributes)
            .map_err(BlockExecutionError::other)?;
        let inspector = TraceInspector::new(self.sink.clone());
        let evm = evm_config
            .evm_factory()
            .create_evm_with_inspector(&mut state, evm_env, inspector);
        let execution_ctx = evm_config
            .context_for_next_block(parent_header, attributes)
            .map_err(BlockExecutionError::other)?;
        let finalized_zk_gas = execution_ctx.finalized_block_zk_gas.clone();
        let executor = TaikoBlockExecutor::new(
            evm,
            execution_ctx,
            evm_config.executor_factory.spec().clone(),
            evm_config.executor_factory.receipt_builder(),
        );

        let transaction_metadata = derived_block
            .transactions_recovered()
            .map(|tx| {
                let transaction: &TransactionSigned = tx.inner();
                (
                    Transaction::value(transaction),
                    matches!(Transaction::kind(transaction), TxKind::Call(_)),
                )
            })
            .collect::<Vec<_>>();
        let all_occurrences = derived_block
            .transactions_recovered()
            .enumerate()
            .map(|(recovered_index, tx)| RecoveredTransactionOccurrence {
                recovered_index,
                manifest_index: recovered_index
                    .checked_sub(1)
                    .and_then(|index| self.manifest_indices.get(index).copied()),
                tx_hash: *tx.inner().tx_hash(),
                is_anchor: recovered_index == 0,
            })
            .collect::<Vec<_>>();
        let mut started = Vec::new();
        let transactions = TracingTransactions::new(
            derived_block.transactions_recovered(),
            self.sink.clone(),
            &mut started,
            &self.manifest_indices,
        );
        let execution_outcome =
            match executor.execute_block_with_committed_transactions(transactions) {
                Ok(outcome) => outcome,
                Err(error) => {
                    self.sink.lock().finish_transaction();
                    self.partial = Some(PartialBlockTrace {
                        block_index: self.block_index,
                        block_number: derived_block.number,
                        started_transaction_count: started.len(),
                        operations: self.sink.snapshot().into_operations(),
                    });
                    return Err(error);
                }
            };
        self.sink.lock().finish_transaction();
        state.merge_transitions(BundleRetention::Reverts);

        let committed_hashes = execution_outcome
            .committed_transactions
            .iter()
            .map(|tx| *tx.inner().tx_hash())
            .collect::<Vec<_>>();
        let committed_statuses = execution_outcome
            .execution_result
            .receipts
            .iter()
            .map(TxReceipt::status)
            .collect::<Vec<_>>();
        let mut transactions = classify_started_occurrences_with_statuses(
            &started,
            &committed_hashes,
            &committed_statuses,
        )
        .map_err(BlockExecutionError::msg)?;
        transactions.extend(
            all_occurrences
                .into_iter()
                .skip(started.len())
                .map(RecoveredTransactionOccurrence::into_unattempted),
        );
        let operations = self.sink.snapshot().into_operations();
        for transaction in &mut transactions {
            let Some(started_tx_index) = transaction.started_tx_index else {
                continue;
            };
            let (value, is_call) = transaction_metadata[transaction.recovered_index];
            let has_operation_trace = operations
                .iter()
                .any(|operation| operation.tx_index == Some(started_tx_index));
            transaction.native_value_transfer = is_native_value_transfer(
                transaction.disposition,
                value,
                is_call,
                has_operation_trace,
            );
        }
        let attempted_transaction_hashes = transactions
            .iter()
            .filter(|tx| matches!(tx.disposition, crate::TransactionDisposition::Attempted))
            .map(|tx| tx.tx_hash)
            .collect();
        let unattempted_transaction_hashes = transactions
            .iter()
            .filter(|tx| matches!(tx.disposition, crate::TransactionDisposition::Unattempted))
            .map(|tx| tx.tx_hash)
            .collect();
        let finalized_block_zkgas = finalized_zk_gas.load(std::sync::atomic::Ordering::Relaxed);
        self.completed = Some(BlockTrace {
            block_index: self.block_index,
            block_number: derived_block.number,
            input_transaction_count: transactions.len(),
            started_transaction_count: started.len(),
            committed_transaction_hashes: committed_hashes,
            attempted_transaction_hashes,
            unattempted_transaction_hashes,
            native_value_transfer_count: transactions
                .iter()
                .filter(|transaction| transaction.native_value_transfer)
                .count(),
            finalized_block_zkgas,
            transactions,
            operations,
        });

        let bundle_state: BundleState = state.take_bundle();
        let hashed_state =
            HashedPostState::from_bundle_state::<KeccakKeyHasher>(bundle_state.state());
        Ok(DerivedBlockExecutionOutcome {
            committed_transactions: execution_outcome.committed_transactions,
            execution_result: execution_outcome.execution_result,
            hashed_state,
            finalized_block_zk_gas: finalized_block_zkgas,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalTraceStatus {
    Complete,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryFailure {
    pub block_index: usize,
    pub manifest_index: usize,
    pub signed_transaction_hash: B256,
    pub outcome: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceFailure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_index: Option<usize>,
    pub stage: String,
    pub error: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParityStatus {
    pub passed: bool,
    pub mismatch_fields: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProposalTrace {
    #[serde(deserialize_with = "deserialize_operation_trace_schema_version")]
    pub schema_version: u32,
    pub guest_input_sha256: String,
    pub guest_input_bincode_length: usize,
    pub status: ProposalTraceStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_output: Option<B256>,
    pub blocks: Vec<BlockTrace>,
    pub partial_blocks: Vec<PartialBlockTrace>,
    pub recovery_failures: Vec<RecoveryFailure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<TraceFailure>,
    pub parity: ParityStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalTraceSummary {
    #[serde(deserialize_with = "deserialize_operation_trace_schema_version")]
    pub schema_version: u32,
    pub full_trace_encoding: String,
    pub guest_input_sha256: String,
    pub guest_input_bincode_length: usize,
    pub status: ProposalTraceStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_output: Option<B256>,
    pub parity_passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_stage: Option<String>,
    pub block_count: usize,
    pub partial_block_count: usize,
    pub recovery_failure_count: usize,
    pub operation_count: usize,
}

impl ProposalTrace {
    #[must_use]
    pub fn summary(&self) -> ProposalTraceSummary {
        ProposalTraceSummary {
            schema_version: self.schema_version,
            full_trace_encoding: "json+gzip".to_string(),
            guest_input_sha256: self.guest_input_sha256.clone(),
            guest_input_bincode_length: self.guest_input_bincode_length,
            status: self.status,
            public_output: self.public_output,
            parity_passed: self.parity.passed,
            failure_stage: self.failure.as_ref().map(|failure| failure.stage.clone()),
            block_count: self.blocks.len(),
            partial_block_count: self.partial_blocks.len(),
            recovery_failure_count: self.recovery_failures.len(),
            operation_count: self
                .blocks
                .iter()
                .map(|block| block.operations.len())
                .chain(
                    self.partial_blocks
                        .iter()
                        .map(|block| block.operations.len()),
                )
                .sum(),
        }
    }
}

/// Returns the canonical bincode SHA-256 and encoded byte length for a guest input.
///
/// # Errors
///
/// Returns an error when the guest input cannot be serialized with bincode.
pub fn guest_input_identity(
    guest_input: &raiko2_primitives_shasta::GuestInput,
) -> anyhow::Result<(String, usize)> {
    let input_bytes = bincode::serialize(guest_input)?;
    Ok(guest_input_identity_from_bincode(&input_bytes))
}

fn guest_input_identity_from_bincode(input_bytes: &[u8]) -> (String, usize) {
    (
        format!("0x{}", hex::encode(Sha256::digest(input_bytes))),
        input_bytes.len(),
    )
}

fn project_parity(
    outcome: &raiko2_stateless::FilteredBlockExecutionOutcome,
) -> ExecutionParityRecord {
    let committed_transaction_hashes = outcome
        .filtered_block
        .body()
        .transactions()
        .map(|tx| *tx.tx_hash())
        .collect();
    ExecutionParityRecord {
        committed_transaction_hashes,
        execution_result: outcome.execution_result.clone(),
        hashed_state: outcome.hashed_state.clone(),
        assembled_block: outcome.filtered_block.clone().into_block(),
        assembled_senders: outcome.filtered_block.senders().to_vec(),
        state_root: outcome.filtered_block.state_root,
        finalized_block_zkgas: outcome.filtered_block.difficulty.to::<u64>(),
    }
}

fn parity_mismatches(
    ordinary: &[ExecutionParityRecord],
    traced: &[ExecutionParityRecord],
    ordinary_output: B256,
    traced_output: B256,
) -> Vec<String> {
    let mut mismatches = Vec::new();
    if ordinary.len() != traced.len() {
        mismatches.push("block_count".to_string());
    }
    for (index, (ordinary, traced)) in ordinary.iter().zip(traced).enumerate() {
        macro_rules! compare {
            ($field:ident) => {
                if ordinary.$field != traced.$field {
                    mismatches.push(format!("blocks[{index}].{}", stringify!($field)));
                }
            };
        }
        compare!(committed_transaction_hashes);
        compare!(execution_result);
        compare!(hashed_state);
        compare!(assembled_block);
        compare!(assembled_senders);
        compare!(state_root);
        compare!(finalized_block_zkgas);
    }
    if ordinary_output != traced_output {
        mismatches.push("public_output".to_string());
    }
    mismatches
}

fn recover_manifest_transactions(
    block_index: usize,
    transactions: &[TransactionSigned],
) -> (Vec<usize>, Vec<RecoveryFailure>) {
    let mut manifest_indices = Vec::new();
    let mut failures = Vec::new();
    for (manifest_index, transaction) in transactions.iter().enumerate() {
        if transaction.clone().try_into_recovered().is_ok() {
            manifest_indices.push(manifest_index);
        } else {
            failures.push(RecoveryFailure {
                block_index,
                manifest_index,
                signed_transaction_hash: *transaction.tx_hash(),
                outcome: "signer_recovery_failed".to_string(),
            });
        }
    }
    (manifest_indices, failures)
}

/// Executes ordinary and traced Shasta reconstruction from fresh copies of the same input.
///
/// # Errors
///
/// Returns an error when the input cannot be serialized or deserialized. Execution and parity
/// failures are represented in the returned trace schema.
pub fn trace_shasta_proposal(
    guest_input: &raiko2_primitives_shasta::GuestInput,
) -> anyhow::Result<ProposalTrace> {
    trace_shasta_proposal_with_parity_mutator(guest_input, |_, _| {})
}

#[allow(clippy::too_many_lines)]
fn trace_shasta_proposal_with_parity_mutator<F>(
    guest_input: &raiko2_primitives_shasta::GuestInput,
    mut mutate_traced_parity: F,
) -> anyhow::Result<ProposalTrace>
where
    F: FnMut(usize, &mut ExecutionParityRecord),
{
    let input_bytes = bincode::serialize(guest_input)?;
    let (input_hash, input_length) = guest_input_identity_from_bincode(&input_bytes);
    let ordinary_input = bincode::deserialize(&input_bytes)?;
    let traced_input = bincode::deserialize(&input_bytes)?;

    let mut ordinary_records = Vec::new();
    let ordinary_result =
        raiko2_guest_common::prove_shasta_proposal_with_reconstructor(&ordinary_input, |request| {
            let outcome =
                raiko2_stateless::reconstruct_block_from_transactions_with_witness_resources(
                    request.anchor_tx,
                    request.transactions,
                    request.block_env,
                    request.witness,
                    request.ancestor_headers,
                    request.shared_state_nodes,
                    request.chain_spec,
                    request.evm_config,
                )
                .map_err(anyhow::Error::from)?;
            ordinary_records.push(project_parity(&outcome));
            Ok(outcome)
        });

    let mut traced_records = Vec::new();
    let mut blocks = Vec::new();
    let mut partial_blocks = Vec::new();
    let mut recovery_failures = Vec::new();
    let mut pending_wrapper_validation_block = None;
    let traced_result = raiko2_guest_common::prove_shasta_proposal_with_reconstructor(
        &traced_input,
        |request| {
            pending_wrapper_validation_block = None;
            let input_transaction_count = request.transactions.len() + 1;
            let (manifest_indices, block_recovery_failures) =
                recover_manifest_transactions(request.index, &request.transactions);
            recovery_failures.extend(block_recovery_failures);
            let mut executor =
                TracingDerivedBlockExecutor::with_manifest_indices(request.index, manifest_indices);
            let outcome = match raiko2_stateless::reconstruct_block_from_transactions_with_executor_and_witness_resources(
                request.anchor_tx,
                request.transactions,
                request.block_env,
                request.witness,
                request.ancestor_headers,
                request.shared_state_nodes,
                request.chain_spec,
                request.evm_config,
                &mut executor,
            ) {
                Ok(outcome) => outcome,
                Err(error) => {
                    if let Some(partial) = executor.take_failure_diagnostics() {
                        partial_blocks.push(partial);
                    }
                    return Err(anyhow::Error::from(error));
                }
            };
            let mut block = executor
                .take_completed()
                .ok_or_else(|| anyhow::anyhow!("traced executor returned no block diagnostics"))?;
            block.input_transaction_count = input_transaction_count;
            let mut parity = project_parity(&outcome);
            mutate_traced_parity(request.index, &mut parity);
            traced_records.push(parity);
            blocks.push(block);
            pending_wrapper_validation_block = Some(request.index);
            Ok(outcome)
        },
    );

    if traced_result.is_err()
        && pending_wrapper_validation_block
            .zip(blocks.last().map(|block| block.block_index))
            .is_some_and(|(pending, last)| pending == last)
    {
        partial_blocks.push(
            blocks
                .pop()
                .expect("last traced block exists")
                .into_partial(),
        );
    }

    if let Err(error) = ordinary_result.as_ref() {
        let failed_block_index = partial_blocks
            .last()
            .map(|block| block.block_index)
            .or_else(|| blocks.last().map(|block| block.block_index));
        return Ok(ProposalTrace {
            schema_version: OPERATION_TRACE_SCHEMA_VERSION,
            guest_input_sha256: input_hash,
            guest_input_bincode_length: input_length,
            status: ProposalTraceStatus::Failed,
            public_output: None,
            blocks,
            partial_blocks,
            recovery_failures,
            failure: Some(TraceFailure {
                block_index: failed_block_index,
                stage: "ordinary".to_string(),
                error: error.to_string(),
            }),
            parity: ParityStatus {
                passed: false,
                mismatch_fields: Vec::new(),
            },
        });
    }

    if let Err(error) = traced_result.as_ref() {
        let failed_block_index = partial_blocks
            .last()
            .map(|block| block.block_index)
            .or_else(|| blocks.last().map(|block| block.block_index));
        return Ok(ProposalTrace {
            schema_version: OPERATION_TRACE_SCHEMA_VERSION,
            guest_input_sha256: input_hash,
            guest_input_bincode_length: input_length,
            status: ProposalTraceStatus::Failed,
            public_output: None,
            blocks,
            partial_blocks,
            recovery_failures,
            failure: Some(TraceFailure {
                block_index: failed_block_index,
                stage: "traced".to_string(),
                error: error.to_string(),
            }),
            parity: ParityStatus {
                passed: false,
                mismatch_fields: Vec::new(),
            },
        });
    }

    let ordinary_output = ordinary_result.expect("ordinary result checked");
    let traced_output = traced_result.expect("traced result checked");

    let mismatch_fields = parity_mismatches(
        &ordinary_records,
        &traced_records,
        ordinary_output,
        traced_output,
    );
    let passed = mismatch_fields.is_empty();
    Ok(ProposalTrace {
        schema_version: OPERATION_TRACE_SCHEMA_VERSION,
        guest_input_sha256: input_hash,
        guest_input_bincode_length: input_length,
        status: if passed {
            ProposalTraceStatus::Complete
        } else {
            ProposalTraceStatus::Failed
        },
        public_output: passed.then_some(traced_output),
        blocks,
        partial_blocks,
        recovery_failures,
        failure: (!passed).then(|| TraceFailure {
            block_index: None,
            stage: "parity".to_string(),
            error: "ordinary/traced execution parity mismatch".to_string(),
        }),
        parity: ParityStatus {
            passed,
            mismatch_fields,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TransactionDisposition;
    use alethia_reth_chainspec::TAIKO_DEVNET;
    use alloy_consensus::{Header, SignableTransaction, TxEip1559};
    use alloy_primitives::{Address, Bytes, Signature, TxKind, U256};
    use reth_ethereum_primitives::BlockBody;
    use reth_primitives_traits::SignerRecoverable;
    use reth_revm::{
        db::InMemoryDB,
        state::{AccountInfo, Bytecode, bytecode::opcode},
    };

    fn signed_transaction(chain_id: u64, signature: Signature) -> TransactionSigned {
        test_transaction(
            chain_id,
            0,
            TxKind::Call(Address::with_last_byte(0x22)),
            U256::ZERO,
            100_000,
            signature,
        )
    }

    fn test_transaction(
        chain_id: u64,
        nonce: u64,
        to: TxKind,
        value: U256,
        gas_limit: u64,
        signature: Signature,
    ) -> TransactionSigned {
        TxEip1559 {
            chain_id,
            nonce,
            gas_limit,
            max_fee_per_gas: 1,
            max_priority_fee_per_gas: 0,
            to,
            value,
            access_list: Vec::new().into(),
            input: Bytes::new(),
        }
        .into_signed(signature)
        .into()
    }

    fn insert_contract(db: &mut InMemoryDB, address: Address, bytecode: Bytecode) {
        db.insert_account_info(
            address,
            AccountInfo {
                nonce: 1,
                code_hash: bytecode.hash_slow(),
                code: Some(bytecode),
                ..Default::default()
            },
        );
    }

    fn repeated_calls(target: Address, count: usize) -> Bytecode {
        let mut code = Vec::with_capacity(count * 36 + 1);
        for _ in 0..count {
            code.extend_from_slice(&[
                opcode::PUSH1,
                0,
                opcode::PUSH1,
                0,
                opcode::PUSH1,
                0,
                opcode::PUSH1,
                0,
                opcode::PUSH1,
                0,
                opcode::PUSH20,
            ]);
            code.extend_from_slice(target.as_slice());
            code.extend_from_slice(&[opcode::PUSH2, 0xff, 0xff, opcode::CALL]);
        }
        code.push(opcode::STOP);
        Bytecode::new_raw(Bytes::from(code))
    }

    #[test]
    fn signer_recovery_failures_keep_manifest_identity() {
        let recoverable = signed_transaction(167, Signature::test_signature());
        let unrecoverable = signed_transaction(167, Signature::new(U256::ZERO, U256::ZERO, false));
        assert!(unrecoverable.clone().try_into_recovered().is_err());

        let (indices, failures) =
            recover_manifest_transactions(7, &[recoverable, unrecoverable.clone()]);

        assert_eq!(indices, vec![0]);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].block_index, 7);
        assert_eq!(failures[0].manifest_index, 1);
        assert_eq!(
            failures[0].signed_transaction_hash,
            *unrecoverable.tx_hash()
        );
        assert_eq!(failures[0].outcome, "signer_recovery_failed");
    }

    #[test]
    fn failed_anchor_receipt_is_rejected_but_retains_partial_diagnostics() {
        let chain_spec = TAIKO_DEVNET.clone();
        let transaction =
            signed_transaction(chain_spec.inner.chain().id(), Signature::test_signature());
        let signer = transaction.recover_signer().expect("recover test signer");
        let body = BlockBody {
            transactions: vec![transaction],
            withdrawals: Some(Vec::new().into()),
            ..Default::default()
        };
        let parent = SealedHeader::seal_slow(Header {
            number: 0,
            timestamp: u64::MAX - 1,
            gas_limit: 30_000_000,
            base_fee_per_gas: Some(1),
            extra_data: Bytes::from(vec![0; 7]),
            ..Default::default()
        });
        let block = RecoveredBlock::new_unhashed(
            Block {
                header: Header {
                    parent_hash: parent.hash(),
                    number: 1,
                    timestamp: u64::MAX,
                    gas_limit: 30_000_000,
                    base_fee_per_gas: Some(1),
                    extra_data: Bytes::from(vec![0; 7]),
                    ..Default::default()
                },
                body,
            },
            vec![signer],
        );
        let mut db = InMemoryDB::default();
        db.insert_account_info(
            signer,
            AccountInfo {
                nonce: 0,
                balance: U256::MAX,
                ..Default::default()
            },
        );
        let target = Address::with_last_byte(0x22);
        let bytecode = reth_revm::state::Bytecode::new_raw(Bytes::from(vec![
            reth_revm::state::bytecode::opcode::PUSH1,
            1,
            reth_revm::state::bytecode::opcode::PUSH1,
            2,
            reth_revm::state::bytecode::opcode::ADD,
            reth_revm::state::bytecode::opcode::INVALID,
        ]));
        db.insert_account_info(
            target,
            AccountInfo {
                nonce: 1,
                code_hash: bytecode.hash_slow(),
                code: Some(bytecode),
                ..Default::default()
            },
        );
        let evm_config = TaikoEvmConfig::new(chain_spec);
        let mut executor = TracingDerivedBlockExecutor::new(3);

        let outcome = raiko2_stateless::DerivedBlockExecutor::execute(
            &mut executor,
            &evm_config,
            &parent,
            &block,
            db,
        )
        .expect("the EVM outcome is available for the stateless anchor check");

        assert!(!outcome.execution_result.receipts[0].status());
        let partial = executor
            .take_failure_diagnostics()
            .expect("fatal diagnostics retained");
        assert!(
            executor.take_completed().is_none(),
            "failed execution rejected"
        );
        assert_eq!(partial.block_index, 3);
        assert_eq!(partial.block_number, 1);
        assert_eq!(partial.started_transaction_count, 1);
        assert!(
            !partial.operations.is_empty(),
            "pre-execution and failed-transaction diagnostics must survive"
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn real_executor_wires_all_transaction_dispositions_into_block_trace() {
        let chain_spec = TAIKO_DEVNET.clone();
        let chain_id = chain_spec.inner.chain().id();
        let signer = Address::with_last_byte(0xaa);
        let simple_target = Address::with_last_byte(0x22);
        let native_target = Address::with_last_byte(0x33);
        let failing_target = Address::with_last_byte(0x44);
        let heavy_target = Address::with_last_byte(0x55);
        let child_target = Address::with_last_byte(0x66);
        let precompile_target = Address::with_last_byte(0x04);
        let beacon_roots_target: Address = "000f3df6d732807ef1319fb7b8bb8522d0beac02"
            .parse()
            .expect("beacon-roots address");
        let blockhashes_target: Address = "0000f90827f1c53a10cb7a02335b175320002935"
            .parse()
            .expect("blockhashes address");
        let signature = Signature::test_signature();

        let anchor = test_transaction(
            chain_id,
            0,
            TxKind::Call(simple_target),
            U256::from(1),
            100_000,
            signature,
        );
        let duplicate = test_transaction(
            chain_id,
            2,
            TxKind::Call(simple_target),
            U256::ZERO,
            100_000,
            signature,
        );
        let intervening = test_transaction(
            chain_id,
            1,
            TxKind::Call(simple_target),
            U256::ZERO,
            100_000,
            signature,
        );
        let native = test_transaction(
            chain_id,
            3,
            TxKind::Call(native_target),
            U256::from(1),
            100_000,
            signature,
        );
        let failed = test_transaction(
            chain_id,
            4,
            TxKind::Call(failing_target),
            U256::from(1),
            100_000,
            signature,
        );
        let create = test_transaction(
            chain_id,
            5,
            TxKind::Create,
            U256::from(1),
            100_000,
            signature,
        );
        let precompile = test_transaction(
            chain_id,
            6,
            TxKind::Call(precompile_target),
            U256::from(1),
            100_000,
            signature,
        );
        let zero_value = test_transaction(
            chain_id,
            7,
            TxKind::Call(native_target),
            U256::ZERO,
            100_000,
            signature,
        );
        let heavy = test_transaction(
            chain_id,
            8,
            TxKind::Call(heavy_target),
            U256::from(1),
            10_000_000,
            signature,
        );
        let untouched = test_transaction(
            chain_id,
            8,
            TxKind::Call(simple_target),
            U256::from(1),
            100_000,
            signature,
        );
        let transactions = vec![
            anchor,
            duplicate.clone(),
            intervening,
            duplicate,
            native,
            failed,
            create,
            precompile,
            zero_value,
            heavy,
            untouched,
        ];
        let body = BlockBody {
            transactions,
            withdrawals: Some(Vec::new().into()),
            ..Default::default()
        };
        let parent = SealedHeader::seal_slow(Header {
            number: 0,
            timestamp: u64::MAX - 1,
            gas_limit: 30_000_000,
            base_fee_per_gas: Some(1),
            extra_data: Bytes::from(vec![0; 7]),
            ..Default::default()
        });
        let block = RecoveredBlock::new_unhashed(
            Block {
                header: Header {
                    parent_hash: parent.hash(),
                    number: 1,
                    timestamp: u64::MAX,
                    gas_limit: 30_000_000,
                    base_fee_per_gas: Some(1),
                    extra_data: Bytes::from(vec![0; 7]),
                    parent_beacon_block_root: Some(B256::with_last_byte(0x42)),
                    ..Default::default()
                },
                body,
            },
            vec![signer; 11],
        );
        let mut db = InMemoryDB::default();
        db.insert_account_info(
            signer,
            AccountInfo {
                balance: U256::MAX,
                ..Default::default()
            },
        );
        insert_contract(
            &mut db,
            simple_target,
            Bytecode::new_raw(Bytes::from(vec![opcode::STOP])),
        );
        insert_contract(
            &mut db,
            failing_target,
            Bytecode::new_raw(Bytes::from(vec![opcode::PUSH1, 1, opcode::INVALID])),
        );
        insert_contract(
            &mut db,
            child_target,
            Bytecode::new_raw(Bytes::from(vec![opcode::STOP])),
        );
        insert_contract(
            &mut db,
            beacon_roots_target,
            Bytecode::new_raw(Bytes::from(vec![opcode::STOP])),
        );
        insert_contract(
            &mut db,
            blockhashes_target,
            Bytecode::new_raw(Bytes::from(vec![opcode::STOP])),
        );
        insert_contract(&mut db, heavy_target, repeated_calls(child_target, 500));

        let evm_config = TaikoEvmConfig::new(chain_spec);
        let mut executor = TracingDerivedBlockExecutor::new(0);
        raiko2_stateless::DerivedBlockExecutor::execute(
            &mut executor,
            &evm_config,
            &parent,
            &block,
            db,
        )
        .expect("recoverable non-anchor failures must return a block outcome");
        let trace = executor.take_completed().expect("completed block trace");

        assert_eq!(trace.transactions.len(), 11);
        let first_transaction_operation = trace
            .operations
            .iter()
            .position(|operation| operation.phase == crate::OperationPhase::Transaction)
            .expect("transaction operation");
        assert!(first_transaction_operation > 0, "system operation prefix");
        assert!(
            trace.operations[..first_transaction_operation]
                .iter()
                .all(|operation| operation.phase == crate::OperationPhase::System),
            "pre-execution system-contract work must precede transaction iteration"
        );
        assert!(
            trace.operations[first_transaction_operation..]
                .iter()
                .all(|operation| operation.phase == crate::OperationPhase::Transaction),
            "system phase must end at the first lazy iterator yield"
        );
        assert_eq!(
            trace.transactions[1].disposition,
            TransactionDisposition::Attempted
        );
        assert_eq!(trace.transactions[1].tx_hash, trace.transactions[3].tx_hash);
        assert_eq!(
            trace.transactions[2].disposition,
            TransactionDisposition::CommittedSuccess
        );
        assert_eq!(
            trace.transactions[3].disposition,
            TransactionDisposition::CommittedSuccess
        );
        assert!(trace.transactions[4].native_value_transfer);
        assert_eq!(
            trace.transactions[5].disposition,
            TransactionDisposition::CommittedFailure
        );
        assert_eq!(
            trace.transactions[6].disposition,
            TransactionDisposition::CommittedSuccess
        );
        assert_eq!(
            trace.transactions[7].disposition,
            TransactionDisposition::CommittedSuccess
        );
        assert_eq!(
            trace.transactions[8].disposition,
            TransactionDisposition::CommittedSuccess
        );
        assert_eq!(
            trace.transactions[9].disposition,
            TransactionDisposition::Attempted
        );
        assert_eq!(
            trace.transactions[10].disposition,
            TransactionDisposition::Unattempted
        );
        assert_eq!(trace.transactions[10].started_tx_index, None);
        assert_eq!(trace.native_value_transfer_count, 1);
        for index in [0, 1, 2, 3, 5, 6, 7, 8, 9, 10] {
            assert!(
                !trace.transactions[index].native_value_transfer,
                "transaction {index} must not be classified as a native transfer"
            );
        }
    }

    #[test]
    fn real_executor_preserves_manifest_indices_across_recovery_failures() {
        let chain_spec = TAIKO_DEVNET.clone();
        let chain_id = chain_spec.inner.chain().id();
        let signer = Address::with_last_byte(0xaa);
        let target = Address::with_last_byte(0x22);
        let signature = Signature::test_signature();
        let anchor = test_transaction(
            chain_id,
            0,
            TxKind::Call(target),
            U256::ZERO,
            100_000,
            signature,
        );
        let unrecoverable = test_transaction(
            chain_id,
            1,
            TxKind::Call(target),
            U256::ZERO,
            100_000,
            Signature::new(U256::ZERO, U256::ZERO, false),
        );
        let candidate = test_transaction(
            chain_id,
            1,
            TxKind::Call(target),
            U256::ZERO,
            100_000,
            signature,
        );
        let (manifest_indices, recovery_failures) =
            recover_manifest_transactions(4, &[unrecoverable.clone(), candidate.clone()]);
        assert_eq!(manifest_indices, vec![1]);
        assert_eq!(recovery_failures.len(), 1);
        assert_eq!(recovery_failures[0].manifest_index, 0);
        assert_eq!(
            recovery_failures[0].signed_transaction_hash,
            *unrecoverable.tx_hash()
        );

        let parent = SealedHeader::seal_slow(Header {
            number: 0,
            timestamp: u64::MAX - 1,
            gas_limit: 30_000_000,
            base_fee_per_gas: Some(1),
            extra_data: Bytes::from(vec![0; 7]),
            ..Default::default()
        });
        let block = RecoveredBlock::new_unhashed(
            Block {
                header: Header {
                    parent_hash: parent.hash(),
                    number: 1,
                    timestamp: u64::MAX,
                    gas_limit: 30_000_000,
                    base_fee_per_gas: Some(1),
                    extra_data: Bytes::from(vec![0; 7]),
                    ..Default::default()
                },
                body: BlockBody {
                    transactions: vec![anchor, candidate],
                    withdrawals: Some(Vec::new().into()),
                    ..Default::default()
                },
            },
            vec![signer; 2],
        );
        let mut db = InMemoryDB::default();
        db.insert_account_info(
            signer,
            AccountInfo {
                balance: U256::MAX,
                ..Default::default()
            },
        );
        insert_contract(
            &mut db,
            target,
            Bytecode::new_raw(Bytes::from(vec![opcode::STOP])),
        );

        let evm_config = TaikoEvmConfig::new(chain_spec);
        let mut executor = TracingDerivedBlockExecutor::with_manifest_indices(4, manifest_indices);
        raiko2_stateless::DerivedBlockExecutor::execute(
            &mut executor,
            &evm_config,
            &parent,
            &block,
            db,
        )
        .expect("recovery gap must not prevent later execution");
        let trace = executor.take_completed().expect("completed block trace");

        assert_eq!(trace.transactions.len(), 2);
        assert_eq!(trace.transactions[0].manifest_index, None);
        assert_eq!(trace.transactions[1].manifest_index, Some(1));
        assert_eq!(
            trace.transactions[1].disposition,
            TransactionDisposition::CommittedSuccess
        );
    }

    #[test]
    fn deliberate_traced_state_perturbation_fails_typed_ab_gate() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_23077_l2_9051439_9051630.json",
        );
        let bytes = std::fs::read(fixture).expect("read proposal fixture");
        let input = serde_json::from_slice(&bytes).expect("parse proposal fixture");
        let trace = trace_shasta_proposal_with_parity_mutator(&input, |index, record| {
            if index == 0 {
                record.state_root = B256::ZERO;
            }
        })
        .expect("trace proposal");

        assert_eq!(trace.status, ProposalTraceStatus::Failed);
        assert!(!trace.parity.passed);
        assert!(
            trace
                .parity
                .mismatch_fields
                .contains(&"blocks[0].state_root".to_string())
        );
        assert_eq!(trace.failure.expect("parity failure").stage, "parity");
    }
}
