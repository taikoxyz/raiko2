use raiko2_opcode_lab::{
    OPCODE_LAB_SPEC_ID, build_benchmark_db, build_context_benchmark_block_env,
    build_context_benchmark_tx, fold_revm_opcode_execution_result,
};
use raiko2_primitives::ContextOpcodeLabInputV1;
use revm::{
    Context, ExecuteEvm, MainBuilder, MainContext, bytecode::Bytecode,
};

/// Executes one context-opcode program with its explicit transaction and block environment.
pub fn execute_context_opcode_bytecode(bytecode: &[u8], input: &ContextOpcodeLabInputV1) -> u64 {
    let bytecode = Bytecode::new_legacy(bytecode.to_vec().into());
    let context = Context::mainnet()
        .modify_cfg_chained(|cfg| cfg.set_spec_and_mainnet_gas_params(OPCODE_LAB_SPEC_ID))
        .modify_block_chained(|block| *block = build_context_benchmark_block_env(input))
        .with_db(build_benchmark_db(bytecode, input.storage.as_ref()));
    let execution = context
        .build_mainnet()
        .transact(build_context_benchmark_tx(input).expect("valid context opcode benchmark tx"))
        .expect("context opcode lab execution");
    fold_revm_opcode_execution_result(&execution.result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_execution_observes_calldata_and_timestamp() {
        let calldata = ContextOpcodeLabInputV1 {
            tx_gas_limit: Some(100_000),
            calldata: vec![0x2a],
            ..Default::default()
        };
        let empty = ContextOpcodeLabInputV1 {
            calldata: Vec::new(),
            ..calldata.clone()
        };
        assert_ne!(
            execute_context_opcode_bytecode(
                &[0x5f, 0x35, 0x5f, 0x52, 0x60, 0x20, 0x5f, 0xf3],
                &calldata,
            ),
            execute_context_opcode_bytecode(
                &[0x5f, 0x35, 0x5f, 0x52, 0x60, 0x20, 0x5f, 0xf3],
                &empty,
            ),
        );

        let timestamp = ContextOpcodeLabInputV1 {
            block_timestamp: Some(17),
            ..calldata.clone()
        };
        let zero_timestamp = ContextOpcodeLabInputV1 {
            block_timestamp: Some(0),
            ..calldata
        };
        assert_ne!(
            execute_context_opcode_bytecode(
                &[0x42, 0x5f, 0x52, 0x60, 0x20, 0x5f, 0xf3],
                &timestamp,
            ),
            execute_context_opcode_bytecode(
                &[0x42, 0x5f, 0x52, 0x60, 0x20, 0x5f, 0xf3],
                &zero_timestamp,
            ),
        );

        let mut value = [0u8; 32];
        value[31] = 7;
        let paid = ContextOpcodeLabInputV1 {
            tx_value: value,
            ..timestamp.clone()
        };
        let zero_value = ContextOpcodeLabInputV1 {
            tx_value: [0; 32],
            ..timestamp
        };
        assert_ne!(
            execute_context_opcode_bytecode(
                &[0x34, 0x5f, 0x52, 0x60, 0x20, 0x5f, 0xf3],
                &paid,
            ),
            execute_context_opcode_bytecode(
                &[0x34, 0x5f, 0x52, 0x60, 0x20, 0x5f, 0xf3],
                &zero_value,
            ),
        );
    }
}
