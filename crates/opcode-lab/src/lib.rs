//! Shared REVM state construction for the controlled opcode laboratory.

#![no_std]

extern crate alloc;

use alloc::{vec, vec::Vec};
use alloy_primitives::keccak256;
use raiko2_primitives::{
    ContextOpcodeLabInputV1, OpcodeLabInput, OpcodeLabStorageAccess, OpcodeLabStorageInput,
};
use revm::{
    bytecode::Bytecode,
    context::{
        BlockEnv, TxEnv,
        transaction::{AccessList, AccessListItem},
        tx::TxEnvBuildError,
    },
    context_interface::result::ExecutionResult,
    database::{
        BENCH_CALLER, BENCH_CALLER_BALANCE, BENCH_TARGET, BENCH_TARGET_BALANCE, InMemoryDB,
    },
    primitives::{B256, Bytes, KECCAK_EMPTY, U256, hardfork::SpecId},
    state::AccountInfo,
};

/// Hardfork used by every stateful opcode-laboratory execution.
pub const OPCODE_LAB_SPEC_ID: SpecId = SpecId::OSAKA;

const ANCHOR_TARGET_SCENARIO: &str = "anchor_target_";
const ANCHOR_CONTROL_SCENARIO: &str = "anchor_control";
const ANCHOR_MAX_COUNT: u64 = 131_072;

fn opcode_anchor_lane(input: &OpcodeLabInput) -> Option<u8> {
    let expected_case = match input.opcode {
        0x50 => "synthetic_anchor_probe_pop",
        0x5f => "synthetic_anchor_probe_push0",
        0x80 => "synthetic_anchor_probe_dup1",
        0x90 => "synthetic_anchor_probe_swap1",
        _ => return None,
    };
    if input.case != expected_case {
        return None;
    }
    let lane = match input.scenario.as_str() {
        ANCHOR_TARGET_SCENARIO => 0,
        ANCHOR_CONTROL_SCENARIO => 1,
        _ => return None,
    };
    let expected_raw_gas = match input.opcode {
        0x50 | 0x5f => 2,
        0x80 | 0x90 => 3,
        _ => return None,
    };
    if input.target_raw_gas != expected_raw_gas
        || input.target_count > ANCHOR_MAX_COUNT
        || input.tx_gas_limit != Some(100_000)
        || input.generator_max_count != Some(ANCHOR_MAX_COUNT)
        || input.fixed_bytecode_len != Some(1)
        || input.bytecode != [0x00]
    {
        return None;
    }
    Some(lane)
}

const fn mix_opcode_anchor(accumulator: u64, value: u64) -> u64 {
    accumulator
        .rotate_left(13)
        .wrapping_mul(0x9e37_79b1_85eb_ca87)
        .wrapping_add(value)
}

/// Computes the deterministic accumulator used by the synthetic anchor guest.
#[must_use]
pub fn opcode_anchor_accumulator(input: &OpcodeLabInput) -> Option<u64> {
    opcode_anchor_lane(input)?;
    let mut accumulator = 0x243f_6a88_85a3_08d3 ^ u64::from(input.opcode);
    for iteration in 0..input.target_count {
        accumulator = mix_opcode_anchor(accumulator, iteration);
        accumulator = mix_opcode_anchor(accumulator, iteration.rotate_left(17));
    }
    Some(accumulator)
}

/// Builds the exact preimage committed by the synthetic anchor guest.
#[must_use]
pub fn opcode_anchor_public_input(input: &OpcodeLabInput, accumulator: u64) -> Option<[u8; 28]> {
    let lane = opcode_anchor_lane(input)?;
    let mut output = [0u8; 28];
    output[0] = 1;
    output[1] = input.opcode;
    output[2] = lane;
    output[4..12].copy_from_slice(&input.target_count.to_le_bytes());
    output[12..20].copy_from_slice(&input.target_raw_gas.to_le_bytes());
    output[20..28].copy_from_slice(&accumulator.to_le_bytes());
    Some(output)
}

/// Computes the exact 32-byte public output committed by `sp1-opcode-lab` for an anchor input.
#[must_use]
pub fn opcode_anchor_public_values(input: &OpcodeLabInput) -> Option<B256> {
    let accumulator = opcode_anchor_accumulator(input)?;
    opcode_anchor_public_input(input, accumulator).map(keccak256)
}

/// Folds one canonical REVM execution result into the value committed by the opcode-lab guest.
#[must_use]
pub fn fold_revm_opcode_execution_result<H>(result: &ExecutionResult<H>) -> u64 {
    let mut accumulator = result
        .tx_gas_used()
        .wrapping_mul(31)
        .wrapping_add(u64::from(result.is_success()));
    if let Some(output) = result.output() {
        accumulator = accumulator
            .wrapping_mul(31)
            .wrapping_add(output.len() as u64);
        for byte in output.iter().take(32) {
            accumulator = accumulator.wrapping_mul(31).wrapping_add(u64::from(*byte));
        }
    }
    accumulator
}

/// Folds one fixed microprogram result into the cross-program accumulator.
#[must_use]
pub const fn fold_revm_opcode_program(accumulator: u64, program_result: u64) -> u64 {
    accumulator.wrapping_mul(31).wrapping_add(program_result)
}

/// Computes the exact 32-byte public output committed by `sp1-revm-opcode-lab`.
#[must_use]
pub fn revm_opcode_public_values(input: &OpcodeLabInput, accumulator: u64) -> B256 {
    let mut output = Vec::new();
    output.extend_from_slice(input.case.as_bytes());
    output.extend_from_slice(input.scenario.as_bytes());
    output.extend_from_slice(&input.opcode.to_le_bytes());
    output.extend_from_slice(&input.target_count.to_le_bytes());
    output.extend_from_slice(&input.target_raw_gas.to_le_bytes());
    output.extend_from_slice(&accumulator.to_le_bytes());
    keccak256(output)
}

/// Computes the exact public output committed by `sp1-context-opcode-lab`.
#[must_use]
pub fn context_opcode_public_values(input_bincode: &[u8], accumulator: u64) -> B256 {
    let mut output = input_bincode.to_vec();
    output.extend_from_slice(&accumulator.to_le_bytes());
    keccak256(output)
}

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

/// Builds the canonical benchmark transaction from the normalized context input.
///
/// # Errors
///
/// Returns [`TxEnvBuildError`] if REVM rejects the benchmark transaction envelope.
pub fn build_context_benchmark_tx(
    input: &ContextOpcodeLabInputV1,
) -> Result<TxEnv, TxEnvBuildError> {
    let mut tx = build_benchmark_tx(input.execution_gas_limit(), input.storage.as_ref())?;
    tx.value = U256::from_be_bytes(input.tx_value);
    tx.data = Bytes::copy_from_slice(&input.calldata);
    Ok(tx)
}

/// Builds the canonical context-opcode block environment.
#[must_use]
pub fn build_context_benchmark_block_env(input: &ContextOpcodeLabInputV1) -> BlockEnv {
    BlockEnv {
        timestamp: U256::from(input.effective_block_timestamp()),
        ..BlockEnv::default()
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::{
        OPCODE_LAB_SPEC_ID, build_benchmark_db, build_benchmark_tx,
        build_context_benchmark_block_env, build_context_benchmark_tx,
        context_opcode_public_values, fold_revm_opcode_program, opcode_anchor_public_values,
        revm_opcode_public_values,
    };
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
    fn public_values_follow_the_frozen_guest_commitment() {
        let input = raiko2_primitives::OpcodeLabInput {
            case: "case".into(),
            scenario: "scenario".into(),
            opcode: 0x01,
            target_count: 2,
            target_raw_gas: 3,
            ..Default::default()
        };

        assert_eq!(fold_revm_opcode_program(7, 11), 228);
        assert_eq!(
            revm_opcode_public_values(&input, 4),
            B256::from_slice(
                &alloy_primitives::hex::decode(
                    "97fadfb98a033ca1f2c4a5fc10e77a2e2fee83ebcff769c6a1af05d943f4345d",
                )
                .unwrap(),
            )
        );
    }

    #[test]
    fn anchor_public_values_bind_the_exact_lane_and_envelope() {
        let target = raiko2_primitives::OpcodeLabInput {
            case: "synthetic_anchor_probe_push0".into(),
            scenario: "anchor_target_".into(),
            opcode: 0x5f,
            target_count: 8,
            target_raw_gas: 2,
            tx_gas_limit: Some(100_000),
            bytecode: vec![0x00],
            generator_max_count: Some(131_072),
            fixed_bytecode_len: Some(1),
            ..Default::default()
        };
        let mut control = target.clone();
        control.scenario = "anchor_control".into();
        assert_ne!(
            opcode_anchor_public_values(&target),
            opcode_anchor_public_values(&control)
        );
        control.tx_gas_limit = None;
        assert_eq!(opcode_anchor_public_values(&control), None);
    }

    #[test]
    fn benchmark_environment_uses_the_explicit_context_input() {
        let mut value = [0u8; 32];
        value[31] = 7;
        let input = raiko2_primitives::ContextOpcodeLabInputV1 {
            tx_gas_limit: Some(123_456),
            tx_value: value,
            calldata: vec![1, 2, 3],
            block_timestamp: Some(17),
            ..Default::default()
        };

        let tx = build_context_benchmark_tx(&input).expect("valid benchmark transaction");
        let block = build_context_benchmark_block_env(&input);

        assert_eq!(tx.gas_limit, 123_456);
        assert_eq!(tx.value, U256::from(7));
        assert_eq!(tx.data.as_ref(), &[1, 2, 3]);
        assert_eq!(block.timestamp, U256::from(17));
    }

    #[test]
    fn context_public_commitment_binds_complete_encoded_input_and_accumulator() {
        let baseline = context_opcode_public_values(b"complete-context-wire", 4);
        assert_ne!(
            context_opcode_public_values(b"complete-context-wirf", 4),
            baseline
        );
        assert_ne!(
            context_opcode_public_values(b"complete-context-wire", 5),
            baseline
        );
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
