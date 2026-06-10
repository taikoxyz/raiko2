#![allow(missing_docs)]

use raiko2_primitives::OpcodeLabInput;

#[test]
fn opcode_lab_input_is_public_and_deserializes_hex_bytecode() {
    let input: OpcodeLabInput = serde_json::from_str(
        r#"{
          "case": "add",
          "scenario": "arithmetic",
          "opcode": 1,
          "target_count": 4,
          "target_raw_gas": 3,
          "bytecode": "0x600160020100"
        }"#,
    )
    .expect("parse lab input");

    assert_eq!(input.bytecode, vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00]);
}
