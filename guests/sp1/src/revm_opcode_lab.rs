//! SP1 guest program for revm-backed opcode prover-gas experiments.
#![no_main]
#![allow(missing_docs)]
sp1_zkvm::entrypoint!(main);

use alloy_primitives::keccak256;
use raiko2_guest_sp1::revm_opcode_lab_impl::execute_revm_bytecode_with_storage;
use raiko2_primitives::OpcodeLabInput;
use sp1_zkvm::io;

pub fn main() {
    let input = io::read::<OpcodeLabInput>();
    input
        .validate_controlled_contract()
        .expect("valid revm opcode controlled-workload contract");
    let gas_limit = input.execution_gas_limit();

    #[cfg(feature = "bench")]
    println!("cycle-tracker-report-start: revm_opcode_lab_execute");
    let mut accumulator = 0u64;
    for program in input
        .execution_programs()
        .expect("valid fixed-footprint microprogram framing")
    {
        accumulator =
            accumulator
                .wrapping_mul(31)
                .wrapping_add(execute_revm_bytecode_with_storage(
                    program,
                    gas_limit,
                    input.storage.as_ref(),
                ));
    }
    #[cfg(feature = "bench")]
    println!("cycle-tracker-report-end: revm_opcode_lab_execute");

    let mut output = Vec::new();
    output.extend_from_slice(input.case.as_bytes());
    output.extend_from_slice(input.scenario.as_bytes());
    output.extend_from_slice(&input.opcode.to_le_bytes());
    output.extend_from_slice(&input.target_count.to_le_bytes());
    output.extend_from_slice(&input.target_raw_gas.to_le_bytes());
    output.extend_from_slice(&accumulator.to_le_bytes());
    let digest = keccak256(output);
    io::commit_slice(digest.as_slice());
}
