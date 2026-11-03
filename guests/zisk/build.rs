use std::{env, fs, path::PathBuf};

fn parse_key(name: &str) -> [u64; 4] {
    let text = env::var(name).unwrap_or_else(|_| {
        panic!("aggregation requires authoritative {name}: four comma-separated decimal u64 limbs; no default or input-supplied key is allowed")
    });
    let limbs: Vec<u64> = text
        .split(',')
        .map(|part| part.trim().parse().expect("invalid decimal VK limb"))
        .collect();
    let key: [u64; 4] = limbs.try_into().expect("VK must have exactly four limbs");
    assert!(
        key.iter().all(|limb| *limb < 0xffff_ffff_0000_0001),
        "noncanonical Goldilocks VK limb"
    );
    assert!(
        key.iter().any(|limb| *limb != 0),
        "zero VK is not an authoritative identity"
    );
    key
}

fn main() {
    for name in ["ZISK_SETUP_VK", "ZISK_PROPOSAL_VK"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    if env::var_os("CARGO_FEATURE_AGGREGATION").is_none() {
        return;
    }
    let setup = parse_key("ZISK_SETUP_VK");
    let proposal = parse_key("ZISK_PROPOSAL_VK");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    fs::write(
        output.join("zisk_keys.rs"),
        format!(
            "const SETUP_VK: [u64; 4] = {setup:?};\nconst PROPOSAL_VK: [u64; 4] = {proposal:?};\n"
        ),
    )
    .expect("write authoritative aggregation keys");
}
