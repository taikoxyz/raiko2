#![allow(missing_docs)]

use raiko2_primitives::{
    OpcodeLabInput, OpcodeLabStorageAccess, OpcodeLabStorageInput, OpcodeLabStorageLane,
    OpcodeLabStorageOperation,
};

const ZERO: &str = "0x0000000000000000000000000000000000000000000000000000000000000000";
const ONE: &str = "0x0000000000000000000000000000000000000000000000000000000000000001";
const TWO: &str = "0x0000000000000000000000000000000000000000000000000000000000000002";

fn push32(value: u8) -> Vec<u8> {
    let mut bytes = vec![0x7f];
    bytes.extend([0; 31]);
    bytes.push(value);
    bytes
}

fn store_program(prefix: bool, measured: bool) -> Vec<u8> {
    let mut program = Vec::new();
    if prefix {
        program.extend(push32(1));
        program.extend(push32(0));
        program.push(0x55);
    }
    program.extend(push32(2));
    program.extend(push32(0));
    if measured {
        program.extend([0x55, 0x5f, 0x50, 0x00]);
    } else {
        program.extend([0x50, 0x50, 0x00]);
    }
    program
}

fn framed(programs: &[Vec<u8>]) -> Vec<u8> {
    let mut encoded = OpcodeLabInput::FIXED_MICROPROGRAM_MAGIC.to_vec();
    encoded.extend(u32::try_from(programs.len()).unwrap().to_be_bytes());
    for program in programs {
        encoded.extend(u32::try_from(program.len()).unwrap().to_be_bytes());
        encoded.extend(program);
    }
    encoded
}

fn storage_input(json: &str) -> OpcodeLabInput {
    serde_json::from_str(json).expect("parse storage input")
}

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
    assert_eq!(input.storage, None);
    assert_eq!(
        serde_json::to_value(&input).unwrap().get("storage"),
        None,
        "stateless JSON must remain byte-for-byte shape compatible"
    );
}

#[test]
fn opcode_lab_stateless_input_round_trips_through_bincode_without_storage() {
    let input = OpcodeLabInput {
        case: "add".into(),
        scenario: "arithmetic".into(),
        opcode: 0x01,
        target_count: 4,
        target_raw_gas: 3,
        tx_gas_limit: Some(100_000),
        bytecode: vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00],
        generator_max_count: Some(8),
        fixed_bytecode_len: Some(6),
        storage: None,
        ..Default::default()
    };

    let encoded = bincode::serialize(&input).expect("serialize stateless opcode input");
    let decoded: OpcodeLabInput =
        bincode::deserialize(&encoded).expect("deserialize stateless opcode input");
    assert_eq!(decoded, input);
}

#[test]
fn opcode_lab_storage_words_round_trip_at_full_width_in_json_and_bincode() {
    let input = storage_input(&format!(
        r#"{{
          "case": "sload_cold_zero",
          "scenario": "load",
          "opcode": 84,
          "target_count": 1,
          "target_raw_gas": 2100,
          "tx_gas_limit": 100000,
          "bytecode": "0x7f{}5400",
          "generator_max_count": 1,
          "fixed_bytecode_len": 35,
          "storage": {{
            "measurement_opcode": 84,
            "lane": "target",
            "slot": "{ZERO}",
            "original_value": "{ONE}",
            "access": "cold",
            "operation": {{"kind": "load", "expected_value": "{ONE}"}}
          }}
        }}"#,
        &ZERO[2..],
    ));

    let json = serde_json::to_string(&input).unwrap();
    let json_round_trip: OpcodeLabInput = serde_json::from_str(&json).unwrap();
    assert_eq!(json_round_trip, input);
    assert!(json.contains(ONE));
    assert!(json.contains(ZERO));

    let encoded = bincode::serialize(&input).unwrap();
    let bincode_round_trip: OpcodeLabInput = bincode::deserialize(&encoded).unwrap();
    assert_eq!(bincode_round_trip, input);
}

#[test]
fn opcode_lab_storage_contract_rejects_invalid_lane_and_target_opcode_mismatch() {
    let invalid_lane = serde_json::from_str::<OpcodeLabInput>(&format!(
        r#"{{
          "case":"bad","scenario":"load","opcode":84,"target_count":0,
          "target_raw_gas":100,"bytecode":"0x00",
          "storage":{{"measurement_opcode":84,"lane":"observer","slot":"{ZERO}",
          "original_value":"{ZERO}","access":"warm",
          "operation":{{"kind":"load","expected_value":"{ZERO}"}}}}
        }}"#
    ));
    assert!(invalid_lane.is_err());

    let mismatch = storage_input(&format!(
        r#"{{
          "case":"bad","scenario":"load","opcode":85,"target_count":0,
          "target_raw_gas":100,"bytecode":"0x00",
          "storage":{{"measurement_opcode":84,"lane":"target","slot":"{ZERO}",
          "original_value":"{ZERO}","access":"warm",
          "operation":{{"kind":"load","expected_value":"{ZERO}"}}}}
        }}"#
    ));
    assert_eq!(
        mismatch.validate_storage_contract(),
        Err("target opcode differs from storage measurement_opcode")
    );
}

#[test]
fn opcode_lab_control_lane_uses_reference_opcode_and_rejects_an_extra_storage_site() {
    let valid = OpcodeLabInput {
        opcode: 0x50,
        target_count: 2,
        target_raw_gas: 2,
        bytecode: store_program(false, false),
        storage: Some(OpcodeLabStorageInput {
            measurement_opcode: 0x55,
            lane: OpcodeLabStorageLane::Control,
            slot: [0; 32],
            original_value: [0; 32],
            access: OpcodeLabStorageAccess::Warm,
            operation: OpcodeLabStorageOperation::Store {
                current_value: [0; 32],
                new_value: {
                    let mut value = [0; 32];
                    value[31] = 2;
                    value
                },
            },
        }),
        ..OpcodeLabInput::default()
    };
    valid
        .validate_storage_contract()
        .expect("control reference opcode is lane-correct");
    assert_eq!(
        OpcodeLabInput {
            opcode: 0x01,
            ..valid.clone()
        }
        .validate_storage_contract(),
        Err("SSTORE control opcode must declare POP")
    );
    assert_eq!(
        OpcodeLabInput {
            target_count: 1,
            ..valid.clone()
        }
        .validate_storage_contract(),
        Err("control opcode count differs from target_count")
    );
    assert_eq!(
        OpcodeLabInput {
            target_raw_gas: 3,
            ..valid.clone()
        }
        .validate_storage_contract(),
        Err("SSTORE control target_raw_gas must equal POP raw gas")
    );

    let invalid = OpcodeLabInput {
        bytecode: store_program(false, true),
        ..valid
    };
    assert_eq!(
        invalid.validate_storage_contract(),
        Err("control lane contains an undeclared storage opcode")
    );
}

#[test]
fn opcode_lab_storage_operation_shape_and_load_result_are_strict() {
    let missing_store_value = serde_json::from_str::<OpcodeLabInput>(&format!(
        r#"{{
          "case":"bad","scenario":"store","opcode":85,"target_count":0,
          "target_raw_gas":100,"bytecode":"0x00",
          "storage":{{"measurement_opcode":85,"lane":"target","slot":"{ZERO}",
          "original_value":"{ZERO}","access":"warm",
          "operation":{{"kind":"store","current_value":"{ZERO}"}}}}
        }}"#
    ));
    assert!(missing_store_value.is_err());

    let load_with_store_value = serde_json::from_str::<OpcodeLabInput>(&format!(
        r#"{{
          "case":"bad","scenario":"load","opcode":84,"target_count":0,
          "target_raw_gas":100,"bytecode":"0x00",
          "storage":{{"measurement_opcode":84,"lane":"target","slot":"{ZERO}",
          "original_value":"{ZERO}","access":"warm",
          "operation":{{"kind":"load","expected_value":"{ZERO}","new_value":"{ONE}"}}}}
        }}"#
    ));
    assert!(load_with_store_value.is_err());

    let expected_mismatch = storage_input(&format!(
        r#"{{
          "case":"bad","scenario":"load","opcode":84,"target_count":0,
          "target_raw_gas":100,"bytecode":"0x00",
          "storage":{{"measurement_opcode":84,"lane":"target","slot":"{ZERO}",
          "original_value":"{ONE}","access":"warm",
          "operation":{{"kind":"load","expected_value":"{TWO}"}}}}
        }}"#
    ));
    assert_eq!(
        expected_mismatch.validate_storage_contract(),
        Err("SLOAD expected_value differs from original_value")
    );
}

#[test]
fn opcode_lab_storage_words_require_canonical_32_byte_lower_hex() {
    for malformed in [
        "0x00",
        "0x000000000000000000000000000000000000000000000000000000000000000000",
        "-1",
        "0xgg00000000000000000000000000000000000000000000000000000000000000",
        "0xA000000000000000000000000000000000000000000000000000000000000000",
    ] {
        let result = serde_json::from_str::<OpcodeLabInput>(&format!(
            r#"{{
              "case":"bad","scenario":"load","opcode":84,"target_count":0,
              "target_raw_gas":100,"bytecode":"0x00",
              "storage":{{"measurement_opcode":84,"lane":"target","slot":"{malformed}",
              "original_value":"{ZERO}","access":"warm",
              "operation":{{"kind":"load","expected_value":"{ZERO}"}}}}
            }}"#
        ));
        assert!(result.is_err(), "accepted malformed word {malformed}");
    }
}

#[test]
fn opcode_lab_dirty_store_requires_warm_access_and_identical_declared_prefixes() {
    let target_program = store_program(true, true);
    let control_program = store_program(true, false);
    let storage = OpcodeLabStorageInput {
        measurement_opcode: 0x55,
        lane: OpcodeLabStorageLane::Target,
        slot: [0; 32],
        original_value: [0; 32],
        access: OpcodeLabStorageAccess::Warm,
        operation: OpcodeLabStorageOperation::Store {
            current_value: {
                let mut value = [0; 32];
                value[31] = 1;
                value
            },
            new_value: {
                let mut value = [0; 32];
                value[31] = 2;
                value
            },
        },
    };
    let target = OpcodeLabInput {
        opcode: 0x55,
        target_count: 1,
        bytecode: framed(std::slice::from_ref(&target_program)),
        generator_max_count: Some(1),
        tx_gas_limit: Some(100_000),
        fixed_bytecode_len: None,
        storage: Some(storage.clone()),
        ..OpcodeLabInput::default()
    };
    target
        .validate_controlled_contract()
        .expect("valid dirty target");

    let control = OpcodeLabInput {
        opcode: 0x50,
        target_count: 2,
        target_raw_gas: 2,
        bytecode: framed(std::slice::from_ref(&control_program)),
        storage: Some(OpcodeLabStorageInput {
            lane: OpcodeLabStorageLane::Control,
            ..storage.clone()
        }),
        ..target.clone()
    };
    control
        .validate_controlled_contract()
        .expect("valid dirty control");

    let cold = OpcodeLabInput {
        storage: Some(OpcodeLabStorageInput {
            access: OpcodeLabStorageAccess::Cold,
            ..storage.clone()
        }),
        ..target.clone()
    };
    assert_eq!(
        cold.validate_storage_contract(),
        Err("dirty SSTORE measured operation must be warm")
    );

    let mut wrong_prefix_program = target_program;
    wrong_prefix_program[32] = 2;
    let wrong_prefix = OpcodeLabInput {
        bytecode: framed(&[wrong_prefix_program]),
        ..target
    };
    assert_eq!(
        wrong_prefix.validate_storage_contract(),
        Err("SSTORE dirty prefix differs from declared current value")
    );
}

#[test]
fn opcode_lab_rejects_multiple_measured_storage_sites_in_one_microprogram() {
    let mut program = push32(0);
    program.extend([0x54, 0x50]);
    program.extend(push32(0));
    program.extend([0x54, 0x50, 0x00]);
    let input = OpcodeLabInput {
        opcode: 0x54,
        target_count: 2,
        bytecode: framed(&[program]),
        storage: Some(OpcodeLabStorageInput {
            measurement_opcode: 0x54,
            lane: OpcodeLabStorageLane::Target,
            slot: [0; 32],
            original_value: [0; 32],
            access: OpcodeLabStorageAccess::Warm,
            operation: OpcodeLabStorageOperation::Load {
                expected_value: [0; 32],
            },
        }),
        ..OpcodeLabInput::default()
    };

    assert_eq!(
        input.validate_storage_contract(),
        Err("microprogram contains multiple measured storage opcodes")
    );
}

#[test]
fn opcode_lab_control_binds_declared_reference_opcode_count_and_raw_gas() {
    let mut reference = push32(0);
    reference.extend([0x19, 0x50, 0x00]);
    let input = OpcodeLabInput {
        opcode: 0x19,
        target_count: 2,
        target_raw_gas: 3,
        bytecode: framed(&[reference.clone(), reference]),
        storage: Some(OpcodeLabStorageInput {
            measurement_opcode: 0x54,
            lane: OpcodeLabStorageLane::Control,
            slot: [0; 32],
            original_value: [0; 32],
            access: OpcodeLabStorageAccess::Warm,
            operation: OpcodeLabStorageOperation::Load {
                expected_value: [0; 32],
            },
        }),
        ..OpcodeLabInput::default()
    };
    input
        .validate_storage_contract()
        .expect("valid concrete SLOAD reference declaration");

    assert_eq!(
        OpcodeLabInput {
            opcode: 0x01,
            ..input.clone()
        }
        .validate_storage_contract(),
        Err("SLOAD control opcode must declare NOT")
    );
    assert_eq!(
        OpcodeLabInput {
            target_count: 1,
            ..input.clone()
        }
        .validate_storage_contract(),
        Err("control opcode count differs from target_count")
    );
    assert_eq!(
        OpcodeLabInput {
            target_raw_gas: 4,
            ..input
        }
        .validate_storage_contract(),
        Err("SLOAD control target_raw_gas must equal NOT raw gas")
    );
}
