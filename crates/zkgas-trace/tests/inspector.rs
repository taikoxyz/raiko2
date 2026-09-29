#![allow(missing_docs)]

use alethia_reth_evm::{
    alloy::TaikoZkGasEvm, factory::TaikoEvmFactory, spec::TaikoSpecId,
    zk_gas::schedule::schedule_for,
};
use alloy_evm::{Evm, EvmEnv, EvmFactory};
use alloy_primitives::{Address, B256, U256, address};
use raiko2_zkgas_trace::{
    DispatchStatus, OpcodeModelInput, OperationComponent, OperationPhase, PricingBasis,
    TraceCollector, TraceInspector, TraceSink,
};
use reth_revm::{
    context::{
        TxEnv,
        transaction::{AccessList, AccessListItem},
    },
    db::InMemoryDB,
    primitives::{Bytes, TxKind},
    state::{AccountInfo, Bytecode, bytecode::opcode},
};
use serde_json::{Value, json};

const CALLER: Address = address!("1000000000000000000000000000000000000000");
const TARGET: Address = address!("2000000000000000000000000000000000000000");
const CHILD: Address = address!("3000000000000000000000000000000000000000");

#[derive(Clone, Copy, Debug)]
enum SpawnFamily {
    Call,
    CallCode,
    DelegateCall,
    StaticCall,
    Create,
    Create2,
}

impl SpawnFamily {
    const ALL: [Self; 6] = [
        Self::Call,
        Self::CallCode,
        Self::DelegateCall,
        Self::StaticCall,
        Self::Create,
        Self::Create2,
    ];

    const fn opcode(self) -> u8 {
        match self {
            Self::Call => opcode::CALL,
            Self::CallCode => opcode::CALLCODE,
            Self::DelegateCall => opcode::DELEGATECALL,
            Self::StaticCall => opcode::STATICCALL,
            Self::Create => opcode::CREATE,
            Self::Create2 => opcode::CREATE2,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Call => "CALL",
            Self::CallCode => "CALLCODE",
            Self::DelegateCall => "DELEGATECALL",
            Self::StaticCall => "STATICCALL",
            Self::Create => "CREATE",
            Self::Create2 => "CREATE2",
        }
    }

    fn spawned_bytecode(self) -> Bytecode {
        let mut code = match self {
            Self::Call | Self::CallCode => vec![
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
            ],
            Self::DelegateCall | Self::StaticCall => vec![
                opcode::PUSH1,
                0,
                opcode::PUSH1,
                0,
                opcode::PUSH1,
                0,
                opcode::PUSH1,
                0,
                opcode::PUSH20,
            ],
            Self::Create => {
                return Bytecode::new_raw(Bytes::from(vec![
                    opcode::PUSH1,
                    0,
                    opcode::PUSH1,
                    0,
                    opcode::PUSH1,
                    0,
                    opcode::CREATE,
                    opcode::STOP,
                ]));
            }
            Self::Create2 => {
                return Bytecode::new_raw(Bytes::from(vec![
                    opcode::PUSH1,
                    0,
                    opcode::PUSH1,
                    0,
                    opcode::PUSH1,
                    0,
                    opcode::PUSH1,
                    0,
                    opcode::CREATE2,
                    opcode::STOP,
                ]));
            }
        };
        code.extend_from_slice(CHILD.as_slice());
        code.extend_from_slice(&[opcode::PUSH2, 0xff, 0xff, self.opcode(), opcode::STOP]);
        Bytecode::new_raw(Bytes::from(code))
    }

    const fn spawn_estimate(self) -> u64 {
        let Some(schedule) = schedule_for(TaikoSpecId::UNZEN) else {
            panic!("Unzen schedule")
        };
        match self {
            Self::Call => schedule.spawn_estimates.call,
            Self::CallCode => schedule.spawn_estimates.callcode,
            Self::DelegateCall => schedule.spawn_estimates.delegatecall,
            Self::StaticCall => schedule.spawn_estimates.staticcall,
            Self::Create => schedule.spawn_estimates.create,
            Self::Create2 => schedule.spawn_estimates.create2,
        }
    }
}

fn evm_env() -> EvmEnv<TaikoSpecId> {
    let mut env: EvmEnv<TaikoSpecId> = EvmEnv::default();
    env.cfg_env.spec = TaikoSpecId::UNZEN;
    env.cfg_env.chain_id = 167;
    env.block_env.gas_limit = 30_000_000;
    env
}

fn tx_env(gas_limit: u64) -> TxEnv {
    TxEnv::builder()
        .caller(CALLER)
        .kind(TxKind::Call(TARGET))
        .chain_id(Some(167))
        .gas_limit(gas_limit)
        .build()
        .expect("valid transaction environment")
}

fn context_tx_env(gas_limit: u64, calldata: Vec<u8>, value: U256) -> TxEnv {
    TxEnv::builder()
        .caller(CALLER)
        .kind(TxKind::Call(TARGET))
        .chain_id(Some(167))
        .gas_limit(gas_limit)
        .data(Bytes::from(calldata))
        .value(value)
        .build()
        .expect("valid context transaction environment")
}

fn storage_tx_env(gas_limit: u64, warm: bool) -> TxEnv {
    let mut builder = TxEnv::builder()
        .caller(CALLER)
        .kind(TxKind::Call(TARGET))
        .chain_id(Some(167))
        .gas_limit(gas_limit);
    if warm {
        builder = builder
            .tx_type(None)
            .access_list(AccessList(vec![AccessListItem {
                address: TARGET,
                storage_keys: vec![B256::ZERO],
            }]));
    }
    builder.build().expect("valid storage transaction")
}

fn db_with_contract(bytecode: Bytecode) -> InMemoryDB {
    let mut db = InMemoryDB::default();
    insert_contract(&mut db, TARGET, bytecode);
    insert_contract(
        &mut db,
        CHILD,
        Bytecode::new_raw(Bytes::from(vec![
            opcode::PUSH1,
            1,
            opcode::PUSH1,
            2,
            opcode::ADD,
            opcode::STOP,
        ])),
    );
    db.insert_account_info(
        CALLER,
        AccountInfo {
            balance: alloy_primitives::U256::MAX,
            ..Default::default()
        },
    );
    db
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

fn execute(bytecode: Bytecode, gas_limit: u64) -> (TraceSink, bool) {
    let sink = TraceSink::default();
    sink.lock().start_transaction(0);
    let mut evm = TaikoEvmFactory.create_evm_with_inspector(
        db_with_contract(bytecode),
        evm_env(),
        TraceInspector::new(sink.clone()),
    );
    let result = evm
        .transact(tx_env(gas_limit))
        .expect("transaction execution");
    sink.lock().finish_transaction();
    (sink, result.result.is_success())
}

fn execute_context(
    bytecode: Bytecode,
    calldata: Vec<u8>,
    value: U256,
    timestamp: u64,
) -> (TraceSink, bool) {
    let sink = TraceSink::default();
    sink.lock().start_transaction(0);
    let mut env = evm_env();
    env.block_env.timestamp = U256::from(timestamp);
    let mut evm = TaikoEvmFactory.create_evm_with_inspector(
        db_with_contract(bytecode),
        env,
        TraceInspector::new(sink.clone()),
    );
    let result = evm
        .transact(context_tx_env(1_000_000, calldata, value))
        .expect("context transaction execution");
    sink.lock().finish_transaction();
    (sink, result.result.is_success())
}

fn execute_storage(
    bytecode: Bytecode,
    gas_limit: u64,
    original: u64,
    warm: bool,
) -> (TraceSink, bool) {
    let sink = TraceSink::default();
    sink.lock().start_transaction(0);
    let mut db = db_with_contract(bytecode);
    db.insert_account_storage(TARGET, U256::ZERO, U256::from(original))
        .expect("insert target storage");
    let mut evm =
        TaikoEvmFactory.create_evm_with_inspector(db, evm_env(), TraceInspector::new(sink.clone()));
    let result = evm
        .transact(storage_tx_env(gas_limit, warm))
        .expect("storage transaction execution");
    sink.lock().finish_transaction();
    (sink, result.result.is_success())
}

fn last_storage_component(sink: &TraceSink, expected_opcode: u8) -> Value {
    let collector = sink.snapshot();
    let operation = collector
        .operations()
        .iter()
        .rev()
        .find(|operation| match &operation.component {
            OperationComponent::Opcode { opcode, .. }
            | OperationComponent::OpcodeFeatureError { opcode, .. } => *opcode == expected_opcode,
            OperationComponent::Precompile { .. } => false,
        })
        .unwrap_or_else(|| panic!("storage opcode 0x{expected_opcode:02x} operation"));
    serde_json::to_value(&operation.component).expect("serialize storage component")
}

fn sload_program() -> Bytecode {
    Bytecode::new_raw(Bytes::from(vec![
        opcode::PUSH0,
        opcode::SLOAD,
        opcode::POP,
        opcode::STOP,
    ]))
}

fn sstore_program(values: &[u8], tail: &[u8]) -> Bytecode {
    let mut code = Vec::with_capacity(values.len() * 4 + tail.len());
    for value in values {
        code.extend_from_slice(&[opcode::PUSH1, *value, opcode::PUSH0, opcode::SSTORE]);
    }
    code.extend_from_slice(tail);
    Bytecode::new_raw(Bytes::from(code))
}

fn executed_opcode_component(bytecode: Vec<u8>, expected_opcode: u8) -> Value {
    let (sink, success) = execute(Bytecode::new_raw(Bytes::from(bytecode)), 1_000_000);
    assert!(success, "opcode 0x{expected_opcode:02x} execution");
    let collector = sink.snapshot();
    let operation = collector
        .operations()
        .iter()
        .rev()
        .find(|operation| {
            matches!(
                operation.component,
                OperationComponent::Opcode { opcode, spawned: Some(false), .. }
                    if opcode == expected_opcode
            )
        })
        .unwrap_or_else(|| panic!("opcode 0x{expected_opcode:02x} operation"));
    serde_json::to_value(&operation.component).expect("serialize opcode component")
}

fn context_opcode_component(
    bytecode: Vec<u8>,
    expected_opcode: u8,
    calldata: Vec<u8>,
    value: U256,
    timestamp: u64,
) -> Value {
    let (sink, success) = execute_context(
        Bytecode::new_raw(Bytes::from(bytecode)),
        calldata,
        value,
        timestamp,
    );
    assert!(success, "opcode 0x{expected_opcode:02x} execution");
    let collector = sink.snapshot();
    let operation = collector
        .operations()
        .iter()
        .rev()
        .find(|operation| {
            matches!(
                operation.component,
                OperationComponent::Opcode { opcode, spawned: Some(false), .. }
                    if opcode == expected_opcode
            )
        })
        .unwrap_or_else(|| panic!("opcode 0x{expected_opcode:02x} operation"));
    serde_json::to_value(&operation.component).expect("serialize context opcode component")
}

#[test]
fn pending_spawn_is_promoted_only_by_dispatch_callback() {
    let mut collector = TraceCollector::default();
    collector.start_transaction(0);
    collector.record_opcode(
        0xf1,
        0,
        17,
        OpcodeModelInput::StaticRawGas { raw_gas: 17 },
        true,
    );

    assert_eq!(collector.operations().len(), 0, "spawn remains pending");
    collector.confirm_spawn(0);

    let operation = &collector.operations()[0];
    assert_eq!(operation.operation_id, 0);
    assert_eq!(operation.phase, OperationPhase::Transaction);
    assert_eq!(operation.tx_index, Some(0));
    assert_eq!(operation.frame_depth, 0);
    assert_eq!(
        operation.component,
        OperationComponent::Opcode {
            opcode: 0xf1,
            pricing_basis: Some(PricingBasis::FixedPerEvent),
            interpreter_raw_gas: None,
            model_input: None,
            spawned: Some(true),
            dispatch_status: DispatchStatus::Confirmed,
        }
    );
}

#[test]
fn rejected_spawn_is_explicitly_unmeasured_and_child_work_is_independent() {
    let mut collector = TraceCollector::default();
    collector.start_transaction(0);
    collector.record_opcode(
        0xf5,
        0,
        32_000,
        OpcodeModelInput::StaticRawGas { raw_gas: 32_000 },
        true,
    );
    collector.record_opcode(
        0x01,
        0,
        3,
        OpcodeModelInput::StaticRawGas { raw_gas: 3 },
        false,
    );

    assert_eq!(collector.operations().len(), 2);
    assert_eq!(
        collector.operations()[0].component,
        OperationComponent::Opcode {
            opcode: 0xf5,
            pricing_basis: None,
            interpreter_raw_gas: None,
            model_input: None,
            spawned: Some(true),
            dispatch_status: DispatchStatus::SelectedNotDispatched,
        }
    );
    assert_eq!(
        collector.operations()[1].component,
        OperationComponent::Opcode {
            opcode: 0x01,
            pricing_basis: Some(PricingBasis::RawGasSlope),
            interpreter_raw_gas: Some(3),
            model_input: Some(OpcodeModelInput::StaticRawGas { raw_gas: 3 }),
            spawned: Some(false),
            dispatch_status: DispatchStatus::NotApplicable,
        }
    );
}

#[test]
fn system_opcode_and_precompile_native_gas_have_monotonic_ids() {
    let mut collector = TraceCollector::default();
    collector.record_opcode(
        0x60,
        0,
        3,
        OpcodeModelInput::StaticRawGas { raw_gas: 3 },
        false,
    );
    collector.start_transaction(0);
    collector.record_precompile(
        address!("0000000000000000000000000000000000000001"),
        1,
        3_000,
    );

    assert_eq!(collector.operations()[0].operation_id, 0);
    assert_eq!(collector.operations()[0].phase, OperationPhase::System);
    assert_eq!(collector.operations()[1].operation_id, 1);
    assert_eq!(collector.operations()[1].phase, OperationPhase::Transaction);
    assert_eq!(
        collector.operations()[1].component,
        OperationComponent::Precompile {
            address: address!("0000000000000000000000000000000000000001"),
            pricing_basis: PricingBasis::RawGasSlope,
            native_gas: 3_000,
        }
    );
}

#[test]
fn real_inspector_covers_all_spawn_and_nonspawn_families() {
    for family in SpawnFamily::ALL {
        let (sink, success) = execute(family.spawned_bytecode(), 500_000);
        assert!(success, "{} spawned execution", family.name());
        let collector = sink.snapshot();
        let spawned = collector
            .operations()
            .iter()
            .find(|operation| {
                matches!(
                    operation.component,
                    OperationComponent::Opcode { opcode, spawned: Some(true), .. }
                        if opcode == family.opcode()
                )
            })
            .unwrap_or_else(|| panic!("{} spawned operation", family.name()));
        assert_eq!(
            spawned.component,
            OperationComponent::Opcode {
                opcode: family.opcode(),
                pricing_basis: Some(PricingBasis::FixedPerEvent),
                interpreter_raw_gas: None,
                model_input: None,
                spawned: Some(true),
                dispatch_status: DispatchStatus::Confirmed,
            },
            "{} confirmed wrapper",
            family.name()
        );

        let failing = Bytecode::new_raw(Bytes::from(vec![family.opcode()]));
        let (sink, success) = execute(failing, 100_000);
        assert!(!success, "{} malformed nonspawn execution", family.name());
        let collector = sink.snapshot();
        let nonspawn = collector
            .operations()
            .iter()
            .find(|operation| {
                matches!(
                    operation.component,
                    OperationComponent::Opcode { opcode, spawned: Some(false), .. }
                        if opcode == family.opcode()
                )
            })
            .unwrap_or_else(|| panic!("{} nonspawn operation", family.name()));
        assert!(
            matches!(
                nonspawn.component,
                OperationComponent::Opcode {
                    pricing_basis: Some(PricingBasis::RawGasSlope),
                    interpreter_raw_gas: Some(_),
                    dispatch_status: DispatchStatus::NotApplicable,
                    ..
                }
            ),
            "{} nonspawn raw gas",
            family.name()
        );
    }
}

#[test]
fn real_inspector_records_dynamic_opcode_and_precompile_separately() {
    let dynamic = Bytecode::new_raw(Bytes::from(vec![
        opcode::PUSH2,
        0x01,
        0x00,
        opcode::PUSH1,
        0,
        opcode::KECCAK256,
        opcode::STOP,
    ]));
    let (sink, success) = execute(dynamic, 100_000);
    assert!(success);
    let collector = sink.snapshot();
    assert!(collector.operations().iter().any(|operation| {
        matches!(
            operation.component,
            OperationComponent::Opcode {
                opcode: opcode::KECCAK256,
                interpreter_raw_gas: Some(raw_gas),
                spawned: Some(false),
                ..
            } if raw_gas > 30
        )
    }));

    let identity = Bytecode::new_raw(Bytes::from(vec![
        opcode::PUSH1,
        0,
        opcode::PUSH1,
        0,
        opcode::PUSH1,
        0,
        opcode::PUSH1,
        0,
        opcode::PUSH1,
        4,
        opcode::PUSH2,
        0xff,
        0xff,
        opcode::STATICCALL,
        opcode::STOP,
    ]));
    let (sink, success) = execute(identity, 100_000);
    assert!(success);
    let collector = sink.snapshot();
    let wrapper = collector
        .operations()
        .iter()
        .filter(|operation| {
            matches!(
                operation.component,
                OperationComponent::Opcode {
                    opcode: opcode::STATICCALL,
                    spawned: Some(true),
                    ..
                }
            )
        })
        .count();
    let native = collector
        .operations()
        .iter()
        .filter(|operation| matches!(operation.component, OperationComponent::Precompile { .. }))
        .count();
    assert_eq!(wrapper, 1, "one fixed wrapper");
    assert_eq!(native, 1, "one separate native precompile row");
}

#[test]
fn real_inspector_marks_limit_rejected_spawn_as_not_dispatched() {
    for family in SpawnFamily::ALL {
        let schedule = schedule_for(TaikoSpecId::UNZEN).expect("Unzen schedule");
        let spawn_charge = family.spawn_estimate()
            * u64::from(schedule.opcode_multipliers[usize::from(family.opcode())]);
        let sink = TraceSink::default();
        sink.lock().start_transaction(0);
        let mut evm = TaikoEvmFactory.create_evm_with_inspector(
            db_with_contract(family.spawned_bytecode()),
            evm_env(),
            TraceInspector::new(sink.clone()),
        );
        evm.reserve_block_zk_gas(schedule.block_limit - spawn_charge + 1)
            .expect("reservation fits");
        let error = evm
            .transact(tx_env(500_000))
            .expect_err("fixed wrapper must exceed remaining limit");
        sink.lock().finish_transaction();
        assert!(
            error.to_string().contains("zk gas limit exceeded"),
            "{}",
            family.name()
        );
        let collector = sink.snapshot();
        let rejected = collector
            .operations()
            .iter()
            .find(|operation| {
                matches!(
                    operation.component,
                    OperationComponent::Opcode { opcode, spawned: Some(true), .. }
                        if opcode == family.opcode()
                )
            })
            .unwrap_or_else(|| panic!("{} rejected wrapper", family.name()));
        assert_eq!(
            rejected.component,
            OperationComponent::Opcode {
                opcode: family.opcode(),
                pricing_basis: None,
                interpreter_raw_gas: None,
                model_input: None,
                spawned: Some(true),
                dispatch_status: DispatchStatus::SelectedNotDispatched,
            }
        );
        assert!(
            collector
                .operations()
                .iter()
                .all(|operation| operation.frame_depth <= rejected.frame_depth),
            "{} must not execute child work: {:?}",
            family.name(),
            collector.operations()
        );
    }
}

#[test]
fn static_opcode_emits_explicit_tagged_model_input() {
    let component = executed_opcode_component(
        vec![
            opcode::PUSH1,
            1,
            opcode::PUSH1,
            2,
            opcode::ADD,
            opcode::STOP,
        ],
        opcode::ADD,
    );

    assert_eq!(
        component["model_input"],
        json!({"kind": "static_raw_gas", "raw_gas": 3})
    );
}

#[test]
fn fixed_context_opcodes_emit_exact_typed_inputs() {
    for target in [opcode::ADDRESS, opcode::CALLER] {
        let component = context_opcode_component(
            vec![target, opcode::POP, opcode::STOP],
            target,
            Vec::new(),
            U256::ZERO,
            1_800_000_017,
        );
        assert_eq!(
            component["model_input"],
            json!({"kind": "context_fixed"}),
            "opcode 0x{target:02x}",
        );
    }
}

#[test]
fn context_values_use_current_call_value_and_actual_block_timestamp() {
    for (value, expected) in [(U256::ZERO, "zero"), (U256::from(7), "nonzero")] {
        let component = context_opcode_component(
            vec![opcode::CALLVALUE, opcode::POP, opcode::STOP],
            opcode::CALLVALUE,
            Vec::new(),
            value,
            1_800_000_017,
        );
        assert_eq!(
            component["model_input"],
            json!({"kind": "context_value", "value_class": expected}),
        );
    }

    let component = context_opcode_component(
        vec![opcode::TIMESTAMP, opcode::POP, opcode::STOP],
        opcode::TIMESTAMP,
        Vec::new(),
        U256::ZERO,
        1_800_000_017,
    );
    assert_eq!(
        component["model_input"],
        json!({"kind": "context_value", "value_class": "nonzero"}),
    );
}

#[test]
fn calldataload_classifies_zero_partial_full_and_oversized_offsets() {
    let cases = [
        ("empty", Vec::new(), vec![opcode::PUSH0], "zero"),
        ("out of range", vec![0; 4], vec![opcode::PUSH1, 64], "zero"),
        (
            "partial",
            (0..33).collect(),
            vec![opcode::PUSH1, 17],
            "partial",
        ),
        ("full", vec![0; 32], vec![opcode::PUSH0], "full"),
        (
            "larger than host index",
            vec![0; 32],
            {
                let mut push = vec![opcode::PUSH32];
                push.extend_from_slice(&[0xff; 32]);
                push
            },
            "zero",
        ),
    ];
    for (label, calldata, mut prefix, expected) in cases {
        prefix.extend_from_slice(&[opcode::CALLDATALOAD, opcode::POP, opcode::STOP]);
        let component = context_opcode_component(
            prefix,
            opcode::CALLDATALOAD,
            calldata,
            U256::ZERO,
            1_800_000_017,
        );
        assert_eq!(
            component["model_input"],
            json!({"kind": "calldata_load", "access_class": expected}),
            "{label}",
        );
    }
}

#[test]
fn calldatasize_emits_exact_current_frame_length_at_boundaries() {
    for input_length in [0, 31, 32, 33] {
        let component = context_opcode_component(
            vec![opcode::CALLDATASIZE, opcode::POP, opcode::STOP],
            opcode::CALLDATASIZE,
            vec![0; input_length],
            U256::ZERO,
            1_800_000_017,
        );
        assert_eq!(
            component["model_input"],
            json!({"kind": "calldata_size", "input_length": input_length}),
        );
    }
}

#[test]
fn nested_frame_uses_child_calldata_and_call_value() {
    let child = Bytecode::new_raw(Bytes::from(vec![
        opcode::CALLVALUE,
        opcode::POP,
        opcode::CALLDATASIZE,
        opcode::POP,
        opcode::PUSH0,
        opcode::CALLDATALOAD,
        opcode::POP,
        opcode::STOP,
    ]));
    let mut parent = vec![
        opcode::PUSH4,
        0xde,
        0xad,
        0xbe,
        0xef,
        opcode::PUSH1,
        28,
        opcode::MSTORE,
        opcode::PUSH0,
        opcode::PUSH0,
        opcode::PUSH1,
        4,
        opcode::PUSH1,
        28,
        opcode::PUSH1,
        7,
        opcode::PUSH20,
    ];
    parent.extend_from_slice(CHILD.as_slice());
    parent.extend_from_slice(&[
        opcode::PUSH2,
        0xff,
        0xff,
        opcode::CALL,
        opcode::POP,
        opcode::STOP,
    ]);

    let sink = TraceSink::default();
    sink.lock().start_transaction(0);
    let mut db = db_with_contract(Bytecode::new_raw(Bytes::from(parent)));
    insert_contract(&mut db, CHILD, child);
    let parent = db.cache.accounts.get_mut(&TARGET).expect("parent account");
    parent.info.balance = U256::from(7);
    let mut evm =
        TaikoEvmFactory.create_evm_with_inspector(db, evm_env(), TraceInspector::new(sink.clone()));
    let result = evm
        .transact(context_tx_env(1_000_000, Vec::new(), U256::ZERO))
        .expect("nested context transaction");
    sink.lock().finish_transaction();
    assert!(result.result.is_success());

    let snapshot = sink.snapshot();
    let child_inputs = snapshot
        .operations()
        .iter()
        .filter(|operation| operation.frame_depth > 0)
        .filter_map(|operation| match &operation.component {
            OperationComponent::Opcode {
                opcode,
                model_input: Some(model_input),
                ..
            } if matches!(
                *opcode,
                opcode::CALLVALUE | opcode::CALLDATASIZE | opcode::CALLDATALOAD
            ) =>
            {
                Some((*opcode, serde_json::to_value(model_input).unwrap()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        child_inputs,
        vec![
            (
                opcode::CALLVALUE,
                json!({"kind": "context_value", "value_class": "nonzero"}),
            ),
            (
                opcode::CALLDATASIZE,
                json!({"kind": "calldata_size", "input_length": 4}),
            ),
            (
                opcode::CALLDATALOAD,
                json!({"kind": "calldata_load", "access_class": "partial"}),
            ),
        ],
    );
}

#[test]
fn sload_emits_exact_cold_and_warm_access_inputs() {
    for (warm, expected_access) in [(false, "cold"), (true, "warm")] {
        let (sink, success) = execute_storage(sload_program(), 100_000, 7, warm);
        assert!(success);
        let component = last_storage_component(&sink, opcode::SLOAD);
        assert_eq!(
            component["model_input"],
            json!({"kind": "storage_load", "access": expected_access})
        );
    }
}

#[test]
fn sstore_emits_every_frozen_branch_from_original_current_and_new() {
    let cases: &[(&str, u64, &[u8])] = &[
        ("noop", 0, &[0]),
        ("set", 0, &[1]),
        ("clear", 1, &[0]),
        ("reset", 1, &[2]),
        ("dirty_rewrite", 0, &[1, 2]),
        ("restore_original", 0, &[1, 0]),
    ];
    for (expected_branch, original, values) in cases {
        let (sink, success) = execute_storage(
            sstore_program(values, &[opcode::STOP]),
            200_000,
            *original,
            true,
        );
        assert!(success, "{expected_branch}");
        let component = last_storage_component(&sink, opcode::SSTORE);
        assert_eq!(
            component["model_input"],
            json!({
                "kind": "storage_store",
                "access": "warm",
                "branch": expected_branch,
            }),
            "{expected_branch}",
        );
    }
}

#[test]
fn clean_sstore_preserves_cold_or_access_list_warm_classification() {
    for (warm, expected_access) in [(false, "cold"), (true, "warm")] {
        let (sink, success) =
            execute_storage(sstore_program(&[1], &[opcode::STOP]), 100_000, 0, warm);
        assert!(success);
        let component = last_storage_component(&sink, opcode::SSTORE);
        assert_eq!(
            component["model_input"],
            json!({
                "kind": "storage_store",
                "access": expected_access,
                "branch": "set",
            })
        );
    }
}

#[test]
fn stale_cached_slot_resets_transaction_original_to_present_value() {
    let sink = TraceSink::default();
    let bytecode = sstore_program(&[1], &[opcode::STOP]);
    let mut db = db_with_contract(bytecode);
    db.insert_account_storage(TARGET, U256::ZERO, U256::ZERO)
        .expect("insert target storage");
    let mut evm =
        TaikoEvmFactory.create_evm_with_inspector(db, evm_env(), TraceInspector::new(sink.clone()));

    sink.lock().start_transaction(0);
    let first = evm
        .transact_commit(storage_tx_env(100_000, false))
        .expect("first storage transaction");
    assert!(first.is_success());
    sink.lock().finish_transaction();

    sink.lock().start_transaction(1);
    let mut second_tx = storage_tx_env(100_000, false);
    second_tx.nonce = 1;
    let second = evm
        .transact_commit(second_tx)
        .expect("second storage transaction");
    assert!(second.is_success());
    sink.lock().finish_transaction();

    let collector = sink.snapshot();
    let component = collector
        .operations()
        .iter()
        .rev()
        .find(|operation| {
            operation.tx_index == Some(1)
                && matches!(
                    operation.component,
                    OperationComponent::Opcode {
                        opcode: opcode::SSTORE,
                        ..
                    }
                )
        })
        .expect("second transaction SSTORE");
    assert_eq!(
        serde_json::to_value(&component.component).unwrap()["model_input"],
        json!({"kind": "storage_store", "access": "cold", "branch": "noop"})
    );
}

#[test]
fn completed_sstore_is_retained_when_transaction_later_reverts() {
    let (sink, success) = execute_storage(
        sstore_program(&[1], &[opcode::PUSH0, opcode::PUSH0, opcode::REVERT]),
        100_000,
        0,
        false,
    );
    assert!(!success);
    assert_eq!(
        last_storage_component(&sink, opcode::SSTORE)["model_input"],
        json!({"kind": "storage_store", "access": "cold", "branch": "set"})
    );
}

#[test]
fn nested_call_and_delegatecall_use_the_active_frame_storage_owner() {
    for (family, owner) in [
        (SpawnFamily::Call, CHILD),
        (SpawnFamily::DelegateCall, TARGET),
    ] {
        let sink = TraceSink::default();
        sink.lock().start_transaction(0);
        let mut db = db_with_contract(family.spawned_bytecode());
        insert_contract(&mut db, CHILD, sstore_program(&[0], &[opcode::STOP]));
        db.insert_account_storage(owner, U256::ZERO, U256::from(1))
            .expect("insert active-frame storage");
        let other = if owner == TARGET { CHILD } else { TARGET };
        db.insert_account_storage(other, U256::ZERO, U256::ZERO)
            .expect("insert non-owner storage");
        let mut evm = TaikoEvmFactory.create_evm_with_inspector(
            db,
            evm_env(),
            TraceInspector::new(sink.clone()),
        );
        let result = evm.transact(tx_env(500_000)).expect("nested execution");
        sink.lock().finish_transaction();
        assert!(result.result.is_success(), "{}", family.name());
        assert_eq!(
            last_storage_component(&sink, opcode::SSTORE)["model_input"],
            json!({"kind": "storage_store", "access": "cold", "branch": "clear"}),
            "{} storage owner",
            family.name(),
        );
    }
}

#[test]
fn completed_sstore_is_retained_when_child_frame_reverts() {
    let sink = TraceSink::default();
    sink.lock().start_transaction(0);
    let mut db = db_with_contract(SpawnFamily::Call.spawned_bytecode());
    insert_contract(
        &mut db,
        CHILD,
        sstore_program(&[1], &[opcode::PUSH0, opcode::PUSH0, opcode::REVERT]),
    );
    db.insert_account_storage(CHILD, U256::ZERO, U256::ZERO)
        .expect("insert child storage");
    let mut evm =
        TaikoEvmFactory.create_evm_with_inspector(db, evm_env(), TraceInspector::new(sink.clone()));
    let result = evm.transact(tx_env(500_000)).expect("nested execution");
    sink.lock().finish_transaction();

    assert!(
        result.result.is_success(),
        "parent keeps the failed CALL result"
    );
    assert_eq!(
        last_storage_component(&sink, opcode::SSTORE)["model_input"],
        json!({"kind": "storage_store", "access": "cold", "branch": "set"})
    );
}

#[test]
fn static_context_sstore_rejection_fails_closed_before_typing() {
    let sink = TraceSink::default();
    sink.lock().start_transaction(0);
    let mut db = db_with_contract(SpawnFamily::StaticCall.spawned_bytecode());
    insert_contract(&mut db, CHILD, sstore_program(&[1], &[opcode::STOP]));
    db.insert_account_storage(CHILD, U256::ZERO, U256::ZERO)
        .expect("insert child storage");
    let mut evm =
        TaikoEvmFactory.create_evm_with_inspector(db, evm_env(), TraceInspector::new(sink.clone()));
    let result = evm
        .transact(tx_env(500_000))
        .expect("static child execution");
    sink.lock().finish_transaction();

    assert!(
        result.result.is_success(),
        "parent keeps the failed STATICCALL result"
    );
    let component = last_storage_component(&sink, opcode::SSTORE);
    assert_eq!(
        component["error"],
        json!({"kind": "storage_state", "reason": "instruction_halted"})
    );
    assert!(component.get("model_input").is_none());
}

#[test]
fn completed_sstore_is_retained_when_outer_zk_gas_limit_rejects_it() {
    let schedule = schedule_for(TaikoSpecId::UNZEN).expect("Unzen schedule");
    let raw_charge = |target: u8, raw_gas: u64| {
        raw_gas * u64::from(schedule.opcode_multipliers[usize::from(target)])
    };
    let prefix_charge = raw_charge(opcode::PUSH1, 3) + raw_charge(opcode::PUSH0, 2);
    let store_charge = raw_charge(opcode::SSTORE, 22_100);
    let admitted = prefix_charge + store_charge - 1;

    let sink = TraceSink::default();
    sink.lock().start_transaction(0);
    let bytecode = sstore_program(&[1], &[opcode::STOP]);
    let mut db = db_with_contract(bytecode);
    db.insert_account_storage(TARGET, U256::ZERO, U256::ZERO)
        .expect("insert target storage");
    let mut evm =
        TaikoEvmFactory.create_evm_with_inspector(db, evm_env(), TraceInspector::new(sink.clone()));
    evm.reserve_block_zk_gas(schedule.block_limit - admitted)
        .expect("reservation fits");
    let error = evm
        .transact(storage_tx_env(100_000, false))
        .expect_err("SSTORE zkGas charge must exceed the remaining block budget");
    sink.lock().finish_transaction();

    assert!(error.to_string().contains("zk gas limit exceeded"));
    assert_eq!(
        last_storage_component(&sink, opcode::SSTORE)["model_input"],
        json!({"kind": "storage_store", "access": "cold", "branch": "set"})
    );
}

#[test]
fn dirty_noop_fails_closed_as_storage_feature_error() {
    let (sink, success) =
        execute_storage(sstore_program(&[1, 1], &[opcode::STOP]), 100_000, 0, false);
    assert!(success);
    assert_eq!(
        last_storage_component(&sink, opcode::SSTORE),
        json!({
            "kind": "opcode_feature_error",
            "opcode": opcode::SSTORE,
            "interpreter_raw_gas": 100,
            "error": {"kind": "storage_state", "reason": "dirty_noop"},
        })
    );
}

#[test]
fn storage_instruction_halts_fail_closed_before_typing() {
    let (sink, success) = execute(Bytecode::new_raw(Bytes::from(vec![opcode::SLOAD])), 100_000);
    assert!(!success);
    let component = last_storage_component(&sink, opcode::SLOAD);
    assert_eq!(
        component["error"],
        json!({"kind": "stack_underflow", "feature": "storage_key"})
    );
    assert!(component.get("model_input").is_none());
    let roundtrip: OperationComponent =
        serde_json::from_value(component.clone()).expect("SLOAD stack error roundtrip");
    assert_eq!(serde_json::to_value(roundtrip).unwrap(), component);

    for (gas_limit, stage) in [(23_100, "stipend"), (24_000, "dynamic charge")] {
        let (sink, success) =
            execute_storage(sstore_program(&[1], &[opcode::STOP]), gas_limit, 0, false);
        assert!(!success, "SSTORE {stage} rejection");
        let component = last_storage_component(&sink, opcode::SSTORE);
        assert_eq!(
            component["error"],
            json!({"kind": "storage_state", "reason": "instruction_halted"}),
            "{stage}",
        );
        assert!(component.get("model_input").is_none(), "{stage}");
    }
}

#[test]
fn storage_stack_feature_error_schema_is_exact() {
    for valid in [
        json!({
            "kind": "opcode_feature_error",
            "opcode": opcode::SLOAD,
            "interpreter_raw_gas": 0,
            "error": {"kind": "stack_underflow", "feature": "storage_key"},
        }),
        json!({
            "kind": "opcode_feature_error",
            "opcode": opcode::SSTORE,
            "interpreter_raw_gas": 0,
            "error": {"kind": "stack_underflow", "feature": "new_storage_value"},
        }),
    ] {
        let roundtrip: OperationComponent =
            serde_json::from_value(valid.clone()).expect("valid storage stack error");
        assert_eq!(serde_json::to_value(roundtrip).unwrap(), valid);
    }

    for wrong_feature in ["new_storage_value", "raw_gas"] {
        let malformed = json!({
            "kind": "opcode_feature_error",
            "opcode": opcode::SLOAD,
            "interpreter_raw_gas": 0,
            "error": {"kind": "stack_underflow", "feature": wrong_feature},
        });
        assert!(serde_json::from_value::<OperationComponent>(malformed).is_err());
    }

    let malformed = json!({
        "kind": "opcode_feature_error",
        "opcode": opcode::SLOAD,
        "interpreter_raw_gas": 0,
        "error": {"kind": "storage_state", "reason": "dirty_noop"},
    });
    assert!(serde_json::from_value::<OperationComponent>(malformed).is_err());
}

#[test]
fn storage_model_input_schema_is_exact_and_rejects_dirty_cold() {
    for valid in [
        json!({
            "kind": "opcode",
            "opcode": opcode::SLOAD,
            "pricing_basis": "raw_gas_slope",
            "interpreter_raw_gas": 2100,
            "model_input": {"kind": "storage_load", "access": "cold"},
            "spawned": false,
            "dispatch_status": "not_applicable",
        }),
        json!({
            "kind": "opcode",
            "opcode": opcode::SSTORE,
            "pricing_basis": "raw_gas_slope",
            "interpreter_raw_gas": 100,
            "model_input": {
                "kind": "storage_store",
                "access": "warm",
                "branch": "dirty_rewrite",
            },
            "spawned": false,
            "dispatch_status": "not_applicable",
        }),
    ] {
        let roundtrip: OperationComponent =
            serde_json::from_value(valid.clone()).expect("valid typed storage input");
        assert_eq!(serde_json::to_value(roundtrip).unwrap(), valid);
    }

    for malformed in [
        json!({
            "kind": "opcode",
            "opcode": opcode::SLOAD,
            "pricing_basis": "raw_gas_slope",
            "interpreter_raw_gas": 2100,
            "model_input": {"kind": "static_raw_gas", "raw_gas": 2100},
            "spawned": false,
            "dispatch_status": "not_applicable",
        }),
        json!({
            "kind": "opcode",
            "opcode": opcode::SSTORE,
            "pricing_basis": "raw_gas_slope",
            "interpreter_raw_gas": 2200,
            "model_input": {
                "kind": "storage_store",
                "access": "cold",
                "branch": "dirty_rewrite",
            },
            "spawned": false,
            "dispatch_status": "not_applicable",
        }),
        json!({
            "kind": "opcode",
            "opcode": opcode::SSTORE,
            "pricing_basis": "raw_gas_slope",
            "interpreter_raw_gas": 2200,
            "model_input": {
                "kind": "storage_store",
                "access": "cold",
                "branch": "restore_original",
            },
            "spawned": false,
            "dispatch_status": "not_applicable",
        }),
    ] {
        assert!(
            serde_json::from_value::<OperationComponent>(malformed).is_err(),
            "invalid typed storage input must fail closed",
        );
    }
}

#[test]
fn undefined_opcode_emits_explicit_invalid_model_input() {
    let (sink, success) = execute(Bytecode::new_raw(Bytes::from(vec![0x0c])), 100_000);
    assert!(!success);
    let collector = sink.snapshot();
    let operation = collector
        .operations()
        .iter()
        .find(|operation| {
            matches!(
                operation.component,
                OperationComponent::Opcode { opcode: 0x0c, .. }
            )
        })
        .expect("undefined opcode operation");
    let component = serde_json::to_value(&operation.component).unwrap();
    assert_eq!(component["model_input"], json!({"kind": "invalid"}));
}

#[test]
fn malformed_structured_opcode_emits_feature_error_instead_of_static_fallback() {
    let (sink, success) = execute(Bytecode::new_raw(Bytes::from(vec![opcode::EXP])), 100_000);
    assert!(!success);
    let collector = sink.snapshot();
    let component = collector
        .operations()
        .iter()
        .find(|operation| {
            matches!(
                operation.component,
                OperationComponent::OpcodeFeatureError {
                    opcode: opcode::EXP,
                    ..
                }
            )
        })
        .expect("EXP feature error");
    assert_eq!(
        serde_json::to_value(&component.component).unwrap(),
        json!({
            "kind": "opcode_feature_error",
            "opcode": opcode::EXP,
            "interpreter_raw_gas": 10,
            "error": {
                "kind": "stack_underflow",
                "feature": "exponent_byte_length",
            },
        })
    );
    assert!(collector.operations().iter().all(|operation| {
        !matches!(
            operation.component,
            OperationComponent::Opcode {
                opcode: opcode::EXP,
                model_input: Some(OpcodeModelInput::StaticRawGas { .. }),
                ..
            }
        )
    }));
}

#[test]
fn exp_captures_exponent_byte_length_from_the_pre_step_stack() {
    let component = executed_opcode_component(
        vec![
            opcode::PUSH5,
            1,
            0,
            0,
            0,
            0,
            opcode::PUSH1,
            2,
            opcode::EXP,
            opcode::STOP,
        ],
        opcode::EXP,
    );

    assert_eq!(
        component["model_input"],
        json!({"kind": "exp", "exponent_byte_length": 5})
    );
}

#[test]
fn keccak_captures_input_length_and_exact_memory_growth_features() {
    let component = executed_opcode_component(
        vec![
            opcode::PUSH1,
            137,
            opcode::PUSH2,
            0x10,
            0x00,
            opcode::KECCAK256,
            opcode::STOP,
        ],
        opcode::KECCAK256,
    );

    assert_eq!(
        component["model_input"],
        json!({
            "kind": "keccak",
            "input_length": 137,
            "memory_growth_event": 1,
            "memory_evm_gas_delta": 433,
            "memory_4k_boundary_event": 1,
        })
    );
}

#[test]
fn all_memory_access_opcodes_emit_the_shared_memory_input() {
    let cases = [
        (
            opcode::MLOAD,
            vec![opcode::PUSH2, 0x10, 0x00, opcode::MLOAD, opcode::STOP],
        ),
        (
            opcode::MSTORE,
            vec![
                opcode::PUSH1,
                1,
                opcode::PUSH2,
                0x10,
                0x00,
                opcode::MSTORE,
                opcode::STOP,
            ],
        ),
        (
            opcode::MSTORE8,
            vec![
                opcode::PUSH1,
                1,
                opcode::PUSH2,
                0x10,
                0x00,
                opcode::MSTORE8,
                opcode::STOP,
            ],
        ),
    ];

    for (target, bytecode) in cases {
        let component = executed_opcode_component(bytecode, target);
        assert_eq!(
            component["model_input"],
            json!({
                "kind": "memory_access",
                "memory_growth_event": 1,
                "memory_evm_gas_delta": 419,
                "memory_4k_boundary_event": 1,
            }),
            "opcode 0x{target:02x}",
        );
    }
}

#[test]
fn memory_access_after_prior_expansion_emits_zero_growth_for_that_event() {
    let component = executed_opcode_component(
        vec![
            opcode::PUSH1,
            1,
            opcode::PUSH2,
            0x10,
            0x00,
            opcode::MSTORE,
            opcode::PUSH2,
            0x10,
            0x00,
            opcode::MLOAD,
            opcode::STOP,
        ],
        opcode::MLOAD,
    );

    assert_eq!(
        component["model_input"],
        json!({
            "kind": "memory_access",
            "memory_growth_event": 0,
            "memory_evm_gas_delta": 0,
            "memory_4k_boundary_event": 0,
        })
    );
}

#[test]
fn mcopy_captures_copy_words_and_destination_driven_memory_growth() {
    let component = executed_opcode_component(
        vec![
            opcode::PUSH1,
            33,
            opcode::PUSH1,
            0,
            opcode::PUSH2,
            0x10,
            0x00,
            opcode::MCOPY,
            opcode::STOP,
        ],
        opcode::MCOPY,
    );

    assert_eq!(
        component["model_input"],
        json!({
            "kind": "memory_copy",
            "copy_words": 2,
            "memory_growth_event": 1,
            "memory_evm_gas_delta": 423,
            "memory_4k_boundary_event": 1,
        })
    );
}

#[test]
fn opcode_model_input_schema_rejects_missing_extra_and_wrong_fields() {
    let valid = json!({
        "kind": "opcode",
        "opcode": opcode::ADD,
        "pricing_basis": "raw_gas_slope",
        "interpreter_raw_gas": 3,
        "model_input": {"kind": "static_raw_gas", "raw_gas": 3},
        "spawned": false,
        "dispatch_status": "not_applicable",
    });
    let roundtrip: OperationComponent =
        serde_json::from_value(valid.clone()).expect("valid typed opcode component");
    assert_eq!(serde_json::to_value(roundtrip).unwrap(), valid);

    for (label, malformed) in [
        (
            "missing",
            json!({
                "kind": "opcode",
                "opcode": opcode::ADD,
                "pricing_basis": "raw_gas_slope",
                "interpreter_raw_gas": 3,
                "spawned": false,
                "dispatch_status": "not_applicable",
            }),
        ),
        (
            "extra",
            json!({
                "kind": "opcode",
                "opcode": opcode::ADD,
                "pricing_basis": "raw_gas_slope",
                "interpreter_raw_gas": 3,
                "model_input": {
                    "kind": "static_raw_gas",
                    "raw_gas": 3,
                    "input_length": 0,
                },
                "spawned": false,
                "dispatch_status": "not_applicable",
            }),
        ),
        (
            "wrong model",
            json!({
                "kind": "opcode",
                "opcode": opcode::EXP,
                "pricing_basis": "raw_gas_slope",
                "interpreter_raw_gas": 60,
                "model_input": {"kind": "static_raw_gas", "raw_gas": 60},
                "spawned": false,
                "dispatch_status": "not_applicable",
            }),
        ),
        (
            "feature error on static opcode",
            json!({
                "kind": "opcode_feature_error",
                "opcode": opcode::ADD,
                "interpreter_raw_gas": 3,
                "error": {
                    "kind": "stack_underflow",
                    "feature": "raw_gas",
                },
            }),
        ),
    ] {
        assert!(
            serde_json::from_value::<OperationComponent>(malformed).is_err(),
            "{label} model input must fail closed",
        );
    }
}

#[test]
fn context_model_input_schema_is_exact_and_opcode_specific() {
    let valid = [
        (opcode::ADDRESS, json!({"kind": "context_fixed"})),
        (opcode::CALLER, json!({"kind": "context_fixed"})),
        (
            opcode::CALLVALUE,
            json!({"kind": "context_value", "value_class": "zero"}),
        ),
        (
            opcode::CALLDATALOAD,
            json!({"kind": "calldata_load", "access_class": "partial"}),
        ),
        (
            opcode::CALLDATASIZE,
            json!({"kind": "calldata_size", "input_length": 33}),
        ),
        (
            opcode::TIMESTAMP,
            json!({"kind": "context_value", "value_class": "nonzero"}),
        ),
    ];
    for (target, model_input) in valid {
        let component = json!({
            "kind": "opcode",
            "opcode": target,
            "pricing_basis": "raw_gas_slope",
            "interpreter_raw_gas": 2,
            "model_input": model_input,
            "spawned": false,
            "dispatch_status": "not_applicable",
        });
        let roundtrip: OperationComponent = serde_json::from_value(component.clone())
            .unwrap_or_else(|error| panic!("opcode 0x{target:02x}: {error}"));
        assert_eq!(serde_json::to_value(roundtrip).unwrap(), component);
    }

    let malformed = [
        (
            opcode::ADDRESS,
            json!({"kind": "context_value", "value_class": "zero"}),
        ),
        (
            opcode::CALLER,
            json!({"kind": "static_raw_gas", "raw_gas": 2}),
        ),
        (opcode::CALLVALUE, json!({"kind": "context_fixed"})),
        (
            opcode::CALLVALUE,
            json!({"kind": "context_value", "value_class": "positive"}),
        ),
        (
            opcode::CALLDATALOAD,
            json!({"kind": "calldata_size", "input_length": 0}),
        ),
        (
            opcode::CALLDATALOAD,
            json!({"kind": "calldata_load", "access_class": "empty"}),
        ),
        (
            opcode::CALLDATASIZE,
            json!({"kind": "calldata_load", "access_class": "zero"}),
        ),
        (opcode::TIMESTAMP, json!({"kind": "context_fixed"})),
        (opcode::ADD, json!({"kind": "context_fixed"})),
        (
            opcode::ADDRESS,
            json!({"kind": "context_fixed", "value_class": "zero"}),
        ),
    ];
    for (target, model_input) in malformed {
        let component = json!({
            "kind": "opcode",
            "opcode": target,
            "pricing_basis": "raw_gas_slope",
            "interpreter_raw_gas": 2,
            "model_input": model_input,
            "spawned": false,
            "dispatch_status": "not_applicable",
        });
        assert!(
            serde_json::from_value::<OperationComponent>(component).is_err(),
            "opcode 0x{target:02x} must reject incompatible context input",
        );
    }
}

#[test]
fn exp_model_input_schema_rejects_exponents_larger_than_a_word() {
    let valid = json!({
        "kind": "opcode",
        "opcode": opcode::EXP,
        "pricing_basis": "raw_gas_slope",
        "interpreter_raw_gas": 660,
        "model_input": {"kind": "exp", "exponent_byte_length": 32},
        "spawned": false,
        "dispatch_status": "not_applicable",
    });
    let roundtrip: OperationComponent =
        serde_json::from_value(valid.clone()).expect("32-byte exponent");
    assert_eq!(serde_json::to_value(roundtrip).unwrap(), valid);

    let mut oversized = valid;
    oversized["model_input"]["exponent_byte_length"] = json!(33);
    assert!(
        serde_json::from_value::<OperationComponent>(oversized).is_err(),
        "EVM exponent must fit in a 32-byte word",
    );
}

#[test]
fn spawn_wrapper_schema_accepts_only_spawn_family_opcodes() {
    for family in SpawnFamily::ALL {
        for (pricing_basis, dispatch_status) in [
            (json!("fixed_per_event"), "confirmed"),
            (Value::Null, "selected_not_dispatched"),
        ] {
            let valid = json!({
                "kind": "opcode",
                "opcode": family.opcode(),
                "pricing_basis": pricing_basis,
                "spawned": true,
                "dispatch_status": dispatch_status,
            });
            let roundtrip: OperationComponent = serde_json::from_value(valid.clone())
                .unwrap_or_else(|error| panic!("{} {dispatch_status}: {error}", family.name()));
            let mut expected = valid;
            if expected["pricing_basis"].is_null() {
                expected.as_object_mut().unwrap().remove("pricing_basis");
            }
            assert_eq!(serde_json::to_value(roundtrip).unwrap(), expected);
        }
    }

    for (pricing_basis, dispatch_status) in [
        (json!("fixed_per_event"), "confirmed"),
        (Value::Null, "selected_not_dispatched"),
    ] {
        let malformed = json!({
            "kind": "opcode",
            "opcode": opcode::ADD,
            "pricing_basis": pricing_basis,
            "spawned": true,
            "dispatch_status": dispatch_status,
        });
        assert!(
            serde_json::from_value::<OperationComponent>(malformed).is_err(),
            "{dispatch_status} wrapper must reject a non-spawn opcode",
        );
    }
}

#[test]
fn confirmed_spawn_substitution_omits_model_input() {
    let (sink, success) = execute(SpawnFamily::Call.spawned_bytecode(), 500_000);
    assert!(success);
    let collector = sink.snapshot();
    let wrapper = collector
        .operations()
        .iter()
        .find(|operation| {
            matches!(
                operation.component,
                OperationComponent::Opcode {
                    opcode: opcode::CALL,
                    spawned: Some(true),
                    dispatch_status: DispatchStatus::Confirmed,
                    ..
                }
            )
        })
        .expect("confirmed CALL wrapper");
    let serialized = serde_json::to_value(&wrapper.component).unwrap();
    assert!(serialized.get("model_input").is_none());
    assert!(serialized.get("interpreter_raw_gas").is_none());
}
