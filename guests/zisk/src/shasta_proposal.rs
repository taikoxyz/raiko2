//! Full Shasta proposal validation through the shared implementation.
#![no_main]

mod sys;

use bincode::Options;
use raiko2_primitives_shasta::GuestInput;

ziskos::entrypoint!(main);

fn main() {
    let bytes = ziskos::io::read_slice();
    // bincode::serialize uses the 1.3 legacy fixed-int encoding. Reject suffixes.
    let input: GuestInput = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .reject_trailing_bytes()
        .deserialize(bytes.as_ref())
        .expect("invalid bincode 1.3 proposal payload");
    let hash = raiko2_guest_common::prove_shasta_proposal(&input)
        .expect("complete proposal validation failed");
    ziskos::io::commit_slice(hash.as_slice());
}
