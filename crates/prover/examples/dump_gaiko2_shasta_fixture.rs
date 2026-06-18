#![allow(missing_docs)]

use std::{env, fs, path::PathBuf};

use raiko2_primitives_shasta::GuestInput;
use raiko2_prover::gaiko2::adapter::{build_shasta_packet, build_shasta_packet_with_guest_input};

fn main() {
    let mut include_guest_input = false;
    let mut positional = Vec::new();
    for arg in env::args().skip(1) {
        if arg == "--include-guest-input" {
            include_guest_input = true;
        } else {
            positional.push(arg);
        }
    }
    assert!(
        positional.len() == 2,
        "usage: [--include-guest-input] <input-guest-json> <output-packet-json>"
    );
    let input = &positional[0];
    let output = &positional[1];

    let input_path = PathBuf::from(input);
    let output_path = PathBuf::from(output);

    let raw = fs::read_to_string(&input_path).expect("read guest input fixture");
    let guest_input: GuestInput = serde_json::from_str(&raw).expect("parse GuestInput fixture");
    let packet = if include_guest_input {
        build_shasta_packet_with_guest_input(&guest_input)
            .expect("adapt guest input to raiko2-sgx packet")
    } else {
        build_shasta_packet(&guest_input).expect("adapt guest input to gaiko2 packet")
    };
    let serialized = serde_json::to_vec_pretty(&packet).expect("serialize gaiko2 packet");

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).expect("create output directory");
    }
    fs::write(&output_path, serialized).expect("write gaiko2 packet fixture");
}
