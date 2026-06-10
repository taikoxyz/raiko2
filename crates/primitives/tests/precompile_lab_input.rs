#![allow(missing_docs)]

use raiko2_primitives::PrecompileLabInput;

#[test]
fn precompile_lab_input_is_public_and_deserializes_hex_input() {
    let input: PrecompileLabInput = serde_json::from_str(
        r#"{
          "case": "sha256",
          "scenario": "precompile",
          "address": 2,
          "target_count": 4,
          "input_size": 32,
          "target_raw_gas": 84,
          "input": "0x01020304"
        }"#,
    )
    .expect("parse lab input");

    assert_eq!(input.address, 2);
    assert_eq!(input.input, vec![0x01, 0x02, 0x03, 0x04]);
}
