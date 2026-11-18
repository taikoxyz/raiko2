//! Etna fixture linked against the production RISC Zero workspace's crypto patches.
#![no_main]

risc0_zkvm::guest::entry!(main);

pub fn main() {
    let journal = etna_guest_compat_fixture::run();
    risc0_zkvm::guest::env::commit_slice(&journal);
}
