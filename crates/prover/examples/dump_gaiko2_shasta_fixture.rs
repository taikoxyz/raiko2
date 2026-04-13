#![allow(missing_docs)]

use std::{env, fs, path::PathBuf};

use raiko2_primitives_shasta::GuestInput;
use raiko2_prover::gaiko2::adapter::build_shasta_packet;

fn main() {
    let mut args = env::args().skip(1);
    let input = args
        .next()
        .expect("usage: <input-guest-json> <output-packet-json>");
    let output = args
        .next()
        .expect("usage: <input-guest-json> <output-packet-json>");
    assert!(
        args.next().is_none(),
        "usage: <input-guest-json> <output-packet-json>"
    );

    let input_path = PathBuf::from(input);
    let output_path = PathBuf::from(output);

    let raw = fs::read_to_string(&input_path).expect("read guest input fixture");
    let guest_input: GuestInput = serde_json::from_str(&raw).expect("parse GuestInput fixture");
    let packet = build_shasta_packet(&guest_input).expect("adapt guest input to gaiko2 packet");
    let serialized = serde_json::to_vec_pretty(&packet).expect("serialize gaiko2 packet");

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).expect("create output directory");
    }
    fs::write(&output_path, serialized).expect("write gaiko2 packet fixture");
}
