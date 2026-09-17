//! REVM inspector-backed operation collection.

use alethia_reth_evm::alloy::TaikoEvmContext;
use alloy_primitives::Address;
use reth_revm::{
    Database, Inspector,
    context::{ContextTr, JournalTr},
    handler::FrameResult,
    interpreter::{
        CallInputs, CallOutcome, CreateInputs, CreateOutcome, FrameInput, Interpreter,
        InterpreterAction,
        interpreter::EthInterpreter,
        interpreter_types::{Jumps, LoopControl},
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PricingBasis {
    RawGasSlope,
    FixedPerEvent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchStatus {
    NotApplicable,
    Confirmed,
    SelectedNotDispatched,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationPhase {
    System,
    Transaction,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OperationComponent {
    Opcode {
        opcode: u8,
        #[serde(skip_serializing_if = "Option::is_none")]
        pricing_basis: Option<PricingBasis>,
        #[serde(skip_serializing_if = "Option::is_none")]
        interpreter_raw_gas: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        spawned: Option<bool>,
        dispatch_status: DispatchStatus,
    },
    Precompile {
        address: Address,
        pricing_basis: PricingBasis,
        native_gas: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationTrace {
    pub operation_id: u64,
    pub phase: OperationPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_index: Option<usize>,
    pub frame_depth: usize,
    pub component: OperationComponent,
}

#[derive(Clone, Copy, Debug)]
struct PendingSpawn {
    opcode: u8,
    phase: OperationPhase,
    tx_index: Option<usize>,
    frame_depth: usize,
}

#[derive(Clone, Debug, Default)]
pub struct TraceCollector {
    operations: Vec<OperationTrace>,
    current_tx_index: Option<usize>,
    pending_spawn: Option<PendingSpawn>,
}

impl TraceCollector {
    pub fn start_transaction(&mut self, tx_index: usize) {
        self.flush_pending_spawn();
        self.current_tx_index = Some(tx_index);
    }

    pub fn finish_transaction(&mut self) {
        self.flush_pending_spawn();
        self.current_tx_index = None;
    }

    pub fn record_opcode(
        &mut self,
        opcode: u8,
        frame_depth: usize,
        interpreter_raw_gas: u64,
        selected_new_frame: bool,
    ) {
        self.flush_pending_spawn();
        let phase = self.phase();
        if selected_new_frame {
            self.pending_spawn = Some(PendingSpawn {
                opcode,
                phase,
                tx_index: self.current_tx_index,
                frame_depth,
            });
            return;
        }
        self.push(
            phase,
            self.current_tx_index,
            frame_depth,
            OperationComponent::Opcode {
                opcode,
                pricing_basis: Some(PricingBasis::RawGasSlope),
                interpreter_raw_gas: Some(interpreter_raw_gas),
                spawned: Some(false),
                dispatch_status: DispatchStatus::NotApplicable,
            },
        );
    }

    pub fn confirm_spawn(&mut self, frame_depth: usize) {
        let Some(pending) = self.pending_spawn.take() else {
            return;
        };
        if pending.frame_depth != frame_depth {
            self.emit_unconfirmed(pending);
            return;
        }
        self.push(
            pending.phase,
            pending.tx_index,
            pending.frame_depth,
            OperationComponent::Opcode {
                opcode: pending.opcode,
                pricing_basis: Some(PricingBasis::FixedPerEvent),
                interpreter_raw_gas: None,
                spawned: Some(true),
                dispatch_status: DispatchStatus::Confirmed,
            },
        );
    }

    pub fn record_precompile(&mut self, address: Address, frame_depth: usize, native_gas: u64) {
        self.flush_pending_spawn();
        self.push(
            self.phase(),
            self.current_tx_index,
            frame_depth,
            OperationComponent::Precompile {
                address,
                pricing_basis: PricingBasis::RawGasSlope,
                native_gas,
            },
        );
    }

    pub fn flush_pending_spawn(&mut self) {
        if let Some(pending) = self.pending_spawn.take() {
            self.emit_unconfirmed(pending);
        }
    }

    pub fn flush_pending_spawn_at(&mut self, frame_depth: usize) {
        if self
            .pending_spawn
            .as_ref()
            .is_some_and(|pending| pending.frame_depth == frame_depth)
        {
            self.flush_pending_spawn();
        }
    }

    #[must_use]
    pub fn operations(&self) -> &[OperationTrace] {
        &self.operations
    }

    #[must_use]
    pub fn into_operations(mut self) -> Vec<OperationTrace> {
        self.flush_pending_spawn();
        self.operations
    }

    const fn phase(&self) -> OperationPhase {
        if self.current_tx_index.is_some() {
            OperationPhase::Transaction
        } else {
            OperationPhase::System
        }
    }

    fn emit_unconfirmed(&mut self, pending: PendingSpawn) {
        self.push(
            pending.phase,
            pending.tx_index,
            pending.frame_depth,
            OperationComponent::Opcode {
                opcode: pending.opcode,
                pricing_basis: None,
                interpreter_raw_gas: None,
                spawned: Some(true),
                dispatch_status: DispatchStatus::SelectedNotDispatched,
            },
        );
    }

    fn push(
        &mut self,
        phase: OperationPhase,
        tx_index: Option<usize>,
        frame_depth: usize,
        component: OperationComponent,
    ) {
        self.operations.push(OperationTrace {
            operation_id: self.operations.len() as u64,
            phase,
            tx_index,
            frame_depth,
            component,
        });
    }
}

#[derive(Clone, Debug, Default)]
pub struct TraceSink(Arc<Mutex<TraceCollector>>);

impl TraceSink {
    pub fn lock(&self) -> MutexGuard<'_, TraceCollector> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[must_use]
    pub fn snapshot(&self) -> TraceCollector {
        self.lock().clone()
    }
}

#[derive(Clone, Copy, Debug)]
struct StepSnapshot {
    opcode: u8,
    gas_remaining: u64,
}

#[derive(Debug)]
pub struct TraceInspector {
    sink: TraceSink,
    pending_steps: BTreeMap<usize, StepSnapshot>,
}

impl TraceInspector {
    #[must_use]
    pub const fn new(sink: TraceSink) -> Self {
        Self {
            sink,
            pending_steps: BTreeMap::new(),
        }
    }

    #[must_use]
    pub const fn sink(&self) -> &TraceSink {
        &self.sink
    }
}

impl<DB> Inspector<TaikoEvmContext<DB>, EthInterpreter> for TraceInspector
where
    DB: Database,
{
    fn step(
        &mut self,
        interp: &mut Interpreter<EthInterpreter>,
        context: &mut TaikoEvmContext<DB>,
    ) {
        let depth = context.journal().depth();
        self.pending_steps.insert(
            depth,
            StepSnapshot {
                opcode: interp.bytecode.opcode(),
                gas_remaining: interp.gas.remaining(),
            },
        );
    }

    fn step_end(
        &mut self,
        interp: &mut Interpreter<EthInterpreter>,
        context: &mut TaikoEvmContext<DB>,
    ) {
        let depth = context.journal().depth();
        let Some(step) = self.pending_steps.remove(&depth) else {
            return;
        };
        let selected_new_frame = matches!(
            interp.bytecode.action(),
            Some(InterpreterAction::NewFrame(_))
        );
        self.sink.lock().record_opcode(
            step.opcode,
            depth,
            step.gas_remaining.saturating_sub(interp.gas.remaining()),
            selected_new_frame,
        );
    }

    fn call(
        &mut self,
        context: &mut TaikoEvmContext<DB>,
        _inputs: &mut CallInputs,
    ) -> Option<CallOutcome> {
        self.sink.lock().confirm_spawn(context.journal().depth());
        None
    }

    fn call_end(
        &mut self,
        context: &mut TaikoEvmContext<DB>,
        inputs: &CallInputs,
        outcome: &mut CallOutcome,
    ) {
        if outcome.was_precompile_called {
            let native_gas = inputs
                .gas_limit
                .saturating_sub(outcome.result.gas.remaining());
            self.sink.lock().record_precompile(
                inputs.bytecode_address,
                context.journal().depth(),
                native_gas,
            );
        }
    }

    fn create(
        &mut self,
        context: &mut TaikoEvmContext<DB>,
        _inputs: &mut CreateInputs,
    ) -> Option<CreateOutcome> {
        self.sink.lock().confirm_spawn(context.journal().depth());
        None
    }

    fn frame_end(
        &mut self,
        context: &mut TaikoEvmContext<DB>,
        _frame_input: &FrameInput,
        _frame_result: &mut FrameResult,
    ) {
        let depth = context.journal().depth();
        self.pending_steps.remove(&depth);
        self.sink.lock().flush_pending_spawn_at(depth);
    }
}
