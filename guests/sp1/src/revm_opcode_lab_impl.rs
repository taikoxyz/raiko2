use revm::{
    bytecode::Bytecode, context::TxEnv, database::BenchmarkDB, primitives::hardfork::SpecId,
    Context, ExecuteEvm, MainBuilder, MainContext,
};

pub fn execute_revm_bytecode(bytecode: &[u8], gas_limit: u64) -> u64 {
    let bytecode = Bytecode::new_legacy(bytecode.to_vec().into());
    let ctx = Context::mainnet()
        .modify_cfg_chained(|cfg| cfg.set_spec_and_mainnet_gas_params(SpecId::PRAGUE))
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
