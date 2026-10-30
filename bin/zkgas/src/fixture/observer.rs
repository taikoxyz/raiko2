//! Observation only: execution, filtering and state validation stay in the production executor.
use alethia_reth_block::{
    config::{TaikoEvmConfig, TaikoNextBlockEnvAttributes},
    derived_block::DerivedBlockExecutionOutcome,
    executor::TaikoBlockExecutor,
};
use alethia_reth_evm::{alloy::TaikoEvmContext, zk_gas::pricing::ZkGasMode};
use alloy_evm::EvmFactory as _;
use reth_ethereum_primitives::Block;
use reth_evm::{ConfigureEvm as _, block::BlockExecutionError};
use reth_primitives_traits::{RecoveredBlock, SealedHeader};
use reth_revm::{
    Database, Inspector, State,
    context::{ContextTr as _, JournalTr as _},
    db::states::bundle_state::BundleRetention,
    interpreter::{
        InstructionResult, Interpreter,
        interpreter::EthInterpreter,
        interpreter_types::{Jumps as _, LoopControl as _},
    },
};
use reth_trie_common::{HashedPostState, KeccakKeyHasher};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, Default, Serialize)]
pub struct OpcodeObservation {
    /// Completed opcode instructions, including work in reverted child frames.
    pub completed: BTreeMap<String, u64>,
    /// Instructions entered, including instructions that halt exceptionally.
    pub attempted: BTreeMap<String, u64>,
    /// Explicit STOP, RETURN, REVERT and SELFDESTRUCT completions.
    pub terminals: BTreeMap<String, u64>,
    pub exceptional_instructions: u64,
}
#[derive(Default)]
struct Ledger {
    active: bool,
    counts: OpcodeObservation,
}
#[derive(Default)]
struct FixtureInspector {
    ledger: Arc<Mutex<Ledger>>,
    pending: BTreeMap<usize, u8>,
}
impl<DB: Database> Inspector<TaikoEvmContext<DB>, EthInterpreter> for FixtureInspector {
    fn step(
        &mut self,
        interpreter: &mut Interpreter<EthInterpreter>,
        context: &mut TaikoEvmContext<DB>,
    ) {
        if self
            .ledger
            .lock()
            .expect("fixture observer poisoned")
            .active
        {
            let opcode = interpreter.bytecode.opcode();
            self.pending.insert(context.journal().depth(), opcode);
            *self
                .ledger
                .lock()
                .expect("fixture observer poisoned")
                .counts
                .attempted
                .entry(format!("opcode:0x{opcode:02x}"))
                .or_default() += 1;
        }
    }
    fn step_end(
        &mut self,
        interpreter: &mut Interpreter<EthInterpreter>,
        context: &mut TaikoEvmContext<DB>,
    ) {
        let Some(opcode) = self.pending.remove(&context.journal().depth()) else {
            return;
        };
        let result = interpreter.bytecode.instruction_result();
        let completed = result.is_none_or(|result| {
            result.is_ok() || (opcode == 0xfd && matches!(result, InstructionResult::Revert))
        });
        let mut ledger = self.ledger.lock().expect("fixture observer poisoned");
        if completed {
            *ledger
                .counts
                .completed
                .entry(format!("opcode:0x{opcode:02x}"))
                .or_default() += 1;
            if let Some(name) = match opcode {
                0x00 => Some("stop"),
                0xf3 => Some("return"),
                0xfd => Some("revert"),
                0xff => Some("selfdestruct"),
                _ => None,
            } {
                *ledger.counts.terminals.entry(name.into()).or_default() += 1;
            }
        } else {
            ledger.counts.exceptional_instructions += 1;
        }
    }
}
#[derive(Default)]
pub(super) struct FixtureExecutor {
    pub observation: OpcodeObservation,
    /// Actual code produced by committed native execution, for later block witnesses.
    pub generated_codes: Vec<alloy_primitives::Bytes>,
    pub execution_profile: raiko2_stateless::workload_lab::ExecutionProfile,
}
impl raiko2_stateless::DerivedBlockExecutor for FixtureExecutor {
    fn execute<DB: Database + std::fmt::Debug>(
        &mut self,
        config: &TaikoEvmConfig,
        parent: &SealedHeader,
        block: &RecoveredBlock<Block>,
        db: DB,
    ) -> Result<DerivedBlockExecutionOutcome, BlockExecutionError> {
        let mut state = State::builder()
            .with_database(db)
            .with_bundle_update()
            .build();
        let ledger = Arc::new(Mutex::new(Ledger::default()));
        let header = block.header();
        let attributes = TaikoNextBlockEnvAttributes {
            timestamp: header.timestamp,
            suggested_fee_recipient: header.beneficiary,
            prev_randao: header.mix_hash,
            gas_limit: header.gas_limit,
            extra_data: header.extra_data.clone(),
            base_fee_per_gas: header
                .base_fee_per_gas
                .ok_or_else(|| BlockExecutionError::msg("missing base fee"))?,
            parent_beacon_block_root: header.parent_beacon_block_root,
        };
        let environment = config
            .next_evm_env(parent, &attributes)
            .map_err(BlockExecutionError::other)?;
        let mut evm = config.evm_factory().create_evm_with_inspector(
            &mut state,
            environment,
            FixtureInspector {
                ledger: ledger.clone(),
                pending: BTreeMap::new(),
            },
        );
        raiko2_stateless::workload_lab::apply_execution_profile(
            evm.meter_mut(),
            self.execution_profile,
        )
        .map_err(BlockExecutionError::other)?;
        let context = config
            .context_for_next_block(parent, attributes)
            .map_err(BlockExecutionError::other)?;
        let finalized = context.finalized_block_zk_gas.clone();
        let executor = TaikoBlockExecutor::new_with_zk_gas_mode(
            evm,
            context,
            config.executor_factory.spec().clone(),
            config.executor_factory.receipt_builder(),
            ZkGasMode::Shadow,
        );
        let mut consumed = 0;
        let mut inner = block.transactions_recovered();
        let transactions = std::iter::from_fn(|| {
            let transaction = inner.next();
            ledger.lock().expect("fixture observer poisoned").active =
                transaction.is_some() && consumed == 1;
            if transaction.is_some() {
                consumed += 1;
            }
            transaction
        });
        let execution = executor.execute_block_with_committed_transactions(transactions)?;
        if consumed != 2
            || execution.committed_transactions.len() != 2
            || execution.attempted_transaction_count != 2
        {
            return Err(BlockExecutionError::msg(
                "fixture must execute exactly one Anchor and one user transaction",
            ));
        }
        self.observation = ledger
            .lock()
            .expect("fixture observer poisoned")
            .counts
            .clone();
        state.merge_transitions(BundleRetention::Reverts);
        let bundle = state.take_bundle();
        self.generated_codes = bundle
            .contracts
            .values()
            .map(|code| code.original_bytes())
            .collect();
        self.generated_codes.sort();
        self.generated_codes.dedup();
        Ok(DerivedBlockExecutionOutcome {
            committed_transactions: execution.committed_transactions,
            execution_result: execution.execution_result,
            hashed_state: HashedPostState::from_bundle_state::<KeccakKeyHasher>(bundle.state()),
            finalized_block_zk_gas: finalized.load(std::sync::atomic::Ordering::Relaxed),
            zk_gas_shadow_report: execution.zk_gas_shadow_report,
            attempted_zk_gas_shadow_report: execution.attempted_zk_gas_shadow_report,
            attempted_transaction_count: execution.attempted_transaction_count,
        })
    }
}
