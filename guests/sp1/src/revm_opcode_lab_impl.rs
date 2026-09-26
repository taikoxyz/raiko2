use revm::{
    bytecode::Bytecode, context::TxEnv, database::BenchmarkDB, primitives::hardfork::SpecId,
    Context, ExecuteEvm, MainBuilder, MainContext,
};

#[cfg(test)]
std::thread_local! {
    static TEST_REVM_SPEC_ID: std::cell::Cell<SpecId> = const {
        std::cell::Cell::new(SpecId::OSAKA)
    };
    static TEST_REVM_SUCCESS: std::cell::Cell<Option<bool>> = const {
        std::cell::Cell::new(None)
    };
}

#[cfg(not(test))]
macro_rules! configured_revm_spec {
    () => {
        SpecId::OSAKA
    };
}

#[cfg(test)]
macro_rules! configured_revm_spec {
    () => {
        TEST_REVM_SPEC_ID.with(std::cell::Cell::get)
    };
}

pub fn execute_revm_bytecode(bytecode: &[u8], gas_limit: u64) -> u64 {
    let bytecode = Bytecode::new_legacy(bytecode.to_vec().into());
    let ctx = Context::mainnet()
        .modify_cfg_chained(|cfg| cfg.set_spec_and_mainnet_gas_params(configured_revm_spec!()))
        .with_db(BenchmarkDB::new_bytecode(bytecode));
    let mut evm = ctx.build_mainnet();
    let result = evm
        .transact(
            TxEnv::builder_for_bench()
                .gas_limit(gas_limit.max(100_000))
                .build()
                .expect("valid revm benchmark tx"),
        )
        .expect("revm opcode lab execution")
        .result;

    #[cfg(test)]
    TEST_REVM_SUCCESS.with(|success| success.set(Some(result.is_success())));

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
