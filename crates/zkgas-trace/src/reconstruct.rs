#![allow(clippy::redundant_pub_crate)]

use alethia_reth_block::{
    config::{MissingBaseFee, TaikoEvmConfig, TaikoNextBlockEnvAttributes},
    derived_block::DerivedBlockExecutionOutcome,
    executor::TaikoBlockExecutor,
};
use alethia_reth_evm::{
    alloy::TaikoEvmContext,
    zk_gas::pricing::{ResourceSelectors, ZkGasMode, ZkGasShadowReport},
};
use alloy_consensus::Transaction as _;
use alloy_evm::EvmFactory as _;
use alloy_primitives::{Address, B256, U256};
use reth_ethereum_primitives::{Block, TransactionSigned};
use reth_evm::{ConfigureEvm as _, block::BlockExecutionError};
use reth_primitives_traits::{RecoveredBlock, SealedHeader};
use reth_revm::{
    Database, Inspector, State,
    context::{ContextTr as _, JournalTr as _},
    db::states::{BundleState, bundle_state::BundleRetention},
    interpreter::{
        CallInputs, CallOutcome, CreateInputs, CreateOutcome, InstructionResult, Interpreter,
        interpreter::EthInterpreter,
        interpreter_types::{InputsTr as _, Jumps as _, LegacyBytecode as _, LoopControl as _},
    },
    state::EvmState,
};
use reth_trie_common::{HashedPostState, KeccakKeyHasher};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};
use thiserror::Error;

use crate::runtime_quote::{
    EcmulSemanticCountersSnapshot, NodeSemanticCountersSnapshot, SemanticCounterDiagnostic,
};

const fn unavailable_semantics(reason: SemanticCounterDiagnostic) -> NodeSemanticCountersSnapshot {
    NodeSemanticCountersSnapshot {
        ecmul: None,
        diagnostic: Some(reason),
        access_list_raw_storage_keys: None,
    }
}

fn add_ecmul(
    left: EcmulSemanticCountersSnapshot,
    right: EcmulSemanticCountersSnapshot,
) -> Option<EcmulSemanticCountersSnapshot> {
    Some(EcmulSemanticCountersSnapshot {
        successful_calls: left.successful_calls.checked_add(right.successful_calls)?,
        successful_native_gas: left
            .successful_native_gas
            .checked_add(right.successful_native_gas)?,
        successful_scalar_work_units: left
            .successful_scalar_work_units
            .checked_add(right.successful_scalar_work_units)?,
        failed_calls: left.failed_calls.checked_add(right.failed_calls)?,
        failed_native_gas: left
            .failed_native_gas
            .checked_add(right.failed_native_gas)?,
    })
}

fn merge_semantics(
    target: &mut NodeSemanticCountersSnapshot,
    incoming: NodeSemanticCountersSnapshot,
) {
    if target.diagnostic.is_some() {
        return;
    }
    if let Some(reason) = incoming.diagnostic {
        *target = unavailable_semantics(reason);
        return;
    }
    *target = match target
        .ecmul
        .zip(incoming.ecmul)
        .and_then(|(left, right)| add_ecmul(left, right))
    {
        Some(ecmul) => NodeSemanticCountersSnapshot {
            ecmul: Some(ecmul),
            diagnostic: None,
            access_list_raw_storage_keys: None,
        },
        None => unavailable_semantics(SemanticCounterDiagnostic::ArithmeticOverflow),
    };
}

#[derive(Debug, Default)]
struct SemanticCandidate {
    ecmul: EcmulSemanticCountersSnapshot,
    pending_calls: u64,
    diagnostic: Option<SemanticCounterDiagnostic>,
}

impl SemanticCandidate {
    fn fail(&mut self, reason: SemanticCounterDiagnostic) {
        self.diagnostic.get_or_insert(reason);
    }

    fn start_call(&mut self) {
        if let Some(count) = self.pending_calls.checked_add(1) {
            self.pending_calls = count;
        } else {
            self.fail(SemanticCounterDiagnostic::ArithmeticOverflow);
        }
    }

    fn finish_call(
        &mut self,
        was_precompile: bool,
        native_gas: Option<u64>,
        succeeded: bool,
        scalar: U256,
    ) {
        let Some(pending) = self.pending_calls.checked_sub(1) else {
            self.fail(SemanticCounterDiagnostic::UnbalancedCallHooks);
            return;
        };
        self.pending_calls = pending;
        if !was_precompile || self.diagnostic.is_some() {
            return;
        }
        let Some(native_gas) = native_gas else {
            self.fail(SemanticCounterDiagnostic::NativeGasAccounting);
            return;
        };
        let mut delta = EcmulSemanticCountersSnapshot::default();
        if succeeded {
            if native_gas != 6000 {
                self.fail(SemanticCounterDiagnostic::NativeGasAccounting);
                return;
            }
            let order = U256::from_be_bytes(alloy_primitives::hex!(
                "30644e72e131a029b85045b68181585d2833e84879b9709143e1f593f0000001"
            ));
            let reduced = scalar % order;
            delta.successful_calls = 1;
            delta.successful_native_gas = native_gas;
            delta.successful_scalar_work_units = reduced.bit_len() as u64
                + reduced
                    .to_be_bytes::<32>()
                    .iter()
                    .map(|byte| u64::from(byte.count_ones()))
                    .sum::<u64>();
        } else {
            delta.failed_calls = 1;
            delta.failed_native_gas = native_gas;
        }
        if let Some(value) = add_ecmul(self.ecmul, delta) {
            self.ecmul = value;
        } else {
            self.fail(SemanticCounterDiagnostic::ArithmeticOverflow);
        }
    }

    fn snapshot(&self) -> NodeSemanticCountersSnapshot {
        match self
            .diagnostic
            .or((self.pending_calls != 0).then_some(SemanticCounterDiagnostic::UnbalancedCallHooks))
        {
            Some(reason) => unavailable_semantics(reason),
            None => NodeSemanticCountersSnapshot {
                ecmul: Some(self.ecmul),
                diagnostic: None,
                access_list_raw_storage_keys: None,
            },
        }
    }
}

#[derive(Clone, Debug)]
struct PendingSemanticCall {
    candidate: Arc<Mutex<SemanticCandidate>>,
    scalar: U256,
}

const fn committed_semantic_counters(
    ledger: &CandidateResourceLedger,
    report: &ZkGasShadowReport,
) -> NodeSemanticCountersSnapshot {
    let mut counters = ledger.semantic_snapshot();
    counters.access_list_raw_storage_keys =
        Some(report.access_list_construction().raw_storage_keys);
    counters
}

fn ecmul_scalar(input: &[u8]) -> U256 {
    let mut scalar = [0_u8; 32];
    let suffix = input.get(64..).unwrap_or_default();
    let length = suffix.len().min(32);
    scalar[..length].copy_from_slice(&suffix[..length]);
    U256::from_be_bytes(scalar)
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
enum ResourceError {
    #[error("resource selector {0} exceeds u64")]
    Count(&'static str),
    #[error("unique code byte total overflows")]
    CodeBytes,
    #[error("code hash {hash} has inconsistent lengths {first} and {second}")]
    CodeLength { hash: B256, first: u64, second: u64 },
}

#[derive(Debug, Default)]
struct ResourceLedger {
    accounts: BTreeSet<Address>,
    storage_accesses: BTreeSet<(Address, U256)>,
    storage_writes: BTreeSet<(Address, U256)>,
    code_lengths: BTreeMap<B256, u64>,
    error: Option<ResourceError>,
}

impl ResourceLedger {
    fn account(&mut self, address: Address) {
        self.accounts.insert(address);
    }

    fn storage_access(&mut self, address: Address, slot: U256) {
        self.account(address);
        self.storage_accesses.insert((address, slot));
    }

    fn storage_write(&mut self, address: Address, slot: U256) {
        self.storage_access(address, slot);
        self.storage_writes.insert((address, slot));
    }

    fn code(&mut self, hash: B256, bytes: &[u8]) {
        if bytes.is_empty() || self.error.is_some() {
            return;
        }
        let Ok(length) = u64::try_from(bytes.len()) else {
            self.error = Some(ResourceError::Count("code_bytes"));
            return;
        };
        self.code_length(hash, length);
    }

    fn code_length(&mut self, hash: B256, length: u64) {
        if self.error.is_some() {
            return;
        }
        if let Some(first) = self.code_lengths.get(&hash) {
            if *first != length {
                self.error = Some(ResourceError::CodeLength {
                    hash,
                    first: *first,
                    second: length,
                });
            }
        } else {
            self.code_lengths.insert(hash, length);
        }
    }

    fn merge(&mut self, other: Self) -> Result<(), ResourceError> {
        if let Some(error) = other.error {
            return Err(error);
        }
        self.accounts.extend(other.accounts);
        self.storage_accesses.extend(other.storage_accesses);
        self.storage_writes.extend(other.storage_writes);
        for (hash, length) in other.code_lengths {
            self.code_length(hash, length);
        }
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        Ok(())
    }

    fn selectors(&self) -> Result<ResourceSelectors, ResourceError> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        let count =
            |label, value: usize| u64::try_from(value).map_err(|_| ResourceError::Count(label));
        let unique_code_bytes = self
            .code_lengths
            .values()
            .try_fold(0_u64, |total, value| total.checked_add(*value))
            .ok_or(ResourceError::CodeBytes)?;
        Ok(ResourceSelectors {
            unique_account_count: count("accounts", self.accounts.len())?,
            unique_storage_slot_access_count: count(
                "storage_accesses",
                self.storage_accesses.len(),
            )?,
            unique_storage_slot_write_count: count("storage_writes", self.storage_writes.len())?,
            unique_code_hash_count: count("code_hashes", self.code_lengths.len())?,
            unique_code_bytes,
        })
    }
}

#[derive(Debug, Default)]
struct CandidateResourceLedger {
    committed: ResourceLedger,
    active: Option<ResourceLedger>,
    committed_semantics: NodeSemanticCountersSnapshot,
    active_semantics: Option<Arc<Mutex<SemanticCandidate>>>,
}

impl CandidateResourceLedger {
    fn begin_candidate(&mut self) {
        debug_assert!(
            self.active.is_none(),
            "candidate resource scope already active"
        );
        self.active = Some(ResourceLedger::default());
        self.active_semantics = Some(Arc::new(Mutex::new(SemanticCandidate::default())));
    }

    const fn observed(&mut self) -> Option<&mut ResourceLedger> {
        self.active.as_mut()
    }

    const fn active(&mut self) -> &mut ResourceLedger {
        self.active
            .as_mut()
            .expect("candidate resource scope is not active")
    }

    fn reconcile_active_state(&mut self, state: &EvmState) {
        let ledger = self.active();
        for (address, account) in state {
            ledger.account(*address);
            for slot in account.storage.keys().copied() {
                ledger.storage_access(*address, slot);
            }
            if let Some(code) = &account.info.code {
                ledger.code(account.info.code_hash, code.original_byte_slice());
            }
        }
    }

    fn commit_active_state(&mut self, state: &EvmState) {
        if self.active.is_none() {
            return;
        }
        self.reconcile_active_state(state);
        if let Err(error) = self.commit_candidate() {
            self.committed.error.get_or_insert(error);
        }
    }

    fn commit_candidate(&mut self) -> Result<(), ResourceError> {
        let candidate = self
            .active
            .take()
            .expect("candidate resource scope is not active");
        let semantics = self
            .active_semantics
            .take()
            .expect("candidate semantic scope is not active");
        self.committed.merge(candidate)?;
        merge_semantics(
            &mut self.committed_semantics,
            semantics
                .lock()
                .expect("semantic candidate poisoned")
                .snapshot(),
        );
        Ok(())
    }

    fn discard_candidate(&mut self) {
        self.active = None;
        self.active_semantics = None;
    }

    const fn committed(&self) -> &ResourceLedger {
        &self.committed
    }

    const fn semantic_snapshot(&self) -> NodeSemanticCountersSnapshot {
        self.committed_semantics
    }
}

#[cfg(test)]
std::thread_local! {
    static FIXTURE_ECMUL_OBSERVATIONS: std::cell::RefCell<(Vec<FixtureEcmulCall>, u64, u64)> = const { std::cell::RefCell::new((Vec::new(), 0, 0)) };
    static FIXTURE_OPCODE_COUNTS: std::cell::RefCell<(u64, u64, u64)> = const { std::cell::RefCell::new((0, 0, 0)) };
}

#[cfg(test)]
pub(crate) fn take_fixture_opcode_counts() -> (u64, u64, u64) {
    FIXTURE_OPCODE_COUNTS.with(|counts| std::mem::take(&mut *counts.borrow_mut()))
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FixtureEcmulCall {
    pub block_number: u64,
    pub input: Vec<u8>,
    pub reduced_scalar: U256,
    pub work_units: u64,
    pub native_gas: u64,
    pub succeeded: bool,
}

#[cfg(test)]
pub(crate) fn take_fixture_ecmul_calls() -> (Vec<FixtureEcmulCall>, u64, u64) {
    FIXTURE_ECMUL_OBSERVATIONS.with(|calls| std::mem::take(&mut *calls.borrow_mut()))
}

#[derive(Clone, Copy, Debug)]
struct PendingStep {
    #[cfg(test)]
    opcode: u8,
    resource_account: Option<Address>,
    materialized_code_account: Option<Address>,
    storage: Option<(Address, U256, bool)>,
}

fn record_pending_step(
    ledger: &mut ResourceLedger,
    step: PendingStep,
    instruction_succeeded: bool,
) {
    if instruction_succeeded && let Some(address) = step.resource_account {
        ledger.account(address);
    }
    if let Some((address, slot, write)) = step.storage {
        if write {
            ledger.storage_write(address, slot);
        } else {
            ledger.storage_access(address, slot);
        }
    }
}

#[derive(Clone, Debug, Default)]
struct ResourceInspector {
    #[cfg(test)]
    pending_fixture_calls: Vec<Option<FixtureEcmulCall>>,
    pending_semantic_calls: Vec<Option<PendingSemanticCall>>,
    ledger: Arc<Mutex<CandidateResourceLedger>>,
    pending: BTreeMap<usize, PendingStep>,
}

impl ResourceInspector {
    const fn with_ledger(ledger: Arc<Mutex<CandidateResourceLedger>>) -> Self {
        Self {
            #[cfg(test)]
            pending_fixture_calls: Vec::new(),
            pending_semantic_calls: Vec::new(),
            ledger,
            pending: BTreeMap::new(),
        }
    }
}

fn resource_account(interpreter: &Interpreter<EthInterpreter>, opcode: u8) -> Option<Address> {
    matches!(opcode, 0x31 | 0x3b | 0x3c | 0x3f | 0xff)
        .then(|| interpreter.stack.peek(0).ok())
        .flatten()
        .map(|value| Address::from_word(B256::from(value.to_be_bytes())))
}

impl<DB: Database> Inspector<TaikoEvmContext<DB>, EthInterpreter> for ResourceInspector {
    fn initialize_interp(
        &mut self,
        interpreter: &mut Interpreter<EthInterpreter>,
        _context: &mut TaikoEvmContext<DB>,
    ) {
        let mut resources = self.ledger.lock().expect("resource ledger poisoned");
        let Some(ledger) = resources.observed() else {
            return;
        };
        ledger.account(interpreter.input.caller_address());
        ledger.account(interpreter.input.target_address());
        if let Some(address) = interpreter.input.bytecode_address() {
            ledger.account(*address);
        }
        let hash = interpreter.bytecode.get_or_calculate_hash();
        ledger.code(hash, interpreter.bytecode.bytecode_slice());
    }

    fn step(
        &mut self,
        interpreter: &mut Interpreter<EthInterpreter>,
        context: &mut TaikoEvmContext<DB>,
    ) {
        let opcode = interpreter.bytecode.opcode();
        let storage = matches!(opcode, 0x54 | 0x55)
            .then(|| interpreter.stack.peek(0).ok())
            .flatten()
            .map(|slot| (interpreter.input.target_address(), slot, opcode == 0x55));
        let resource_account = resource_account(interpreter, opcode);
        self.pending.insert(
            context.journal().depth(),
            PendingStep {
                #[cfg(test)]
                opcode,
                resource_account,
                materialized_code_account: matches!(opcode, 0x3b | 0x3c)
                    .then_some(resource_account)
                    .flatten(),
                storage,
            },
        );
    }

    fn step_end(
        &mut self,
        interpreter: &mut Interpreter<EthInterpreter>,
        context: &mut TaikoEvmContext<DB>,
    ) {
        let Some(step) = self.pending.remove(&context.journal().depth()) else {
            return;
        };
        let instruction_succeeded = interpreter
            .bytecode
            .instruction_result()
            .is_none_or(InstructionResult::is_ok);
        let materialized_code = instruction_succeeded
            .then_some(step.materialized_code_account)
            .flatten()
            .and_then(|address| context.journal().evm_state().get(&address))
            .and_then(|account| {
                account
                    .info
                    .code
                    .as_ref()
                    .map(|code| (account.info.code_hash, code.original_byte_slice()))
            });
        let mut resources = self.ledger.lock().expect("resource ledger poisoned");
        let Some(ledger) = resources.observed() else {
            return;
        };
        #[cfg(test)]
        if instruction_succeeded {
            FIXTURE_OPCODE_COUNTS.with(|counts| {
                let mut counts = counts.borrow_mut();
                counts.0 += 1;
                counts.1 += u64::from(step.opcode == 0x40);
                counts.2 += u64::from(
                    step.opcode == 0xf4
                        && matches!(
                            interpreter.bytecode.action(),
                            Some(reth_revm::interpreter::InterpreterAction::NewFrame(_))
                        ),
                );
            });
        }
        record_pending_step(ledger, step, instruction_succeeded);
        if let Some((hash, bytes)) = materialized_code {
            ledger.code(hash, bytes);
        }
    }

    fn call(
        &mut self,
        context: &mut TaikoEvmContext<DB>,
        inputs: &mut CallInputs,
    ) -> Option<CallOutcome> {
        let candidate = (inputs.bytecode_address == Address::with_last_byte(7))
            .then(|| {
                self.ledger
                    .lock()
                    .expect("resource ledger poisoned")
                    .active_semantics
                    .clone()
            })
            .flatten();
        let pending = candidate.map(|candidate| {
            let input = inputs.input.as_bytes(context);
            let scalar = ecmul_scalar(&input);
            candidate
                .lock()
                .expect("semantic candidate poisoned")
                .start_call();
            PendingSemanticCall { candidate, scalar }
        });
        self.pending_semantic_calls.push(pending);
        #[cfg(test)]
        {
            let observed = self
                .ledger
                .lock()
                .expect("resource ledger poisoned")
                .active
                .is_some();
            let captured = (observed && inputs.bytecode_address == Address::with_last_byte(7))
                .then(|| {
                    let input = inputs.input.as_bytes(context).to_vec();
                    let mut scalar_bytes = [0_u8; 32];
                    let suffix = input.get(64..).unwrap_or_default();
                    let copied = suffix.len().min(32);
                    scalar_bytes[..copied].copy_from_slice(&suffix[..copied]);
                    // Independent remainder, rather than the pricing runtime's subtraction loop.
                    let order = U256::from_be_bytes(alloy_primitives::hex!(
                        "30644e72e131a029b85045b68181585d2833e84879b9709143e1f593f0000001"
                    ));
                    let reduced_scalar = U256::from_be_bytes(scalar_bytes) % order;
                    let work_units = reduced_scalar.bit_len() as u64
                        + reduced_scalar
                            .to_be_bytes::<32>()
                            .iter()
                            .map(|byte| u64::from(byte.count_ones()))
                            .sum::<u64>();
                    FixtureEcmulCall {
                        block_number: u64::try_from(context.block().number)
                            .expect("fixture block fits u64"),
                        input,
                        reduced_scalar,
                        work_units,
                        native_gas: 0,
                        succeeded: false,
                    }
                });
            self.pending_fixture_calls.push(captured);
            FIXTURE_ECMUL_OBSERVATIONS.with(|calls| calls.borrow_mut().1 += 1);
        }
        let mut resources = self.ledger.lock().expect("resource ledger poisoned");
        let ledger = resources.observed()?;
        ledger.account(inputs.caller);
        ledger.account(inputs.target_address);
        ledger.account(inputs.bytecode_address);
        None
    }

    fn call_end(
        &mut self,
        _context: &mut TaikoEvmContext<DB>,
        inputs: &CallInputs,
        outcome: &mut CallOutcome,
    ) {
        match self.pending_semantic_calls.pop() {
            Some(Some(pending)) => {
                let mut candidate = pending
                    .candidate
                    .lock()
                    .expect("semantic candidate poisoned");
                if inputs.bytecode_address == Address::with_last_byte(7) {
                    candidate.finish_call(
                        outcome.was_precompile_called,
                        inputs.gas_limit.checked_sub(outcome.result.gas.remaining()),
                        outcome.result.result.is_ok(),
                        pending.scalar,
                    );
                } else {
                    candidate.fail(SemanticCounterDiagnostic::CallHookMismatch);
                }
            }
            Some(None) => {}
            None => {
                if let Some(candidate) = self
                    .ledger
                    .lock()
                    .expect("resource ledger poisoned")
                    .active_semantics
                    .as_ref()
                {
                    candidate
                        .lock()
                        .expect("semantic candidate poisoned")
                        .fail(SemanticCounterDiagnostic::UnbalancedCallHooks);
                }
            }
        }
        #[cfg(test)]
        {
            let captured = self
                .pending_fixture_calls
                .pop()
                .expect("fixture call hooks balance");
            FIXTURE_ECMUL_OBSERVATIONS.with(|calls| {
                let mut calls = calls.borrow_mut();
                calls.2 += 1;
                if let Some(mut event) = captured
                    && outcome.was_precompile_called
                {
                    event.native_gas = inputs
                        .gas_limit
                        .checked_sub(outcome.result.gas.remaining())
                        .expect("precompile cannot return more gas than supplied");
                    event.succeeded = outcome.result.result.is_ok();
                    calls.0.push(event);
                }
            });
        }
    }

    fn create(
        &mut self,
        _context: &mut TaikoEvmContext<DB>,
        inputs: &mut CreateInputs,
    ) -> Option<CreateOutcome> {
        if let Some(ledger) = self
            .ledger
            .lock()
            .expect("resource ledger poisoned")
            .observed()
        {
            ledger.account(inputs.caller());
        }
        None
    }
}

struct CandidateTransactions<'a, I> {
    inner: I,
    consumed: &'a mut usize,
    resources: Arc<Mutex<CandidateResourceLedger>>,
}

impl<I> Drop for CandidateTransactions<'_, I> {
    fn drop(&mut self) {
        self.resources
            .lock()
            .expect("resource ledger poisoned")
            .discard_candidate();
    }
}

impl<'tx, I> Iterator for CandidateTransactions<'_, I>
where
    I: Iterator<Item = alloy_consensus::transaction::Recovered<&'tx TransactionSigned>>,
{
    type Item = alloy_consensus::transaction::Recovered<&'tx TransactionSigned>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut resources = self.resources.lock().expect("resource ledger poisoned");
        resources.discard_candidate();
        let transaction = self.inner.next()?;
        *self.consumed += 1;
        resources.begin_candidate();
        if let Some(access_list) = (*transaction.inner()).access_list() {
            for item in &access_list.0 {
                resources.active().account(item.address);
                for key in &item.storage_keys {
                    resources
                        .active()
                        .storage_access(item.address, U256::from_be_bytes(key.0));
                }
            }
        }
        Some(transaction)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct BlockExecutionObservation {
    pub committed_report: ZkGasShadowReport,
    pub attempted_report: ZkGasShadowReport,
    pub exact_resources: ResourceSelectors,
    pub consumed_candidate_count: usize,
    pub attempted_transaction_count: u64,
    pub finalized_block_zkgas: u64,
    pub semantic_counters: NodeSemanticCountersSnapshot,
}

#[derive(Debug, Default)]
pub(crate) struct QuoteDerivedBlockExecutor {
    completed: Option<BlockExecutionObservation>,
    execution_profile: raiko2_stateless::workload_lab::ExecutionProfile,
}

impl QuoteDerivedBlockExecutor {
    pub(crate) const fn new(
        execution_profile: raiko2_stateless::workload_lab::ExecutionProfile,
    ) -> Self {
        Self {
            completed: None,
            execution_profile,
        }
    }

    pub(crate) const fn take(&mut self) -> Option<BlockExecutionObservation> {
        self.completed.take()
    }

    fn install_candidate_commit_hook<DB: Database>(
        state: &mut State<DB>,
        resources: Arc<Mutex<CandidateResourceLedger>>,
    ) {
        state.set_state_hook(Some(Box::new(move |committed: EvmState| {
            resources
                .lock()
                .expect("resource ledger poisoned")
                .commit_active_state(&committed);
        })));
    }
}

fn attributes_from_block(
    block: &RecoveredBlock<Block>,
) -> Result<TaikoNextBlockEnvAttributes, BlockExecutionError> {
    let header = block.header();
    Ok(TaikoNextBlockEnvAttributes {
        timestamp: header.timestamp,
        suggested_fee_recipient: header.beneficiary,
        prev_randao: header.mix_hash,
        gas_limit: header.gas_limit,
        extra_data: header.extra_data.clone(),
        base_fee_per_gas: header.base_fee_per_gas.ok_or_else(|| {
            BlockExecutionError::other(MissingBaseFee {
                block_number: header.number,
            })
        })?,
        parent_beacon_block_root: header.parent_beacon_block_root,
    })
}

impl raiko2_stateless::DerivedBlockExecutor for QuoteDerivedBlockExecutor {
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
        let ledger = Arc::new(Mutex::new(CandidateResourceLedger::default()));
        Self::install_candidate_commit_hook(&mut state, ledger.clone());
        let attributes = attributes_from_block(derived_block)?;
        let environment = evm_config
            .next_evm_env(parent_header, &attributes)
            .map_err(BlockExecutionError::other)?;
        let mut evm = evm_config.evm_factory().create_evm_with_inspector(
            &mut state,
            environment,
            ResourceInspector::with_ledger(ledger.clone()),
        );
        raiko2_stateless::workload_lab::apply_execution_profile(
            evm.meter_mut(),
            self.execution_profile,
        )
        .map_err(BlockExecutionError::other)?;
        let execution_context = evm_config
            .context_for_next_block(parent_header, attributes)
            .map_err(BlockExecutionError::other)?;
        let finalized_zk_gas = execution_context.finalized_block_zk_gas.clone();
        let executor = TaikoBlockExecutor::new_with_zk_gas_mode(
            evm,
            execution_context,
            evm_config.executor_factory.spec().clone(),
            evm_config.executor_factory.receipt_builder(),
            ZkGasMode::Shadow,
        );
        let mut consumed = 0usize;
        let transactions = CandidateTransactions {
            inner: derived_block.transactions_recovered(),
            consumed: &mut consumed,
            resources: ledger.clone(),
        };
        let execution = executor.execute_block_with_committed_transactions(transactions)?;
        ledger
            .lock()
            .expect("resource ledger poisoned")
            .discard_candidate();
        state.merge_transitions(BundleRetention::Reverts);
        let exact_resources = ledger
            .lock()
            .expect("resource ledger poisoned")
            .committed()
            .selectors()
            .map_err(BlockExecutionError::other)?;
        let committed_report = execution
            .zk_gas_shadow_report
            .ok_or_else(|| BlockExecutionError::msg("missing zkGas shadow report"))?;
        let attempted_report = execution
            .attempted_zk_gas_shadow_report
            .ok_or_else(|| BlockExecutionError::msg("missing attempted zkGas shadow report"))?;
        let finalized_block_zkgas = finalized_zk_gas.load(std::sync::atomic::Ordering::Relaxed);
        let semantic_counters = committed_semantic_counters(
            &ledger.lock().expect("resource ledger poisoned"),
            &committed_report,
        );
        self.completed = Some(BlockExecutionObservation {
            committed_report,
            attempted_report,
            exact_resources,
            consumed_candidate_count: consumed,
            attempted_transaction_count: execution.attempted_transaction_count,
            finalized_block_zkgas,
            semantic_counters,
        });

        let bundle_state: BundleState = state.take_bundle();
        Ok(DerivedBlockExecutionOutcome {
            committed_transactions: execution.committed_transactions,
            execution_result: execution.execution_result,
            hashed_state: HashedPostState::from_bundle_state::<KeccakKeyHasher>(
                bundle_state.state(),
            ),
            finalized_block_zk_gas: finalized_block_zkgas,
            zk_gas_shadow_report: Some(committed_report),
            attempted_zk_gas_shadow_report: Some(attempted_report),
            attempted_transaction_count: execution.attempted_transaction_count,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alethia_reth_evm::{
        alloy::TaikoZkGasEvm as _, factory::TaikoEvmFactory, spec::TaikoSpecId,
    };
    use alloy_evm::{Evm as _, EvmEnv};
    use alloy_primitives::{Bytes, TxKind, keccak256};
    use reth_revm::{
        context::{
            TxEnv,
            transaction::{Authorization, RecoveredAuthority, RecoveredAuthorization},
        },
        db::InMemoryDB,
        state::{AccountInfo, Bytecode},
    };

    #[test]
    fn ledger_deduplicates_resources_and_code_bytes() {
        let mut ledger = ResourceLedger::default();
        let address = Address::with_last_byte(1);
        let slot = U256::from(7);
        let hash = B256::with_last_byte(9);
        ledger.storage_write(address, slot);
        ledger.storage_write(address, slot);
        ledger.code(hash, &[1, 2, 3]);
        ledger.code(hash, &[1, 2, 3]);

        assert_eq!(
            ledger.selectors().unwrap(),
            ResourceSelectors {
                unique_account_count: 1,
                unique_storage_slot_access_count: 1,
                unique_storage_slot_write_count: 1,
                unique_code_hash_count: 1,
                unique_code_bytes: 3,
            }
        );
    }

    #[test]
    fn failed_step_records_storage_attempt_but_not_resource_account() {
        let storage_target = Address::with_last_byte(0xa1);
        let resource_account = Address::with_last_byte(0xa2);
        let slot = U256::from(7);
        let mut ledger = ResourceLedger::default();

        record_pending_step(
            &mut ledger,
            PendingStep {
                opcode: 0,
                resource_account: Some(resource_account),
                materialized_code_account: None,
                storage: Some((storage_target, slot, true)),
            },
            false,
        );

        assert!(ledger.accounts.contains(&storage_target));
        assert!(!ledger.accounts.contains(&resource_account));
        assert!(ledger.storage_accesses.contains(&(storage_target, slot)));
        assert!(ledger.storage_writes.contains(&(storage_target, slot)));
    }

    #[test]
    fn discarded_candidate_does_not_merge_its_resources() {
        let rejected_account = Address::with_last_byte(0xb1);
        let rejected_slot = U256::from(0xb2);
        let committed_account = Address::with_last_byte(0xb3);
        let mut resources = CandidateResourceLedger::default();

        resources.begin_candidate();
        resources.active().account(rejected_account);
        resources
            .active()
            .storage_access(rejected_account, rejected_slot);
        resources.discard_candidate();

        resources.begin_candidate();
        resources.active().account(committed_account);
        resources.commit_candidate().unwrap();

        assert_eq!(
            resources.committed().selectors().unwrap(),
            ResourceSelectors {
                unique_account_count: 1,
                ..Default::default()
            }
        );
    }

    #[test]
    fn semantic_ecmul_discarded_candidate_and_late_callbacks_do_not_leak() {
        let mut resources = CandidateResourceLedger::default();
        resources.begin_candidate();
        let rejected = resources.active_semantics.clone().unwrap();
        rejected.lock().unwrap().start_call();
        resources.discard_candidate();
        resources.begin_candidate();
        rejected
            .lock()
            .unwrap()
            .finish_call(true, Some(6000), true, U256::from(2));
        resources.commit_candidate().unwrap();
        assert_eq!(
            resources.semantic_snapshot(),
            NodeSemanticCountersSnapshot::default()
        );
    }

    #[test]
    fn semantic_ecmul_outcomes_and_incomplete_observations_stay_distinct() {
        let mut resources = CandidateResourceLedger::default();
        resources.begin_candidate();
        let candidate = resources.active_semantics.clone().unwrap();
        {
            let mut candidate = candidate.lock().unwrap();
            candidate.start_call();
            candidate.finish_call(true, Some(6000), true, U256::from(2));
            candidate.start_call();
            candidate.finish_call(true, Some(200_000), false, U256::MAX);
            candidate.start_call();
            candidate.finish_call(false, None, true, U256::MAX);
        }
        resources.commit_candidate().unwrap();
        let observed = resources.semantic_snapshot();
        assert_eq!(observed.diagnostic, None);
        let counters = observed.ecmul.unwrap();
        assert_eq!(counters.successful_calls, 1);
        assert_eq!(counters.successful_native_gas, 6000);
        assert_eq!(counters.successful_scalar_work_units, 3);
        assert_eq!(counters.failed_calls, 1);
        assert_eq!(counters.failed_native_gas, 200_000);

        resources.begin_candidate();
        resources
            .active_semantics
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .start_call();
        resources.commit_candidate().unwrap();
        assert_eq!(resources.semantic_snapshot().ecmul, None);
        assert_eq!(
            resources.semantic_snapshot().diagnostic,
            Some(SemanticCounterDiagnostic::UnbalancedCallHooks)
        );
    }

    #[test]
    fn semantic_ecmul_overflow_is_unavailable_instead_of_zero() {
        let mut resources = CandidateResourceLedger::default();
        resources.begin_candidate();
        let candidate = resources.active_semantics.clone().unwrap();
        {
            let mut candidate = candidate.lock().unwrap();
            candidate.ecmul.successful_calls = u64::MAX;
            candidate.start_call();
            candidate.finish_call(true, Some(6000), true, U256::from(2));
        }
        resources.commit_candidate().unwrap();
        assert_eq!(resources.semantic_snapshot().ecmul, None);
        assert_eq!(
            resources.semantic_snapshot().diagnostic,
            Some(SemanticCounterDiagnostic::ArithmeticOverflow)
        );
    }

    #[test]
    fn semantic_ecmul_inconsistent_native_gas_has_a_typed_diagnostic() {
        for gas in [None, Some(5999)] {
            let mut candidate = SemanticCandidate::default();
            candidate.start_call();
            candidate.finish_call(true, gas, true, U256::from(2));
            assert_eq!(candidate.snapshot().ecmul, None);
            assert_eq!(
                candidate.snapshot().diagnostic,
                Some(SemanticCounterDiagnostic::NativeGasAccounting)
            );
        }
    }

    fn semantic_ecmul_program(valid_point: bool, terminal: u8) -> Bytes {
        let mut code = vec![
            0x60,
            1,
            0x60,
            0,
            0x52, // x=1 at memory[0]
            0x60,
            if valid_point { 2 } else { 3 },
            0x60,
            32,
            0x52, // y at memory[32]
            0x60,
            2,
            0x60,
            64,
            0x52, // scalar=2 at memory[64]
            0x60,
            64,
            0x60,
            128,
            0x60,
            96,
            0x60,
            0,
            0x60,
            7,
            0x61,
            0x17,
            0x70,
            0xfa,
            0x50, // STATICCALL real precompile with 6000 gas; POP success
        ];
        if terminal == 0xfd {
            code.extend_from_slice(&[0x60, 0, 0x60, 0]);
        }
        code.push(terminal);
        Bytes::from(code)
    }

    fn execute_semantic_ecmul(
        code: Bytes,
        extra: &[(Address, Bytes)],
        include: bool,
    ) -> (NodeSemanticCountersSnapshot, bool) {
        let caller = Address::with_last_byte(0xd1);
        let target = Address::with_last_byte(0xd2);
        let mut database = InMemoryDB::default();
        database.insert_account_info(
            caller,
            AccountInfo {
                balance: U256::MAX,
                ..Default::default()
            },
        );
        for (address, raw) in std::iter::once((target, code)).chain(extra.iter().cloned()) {
            database.insert_account_info(
                address,
                AccountInfo {
                    code_hash: keccak256(&raw),
                    code: Some(Bytecode::new_raw(raw)),
                    ..Default::default()
                },
            );
        }
        let mut state = State::builder().with_database(database).build();
        let ledger = Arc::new(Mutex::new(CandidateResourceLedger::default()));
        QuoteDerivedBlockExecutor::install_candidate_commit_hook(&mut state, ledger.clone());
        ledger.lock().unwrap().begin_candidate();
        let mut environment: EvmEnv<TaikoSpecId> = EvmEnv::default();
        environment.cfg_env.spec = TaikoSpecId::UNZEN;
        environment.cfg_env.chain_id = 167;
        environment.block_env.gas_limit = 30_000_000;
        let mut evm = TaikoEvmFactory.create_evm_with_inspector(
            &mut state,
            environment,
            ResourceInspector::with_ledger(ledger.clone()),
        );
        evm.set_zk_gas_pricing_enabled(true);
        let transaction = TxEnv::builder()
            .caller(caller)
            .kind(TxKind::Call(target))
            .chain_id(Some(167))
            .gas_limit(1_000_000)
            .build()
            .unwrap();
        let succeeded = if include {
            evm.transact_commit(transaction)
                .expect("included semantic test")
                .is_success()
        } else {
            evm.transact(transaction)
                .expect("discarded semantic test")
                .result
                .is_success()
        };
        drop(evm);
        let mut ledger = ledger.lock().unwrap();
        if !include {
            ledger.discard_candidate();
        }
        (ledger.semantic_snapshot(), succeeded)
    }

    #[test]
    fn semantic_ecmul_real_hooks_retain_included_revert_halt_and_caught_child_work() {
        for terminal in [0x00, 0xfd, 0xfe] {
            let (snapshot, succeeded) =
                execute_semantic_ecmul(semantic_ecmul_program(true, terminal), &[], true);
            assert_eq!(succeeded, terminal == 0x00);
            assert_eq!(snapshot.diagnostic, None);
            let counters = snapshot.ecmul.unwrap();
            assert_eq!(counters.successful_calls, 1);
            assert_eq!(counters.successful_native_gas, 6000);
            assert_eq!(counters.successful_scalar_work_units, 3);
            assert_eq!(counters.failed_calls, 0);
        }
        let child = Address::with_last_byte(0xd3);
        let mut parent = vec![0x60, 0, 0x60, 0, 0x60, 0, 0x60, 0, 0x73];
        parent.extend_from_slice(child.as_slice());
        parent.extend_from_slice(&[0x62, 0x03, 0x0d, 0x40, 0xfa, 0x50, 0x00]);
        let (snapshot, succeeded) = execute_semantic_ecmul(
            Bytes::from(parent),
            &[(child, semantic_ecmul_program(true, 0xfd))],
            true,
        );
        assert!(succeeded, "parent catches child REVERT");
        assert_eq!(snapshot.ecmul.unwrap().successful_scalar_work_units, 3);
    }

    #[test]
    fn semantic_ecmul_real_failure_and_discard_are_not_successful_work() {
        let (failed, succeeded) =
            execute_semantic_ecmul(semantic_ecmul_program(false, 0x00), &[], true);
        assert!(
            succeeded,
            "outer transaction catches failed off-curve precompile"
        );
        let counters = failed.ecmul.unwrap();
        assert_eq!(counters.successful_calls, 0);
        assert_eq!(counters.successful_scalar_work_units, 0);
        assert_eq!(counters.failed_calls, 1);
        assert_eq!(counters.failed_native_gas, 6000);
        let (discarded, succeeded) =
            execute_semantic_ecmul(semantic_ecmul_program(true, 0x00), &[], false);
        assert!(succeeded);
        assert_eq!(discarded, NodeSemanticCountersSnapshot::default());
    }

    #[test]
    fn semantic_ecmul_scalar_padding_and_reduction_are_explicit() {
        assert_eq!(ecmul_scalar(&[0; 64]), U256::ZERO);
        let mut short = vec![0; 65];
        short[64] = 1;
        assert_eq!(ecmul_scalar(&short), U256::from(1) << 248);
        let mut input = vec![0; 96];
        input[95] = 2;
        input.extend_from_slice(&[0xff; 32]);
        assert_eq!(ecmul_scalar(&input), U256::from(2));
        let order = U256::from_be_bytes(alloy_primitives::hex!(
            "30644e72e131a029b85045b68181585d2833e84879b9709143e1f593f0000001"
        ));
        let mut candidate = SemanticCandidate::default();
        candidate.start_call();
        candidate.finish_call(true, Some(6000), true, order);
        assert_eq!(
            candidate
                .snapshot()
                .ecmul
                .unwrap()
                .successful_scalar_work_units,
            0
        );
    }

    #[test]
    fn semantic_access_list_raw_keys_reuse_existing_committed_receipt() {
        let report = alethia_reth_evm::zk_gas::pricing::ZkGasBlockSnapshot {
            access_list_items: 2,
            access_list_construction:
                alethia_reth_evm::zk_gas::pricing::AccessListConstructionDiagnostics {
                    raw_address_items: 2,
                    raw_address_pairs: 1,
                    raw_storage_keys: 3,
                    max_low32_group: 3,
                    comparison_pairs: 3,
                    other_pairs: 0,
                    owner_pairs: 3,
                    incomplete: None,
                },
            ..Default::default()
        }
        .shadow_report();
        assert!(matches!(
            report.status(),
            alethia_reth_evm::zk_gas::pricing::ZkGasShadowStatus::Incomplete { .. }
        ));
        let exported = committed_semantic_counters(&CandidateResourceLedger::default(), &report);
        assert_eq!(exported.access_list_raw_storage_keys, Some(3));
        assert_eq!(exported.ecmul.unwrap().successful_calls, 0);
    }

    #[test]
    fn dropping_candidate_iterator_discards_the_last_uncommitted_scope() {
        let rejected_account = Address::with_last_byte(0xb4);
        let resources = Arc::new(Mutex::new(CandidateResourceLedger::default()));
        resources
            .lock()
            .expect("resource ledger poisoned")
            .begin_candidate();
        resources
            .lock()
            .expect("resource ledger poisoned")
            .active()
            .account(rejected_account);
        let mut consumed = 0;

        drop(CandidateTransactions {
            inner: std::iter::empty::<alloy_consensus::transaction::Recovered<&TransactionSigned>>(
            ),
            consumed: &mut consumed,
            resources: resources.clone(),
        });

        let resources = resources.lock().expect("resource ledger poisoned");
        assert!(resources.active.is_none());
        assert_eq!(
            resources.committed().selectors().unwrap(),
            ResourceSelectors::default()
        );
    }

    #[test]
    fn reconciliation_includes_inline_eip7702_delegation_code() {
        let caller = Address::with_last_byte(0xc1);
        let target = Address::with_last_byte(0xc2);
        let authority = Address::with_last_byte(0xc3);
        let delegate = Address::with_last_byte(0xc4);
        let mut database = InMemoryDB::default();
        database.insert_account_info(
            caller,
            AccountInfo {
                balance: U256::MAX,
                ..Default::default()
            },
        );
        database.insert_account_info(target, AccountInfo::default());
        database.insert_account_info(authority, AccountInfo::default());
        let mut state = State::builder().with_database(database).build();
        let ledger = Arc::new(Mutex::new(CandidateResourceLedger::default()));
        QuoteDerivedBlockExecutor::install_candidate_commit_hook(&mut state, ledger.clone());
        ledger
            .lock()
            .expect("resource ledger poisoned")
            .begin_candidate();
        let mut environment: EvmEnv<TaikoSpecId> = EvmEnv::default();
        environment.cfg_env.spec = TaikoSpecId::UNZEN;
        environment.cfg_env.chain_id = 167;
        environment.block_env.gas_limit = 30_000_000;
        let mut evm = TaikoEvmFactory.create_evm_with_inspector(
            &mut state,
            environment,
            ResourceInspector::with_ledger(ledger.clone()),
        );
        evm.set_zk_gas_pricing_enabled(true);
        let result = evm
            .transact_commit(
                TxEnv::builder()
                    .caller(caller)
                    .kind(TxKind::Call(target))
                    .chain_id(Some(167))
                    .gas_limit(1_000_000)
                    .authorization_list_recovered(vec![RecoveredAuthorization::new_unchecked(
                        Authorization {
                            chain_id: U256::from(167),
                            address: delegate,
                            nonce: 0,
                        },
                        RecoveredAuthority::Valid(authority),
                    )])
                    .build()
                    .expect("valid EIP-7702 transaction"),
            )
            .expect("EIP-7702 execution");
        assert!(result.is_success());
        // Raw TxEnv fixtures have no signed envelope; supply explicit synthetic metadata.
        evm.record_transaction_payload(
            alethia_reth_evm::zk_gas::pricing::TransactionPayloadSelectors {
                canonical_transaction_bytes: 100,
                nonempty_payload_transactions: 0,
            },
        );
        evm.commit_transaction_zk_gas_pricing()
            .expect("pricing commit");
        let local = evm
            .zk_gas_pricing_snapshot()
            .expect("pricing snapshot")
            .shadow_report();
        drop(evm);

        let authority_account = state
            .cache
            .accounts
            .get(&authority)
            .and_then(|account| account.account.as_ref())
            .expect("authorization authority cached");
        let delegation = authority_account
            .info
            .code
            .as_ref()
            .expect("authorization installs delegation code inline");
        assert_eq!(delegation.original_byte_slice().len(), 23);
        assert!(
            !state
                .cache
                .contracts
                .contains_key(&authority_account.info.code_hash),
            "delegation must exercise the account-inline reconciliation path"
        );

        let exact = ledger
            .lock()
            .expect("resource ledger poisoned")
            .committed()
            .selectors()
            .expect("exact resources");
        let alethia_reth_evm::zk_gas::pricing::ZkGasShadowStatus::Incomplete { known, .. } =
            local.status()
        else {
            panic!("expected incomplete local report")
        };
        assert!(exact.unique_code_hash_count >= known.evm_local_resources().unique_code_hash_count);
        assert!(exact.unique_code_bytes >= known.evm_local_resources().unique_code_bytes);
        assert!(exact.unique_code_bytes >= 23);
    }

    #[test]
    fn committed_eip7702_delegations_are_retained_before_later_overwrite() {
        let authority = Address::with_last_byte(0xe1);
        let first_delegate = Address::with_last_byte(0xe2);
        let second_delegate = Address::with_last_byte(0xe3);
        let delegation = |delegate: Address| {
            let mut bytes = vec![0xef, 0x01, 0x00];
            bytes.extend_from_slice(delegate.as_slice());
            let bytes = Bytes::from(bytes);
            AccountInfo {
                code_hash: keccak256(&bytes),
                code: Some(Bytecode::new_raw(bytes)),
                ..Default::default()
            }
        };
        let resources = Arc::new(Mutex::new(CandidateResourceLedger::default()));
        let mut state = State::builder()
            .with_database(InMemoryDB::default())
            .build();
        QuoteDerivedBlockExecutor::install_candidate_commit_hook(&mut state, resources.clone());

        for delegate in [first_delegate, second_delegate] {
            let mut changes = reth_revm::state::EvmState::default();
            changes.insert(
                authority,
                reth_revm::state::Account::from(delegation(delegate)),
            );
            resources
                .lock()
                .expect("resource ledger poisoned")
                .begin_candidate();
            reth_revm::DatabaseCommit::commit(&mut state, changes);
        }

        let exact = resources
            .lock()
            .expect("resource ledger poisoned")
            .committed()
            .selectors()
            .unwrap();
        assert_eq!(exact.unique_code_hash_count, 2);
        assert_eq!(exact.unique_code_bytes, 46);
    }

    fn extcode_materialization_program(copy_address: Address, size_address: Address) -> Bytecode {
        let mut code = vec![0x60, 1, 0x5f, 0x5f, 0x73];
        code.extend_from_slice(copy_address.as_slice());
        code.extend_from_slice(&[0x3c, 0x73]);
        code.extend_from_slice(size_address.as_slice());
        code.extend_from_slice(&[0x3b, 0x50, 0x00]);
        Bytecode::new_raw(Bytes::from(code))
    }

    fn extcodehash_program(first: Address, second: Address) -> Bytecode {
        let mut code = vec![0x73];
        code.extend_from_slice(first.as_slice());
        code.extend_from_slice(&[0x3f, 0x50, 0x73]);
        code.extend_from_slice(second.as_slice());
        code.extend_from_slice(&[0x3f, 0x50, 0x00]);
        Bytecode::new_raw(Bytes::from(code))
    }

    fn execute_extcode_program(
        program: Bytecode,
        contracts: &[(Address, Bytecode)],
    ) -> ResourceSelectors {
        let caller = Address::with_last_byte(0xf1);
        let target = Address::with_last_byte(0xf2);
        let mut database = InMemoryDB::default();
        database.insert_account_info(
            caller,
            AccountInfo {
                balance: U256::MAX,
                ..Default::default()
            },
        );
        for (address, code) in std::iter::once((target, program)).chain(contracts.iter().cloned()) {
            let code_hash = code.hash_slow();
            database.insert_account_info(
                address,
                AccountInfo {
                    nonce: 1,
                    code_hash,
                    code: Some(code),
                    ..Default::default()
                },
            );
            database
                .cache
                .accounts
                .get_mut(&address)
                .expect("inserted contract account")
                .info
                .code = None;
        }

        let mut state = State::builder().with_database(database).build();
        let ledger = Arc::new(Mutex::new(CandidateResourceLedger::default()));
        ledger
            .lock()
            .expect("resource ledger poisoned")
            .begin_candidate();
        let mut environment: EvmEnv<TaikoSpecId> = EvmEnv::default();
        environment.cfg_env.spec = TaikoSpecId::UNZEN;
        environment.cfg_env.chain_id = 167;
        environment.block_env.gas_limit = 30_000_000;
        let mut evm = TaikoEvmFactory.create_evm_with_inspector(
            &mut state,
            environment,
            ResourceInspector::with_ledger(ledger.clone()),
        );
        let result = evm
            .transact(
                TxEnv::builder()
                    .caller(caller)
                    .kind(TxKind::Call(target))
                    .chain_id(Some(167))
                    .gas_limit(1_000_000)
                    .build()
                    .expect("valid EXTCODE transaction"),
            )
            .expect("EXTCODE transaction execution");
        assert!(result.result.is_success());
        drop(evm);
        ledger
            .lock()
            .expect("resource ledger poisoned")
            .commit_candidate()
            .expect("commit candidate resources");
        ledger
            .lock()
            .expect("resource ledger poisoned")
            .committed()
            .selectors()
            .expect("EXTCODE resources")
    }

    #[test]
    fn committed_candidate_retains_materialized_extcode_bodies() {
        let first = Address::with_last_byte(0xf3);
        let second = Address::with_last_byte(0xf4);
        let first_code = Bytecode::new_raw(Bytes::from(vec![0x00; 24_000]));
        let mut second_bytes = vec![0x00; 24_000];
        second_bytes[23_999] = 0x5b;
        let second_code = Bytecode::new_raw(Bytes::from(second_bytes));
        let contracts = [(first, first_code), (second, second_code)];

        let repeated =
            execute_extcode_program(extcode_materialization_program(first, first), &contracts);
        let distinct =
            execute_extcode_program(extcode_materialization_program(first, second), &contracts);
        let hash_only = execute_extcode_program(extcodehash_program(first, second), &contracts);

        assert_eq!(
            distinct.unique_code_hash_count,
            repeated.unique_code_hash_count + 1
        );
        assert_eq!(
            distinct.unique_code_bytes,
            repeated.unique_code_bytes + 24_000
        );
        assert_eq!(
            hash_only.unique_code_hash_count, 1,
            "only the executed parent code"
        );
        assert_eq!(
            hash_only.unique_code_bytes,
            u64::try_from(extcodehash_program(first, second).len()).unwrap(),
            "EXTCODEHASH must not materialize the referenced bodies",
        );
    }

    #[test]
    fn failed_child_sstore_is_still_an_exact_storage_write() {
        let caller = Address::with_last_byte(0xd1);
        let target = Address::with_last_byte(0xd2);
        let child = Address::with_last_byte(0xd3);
        let child_code = Bytes::from_static(&[
            0x60, 0x01, // PUSH1 value
            0x60, 0x00, // PUSH1 slot
            0x55, // SSTORE (halts this static child frame)
            0x00, // STOP
        ]);
        let mut target_code = vec![
            0x60, 0x00, // return size
            0x60, 0x00, // return offset
            0x60, 0x00, // calldata size
            0x60, 0x00, // calldata offset
            0x73, // PUSH20 child
        ];
        target_code.extend_from_slice(child.as_slice());
        target_code.extend_from_slice(&[
            0x61, 0xff, 0xff, // PUSH2 gas
            0xfa, // STATICCALL; returns false after child SSTORE halt
            0x50, // POP success flag
            0x00, // STOP, so the outer transaction commits
        ]);

        let account_with_code = |raw: Bytes| AccountInfo {
            code_hash: keccak256(&raw),
            code: Some(Bytecode::new_raw(raw)),
            ..Default::default()
        };
        let mut database = InMemoryDB::default();
        database.insert_account_info(
            caller,
            AccountInfo {
                balance: U256::MAX,
                ..Default::default()
            },
        );
        database.insert_account_info(target, account_with_code(target_code.into()));
        database.insert_account_info(child, account_with_code(child_code));
        let mut state = State::builder().with_database(database).build();
        let ledger = Arc::new(Mutex::new(CandidateResourceLedger::default()));
        QuoteDerivedBlockExecutor::install_candidate_commit_hook(&mut state, ledger.clone());
        ledger
            .lock()
            .expect("resource ledger poisoned")
            .begin_candidate();
        let mut environment: EvmEnv<TaikoSpecId> = EvmEnv::default();
        environment.cfg_env.spec = TaikoSpecId::UNZEN;
        environment.cfg_env.chain_id = 167;
        environment.block_env.gas_limit = 30_000_000;
        let mut evm = TaikoEvmFactory.create_evm_with_inspector(
            &mut state,
            environment,
            ResourceInspector::with_ledger(ledger.clone()),
        );
        evm.set_zk_gas_pricing_enabled(true);
        let result = evm
            .transact_commit(
                TxEnv::builder()
                    .caller(caller)
                    .kind(TxKind::Call(target))
                    .chain_id(Some(167))
                    .gas_limit(1_000_000)
                    .build()
                    .expect("valid call transaction"),
            )
            .expect("outer execution");
        assert!(result.is_success(), "outer transaction must commit");
        // Raw TxEnv fixtures have no signed envelope; supply explicit synthetic metadata.
        evm.record_transaction_payload(
            alethia_reth_evm::zk_gas::pricing::TransactionPayloadSelectors {
                canonical_transaction_bytes: 100,
                nonempty_payload_transactions: 0,
            },
        );
        evm.commit_transaction_zk_gas_pricing()
            .expect("pricing commit");
        let local = evm
            .zk_gas_pricing_snapshot()
            .expect("pricing snapshot")
            .shadow_report();
        drop(evm);

        let exact = ledger
            .lock()
            .expect("resource ledger poisoned")
            .committed()
            .selectors()
            .expect("exact resources");
        let alethia_reth_evm::zk_gas::pricing::ZkGasShadowStatus::Incomplete { known, .. } =
            local.status()
        else {
            panic!("expected incomplete local report")
        };
        let local = known.evm_local_resources();
        assert_eq!(local.unique_storage_slot_write_count, 1);
        assert!(
            exact.unique_storage_slot_write_count >= local.unique_storage_slot_write_count,
            "failed child storage attempts must remain eligible for exact completion"
        );
    }
}
