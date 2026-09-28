//! REVM inspector-backed operation collection.

use alethia_reth_evm::alloy::TaikoEvmContext;
use alloy_primitives::{Address, U256};
use reth_revm::{
    Database, Inspector,
    bytecode::OpCode,
    context::{ContextTr, JournalTr},
    handler::FrameResult,
    interpreter::{
        CallInputs, CallOutcome, CreateInputs, CreateOutcome, FrameInput, Interpreter,
        InterpreterAction,
        interpreter::EthInterpreter,
        interpreter_types::{InputsTr, Jumps, LoopControl},
    },
};
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
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
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OpcodeModelInput {
    StaticRawGas {
        raw_gas: u64,
    },
    Exp {
        exponent_byte_length: u64,
    },
    Keccak {
        input_length: u64,
        memory_growth_event: u64,
        memory_evm_gas_delta: u64,
        memory_4k_boundary_event: u64,
    },
    MemoryAccess {
        memory_growth_event: u64,
        memory_evm_gas_delta: u64,
        memory_4k_boundary_event: u64,
    },
    MemoryCopy {
        copy_words: u64,
        memory_growth_event: u64,
        memory_evm_gas_delta: u64,
        memory_4k_boundary_event: u64,
    },
    StorageLoad {
        access: StorageAccess,
    },
    StorageStore {
        access: StorageAccess,
        branch: StorageStoreBranch,
    },
    Invalid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageAccess {
    Cold,
    Warm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageStoreBranch {
    Noop,
    Set,
    Clear,
    Reset,
    DirtyRewrite,
    RestoreOriginal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageStateError {
    InstructionHalted,
    DirtyNoop,
    DirtyCold,
    MissingPostState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OpcodeFeatureError {
    StackUnderflow { feature: String },
    ValueOutOfRange { feature: String },
    StorageState { reason: StorageStateError },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OperationComponent {
    Opcode {
        opcode: u8,
        #[serde(skip_serializing_if = "Option::is_none")]
        pricing_basis: Option<PricingBasis>,
        #[serde(skip_serializing_if = "Option::is_none")]
        interpreter_raw_gas: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model_input: Option<OpcodeModelInput>,
        #[serde(skip_serializing_if = "Option::is_none")]
        spawned: Option<bool>,
        dispatch_status: DispatchStatus,
    },
    OpcodeFeatureError {
        opcode: u8,
        interpreter_raw_gas: u64,
        error: OpcodeFeatureError,
    },
    Precompile {
        address: Address,
        pricing_basis: PricingBasis,
        native_gas: u64,
    },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum OperationComponentWire {
    Opcode {
        opcode: u8,
        #[serde(default)]
        pricing_basis: Option<PricingBasis>,
        #[serde(default)]
        interpreter_raw_gas: Option<u64>,
        #[serde(default)]
        model_input: Option<OpcodeModelInput>,
        #[serde(default)]
        spawned: Option<bool>,
        dispatch_status: DispatchStatus,
    },
    OpcodeFeatureError {
        opcode: u8,
        interpreter_raw_gas: u64,
        error: OpcodeFeatureError,
    },
    Precompile {
        address: Address,
        pricing_basis: PricingBasis,
        native_gas: u64,
    },
}

impl<'de> Deserialize<'de> for OperationComponent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = OperationComponentWire::deserialize(deserializer)?;
        match wire {
            OperationComponentWire::Opcode {
                opcode,
                pricing_basis,
                interpreter_raw_gas,
                model_input,
                spawned,
                dispatch_status,
            } => {
                validate_opcode_component(
                    opcode,
                    pricing_basis,
                    interpreter_raw_gas,
                    model_input.as_ref(),
                    spawned,
                    dispatch_status,
                )
                .map_err(D::Error::custom)?;
                Ok(Self::Opcode {
                    opcode,
                    pricing_basis,
                    interpreter_raw_gas,
                    model_input,
                    spawned,
                    dispatch_status,
                })
            }
            OperationComponentWire::Precompile {
                address,
                pricing_basis,
                native_gas,
            } => Ok(Self::Precompile {
                address,
                pricing_basis,
                native_gas,
            }),
            OperationComponentWire::OpcodeFeatureError {
                opcode,
                interpreter_raw_gas,
                error,
            } => {
                validate_opcode_feature_error(opcode, &error).map_err(D::Error::custom)?;
                Ok(Self::OpcodeFeatureError {
                    opcode,
                    interpreter_raw_gas,
                    error,
                })
            }
        }
    }
}

fn validate_opcode_feature_error(
    opcode: u8,
    error: &OpcodeFeatureError,
) -> Result<(), &'static str> {
    let feature = match error {
        OpcodeFeatureError::StorageState { reason } => {
            return if matches!(
                (opcode, reason),
                (0x54, StorageStateError::InstructionHalted) | (0x55, _)
            ) {
                Ok(())
            } else {
                Err("storage feature error is incompatible with the opcode")
            };
        }
        OpcodeFeatureError::StackUnderflow { feature } if opcode == 0x54 => {
            return if feature == "storage_key" {
                Ok(())
            } else {
                Err("SLOAD feature error names the wrong feature")
            };
        }
        OpcodeFeatureError::StackUnderflow { feature } if opcode == 0x55 => {
            return if matches!(feature.as_str(), "storage_key" | "new_storage_value") {
                Ok(())
            } else {
                Err("SSTORE feature error names the wrong feature")
            };
        }
        OpcodeFeatureError::StackUnderflow { feature }
        | OpcodeFeatureError::ValueOutOfRange { feature } => feature.as_str(),
    };
    let expected_feature = match opcode {
        0x0a => "exponent_byte_length",
        0x20 => "input_length",
        0x5e => "copy_words",
        _ => return Err("opcode cannot emit a typed feature error"),
    };
    if feature != expected_feature {
        return Err("opcode feature error names the wrong feature");
    }
    Ok(())
}

fn validate_opcode_component(
    opcode: u8,
    pricing_basis: Option<PricingBasis>,
    interpreter_raw_gas: Option<u64>,
    model_input: Option<&OpcodeModelInput>,
    spawned: Option<bool>,
    dispatch_status: DispatchStatus,
) -> Result<(), &'static str> {
    match dispatch_status {
        DispatchStatus::Confirmed => {
            if !is_spawn_opcode(opcode)
                || pricing_basis != Some(PricingBasis::FixedPerEvent)
                || interpreter_raw_gas.is_some()
                || model_input.is_some()
                || spawned != Some(true)
            {
                return Err("confirmed spawn wrapper fields differ from the schema");
            }
            return Ok(());
        }
        DispatchStatus::SelectedNotDispatched => {
            if !is_spawn_opcode(opcode)
                || pricing_basis.is_some()
                || interpreter_raw_gas.is_some()
                || model_input.is_some()
                || spawned != Some(true)
            {
                return Err("undispatched spawn fields differ from the schema");
            }
            return Ok(());
        }
        DispatchStatus::NotApplicable => {}
    }

    if pricing_basis != Some(PricingBasis::RawGasSlope) || spawned != Some(false) {
        return Err("executed opcode pricing fields differ from the schema");
    }
    let raw_gas = interpreter_raw_gas.ok_or("executed opcode is missing interpreter raw gas")?;
    let model_input = model_input.ok_or("executed opcode is missing model input")?;
    let model_matches = match opcode {
        0x0a => matches!(
            model_input,
            OpcodeModelInput::Exp {
                exponent_byte_length
            } if *exponent_byte_length <= 32
        ),
        0x20 => matches!(model_input, OpcodeModelInput::Keccak { .. }),
        0x51..=0x53 => matches!(model_input, OpcodeModelInput::MemoryAccess { .. }),
        0x54 => matches!(model_input, OpcodeModelInput::StorageLoad { .. }),
        0x55 => matches!(
            model_input,
            OpcodeModelInput::StorageStore { access, branch }
                if !matches!(
                    (access, branch),
                    (
                        StorageAccess::Cold,
                        StorageStoreBranch::DirtyRewrite | StorageStoreBranch::RestoreOriginal
                    )
                )
        ),
        0x5e => matches!(model_input, OpcodeModelInput::MemoryCopy { .. }),
        _ if OpCode::info_by_op(opcode).is_none() => {
            matches!(model_input, OpcodeModelInput::Invalid)
        }
        _ => matches!(
            model_input,
            OpcodeModelInput::StaticRawGas { raw_gas: input_raw_gas }
                if *input_raw_gas == raw_gas
        ),
    };
    if !model_matches {
        return Err("opcode model input is incompatible with the opcode");
    }
    Ok(())
}

const fn is_spawn_opcode(opcode: u8) -> bool {
    matches!(opcode, 0xf0 | 0xf1 | 0xf2 | 0xf4 | 0xf5 | 0xfa)
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
        model_input: OpcodeModelInput,
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
                model_input: Some(model_input),
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
                model_input: None,
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

    pub fn record_opcode_feature_error(
        &mut self,
        opcode: u8,
        frame_depth: usize,
        interpreter_raw_gas: u64,
        error: OpcodeFeatureError,
    ) {
        self.flush_pending_spawn();
        self.push(
            self.phase(),
            self.current_tx_index,
            frame_depth,
            OperationComponent::OpcodeFeatureError {
                opcode,
                interpreter_raw_gas,
                error,
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
                model_input: None,
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

#[derive(Clone, Debug)]
struct StepSnapshot {
    opcode: u8,
    gas_remaining: u64,
    memory_words: usize,
    memory_expansion_cost: u64,
    model_input: Result<PendingOpcodeModelInput, OpcodeFeatureError>,
}

#[derive(Clone, Copy, Debug)]
enum PendingOpcodeModelInput {
    StaticRawGas,
    Exp { exponent_byte_length: u64 },
    Keccak { input_length: u64 },
    MemoryAccess,
    MemoryCopy { copy_words: u64 },
    StorageLoad(PendingStorageInput),
    StorageStore(PendingStorageInput),
    Invalid,
}

#[derive(Clone, Copy, Debug)]
struct PendingStorageInput {
    target: Address,
    key: U256,
    access: StorageAccess,
    original: Option<U256>,
    current: Option<U256>,
    new: Option<U256>,
}

impl PendingOpcodeModelInput {
    fn capture<DB: Database>(
        interp: &Interpreter<EthInterpreter>,
        opcode: u8,
        context: &TaikoEvmContext<DB>,
    ) -> Result<Self, OpcodeFeatureError> {
        let stack_value = |index, feature: &'static str| {
            interp
                .stack
                .peek(index)
                .map_err(|_| OpcodeFeatureError::StackUnderflow {
                    feature: feature.to_string(),
                })
        };
        let stack_u64 = |index, feature: &'static str| {
            u64::try_from(stack_value(index, feature)?).map_err(|_| {
                OpcodeFeatureError::ValueOutOfRange {
                    feature: feature.to_string(),
                }
            })
        };
        match opcode {
            0x0a => {
                let exponent = stack_value(1, "exponent_byte_length")?;
                Ok(Self::Exp {
                    exponent_byte_length: exponent.bit_len().div_ceil(8) as u64,
                })
            }
            0x20 => Ok(Self::Keccak {
                input_length: stack_u64(1, "input_length")?,
            }),
            0x51..=0x53 => Ok(Self::MemoryAccess),
            0x54 => Ok(Self::StorageLoad(capture_storage_input(
                interp, context, None,
            )?)),
            0x55 => Ok(Self::StorageStore(capture_storage_input(
                interp,
                context,
                Some(1),
            )?)),
            0x5e => Ok(Self::MemoryCopy {
                copy_words: stack_u64(2, "copy_words")?.div_ceil(32),
            }),
            _ if OpCode::info_by_op(opcode).is_none() => Ok(Self::Invalid),
            _ => Ok(Self::StaticRawGas),
        }
    }

    fn finish<DB: Database>(
        self,
        interpreter_raw_gas: u64,
        before_memory_words: usize,
        before_memory_expansion_cost: u64,
        interp: &Interpreter<EthInterpreter>,
        context: &TaikoEvmContext<DB>,
    ) -> Result<OpcodeModelInput, StorageStateError> {
        let after_memory_words = interp.memory.len().div_ceil(32);
        let memory_grew = after_memory_words > before_memory_words;
        let memory_growth_event = u64::from(memory_grew);
        let memory_evm_gas_delta = if memory_grew {
            interp
                .gas
                .memory()
                .expansion_cost
                .saturating_sub(before_memory_expansion_cost)
        } else {
            0
        };
        let memory_4k_boundary_event =
            u64::from(extra_4k_pages(after_memory_words) > extra_4k_pages(before_memory_words));
        Ok(match self {
            Self::StaticRawGas => OpcodeModelInput::StaticRawGas {
                raw_gas: interpreter_raw_gas,
            },
            Self::Exp {
                exponent_byte_length,
            } => OpcodeModelInput::Exp {
                exponent_byte_length,
            },
            Self::Keccak { input_length } => OpcodeModelInput::Keccak {
                input_length,
                memory_growth_event,
                memory_evm_gas_delta,
                memory_4k_boundary_event,
            },
            Self::MemoryAccess => OpcodeModelInput::MemoryAccess {
                memory_growth_event,
                memory_evm_gas_delta,
                memory_4k_boundary_event,
            },
            Self::MemoryCopy { copy_words } => OpcodeModelInput::MemoryCopy {
                copy_words,
                memory_growth_event,
                memory_evm_gas_delta,
                memory_4k_boundary_event,
            },
            Self::StorageLoad(storage) => finish_storage_load(storage, context)?,
            Self::StorageStore(storage) => finish_storage_store(storage, context)?,
            Self::Invalid => OpcodeModelInput::Invalid,
        })
    }
}

fn capture_storage_input<DB: Database>(
    interp: &Interpreter<EthInterpreter>,
    context: &TaikoEvmContext<DB>,
    new_stack_index: Option<usize>,
) -> Result<PendingStorageInput, OpcodeFeatureError> {
    let key = interp
        .stack
        .peek(0)
        .map_err(|_| OpcodeFeatureError::StackUnderflow {
            feature: "storage_key".to_string(),
        })?;
    let new = new_stack_index
        .map(|index| {
            interp
                .stack
                .peek(index)
                .map_err(|_| OpcodeFeatureError::StackUnderflow {
                    feature: "new_storage_value".to_string(),
                })
        })
        .transpose()?;
    let target = interp.input.target_address();
    let journal = &context.journaled_state.inner;
    let access_list_warm = journal.warm_addresses.is_storage_warm(&target, &key);
    let cached = journal
        .state
        .get(&target)
        .and_then(|account| account.storage.get(&key));
    let (access, original, current) = cached.map_or_else(
        || {
            (
                if access_list_warm {
                    StorageAccess::Warm
                } else {
                    StorageAccess::Cold
                },
                None,
                None,
            )
        },
        |slot| {
            let stale = slot.is_cold_transaction_id(journal.transaction_id);
            (
                if stale && !access_list_warm {
                    StorageAccess::Cold
                } else {
                    StorageAccess::Warm
                },
                Some(if stale {
                    slot.present_value()
                } else {
                    slot.original_value()
                }),
                Some(slot.present_value()),
            )
        },
    );
    Ok(PendingStorageInput {
        target,
        key,
        access,
        original,
        current,
        new,
    })
}

fn finish_storage_load<DB: Database>(
    storage: PendingStorageInput,
    context: &TaikoEvmContext<DB>,
) -> Result<OpcodeModelInput, StorageStateError> {
    match (storage.original, storage.current) {
        (Some(_), Some(_)) => {}
        (None, None)
            if context
                .journaled_state
                .inner
                .state
                .get(&storage.target)
                .and_then(|account| account.storage.get(&storage.key))
                .is_some() => {}
        _ => return Err(StorageStateError::MissingPostState),
    }
    Ok(OpcodeModelInput::StorageLoad {
        access: storage.access,
    })
}

fn finish_storage_store<DB: Database>(
    storage: PendingStorageInput,
    context: &TaikoEvmContext<DB>,
) -> Result<OpcodeModelInput, StorageStateError> {
    let (original, current) = match (storage.original, storage.current) {
        (Some(original), Some(current)) => (original, current),
        (None, None) => {
            let slot = context
                .journaled_state
                .inner
                .state
                .get(&storage.target)
                .and_then(|account| account.storage.get(&storage.key))
                .ok_or(StorageStateError::MissingPostState)?;
            (slot.original_value(), slot.original_value())
        }
        _ => return Err(StorageStateError::MissingPostState),
    };
    let new = storage.new.ok_or(StorageStateError::MissingPostState)?;
    let branch = if current != original {
        if new == current {
            return Err(StorageStateError::DirtyNoop);
        }
        if storage.access == StorageAccess::Cold {
            return Err(StorageStateError::DirtyCold);
        }
        if new == original {
            StorageStoreBranch::RestoreOriginal
        } else {
            StorageStoreBranch::DirtyRewrite
        }
    } else if new == current {
        StorageStoreBranch::Noop
    } else if current.is_zero() {
        StorageStoreBranch::Set
    } else if new.is_zero() {
        StorageStoreBranch::Clear
    } else {
        StorageStoreBranch::Reset
    };
    Ok(OpcodeModelInput::StorageStore {
        access: storage.access,
        branch,
    })
}

const fn extra_4k_pages(memory_words: usize) -> usize {
    memory_words.div_ceil(128).saturating_sub(1)
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
        let opcode = interp.bytecode.opcode();
        let model_input = PendingOpcodeModelInput::capture(interp, opcode, context);
        self.pending_steps.insert(
            depth,
            StepSnapshot {
                opcode,
                gas_remaining: interp.gas.remaining(),
                memory_words: interp.memory.len().div_ceil(32),
                memory_expansion_cost: interp.gas.memory().expansion_cost,
                model_input,
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
        let interpreter_raw_gas = step.gas_remaining.saturating_sub(interp.gas.remaining());
        let pending_model_input = match step.model_input {
            Ok(model_input) => model_input,
            Err(error) => {
                self.sink.lock().record_opcode_feature_error(
                    step.opcode,
                    depth,
                    interpreter_raw_gas,
                    error,
                );
                return;
            }
        };
        if matches!(step.opcode, 0x54 | 0x55) && interp.bytecode.instruction_result().is_some() {
            self.sink.lock().record_opcode_feature_error(
                step.opcode,
                depth,
                interpreter_raw_gas,
                OpcodeFeatureError::StorageState {
                    reason: StorageStateError::InstructionHalted,
                },
            );
            return;
        }
        let model_input = match pending_model_input.finish(
            interpreter_raw_gas,
            step.memory_words,
            step.memory_expansion_cost,
            interp,
            context,
        ) {
            Ok(model_input) => model_input,
            Err(reason) => {
                self.sink.lock().record_opcode_feature_error(
                    step.opcode,
                    depth,
                    interpreter_raw_gas,
                    OpcodeFeatureError::StorageState { reason },
                );
                return;
            }
        };
        self.sink.lock().record_opcode(
            step.opcode,
            depth,
            interpreter_raw_gas,
            model_input,
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
