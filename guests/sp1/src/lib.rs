#![allow(missing_docs)]

pub mod crypto;
#[cfg(test)]
mod context_opcode_lab_impl;
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

    #[test]
    fn revm_opcode_lab_accepts_osaka_clz_opcode() {
        let bytecode = [0x60, 0x01, 0x1e, 0x00];
        let accumulator = execute_revm_bytecode(&bytecode, 100_000);
        assert_eq!((accumulator / 31) % 31, 1);
    }
}
