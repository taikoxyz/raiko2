//! SP1 guest program for explicit-context REVM opcode prover-gas experiments.
#![no_main]
#![allow(missing_docs)]
sp1_zkvm::entrypoint!(main);

mod context_opcode_lab_impl;

use context_opcode_lab_impl::execute_context_opcode_bytecode;
use raiko2_opcode_lab::{context_opcode_public_values, fold_revm_opcode_program};
use raiko2_primitives::ContextOpcodeLabInputV1;
use sp1_zkvm::io;

pub fn main() {
    let input = io::read::<ContextOpcodeLabInputV1>();
    input
        .validate_controlled_contract()
        .expect("valid context opcode controlled-workload contract");
    #[cfg(feature = "bench")]
    println!("cycle-tracker-report-start: context_opcode_lab_execute");
    let mut accumulator = 0u64;
    for program in input
        .execution_programs()
        .expect("valid fixed-footprint microprogram framing")
    {
        accumulator = fold_revm_opcode_program(
            accumulator,
            execute_context_opcode_bytecode(program, &input),
        );
    }
    #[cfg(feature = "bench")]
    println!("cycle-tracker-report-end: context_opcode_lab_execute");

    let encoded = bincode::serialize(&input).expect("serialize context opcode input");
    let digest = context_opcode_public_values(&encoded, accumulator);
    io::commit_slice(digest.as_slice());
}
