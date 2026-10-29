#![cfg(feature = "fixture")]
use alloy_consensus::Transaction as _;
use raiko2_primitives_shasta::GuestInput;
use serde_json::{Value, json};
use std::{
    fs,
    process::{Command, Output},
};
use tempfile::TempDir;

fn request(code: &str, counts: Value) -> Value {
    json!({"schema_version":1,"program":{"code_hex":code,"input_hex":"0x","expected_route_counts":counts}})
}
fn command(dir: &TempDir) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_zkgas-fixture"));
    command
        .arg("--input")
        .arg(dir.path().join("request.json"))
        .arg("--output")
        .arg(dir.path().join("input.json"))
        .arg("--output-bincode")
        .arg(dir.path().join("input.bin"))
        .arg("--report")
        .arg(dir.path().join("report.json"));
    command
}
fn export(request: &Value) -> (Output, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("request.json"),
        serde_json::to_vec(request).unwrap(),
    )
    .unwrap();
    let output = command(&dir).output().unwrap();
    (output, dir)
}
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn report(dir: &TempDir) -> Value {
    serde_json::from_slice(&fs::read(dir.path().join("report.json")).unwrap()).unwrap()
}
fn input(dir: &TempDir) -> GuestInput {
    serde_json::from_slice(&fs::read(dir.path().join("input.json")).unwrap()).unwrap()
}
fn absent_outputs(dir: &TempDir) {
    for name in ["input.json", "input.bin", "report.json"] {
        assert!(
            !dir.path().join(name).exists(),
            "{name} was published after rejection"
        );
    }
}
#[test]
fn stop_return_and_revert_are_real_completed_terminal_instructions() {
    for (code, terminal, opcode, expected_success) in [
        ("0x00", "stop", "opcode:0x00", true),
        ("0x5f5ff3", "return", "opcode:0xf3", true),
        ("0x5f5ffd", "revert", "opcode:0xfd", false),
    ] {
        let mut value = request(code, json!({(opcode):1}));
        value["expected_user_success"] = expected_success.into();
        let (output, dir) = export(&value);
        success(&output);
        let report = report(&dir);
        assert_eq!(report["terminal_counts"][terminal], 1);
        assert_eq!(report["observed_route_counts"][opcode], 1);
        assert_eq!(
            report["receipts_by_block"][0][1]["success"],
            expected_success
        );
        let gas = report["receipts_by_block"][0][0]["gas_used"]
            .as_u64()
            .unwrap()
            + report["receipts_by_block"][0][1]["gas_used"]
                .as_u64()
                .unwrap();
        assert_eq!(report["blocks"][0]["block_gas_used"], gas);
    }
}
#[test]
fn native_counts_reject_opcode_bytes_inside_push_data_and_wrong_success() {
    // 0x09 is PUSH data, never an executed MULMOD instruction.
    for value in [
        request("0x60095000", json!({"opcode:0x09":1})),
        request("0x5f5ffd", json!({"opcode:0xfd":1})),
    ] {
        let (output, dir) = export(&value);
        assert!(!output.status.success());
        absent_outputs(&dir);
    }
}
#[test]
fn caught_child_revert_retains_child_work_in_counts() {
    // Empty calldata takes the outer branch; CALL to this same account supplies one byte,
    // so the child executes PUSH0/PUSH0/REVERT and the parent catches it then STOPs.
    let code = format!(
        "0x36156008575f5ffd5b5f5f60015f5f73{}620186a0f15000",
        "66".repeat(20)
    );
    let (output, dir) = export(&request(
        &code,
        json!({"opcode:0xf1":1,"opcode:0xfd":1,"opcode:0x00":1}),
    ));
    success(&output);
    let report = report(&dir);
    assert_eq!(report["terminal_counts"]["revert"], 1);
    assert_eq!(report["terminal_counts"]["stop"], 1);
    assert_eq!(report["receipts_by_block"][0][1]["success"], true);
}
#[test]
fn exceptional_instructions_do_not_satisfy_completed_counts() {
    let mut value = request("0x01", json!({"opcode:0x01":1})); // ADD with empty stack.
    value["expected_user_success"] = false.into();
    let (output, dir) = export(&value);
    assert!(!output.status.success());
    absent_outputs(&dir);
    value["program"]["expected_route_counts"]["opcode:0x01"] = 0.into();
    let (output, dir) = export(&value);
    success(&output);
    let report = report(&dir);
    assert_eq!(report["attempted_route_counts"]["opcode:0x01"], 1);
    assert!(report["observed_route_counts"].get("opcode:0x01").is_none());
    assert_eq!(
        report["observations_by_block"][0]["exceptional_instructions"],
        1
    );
}
#[test]
fn out_of_gas_is_attempted_without_counting_the_failed_store() {
    // Expand memory beyond what the fixed15M user gas envelope can pay for.
    let mut value = request("0x5f62ffffff52", json!({"opcode:0x52":1}));
    value["expected_user_success"] = false.into();
    let (output, dir) = export(&value);
    assert!(!output.status.success());
    absent_outputs(&dir);
    value["program"]["expected_route_counts"]["opcode:0x52"] = 0.into();
    let (output, dir) = export(&value);
    success(&output);
    let report = report(&dir);
    assert_eq!(report["attempted_route_counts"]["opcode:0x52"], 1);
    assert!(report["observed_route_counts"].get("opcode:0x52").is_none());
    assert_eq!(report["receipts_by_block"][0][1]["gas_used"], 15_000_000);
}
#[test]
fn raw_storage_preimages_produce_valid_mutation_multiproof() {
    // Read slot2, insert slot3=7, delete slot2, STOP.
    let mut value = request(
        "0x600254506007600355600060025500",
        json!({"opcode:0x54":1,"opcode:0x55":2,"opcode:0x00":1}),
    );
    value["program"]["storage_state"] = json!({"initial_storage":{"0x2":"0x5","0x9":"0xb"},"proof_slots":["0x2","0x3"],"insert_slots":["0x3"],"delete_slots":["0x2"]});
    let (output, dir) = export(&value);
    success(&output);
    let report = report(&dir);
    assert_eq!(report["storage_state"]["initial_storage_entry_count"], 2);
    assert_eq!(report["storage_state"]["insert_slot_count"], 1);
    assert_eq!(report["storage_state"]["delete_slot_count"], 1);
    let input = input(&dir);
    let quote = raiko2_zkgas_trace::quote_shasta_proposal(&input).unwrap();
    assert_eq!(
        serde_json::to_value(quote.public_output).unwrap(),
        report["public_output"]
    );
    assert_ne!(
        input.witnesses[0].block.header.state_root,
        input
            .proposal_ancestor_headers
            .last()
            .unwrap()
            .header
            .as_ref()
            .unwrap()
            .state_root
    );
    value["program"]["storage_state"]["proof_slots"] = json!(["0x3"]);
    let (output, dir) = export(&value);
    assert!(!output.status.success());
    absent_outputs(&dir);
}
#[test]
fn two_blocks_follow_actual_parent_base_fee_nonce_and_state() {
    let mut value = request("0x00", json!({"opcode:0x00":2}));
    value["program"]["block_count"] = 2.into();
    let (output, dir) = export(&value);
    success(&output);
    let input = input(&dir);
    let first = &input.witnesses[0].block;
    let second = &input.witnesses[1].block;
    assert_eq!(second.header.parent_hash, first.header.hash_slow());
    assert_eq!(first.body.transactions[1].nonce(), 0);
    assert_eq!(second.body.transactions[1].nonce(), 1);
    let expected =
        taiko_client_protocol::shasta::constants::calculate_next_block_eip4396_base_fee_for_parent(
            first.header.number,
            first.header.gas_limit,
            first.header.gas_used,
            first.header.timestamp,
            first.header.base_fee_per_gas,
            input
                .proposal_ancestor_headers
                .last()
                .unwrap()
                .header
                .as_ref()
                .unwrap()
                .timestamp,
            taiko_client_protocol::shasta::constants::min_base_fee_for_chain(167_000),
        )
        .unwrap();
    assert_eq!(second.header.base_fee_per_gas, Some(expected));
    assert_ne!(first.header.state_root, second.header.state_root);
}
#[test]
fn json_bincode_and_report_preserve_canonical_identity_and_new_output_policy() {
    let (output, dir) = export(&request("0x00", json!({"opcode:0x00":1})));
    success(&output);
    let bytes = fs::read(dir.path().join("input.bin")).unwrap();
    assert_eq!(bytes, bincode::serialize(&input(&dir)).unwrap());
    let report = report(&dir);
    assert_eq!(
        report["guest_input_sha256"],
        zkgas::identity::sha256_hex(&bytes)
    );
    assert_eq!(report["guest_input_bincode_length"], bytes.len());
    assert_eq!(report["public_values"].as_array().unwrap().len(), 32);
    let output = command(&dir).output().unwrap();
    assert!(!output.status.success());
    assert_eq!(fs::read(dir.path().join("input.bin")).unwrap(), bytes);
}
#[test]
fn existing_report_or_colliding_destinations_publish_nothing() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("request.json"),
        serde_json::to_vec(&request("0x00", json!({"opcode:0x00":1}))).unwrap(),
    )
    .unwrap();
    fs::write(dir.path().join("report.json"), b"original").unwrap();
    let output = command(&dir).output().unwrap();
    assert!(!output.status.success());
    assert_eq!(
        fs::read(dir.path().join("report.json")).unwrap(),
        b"original"
    );
    assert!(!dir.path().join("input.json").exists());
    fs::remove_file(dir.path().join("report.json")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_zkgas-fixture"))
        .arg("--input")
        .arg(dir.path().join("request.json"))
        .arg("--output")
        .arg(dir.path().join("same"))
        .arg("--report")
        .arg(dir.path().join("./same"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!dir.path().join("same").exists());
}
#[test]
fn malformed_and_bounded_requests_fail_before_construction() {
    let good = request("0x00", json!({"opcode:0x00":1}));
    for mutate in [
        |v: &mut Value| v["schema_version"] = 2.into(),
        |v: &mut Value| v["program"]["block_count"] = 33.into(),
        |v: &mut Value| v["program"]["code_hex"] = "00".into(),
        |v: &mut Value| v["program"]["expected_route_counts"] = json!({"opcode:0xFF":1}),
        |v: &mut Value| v["wallet_path"] = "unsupported".into(),
    ] {
        let mut value = good.clone();
        mutate(&mut value);
        assert!(zkgas::fixture::parse_request(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    assert!(zkgas::fixture::parse_request(&vec![b' '; 4 * 1024 * 1024 + 1]).is_err());
}
#[test]
fn directory_and_symlink_requests_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    fs::write(&target, b"{}").unwrap();
    let link = dir.path().join("request.json");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let output = command(&dir).output().unwrap();
    assert!(!output.status.success());
    absent_outputs(&dir);
    fs::remove_file(link).unwrap();
    fs::create_dir(dir.path().join("request.json")).unwrap();
    let output = command(&dir).output().unwrap();
    assert!(!output.status.success());
    absent_outputs(&dir);
}
