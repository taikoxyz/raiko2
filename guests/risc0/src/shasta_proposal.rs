//! RISC0 guest program for single proposal proof.
#![no_main]
#![allow(missing_docs)]
risc0_zkvm::guest::entry!(main);

mod sys;

use raiko2_guest_common::prove_shasta_proposal;
use raiko2_primitives_shasta::GuestInput;
use risc0_zkvm::guest::env;

pub fn main() {
    // Read the guest input prepared by the host
    let guest_input: GuestInput = env::read();

    let (instance_hash, subproof_input_hash) =
        prove_shasta_proposal(&guest_input).expect("proposal proving failed");

    env::commit_slice(instance_hash.as_slice());
    env::commit_slice(subproof_input_hash.as_slice());
}
