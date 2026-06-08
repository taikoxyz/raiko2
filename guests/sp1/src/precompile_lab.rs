//! SP1 guest program for controlled precompile prover-gas experiments.
#![no_main]
#![allow(missing_docs)]
sp1_zkvm::entrypoint!(main);

use alloy_primitives::keccak256;
use raiko2_guest_sp1::precompile_lab_impl::execute_precompile;
use raiko2_primitives::PrecompileLabInput;
use sp1_zkvm::io;

pub fn main() {
    let input = io::read::<PrecompileLabInput>();

    #[cfg(feature = "bench")]
    println!("cycle-tracker-report-start: precompile_lab_execute");
    let accumulator = execute_precompile(&input);
    #[cfg(feature = "bench")]
    println!("cycle-tracker-report-end: precompile_lab_execute");

    let mut output = Vec::new();
    output.extend_from_slice(input.case.as_bytes());
    output.extend_from_slice(&input.address.to_le_bytes());
    output.extend_from_slice(&input.target_count.to_le_bytes());
    output.extend_from_slice(&input.input_size.to_le_bytes());
    output.extend_from_slice(&input.target_raw_gas.to_le_bytes());
    output.extend_from_slice(&accumulator.to_le_bytes());
    let digest = keccak256(output);
    io::commit_slice(digest.as_slice());
}
