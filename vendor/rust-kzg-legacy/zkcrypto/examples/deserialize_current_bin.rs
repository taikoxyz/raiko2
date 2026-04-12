use rust_kzg_zkcrypto::kzg_proofs::KZGSettings;
use std::{env, fs};

fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: deserialize_current_bin <path-to-kzg_settings.bin>");
    let bytes = fs::read(&path).expect("read kzg_settings.bin");
    let settings: KZGSettings = bincode::deserialize(&bytes).expect("deserialize KZGSettings");
    println!(
        "ok max_width={} g1={} g2={} precomp={}",
        settings.fs.max_width,
        settings.secret_g1.len(),
        settings.secret_g2.len(),
        settings.precomputation.is_some()
    );
}
