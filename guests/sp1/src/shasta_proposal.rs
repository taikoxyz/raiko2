//! SP1 guest program for single proposal proof.
#![no_main]
#![allow(missing_docs)]
sp1_zkvm::entrypoint!(main);

mod crypto;
mod sys;

use raiko2_guest_common::prove_shasta_proposal_for_proof_type;
use raiko2_primitives::ProofType;
use raiko2_primitives_shasta::GuestInput;
use sp1_zkvm::io;

pub fn main() {
    crypto::install_guest_crypto();

    // Read the guest input prepared by the host
    // The host has already prepared the witnesses for each block.
    #[cfg(feature = "bench")]
    println!("cycle-tracker-report-start: GuestInput");
    let guest_input = io::read::<GuestInput>();
    #[cfg(feature = "bench")]
    println!("cycle-tracker-report-end: GuestInput");

    #[cfg(feature = "bench")]
    println!("cycle-tracker-report-start: prove_shasta_proposal");
    let subproof_input_hash =
        prove_shasta_proposal_for_proof_type(&guest_input, ProofType::Sp1)
            .expect("proposal proving failed");
    #[cfg(feature = "bench")]
    println!("cycle-tracker-report-end: prove_shasta_proposal");

    io::commit_slice(subproof_input_hash.as_slice());
}
