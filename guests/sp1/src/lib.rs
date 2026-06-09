#![allow(missing_docs)]

pub mod crypto;
pub mod opcode_lab_impl;
pub mod precompile_lab_impl;
pub mod revm_opcode_lab_impl;

#[cfg(test)]
mod tests {
    use crate::revm_opcode_lab_impl::execute_revm_bytecode;

    #[test]
    fn revm_opcode_lab_executes_simple_bytecode() {
        let bytecode = [0x60, 0x02, 0x60, 0x03, 0x01, 0x00];
        assert_ne!(execute_revm_bytecode(&bytecode, 100_000), 0);
    }
}
