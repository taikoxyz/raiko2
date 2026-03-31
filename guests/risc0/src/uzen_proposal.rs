//! RISC0 guest program for single proposal proof.
#![no_main]
#![allow(missing_docs)]
risc0_zkvm::guest::entry!(main);

extern crate alloc;

mod sys;

use alloc::vec;
use bincode;
use raiko2_guest_common::prove_shasta_proposal;
use raiko2_primitives_shasta::GuestInput;
use risc0_zkvm::guest::env;

pub fn main() {
    let mut len = 0u32;
    env::read_slice(core::slice::from_mut(&mut len));

    let mut input_buf = vec![0u8; len as usize];
    env::read_slice(&mut input_buf);
    let guest_input: GuestInput =
        bincode::deserialize(&input_buf).expect("failed to deserialize proposal guest input");

    let (instance_hash, subproof_input_hash) =
        prove_shasta_proposal(&guest_input).expect("proposal proving failed");

    env::commit_slice(instance_hash.as_slice());
    env::commit_slice(subproof_input_hash.as_slice());
}
