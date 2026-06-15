//! SP1 guest program for revm-backed opcode prover-gas experiments.
#![no_main]
#![allow(missing_docs)]
sp1_zkvm::entrypoint!(main);

use alloy_primitives::keccak256;
use raiko2_guest_sp1::revm_opcode_lab_impl::execute_revm_bytecode;
use raiko2_primitives::OpcodeLabInput;
use sp1_zkvm::io;

const GAS_LIMIT_OVERHEAD: u64 = 1_000_000;

pub fn main() {
    let input = io::read::<OpcodeLabInput>();
    let gas_limit = input
        .target_raw_gas
        .saturating_mul(input.target_count)
        .saturating_add(GAS_LIMIT_OVERHEAD);

    #[cfg(feature = "bench")]
    println!("cycle-tracker-report-start: revm_opcode_lab_execute");
    let accumulator = execute_revm_bytecode(&input.bytecode, gas_limit);
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
