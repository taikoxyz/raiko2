//! SP1 guest program for single proposal proof.
#![no_main]
sp1_zkvm::entrypoint!(main);

use raiko2_primitives::{guest::prove_shasta_proposal, GuestInput};
use raiko2_protocol::ProofCarryData;

pub fn main() {
    // Read the guest input prepared by the host
    // The host has already prepared the witnesses for each block
    let input_bytes = sp1_zkvm::io::read_vec();
    let guest_input: GuestInput =
        bincode::deserialize(&input_bytes).expect("Failed to deserialize GuestInput");

    // Read the proof carry data that contains the transition input
    let proof_carry_data: ProofCarryData = sp1_zkvm::io::read();

    let (instance_hash, subproof_input_hash) =
        prove_shasta_proposal(&guest_input, &proof_carry_data).expect("proposal proving failed");

    sp1_zkvm::io::commit_slice(instance_hash.as_slice());
    sp1_zkvm::io::commit_slice(subproof_input_hash.as_slice());
}
