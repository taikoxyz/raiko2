//! RISC0 guest program for single proposal proof.
#![no_main]
#![allow(missing_docs)]
risc0_zkvm::guest::entry!(main);

mod sys;

use raiko2_guest_common::prove_shasta_proposal;
use raiko2_primitives_shasta::GuestInput;
use raiko2_protocol_shasta::shasta::ProofCarryData;
use risc0_zkvm::guest::env;

pub fn main() {
    // Read the guest input prepared by the host
    let guest_input: GuestInput = env::read();

    // Read the proof carry data that contains the transition input
    let proof_carry_data: ProofCarryData = env::read();

    let (instance_hash, subproof_input_hash) =
        prove_shasta_proposal(&guest_input, &proof_carry_data).expect("proposal proving failed");

    env::commit_slice(instance_hash.as_slice());
    env::commit_slice(subproof_input_hash.as_slice());
}
