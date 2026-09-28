//! Shared REVM state construction for the controlled opcode laboratory.

#![no_std]

extern crate alloc;

use alloc::vec;
use raiko2_primitives::{OpcodeLabStorageAccess, OpcodeLabStorageInput};
use revm::{
    bytecode::Bytecode,
    context::{
        TxEnv,
        transaction::{AccessList, AccessListItem},
        tx::TxEnvBuildError,
    },
    database::{
        BENCH_CALLER, BENCH_CALLER_BALANCE, BENCH_TARGET, BENCH_TARGET_BALANCE, InMemoryDB,
    },
    primitives::{B256, KECCAK_EMPTY, U256, hardfork::SpecId},
    state::AccountInfo,
};

/// Hardfork used by every stateful opcode-laboratory execution.
pub const OPCODE_LAB_SPEC_ID: SpecId = SpecId::OSAKA;

/// Builds the canonical benchmark database for one fixed microprogram.
#[must_use]
pub fn build_benchmark_db(
    bytecode: Bytecode,
    storage: Option<&OpcodeLabStorageInput>,
) -> InMemoryDB {
    let mut db = InMemoryDB::default();
    db.insert_account_info(
        BENCH_TARGET,
        AccountInfo {
            nonce: 1,
            balance: BENCH_TARGET_BALANCE,
            code_hash: bytecode.hash_slow(),
            code: Some(bytecode),
            ..Default::default()
        },
    );
    db.insert_account_info(
        BENCH_CALLER,
        AccountInfo {
            balance: BENCH_CALLER_BALANCE,
            code: None,
            code_hash: KECCAK_EMPTY,
            ..Default::default()
        },
    );
    if let Some(storage) = storage {
        let result = db.insert_account_storage(
            BENCH_TARGET,
            U256::from_be_bytes(storage.slot),
            U256::from_be_bytes(storage.original_value),
        );
        match result {
            Ok(()) => {}
            Err(error) => match error {},
        }
    }
    db
}

/// Builds the canonical benchmark transaction for one fixed microprogram.
///
/// # Errors
///
/// Returns [`TxEnvBuildError`] if REVM rejects the benchmark transaction envelope.
pub fn build_benchmark_tx(
    gas_limit: u64,
    storage: Option<&OpcodeLabStorageInput>,
) -> Result<TxEnv, TxEnvBuildError> {
    let mut builder = TxEnv::builder_for_bench().gas_limit(gas_limit);
    if let Some(storage) = storage
        && storage.access == OpcodeLabStorageAccess::Warm
    {
        builder = builder
            .tx_type(None)
            .access_list(AccessList(vec![AccessListItem {
                address: BENCH_TARGET,
                storage_keys: vec![B256::from(storage.slot)],
            }]));
    }
    builder.build()
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::{OPCODE_LAB_SPEC_ID, build_benchmark_db, build_benchmark_tx};
    use raiko2_primitives::{
        OpcodeLabStorageAccess, OpcodeLabStorageInput, OpcodeLabStorageLane,
        OpcodeLabStorageOperation,
    };
    use revm::{
        Context, Database, ExecuteEvm, MainBuilder, MainContext,
        bytecode::Bytecode,
        context::{
            TxEnv,
            transaction::{AccessList, AccessListItem},
        },
        database::{BENCH_CALLER, BENCH_TARGET},
        primitives::{B256, U256, hardfork::SpecId},
    };
    use std::vec;

    const SLOT: [u8; 32] = [0x11; 32];

    fn storage(original_value: [u8; 32], access: OpcodeLabStorageAccess) -> OpcodeLabStorageInput {
        OpcodeLabStorageInput {
            measurement_opcode: 0x54,
            lane: OpcodeLabStorageLane::Target,
            slot: SLOT,
            original_value,
            access,
            operation: OpcodeLabStorageOperation::Load {
                expected_value: original_value,
            },
        }
    }

    #[test]
    fn opcode_lab_uses_osaka() {
        assert_eq!(OPCODE_LAB_SPEC_ID, SpecId::OSAKA);
    }

    #[test]
    fn benchmark_db_preserves_stateless_benchmark_accounts_and_bytecode() {
        let bytecode = Bytecode::new_legacy([0x60, 0x01, 0x00].as_slice().into());
        let mut db = build_benchmark_db(bytecode.clone(), None);

        let target = db
            .basic(BENCH_TARGET)
            .expect("infallible benchmark database")
            .expect("benchmark target exists");
        let caller = db
            .basic(BENCH_CALLER)
            .expect("infallible benchmark database")
            .expect("benchmark caller exists");

        assert_eq!(target.nonce, 1);
        assert_eq!(target.code, Some(bytecode));
        assert_eq!(caller.nonce, 0);
        assert!(caller.code.is_none());
    }

    #[test]
    fn benchmark_db_inserts_zero_and_nonzero_declared_storage() {
        let slot = U256::from_be_bytes(SLOT);
        for original_value in [[0u8; 32], [0x22; 32]] {
            let storage = storage(original_value, OpcodeLabStorageAccess::Cold);
            let db = build_benchmark_db(Bytecode::default(), Some(&storage));
            let target = db
                .cache
                .accounts
                .get(&BENCH_TARGET)
                .expect("benchmark target is cached");

            assert_eq!(
                target.storage.get(&slot),
                Some(&U256::from_be_bytes(original_value))
            );
        }
    }

    #[test]
    fn cold_and_stateless_transactions_have_empty_access_lists() {
        let cold = storage([0x22; 32], OpcodeLabStorageAccess::Cold);

        for storage in [None, Some(&cold)] {
            let tx = build_benchmark_tx(123_456, storage).expect("valid benchmark transaction");
            assert_eq!(tx.gas_limit, 123_456);
            assert!(tx.access_list.0.is_empty());
        }
    }

    #[test]
    fn warm_transaction_has_exact_target_slot_access_list() {
        let warm = storage([0x22; 32], OpcodeLabStorageAccess::Warm);

        let tx = build_benchmark_tx(123_456, Some(&warm)).expect("valid benchmark transaction");

        assert_eq!(
            tx.access_list,
            AccessList(vec![AccessListItem {
                address: BENCH_TARGET,
                storage_keys: vec![B256::from(SLOT)],
            }])
        );
        assert_eq!(
            tx.tx_type, 1,
            "warm access-list transaction must be EIP-2930"
        );
    }

    #[test]
    fn warm_access_list_changes_real_sload_execution_gas() {
        let mut program = vec![0x7f];
        program.extend_from_slice(&SLOT);
        program.extend_from_slice(&[0x54, 0x50, 0x00]);
        let bytecode = Bytecode::new_legacy(program.into());
        let cold = storage([0x22; 32], OpcodeLabStorageAccess::Cold);
        let warm = storage([0x22; 32], OpcodeLabStorageAccess::Warm);
        let execute = |storage: &OpcodeLabStorageInput| {
            let ctx = Context::mainnet()
                .modify_cfg_chained(|cfg| cfg.set_spec_and_mainnet_gas_params(OPCODE_LAB_SPEC_ID))
                .with_db(build_benchmark_db(bytecode.clone(), Some(storage)));
            ctx.build_mainnet()
                .transact(build_benchmark_tx(100_000, Some(storage)).expect("valid tx"))
                .expect("SLOAD execution")
                .result
                .tx_gas_used()
        };

        let cold_gas = execute(&cold);
        let warm_gas = execute(&warm);

        assert_eq!(cold_gas, 23_105);
        assert_eq!(warm_gas, 25_405);
    }

    #[test]
    fn stateless_transaction_preserves_benchmark_envelope() {
        let actual = build_benchmark_tx(123_456, None).expect("valid benchmark transaction");
        let expected = TxEnv::builder_for_bench()
            .gas_limit(123_456)
            .build()
            .expect("valid benchmark transaction");

        assert_eq!(actual, expected);
    }
}
