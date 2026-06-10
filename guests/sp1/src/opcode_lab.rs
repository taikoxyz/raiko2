//! SP1 guest program for controlled opcode prover-gas experiments.
#![no_main]
#![allow(missing_docs)]
sp1_zkvm::entrypoint!(main);

use alloy_primitives::keccak256;
use raiko2_guest_sp1::opcode_lab_impl::execute_bytecode;
use raiko2_primitives::OpcodeLabInput;
use sp1_zkvm::io;

pub fn main() {
    let input = io::read::<OpcodeLabInput>();

    #[cfg(feature = "bench")]
    println!("cycle-tracker-report-start: opcode_lab_execute");
    let accumulator = execute_bytecode(&input.bytecode);
    #[cfg(feature = "bench")]
    println!("cycle-tracker-report-end: opcode_lab_execute");

    let mut output = Vec::new();
    output.extend_from_slice(input.case.as_bytes());
    output.extend_from_slice(&input.opcode.to_le_bytes());
    output.extend_from_slice(&input.target_count.to_le_bytes());
    output.extend_from_slice(&input.target_raw_gas.to_le_bytes());
    output.extend_from_slice(&accumulator.to_le_bytes());
    let digest = keccak256(output);
    io::commit_slice(digest.as_slice());
}
