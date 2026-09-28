//! SP1 guest program for revm-backed opcode prover-gas experiments.
#![no_main]
#![allow(missing_docs)]
sp1_zkvm::entrypoint!(main);

use raiko2_guest_sp1::revm_opcode_lab_impl::execute_revm_bytecode_with_input;
use raiko2_opcode_lab::{fold_revm_opcode_program, revm_opcode_public_values};
use raiko2_primitives::OpcodeLabInput;
use sp1_zkvm::io;

pub fn main() {
    let input = io::read::<OpcodeLabInput>();
    input
        .validate_controlled_contract()
        .expect("valid revm opcode controlled-workload contract");
    #[cfg(feature = "bench")]
    println!("cycle-tracker-report-start: revm_opcode_lab_execute");
    let mut accumulator = 0u64;
    for program in input
        .execution_programs()
        .expect("valid fixed-footprint microprogram framing")
    {
        accumulator = fold_revm_opcode_program(
            accumulator,
            execute_revm_bytecode_with_input(program, &input),
        );
    }
    #[cfg(feature = "bench")]
    println!("cycle-tracker-report-end: revm_opcode_lab_execute");

    let digest = revm_opcode_public_values(&input, accumulator);
    io::commit_slice(digest.as_slice());
}
