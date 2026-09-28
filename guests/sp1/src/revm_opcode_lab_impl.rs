use raiko2_opcode_lab::{
    build_benchmark_block_env, build_benchmark_db, build_benchmark_tx,
    fold_revm_opcode_execution_result, OPCODE_LAB_SPEC_ID,
};
use raiko2_primitives::{OpcodeLabInput, OpcodeLabStorageInput};
use revm::{
    bytecode::Bytecode, context_interface::result::ResultAndState, primitives::hardfork::SpecId,
    Context, ExecuteEvm, MainBuilder, MainContext,
};

#[cfg(test)]
std::thread_local! {
    static TEST_REVM_SPEC_ID: std::cell::Cell<SpecId> = const {
        std::cell::Cell::new(OPCODE_LAB_SPEC_ID)
    };
    static TEST_REVM_SUCCESS: std::cell::Cell<Option<bool>> = const {
        std::cell::Cell::new(None)
    };
}

#[cfg(not(test))]
macro_rules! configured_revm_spec {
    () => {
        OPCODE_LAB_SPEC_ID
    };
}

#[cfg(test)]
macro_rules! configured_revm_spec {
    () => {
        TEST_REVM_SPEC_ID.with(std::cell::Cell::get)
    };
}

pub fn execute_revm_bytecode(bytecode: &[u8], gas_limit: u64) -> u64 {
    execute_revm_bytecode_with_storage(bytecode, gas_limit, None)
}

pub fn execute_revm_bytecode_with_storage(
    bytecode: &[u8],
    gas_limit: u64,
    storage: Option<&OpcodeLabStorageInput>,
) -> u64 {
    let input = OpcodeLabInput {
        tx_gas_limit: Some(gas_limit.max(OpcodeLabInput::MIN_EXECUTION_GAS_LIMIT)),
        storage: storage.cloned(),
        ..Default::default()
    };
    execute_revm_bytecode_with_input_for_spec(bytecode, &input, configured_revm_spec!())
}

/// Executes one opcode-lab program with the canonical transaction and block environments.
pub fn execute_revm_bytecode_with_input(bytecode: &[u8], input: &OpcodeLabInput) -> u64 {
    execute_revm_bytecode_with_input_for_spec(bytecode, input, configured_revm_spec!())
}

fn execute_revm_bytecode_with_input_for_spec(
    bytecode: &[u8],
    input: &OpcodeLabInput,
    spec_id: SpecId,
) -> u64 {
    let execution = execute_revm_bytecode_result(bytecode, input, spec_id);
    #[cfg(test)]
    record_test_revm_success(&execution);
    fold_revm_opcode_execution_result(&execution.result)
}

fn execute_revm_bytecode_result(
    bytecode: &[u8],
    input: &OpcodeLabInput,
    spec_id: SpecId,
) -> ResultAndState {
    let bytecode = Bytecode::new_legacy(bytecode.to_vec().into());
    let ctx = Context::mainnet()
        .modify_cfg_chained(|cfg| cfg.set_spec_and_mainnet_gas_params(spec_id))
        .modify_block_chained(|block| *block = build_benchmark_block_env(input))
        .with_db(build_benchmark_db(bytecode, input.storage.as_ref()));
    let mut evm = ctx.build_mainnet();
    evm.transact(build_benchmark_tx(input).expect("valid revm benchmark tx"))
        .expect("revm opcode lab execution")
}

#[cfg(test)]
fn record_test_revm_success(result: &ResultAndState) {
    TEST_REVM_SUCCESS.with(|success| success.set(Some(result.result.is_success())));
}

#[cfg(test)]
mod tests {
    use super::*;
    use raiko2_primitives::{
        OpcodeLabStorageAccess, OpcodeLabStorageInput, OpcodeLabStorageLane,
        OpcodeLabStorageOperation,
    };
    use revm::{database::BENCH_TARGET, primitives::U256};

    const SLOT: [u8; 32] = [0x11; 32];

    fn storage_load(original_value: [u8; 32]) -> OpcodeLabStorageInput {
        OpcodeLabStorageInput {
            measurement_opcode: 0x54,
            lane: OpcodeLabStorageLane::Target,
            slot: SLOT,
            original_value,
            access: OpcodeLabStorageAccess::Cold,
            operation: OpcodeLabStorageOperation::Load {
                expected_value: original_value,
            },
        }
    }

    fn storage_store(original_value: [u8; 32], new_value: [u8; 32]) -> OpcodeLabStorageInput {
        OpcodeLabStorageInput {
            measurement_opcode: 0x55,
            lane: OpcodeLabStorageLane::Target,
            slot: SLOT,
            original_value,
            access: OpcodeLabStorageAccess::Warm,
            operation: OpcodeLabStorageOperation::Store {
                current_value: original_value,
                new_value,
            },
        }
    }

    fn sload_return_program() -> Vec<u8> {
        let mut bytecode = vec![0x7f];
        bytecode.extend_from_slice(&SLOT);
        bytecode.extend_from_slice(&[0x54, 0x5f, 0x52, 0x60, 0x20, 0x5f, 0xf3]);
        bytecode
    }

    fn sstore_program(new_value: [u8; 32]) -> Vec<u8> {
        let mut bytecode = vec![0x7f];
        bytecode.extend_from_slice(&new_value);
        bytecode.push(0x7f);
        bytecode.extend_from_slice(&SLOT);
        bytecode.extend_from_slice(&[0x55, 0x00]);
        bytecode
    }

    fn execute_revm_bytecode_for_spec(
        bytecode: &[u8],
        gas_limit: u64,
        spec_id: SpecId,
    ) -> (u64, bool) {
        let previous_spec = TEST_REVM_SPEC_ID.with(|current| current.replace(spec_id));
        let accumulator = execute_revm_bytecode(bytecode, gas_limit);
        TEST_REVM_SPEC_ID.with(|current| current.set(previous_spec));
        let is_success = TEST_REVM_SUCCESS
            .with(std::cell::Cell::take)
            .expect("test execution records REVM success");
        (accumulator, is_success)
    }

    #[test]
    fn revm_opcode_lab_accepts_osaka_clz_opcode() {
        let bytecode = [0x60, 0x01, 0x1e, 0x00];
        let (osaka_accumulator, osaka_success) =
            execute_revm_bytecode_for_spec(&bytecode, 100_000, SpecId::OSAKA);
        let (_, prague_success) =
            execute_revm_bytecode_for_spec(&bytecode, 100_000, SpecId::PRAGUE);

        assert!(osaka_success);
        assert_eq!((osaka_accumulator / 31) % 31, 1);
        assert!(!prague_success);
    }

    #[test]
    fn revm_opcode_lab_sload_returns_zero_and_nonzero_prestate() {
        for original_value in [[0u8; 32], [0x22; 32]] {
            let storage = storage_load(original_value);
            let input = OpcodeLabInput {
                tx_gas_limit: Some(100_000),
                storage: Some(storage),
                ..Default::default()
            };
            let execution =
                execute_revm_bytecode_result(&sload_return_program(), &input, SpecId::OSAKA);

            assert!(execution.result.is_success());
            assert_eq!(
                execution.result.output().expect("SLOAD returns one word"),
                original_value.as_slice()
            );
        }
    }

    #[test]
    fn revm_opcode_lab_sstore_applies_zero_to_one_and_one_to_zero() {
        let one = U256::from(1);
        for (original_value, new_value, expected) in [
            ([0u8; 32], one.to_be_bytes(), one),
            (one.to_be_bytes(), [0u8; 32], U256::ZERO),
        ] {
            let storage = storage_store(original_value, new_value);
            let input = OpcodeLabInput {
                tx_gas_limit: Some(100_000),
                storage: Some(storage),
                ..Default::default()
            };
            let execution =
                execute_revm_bytecode_result(&sstore_program(new_value), &input, SpecId::OSAKA);

            assert!(execution.result.is_success());
            assert_eq!(
                execution.state[&BENCH_TARGET].storage[&U256::from_be_bytes(SLOT)].present_value,
                expected
            );
        }
    }
}
