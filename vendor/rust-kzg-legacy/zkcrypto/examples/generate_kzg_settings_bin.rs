use rust_kzg_zkcrypto::{eip_4844::load_trusted_setup_filename_rust, kzg_proofs::KZGSettings};
use std::{env, fs};

fn main() {
    let mut args = env::args().skip(1);
    let trusted_setup = args
        .next()
        .expect("usage: generate_kzg_settings_bin <trusted_setup.txt> <output.bin>");
    let output = args
        .next()
        .expect("usage: generate_kzg_settings_bin <trusted_setup.txt> <output.bin>");

    let settings: KZGSettings =
        load_trusted_setup_filename_rust(&trusted_setup).expect("load trusted setup");
    let encoded = bincode::serialize(&settings).expect("serialize KZGSettings");
    fs::write(&output, encoded).expect("write KZGSettings");

    println!(
        "ok max_width={} g1={} g2={} output={}",
        settings.fs.max_width,
        settings.secret_g1.len(),
        settings.secret_g2.len(),
        output
    );
}
