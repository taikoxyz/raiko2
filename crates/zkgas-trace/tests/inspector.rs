#![allow(missing_docs)]

use alethia_reth_evm::{
    alloy::TaikoZkGasEvm, factory::TaikoEvmFactory, spec::TaikoSpecId,
    zk_gas::schedule::schedule_for,
};
use alloy_evm::{Evm, EvmEnv, EvmFactory};
use alloy_primitives::{Address, address};
use raiko2_zkgas_trace::{
    DispatchStatus, OperationComponent, OperationPhase, PricingBasis, TraceCollector,
    TraceInspector, TraceSink,
};
use reth_revm::{
    context::TxEnv,
    db::InMemoryDB,
    primitives::{Bytes, TxKind},
    state::{AccountInfo, Bytecode, bytecode::opcode},
};

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

#[test]
fn pending_spawn_is_promoted_only_by_dispatch_callback() {
    let mut collector = TraceCollector::default();
    collector.start_transaction(0);
    collector.record_opcode(0xf1, 0, 17, true);

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
            spawned: Some(true),
            dispatch_status: DispatchStatus::Confirmed,
        }
    );
}

#[test]
fn rejected_spawn_is_explicitly_unmeasured_and_child_work_is_independent() {
    let mut collector = TraceCollector::default();
    collector.start_transaction(0);
    collector.record_opcode(0xf5, 0, 32_000, true);
    collector.record_opcode(0x01, 0, 3, false);

    assert_eq!(collector.operations().len(), 2);
    assert_eq!(
        collector.operations()[0].component,
        OperationComponent::Opcode {
            opcode: 0xf5,
            pricing_basis: None,
            interpreter_raw_gas: None,
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
            spawned: Some(false),
            dispatch_status: DispatchStatus::NotApplicable,
        }
    );
}

#[test]
fn system_opcode_and_precompile_native_gas_have_monotonic_ids() {
    let mut collector = TraceCollector::default();
    collector.record_opcode(0x60, 0, 3, false);
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
