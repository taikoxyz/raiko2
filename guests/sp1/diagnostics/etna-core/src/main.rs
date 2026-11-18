//! Etna fixture linked against the production SP1 workspace's crypto patches.
#![no_main]

sp1_zkvm::entrypoint!(main);

pub fn main() {
    let journal = etna_guest_compat_fixture::run();
    sp1_zkvm::io::commit_slice(&journal);
}
