//! SP1 guest program for single proposal proof.
#![no_main]
#![allow(missing_docs)]
sp1_zkvm::entrypoint!(main);

mod sys;

use raiko2_guest_common::prove_shasta_proposal;
use raiko2_primitives_shasta::GuestInput;
use raiko2_protocol_shasta::shasta::ProofCarryData;

pub fn main() {
    // Read the guest input prepared by the host
    // The host has already prepared the witnesses for each block
    let guest_input: GuestInput = sp1_zkvm::io::read();

    // Read the proof carry data that contains the transition input
    let proof_carry_data: ProofCarryData = sp1_zkvm::io::read();

    #[cfg(feature = "bench")]
    println!("cycle-tracker-start: shasta-proposal");
    let (instance_hash, subproof_input_hash) =
        prove_shasta_proposal(&guest_input, &proof_carry_data).expect("proposal proving failed");
    #[cfg(feature = "bench")]
    println!("cycle-tracker-end: shasta-proposal");

    sp1_zkvm::io::commit_slice(instance_hash.as_slice());
    sp1_zkvm::io::commit_slice(subproof_input_hash.as_slice());
}
