//! SP1 guest program for single proposal proof.
#![no_main]
#![allow(missing_docs)]
sp1_zkvm::entrypoint!(main);

mod sys;

use raiko2_guest_common::prove_shasta_proposal;
use raiko2_primitives_shasta::GuestInput;
use sp1_zkvm::io;

pub fn main() {
    // Read the guest input prepared by the host
    // The host has already prepared the witnesses for each block.
    #[cfg(feature = "bench")]
    println!("cycle-tracker-report-start: GuestInput");
    let guest_input = io::read::<GuestInput>();
    #[cfg(feature = "bench")]
    println!("cycle-tracker-report-end: GuestInput");

    #[cfg(feature = "bench")]
    println!("cycle-tracker-report-start: prove_shasta_proposal");
    let (instance_hash, subproof_input_hash) =
        prove_shasta_proposal(&guest_input).expect("proposal proving failed");
    #[cfg(feature = "bench")]
    println!("cycle-tracker-report-end: prove_shasta_proposal");

    io::commit_slice(instance_hash.as_slice());
    io::commit_slice(subproof_input_hash.as_slice());
}
