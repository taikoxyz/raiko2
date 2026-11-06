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
fn export_with_profile(request: &Value, profile: &str) -> (Output, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("request.json"),
        serde_json::to_vec(request).unwrap(),
    )
    .unwrap();
    let output = command(&dir)
        .args(["--execution-profile", profile])
        .output()
        .unwrap();
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
fn explicit_profile_admits_work_above_legacy_cap_in_every_block() {
    use raiko2_zkgas_trace::{ExecutionProfile, quote_shasta_proposal_with_execution_profile};

    // Two MiB expansion costs less than 15M EVM gas but more than 100M legacy
    // Unzen units. The same request must retain the default legacy rejection.
    let mut value = request("0x5f621fffff5200", json!({"opcode:0x52":2,"opcode:0x00":2}));
    value["program"]["block_count"] = 2.into();
    let (legacy, legacy_dir) = export(&value);
    assert!(!legacy.status.success());
    absent_outputs(&legacy_dir);

    let (experimental, dir) = export_with_profile(&value, "evm-gas-only");
    success(&experimental);
    let report = report(&dir);
    assert_eq!(report["execution_profile"], "evm-gas-only");
    assert_eq!(report["observed_route_counts"]["opcode:0x52"], 2);
    assert_eq!(report["observed_route_counts"]["opcode:0x00"], 2);
    let input = input(&dir);
    for witness in &input.witnesses {
        assert!(witness.block.header.difficulty > alloy_primitives::U256::from(100_000_000));
        assert!(witness.block.header.gas_used < 15_500_000);
    }
    assert!(raiko2_zkgas_trace::quote_shasta_proposal(&input).is_err());
    let quote =
        quote_shasta_proposal_with_execution_profile(&input, ExecutionProfile::EvmGasOnly).unwrap();
    assert_eq!(quote.execution_profile, ExecutionProfile::EvmGasOnly);
    assert_eq!(quote.blocks.len(), 2);
    for block in &quote.blocks {
        assert!(block.node.unzen_zkgas > 100_000_000);
    }
    assert_eq!(
        serde_json::to_value(quote.public_output).unwrap(),
        report["public_output"]
    );

    let quote_path = dir.path().join("experimental-quote.jsonl");
    let output = Command::new(env!("CARGO_BIN_EXE_zkgas-quote"))
        .arg("--input-bincode")
        .arg(dir.path().join("input.bin"))
        .args([
            "--corpus-id",
            "profile-test",
            "--sample-id",
            "above-legacy-cap",
        ])
        .args(["--execution-profile", "evm-gas-only"])
        .arg("--output")
        .arg(&quote_path)
        .output()
        .unwrap();
    success(&output);
    let envelope: Value = serde_json::from_slice(&fs::read(quote_path).unwrap()).unwrap();
    assert_eq!(envelope["execution_profile"], "evm-gas-only");
    assert_eq!(envelope["quote"]["execution_profile"], "evm-gas-only");
    assert_eq!(envelope["quote"]["public_output"], report["public_output"]);

    let mut invalid_state = input;
    invalid_state.witnesses[0].block.header.state_root = alloy_primitives::B256::ZERO;
    assert!(
        quote_shasta_proposal_with_execution_profile(&invalid_state, ExecutionProfile::EvmGasOnly,)
            .is_err()
    );
}

#[test]
fn explicit_legacy_profile_preserves_default_bytes_and_wire_shape() {
    let value = request("0x00", json!({"opcode:0x00":1}));
    let (default, default_dir) = export(&value);
    let (explicit, explicit_dir) = export_with_profile(&value, "legacy-unzen");
    success(&default);
    success(&explicit);
    for filename in ["input.bin", "input.json", "report.json"] {
        assert_eq!(
            fs::read(default_dir.path().join(filename)).unwrap(),
            fs::read(explicit_dir.path().join(filename)).unwrap(),
            "default/explicit legacy changed {filename}"
        );
    }
    assert!(report(&default_dir).get("execution_profile").is_none());
    let input = input(&default_dir);
    let quote = raiko2_zkgas_trace::quote_shasta_proposal(&input).unwrap();
    assert!(
        serde_json::to_value(quote)
            .unwrap()
            .get("execution_profile")
            .is_none()
    );
}

#[test]
fn experimental_admission_preserves_evm_gas_failure_and_success_checks() {
    let mut value = request("0x5f62ffffff52", json!({"opcode:0x52":0}));
    let (unexpected_failure, dir) = export_with_profile(&value, "evm-gas-only");
    assert!(!unexpected_failure.status.success());
    absent_outputs(&dir);
    value["expected_user_success"] = false.into();
    let (expected_failure, dir) = export_with_profile(&value, "evm-gas-only");
    success(&expected_failure);
    let report = report(&dir);
    assert_eq!(report["execution_profile"], "evm-gas-only");
    assert_eq!(report["receipts_by_block"][0][1]["success"], false);
    assert_eq!(report["receipts_by_block"][0][1]["gas_used"], 15_000_000);
    assert_eq!(report["attempted_route_counts"]["opcode:0x52"], 1);
    assert!(report["observed_route_counts"].get("opcode:0x52").is_none());
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
    assert!(report["storage_state"].get("final_storage_root").is_none());
    assert!(
        report["storage_state"]
            .get("final_storage_entry_count")
            .is_none()
    );
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

#[test]
fn authorization_request_schema_accepts_bounded_patterns_and_rejects_malformed_lists() {
    for pattern in ["sequential", "repeated", "distinct", "compact"] {
        for count in [1, 1024] {
            let mut value = request("0x60006000f3", json!({"opcode:0xf3":1}));
            value["program"]["authorizations"] = json!({"pattern":pattern,"count":count});
            assert!(zkgas::fixture::parse_request(&serde_json::to_vec(&value).unwrap()).is_ok());
        }
    }
    for auth in [
        json!({"pattern":"sequential","count":0}),
        json!({"pattern":"sequential","count":1025}),
        json!({"pattern":"unknown","count":1}),
        json!({"pattern":"sequential","count":true}),
        json!({"pattern":"sequential","count":1,"extra":true}),
        json!({"pattern":"sequential"}),
        json!([]),
    ] {
        let mut value = request("0x60006000f3", json!({"opcode:0xf3":1}));
        value["program"]["authorizations"] = auth;
        assert!(zkgas::fixture::parse_request(&serde_json::to_vec(&value).unwrap()).is_err());
    }
}

#[test]
fn authorization_patterns_execute_canonical_type4_with_two_block_nonce_progression() {
    use alloy_primitives::{Address, keccak256};
    let delegation = [vec![0xef, 0x01, 0x00], vec![0x66; 20]].concat();
    for pattern in ["sequential", "repeated", "distinct"] {
        let mut value = request("0x60006000f3", json!({"opcode:0xf3":2}));
        value["program"]["block_count"] = 2.into();
        value["program"]["authorizations"] = json!({"pattern":pattern,"count":4});
        let (output, dir) = export_with_profile(&value, "evm-gas-only");
        success(&output);
        let report = report(&dir);
        let input = input(&dir);
        assert_eq!(report["authorizations"]["pattern"], pattern);
        assert_eq!(report["authorizations"]["entries_per_block"], 4);
        assert_eq!(
            report["authorizations"]["blocks"].as_array().unwrap().len(),
            2
        );
        let mut prior_authorities = Vec::new();
        for (index, witness) in input.witnesses.iter().enumerate() {
            let transaction = &witness.block.body.transactions[1];
            let tx = transaction
                .as_eip7702()
                .expect("canonical type-4 transaction")
                .tx();
            assert_eq!(tx.authorization_list.len(), 4);
            assert_eq!(tx.to, Address::repeat_byte(0x66));
            assert_eq!(tx.gas_limit, 15_000_000);
            assert_eq!(tx.chain_id, 167_000);
            assert_eq!(tx.nonce, index as u64);
            assert_eq!(tx.max_fee_per_gas, 25_000_000);
            assert_eq!(tx.max_priority_fee_per_gas, 0);
            assert_eq!(tx.value, alloy_primitives::U256::ZERO);
            assert!(tx.input.is_empty());
            let recovered = tx
                .authorization_list
                .iter()
                .enumerate()
                .map(|(i, auth)| {
                    assert_eq!(auth.inner().address, Address::repeat_byte(0x66));
                    assert_eq!(auth.inner().chain_id, alloy_primitives::U256::from(167_000));
                    assert_eq!(
                        auth.inner().nonce,
                        if pattern == "sequential" {
                            (index * 4 + i) as u64
                        } else {
                            index as u64
                        }
                    );
                    auth.recover_authority().unwrap()
                })
                .collect::<Vec<_>>();
            if index == 1 {
                assert_eq!(recovered, prior_authorities);
            }
            prior_authorities = recovered.clone();
            let block_report = &report["authorizations"]["blocks"][index];
            assert_eq!(block_report["signed_entries"], 4);
            assert_eq!(
                block_report["applied_delegations"],
                if pattern == "repeated" { 1 } else { 4 }
            );
            let authorities = block_report["authorities"].as_array().unwrap();
            assert_eq!(authorities.len(), if pattern == "distinct" { 4 } else { 1 });
            for authority in authorities {
                let address: Address =
                    serde_json::from_value(authority["address"].clone()).unwrap();
                assert!(recovered.contains(&address));
                assert_eq!(
                    authority["nonce"],
                    if pattern == "sequential" {
                        ((index + 1) * 4) as u64
                    } else {
                        (index + 1) as u64
                    }
                );
                assert_eq!(
                    authority["code_hash"],
                    serde_json::to_value(keccak256(&delegation)).unwrap()
                );
            }
            assert_eq!(
                witness.witness.codes.contains(&delegation.clone().into()),
                index == 1
            );
        }
        assert!(
            report["receipts_by_block"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r[1]["success"] == true)
        );
    }
}

#[test]
fn omitted_authorizations_match_frozen_native_default_bytes_and_report() {
    let Ok(frozen) = std::env::var("FROZEN_NATIVE_FIXTURE_BIN") else {
        return;
    };
    let value = request("0x60006000f3", json!({"opcode:0xf3":1}));
    let (output, current_dir) = export_with_profile(&value, "evm-gas-only");
    success(&output);
    let frozen_dir = tempfile::tempdir().unwrap();
    fs::write(
        frozen_dir.path().join("request.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    let output = Command::new(frozen)
        .arg("--input")
        .arg(frozen_dir.path().join("request.json"))
        .arg("--output")
        .arg(frozen_dir.path().join("input.json"))
        .arg("--output-bincode")
        .arg(frozen_dir.path().join("input.bin"))
        .arg("--report")
        .arg(frozen_dir.path().join("report.json"))
        .args(["--execution-profile", "evm-gas-only"])
        .output()
        .unwrap();
    success(&output);
    for name in ["input.bin", "input.json", "report.json"] {
        assert_eq!(
            fs::read(current_dir.path().join(name)).unwrap(),
            fs::read(frozen_dir.path().join(name)).unwrap(),
            "default changed {name}"
        );
    }
    assert!(report(&current_dir).get("authorizations").is_none());
}

#[test]
fn single_entry_authorization_patterns_share_canonical_guest_input() {
    let mut canonical = None;
    for pattern in ["sequential", "repeated", "distinct"] {
        let mut value = request("0x60006000f3", json!({"opcode:0xf3":1}));
        value["program"]["authorizations"] = json!({"pattern":pattern,"count":1});
        let (output, dir) = export_with_profile(&value, "evm-gas-only");
        success(&output);
        let bytes = fs::read(dir.path().join("input.bin")).unwrap();
        if let Some(expected) = &canonical {
            assert_eq!(&bytes, expected);
        }
        canonical = Some(bytes);
    }
}

#[test]
fn authorization_fixture_preserves_program_storage_multiproof_behavior() {
    let mut value = request(
        "0x600254506007600355600060025500",
        json!({"opcode:0x54":1,"opcode:0x55":2,"opcode:0x00":1}),
    );
    value["program"]["authorizations"] = json!({"pattern":"distinct","count":4});
    value["program"]["storage_state"] = json!({"initial_storage":{"0x2":"0x5","0x9":"0xb"},"proof_slots":["0x2","0x3"],"insert_slots":["0x3"],"delete_slots":["0x2"]});
    let (output, dir) = export_with_profile(&value, "evm-gas-only");
    success(&output);
    let report = report(&dir);
    assert_eq!(report["storage_state"]["insert_slot_count"], 1);
    assert_eq!(report["storage_state"]["delete_slot_count"], 1);
    assert_eq!(
        report["authorizations"]["blocks"][0]["applied_delegations"],
        4
    );
}

#[test]
fn authorization_delegations_survive_a_user_program_revert() {
    let mut value = request("0x60006000fd", json!({"opcode:0xfd":1}));
    value["expected_user_success"] = false.into();
    value["program"]["authorizations"] = json!({"pattern":"repeated","count":4});
    let (output, dir) = export_with_profile(&value, "evm-gas-only");
    success(&output);
    let report = report(&dir);
    assert_eq!(report["receipts_by_block"][0][1]["success"], false);
    assert_eq!(
        report["authorizations"]["blocks"][0]["applied_delegations"],
        1
    );
    assert_eq!(
        report["authorizations"]["blocks"][0]["authorities"][0]["nonce"],
        1
    );
}

#[test]
fn compact_authorizations_have_short_canonical_rlp_and_stale_three_block_state() {
    use alloy_eips::eip2718::{Decodable2718, Encodable2718};
    use alloy_primitives::{Address, U256, keccak256};
    let mut value = request("0x60006000f3", json!({"opcode:0xf3":3,"opcode:0xfa":0}));
    value["program"]["block_count"] = 3.into();
    value["program"]["authorizations"] = json!({"pattern":"compact","count":16});
    let (output, dir) = export_with_profile(&value, "evm-gas-only");
    success(&output);
    let input = input(&dir);
    let report = report(&dir);
    let delegation = [vec![0xef, 0x01, 0x00], vec![0x66; 20]].concat();
    let expected_rlp = hex::decode(format!("dd83028c5894{}80800101", "66".repeat(20))).unwrap();
    let mut authority = None;
    let mut signed_entries = 0;
    for (index, witness) in input.witnesses.iter().enumerate() {
        let transaction = &witness.block.body.transactions[1];
        let bytes = transaction.encoded_2718();
        assert_eq!(bytes[0], 4);
        let mut slice = bytes.as_slice();
        assert_eq!(
            reth_ethereum_primitives::TransactionSigned::decode_2718(&mut slice).unwrap(),
            *transaction
        );
        assert!(slice.is_empty());
        let tx = transaction.as_eip7702().unwrap().tx();
        assert_eq!(tx.authorization_list.len(), 16);
        for auth in &tx.authorization_list {
            assert_eq!(auth.inner().chain_id, U256::from(167_000));
            assert_eq!(auth.inner().address, Address::repeat_byte(0x66));
            assert_eq!(auth.inner().nonce, 0);
            assert_eq!(auth.r(), U256::from(1));
            assert_eq!(auth.s(), U256::from(1));
            assert_eq!(auth.y_parity(), 0);
            assert_eq!(alloy_rlp::encode(auth), expected_rlp);
            let recovered = auth.recover_authority().unwrap();
            if let Some(expected) = authority {
                assert_eq!(recovered, expected);
            }
            authority = Some(recovered);
        }
        signed_entries += tx.authorization_list.len();
        let block = &report["authorizations"]["blocks"][index];
        assert_eq!(block["signed_entries"], 16);
        assert_eq!(block["applied_delegations"], if index == 0 { 1 } else { 0 });
        assert_eq!(block["authorities"].as_array().unwrap().len(), 1);
        assert_eq!(block["authorities"][0]["nonce"], 1);
        assert_eq!(
            block["authorities"][0]["previous_nonce"],
            if index == 0 { 0 } else { 1 }
        );
        assert_eq!(
            block["authorities"][0]["address"],
            serde_json::to_value(authority.unwrap()).unwrap()
        );
        assert_eq!(
            block["authorities"][0]["code_hash"],
            serde_json::to_value(keccak256(&delegation)).unwrap()
        );
        assert_eq!(
            witness.witness.codes.contains(&delegation.clone().into()),
            index != 0
        );
        let mut expected_codes = input.witnesses[0].witness.codes.clone();
        if index != 0 {
            // The existing producer retains both actual bundle contracts: the
            // empty code and delegation marker. No other code is added or cloned.
            expected_codes.push(alloy_primitives::Bytes::new());
            expected_codes.push(delegation.clone().into());
        }
        assert_eq!(witness.witness.codes, expected_codes);
        assert!(
            report["receipts_by_block"][index][1]["success"]
                .as_bool()
                .unwrap()
        );
    }
    assert_eq!(signed_entries, 48);
    assert_eq!(expected_rlp.len(), 30);
}

#[test]
fn original_authorization_patterns_match_frozen_native_inputs_and_reports() {
    let Ok(frozen) = std::env::var("FROZEN_AUTHORIZATION_NATIVE_FIXTURE_BIN") else {
        return;
    };
    for pattern in ["sequential", "repeated", "distinct"] {
        let mut value = request("0x60006000f3", json!({"opcode:0xf3":2}));
        value["program"]["block_count"] = 2.into();
        value["program"]["authorizations"] = json!({"pattern":pattern,"count":4});
        let (output, current_dir) = export_with_profile(&value, "evm-gas-only");
        success(&output);
        let frozen_dir = tempfile::tempdir().unwrap();
        fs::write(
            frozen_dir.path().join("request.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        let output = Command::new(&frozen)
            .arg("--input")
            .arg(frozen_dir.path().join("request.json"))
            .arg("--output")
            .arg(frozen_dir.path().join("input.json"))
            .arg("--output-bincode")
            .arg(frozen_dir.path().join("input.bin"))
            .arg("--report")
            .arg(frozen_dir.path().join("report.json"))
            .args(["--execution-profile", "evm-gas-only"])
            .output()
            .unwrap();
        success(&output);
        for name in ["input.bin", "input.json", "report.json"] {
            assert_eq!(
                fs::read(current_dir.path().join(name)).unwrap(),
                fs::read(frozen_dir.path().join(name)).unwrap(),
                "{pattern} changed {name}"
            );
        }
    }
}

#[test]
fn access_lists_preserve_raw_duplicates_and_report_included_signed_entries() {
    let mut value = request("0x60006000f3", json!({"opcode:0xf3":1}));
    value["program"]["access_list"] = json!({"pattern":"repeated_address","count":4});
    let (output, dir) = export_with_profile(&value, "evm-gas-only");
    success(&output);
    let actual = input(&dir);
    let list = actual.witnesses[0].block.body.transactions[1]
        .access_list()
        .unwrap();
    assert_eq!(list.0.len(), 4);
    assert!(list.0.windows(2).all(|pair| pair[0] == pair[1]));
    let report = report(&dir);
    let block = &report["access_list"]["blocks"][0];
    assert_eq!(block["raw_address_items"], 4);
    assert_eq!(block["raw_storage_keys"], 0);
    assert_eq!(block["unique_addresses"], 1);
    assert_eq!(block["unique_storage_pairs"], 0);
}

#[test]
fn access_list_descriptors_are_typed_and_do_not_raise_intrinsic_gas_limits() {
    for descriptor in [
        json!([]),
        json!({}),
        json!({"pattern":"unknown","count":1}),
        json!({"pattern":"repeated_key","count":0}),
        json!({"pattern":"repeated_key","count":8193}),
        json!({"pattern":"repeated_key","count":true}),
        json!({"pattern":"repeated_key","count":"1"}),
        json!({"pattern":"repeated_key","count":1,"extra":0}),
    ] {
        let mut value = request("0x60006000f3", json!({"opcode:0xf3":1}));
        value["program"]["access_list"] = descriptor;
        let (output, dir) = export(&value);
        assert!(!output.status.success());
        absent_outputs(&dir);
    }
    let mut value = request("0x60006000f3", json!({"opcode:0xf3":1}));
    value["program"]["access_list"] = json!({"pattern":"repeated_key","count":8192});
    assert!(zkgas::fixture::parse_request(&serde_json::to_vec(&value).unwrap()).is_ok());
    let intrinsic = reth_revm::context_interface::cfg::GasParams::new_spec(
        reth_revm::primitives::hardfork::SpecId::OSAKA,
    )
    .initial_tx_gas(&[], false, 1, 8192, 0, None);
    assert!(intrinsic.initial_regular_gas > 15_000_000);
    let (output, dir) = export_with_profile(&value, "evm-gas-only");
    assert!(
        !output.status.success(),
        "8192 keys exceed the real 15M intrinsic budget"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("fixture must execute exactly one Anchor and one user transaction"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    absent_outputs(&dir);
}

#[test]
fn access_list_patterns_preserve_order_grouping_and_authorization_composition() {
    use alloy_eips::{Decodable2718, Encodable2718};
    use alloy_primitives::{Address, B256, U256};
    use std::collections::BTreeSet;
    for (pattern, raw_addresses, raw_keys, addresses, pairs) in [
        ("repeated_address", 4, 0, 1, 0),
        ("distinct_addresses", 4, 0, 4, 0),
        ("repeated_key", 1, 4, 1, 1),
        ("distinct_keys", 1, 4, 1, 4),
        ("repeated_items", 4, 4, 1, 1),
        ("reversed_distinct_keys", 1, 4, 1, 4),
    ] {
        for authorizations in [false, true] {
            let mut value = request("0x60006000f3", json!({"opcode:0xf3":3}));
            value["program"]["block_count"] = 3.into();
            value["program"]["access_list"] = json!({"pattern":pattern,"count":4});
            if authorizations {
                value["program"]["authorizations"] = json!({"pattern":"compact","count":4});
            }
            let (output, dir) = export_with_profile(&value, "evm-gas-only");
            success(&output);
            let input = input(&dir);
            let report = report(&dir);
            assert_eq!(report["access_list"]["pattern"], pattern);
            assert_eq!(report["access_list"]["count_per_block"], 4);
            for (index, witness) in input.witnesses.iter().enumerate() {
                let tx = &witness.block.body.transactions[1];
                assert_eq!(tx.gas_limit(), 15_000_000);
                assert_eq!(tx.nonce(), index as u64);
                assert_eq!(tx.as_eip7702().is_some(), authorizations);
                let encoded = tx.encoded_2718();
                let mut bytes = encoded.as_slice();
                let decoded =
                    reth_ethereum_primitives::TransactionSigned::decode_2718(&mut bytes).unwrap();
                assert_eq!(&decoded, tx);
                assert!(bytes.is_empty());
                let list = decoded.access_list().unwrap();
                assert_eq!(list.0.len(), raw_addresses);
                assert_eq!(
                    list.0.iter().map(|a| a.storage_keys.len()).sum::<usize>(),
                    raw_keys
                );
                assert_eq!(
                    list.0
                        .iter()
                        .map(|a| a.address)
                        .collect::<BTreeSet<_>>()
                        .len(),
                    addresses
                );
                assert_eq!(
                    list.0
                        .iter()
                        .flat_map(|a| a.storage_keys.iter().map(move |key| (a.address, *key)))
                        .collect::<BTreeSet<_>>()
                        .len(),
                    pairs
                );
                assert_eq!(list.0[0].address, Address::repeat_byte(0x66));
                if pattern == "reversed_distinct_keys" {
                    assert_eq!(
                        list.0[0].storage_keys[0],
                        B256::from(U256::from(4).to_be_bytes::<32>())
                    );
                }
                if pattern == "distinct_keys" {
                    assert_eq!(
                        list.0[0].storage_keys[0],
                        B256::from(U256::from(1).to_be_bytes::<32>())
                    );
                }
                let block = &report["access_list"]["blocks"][index];
                assert_eq!(block["block_index"], index);
                assert_eq!(
                    block["transaction_type"],
                    if authorizations { 4 } else { 2 }
                );
                assert_eq!(block["raw_address_items"], raw_addresses);
                assert_eq!(block["raw_storage_keys"], raw_keys);
                assert_eq!(block["unique_addresses"], addresses);
                assert_eq!(block["unique_storage_pairs"], pairs);
                assert_eq!(report["receipts_by_block"][index][1]["success"], true);
            }
        }
    }
}

#[test]
fn access_list_count_one_shapes_share_bytes_and_do_not_preload_state() {
    let mut reference_nodes = None;
    for patterns in [
        vec!["repeated_address", "distinct_addresses"],
        vec![
            "repeated_key",
            "distinct_keys",
            "repeated_items",
            "reversed_distinct_keys",
        ],
    ] {
        let mut canonical = None;
        for pattern in patterns {
            let mut value = request("0x60006000f3", json!({"opcode:0xf3":1}));
            value["program"]["access_list"] = json!({"pattern":pattern,"count":1});
            let (output, dir) = export_with_profile(&value, "evm-gas-only");
            success(&output);
            let input = input(&dir);
            let bytes = fs::read(dir.path().join("input.bin")).unwrap();
            if let Some(previous) = &canonical {
                assert_eq!(&bytes, previous);
            } else {
                canonical = Some(bytes);
            }
            if let Some(nodes) = &reference_nodes {
                assert_eq!(
                    &(
                        input.proposal_state_nodes.clone(),
                        input.witnesses[0].witness.clone()
                    ),
                    nodes
                );
            } else {
                assert!(
                    !input.proposal_state_nodes.is_empty()
                        || !input.witnesses[0].witness.state.is_empty()
                );
                reference_nodes = Some((
                    input.proposal_state_nodes.clone(),
                    input.witnesses[0].witness.clone(),
                ));
            }
        }
    }
    let value = request("0x60006000f3", json!({"opcode:0xf3":1}));
    let (output, dir) = export_with_profile(&value, "evm-gas-only");
    success(&output);
    assert_eq!(
        Some((
            input(&dir).proposal_state_nodes.clone(),
            input(&dir).witnesses[0].witness.clone()
        )),
        reference_nodes
    );
}

#[test]
fn omitted_access_list_preserves_frozen_input_bytes_and_stable_native_fields() {
    let Ok(frozen) = std::env::var("FROZEN_ACCESS_LIST_NATIVE_FIXTURE_BIN") else {
        return;
    };
    for authorizations in [false, true] {
        let mut value = request("0x60006000f3", json!({"opcode:0xf3":2}));
        value["program"]["block_count"] = 2.into();
        if authorizations {
            value["program"]["authorizations"] = json!({"pattern":"compact","count":4});
        }
        let (output, dir) = export_with_profile(&value, "evm-gas-only");
        success(&output);
        let old = tempfile::tempdir().unwrap();
        fs::write(
            old.path().join("request.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        let output = Command::new(&frozen)
            .args(["--execution-profile", "evm-gas-only"])
            .arg("--input")
            .arg(old.path().join("request.json"))
            .arg("--output")
            .arg(old.path().join("input.json"))
            .arg("--output-bincode")
            .arg(old.path().join("input.bin"))
            .arg("--report")
            .arg(old.path().join("report.json"))
            .output()
            .unwrap();
        success(&output);
        for name in ["input.bin", "input.json"] {
            assert_eq!(
                fs::read(dir.path().join(name)).unwrap(),
                fs::read(old.path().join(name)).unwrap()
            );
        }
        let current = report(&dir);
        let previous = report(&old);
        assert!(current.get("access_list").is_none());
        for key in [
            "guest_input_sha256",
            "guest_input_bincode_length",
            "public_output",
            "observed_route_counts",
            "receipts_by_block",
            "authorizations",
        ] {
            assert_eq!(
                current.get(key),
                previous.get(key),
                "stable native field {key}"
            );
        }
    }
}

fn transaction_request(
    kind: &str,
    count: usize,
    gas: u64,
    code: &str,
    counts: Value,
    runtime: Option<&str>,
) -> Value {
    let mut value = request(code, counts);
    value["program"]["transactions"] = json!({"kind":kind,"count_per_block":count,"gas_limit":gas});
    if let Some(runtime) = runtime {
        value["program"]["transactions"]["expected_runtime_hex"] = runtime.into();
    }
    value
}

#[test]
fn transaction_descriptor_repeated_calls_observe_all_users_and_sum_receipt_gas() {
    let mut value = transaction_request(
        "call",
        3,
        100_000,
        "0x60006000f3",
        json!({"opcode:0xf3":6}),
        None,
    );
    value["program"]["block_count"] = 2.into();
    let (output, dir) = export_with_profile(&value, "evm-gas-only");
    success(&output);
    let report = report(&dir);
    let input = input(&dir);
    assert_eq!(report["transaction_count"], 8);
    assert_eq!(report["transactions"]["kind"], "call");
    assert_eq!(report["transactions"]["count_per_block"], 3);
    assert_eq!(report["observed_route_counts"]["opcode:0xf3"], 6);
    let mut gas = 0;
    for (block_index, witness) in input.witnesses.iter().enumerate() {
        assert_eq!(witness.block.body.transactions.len(), 4);
        let receipts = report["receipts_by_block"][block_index].as_array().unwrap();
        let expected: u64 = receipts[1..]
            .iter()
            .map(|r| r["gas_used"].as_u64().unwrap())
            .sum();
        assert_eq!(
            report["blocks"][block_index]["user_transaction_gas_used"],
            expected
        );
        gas += expected;
        for (index, tx) in witness.block.body.transactions[1..].iter().enumerate() {
            assert_eq!(tx.nonce(), (block_index * 3 + index) as u64);
            assert_eq!(tx.gas_limit(), 100_000);
            assert_eq!(tx.to(), Some(alloy_primitives::Address::repeat_byte(0x66)));
            assert_eq!(tx.max_fee_per_gas(), 1_000_000_000);
            assert_eq!(
                report["transactions"]["blocks"][block_index]["users"][index]["nonce"],
                tx.nonce()
            );
            assert_eq!(
                report["transactions"]["blocks"][block_index]["users"][index]["transaction_hash"],
                serde_json::to_value(tx.hash()).unwrap()
            );
        }
    }
    assert_eq!(report["user_transaction_gas_used_total"], gas);
}

fn storage_transaction_request(count: usize) -> Value {
    // Increment the shared, initialized phase slot once per user, retaining background slot9.
    let mut value = transaction_request(
        "call",
        count,
        100_000,
        "0x5f546001015f5500",
        json!({"opcode:0x54":count,"opcode:0x55":count,"opcode:0x00":count}),
        None,
    );
    value["program"]["storage_state"] = json!({
        "initial_storage":{"0x0":"0x1","0x9":"0xb"},
        "proof_slots":["0x0"]
    });
    value
}

#[test]
fn storage_transaction_descriptor_calls_share_initialized_state_for_one_two_and_eight_users() {
    for count in [1, 2, 8] {
        let value = storage_transaction_request(count);
        let (output, dir) = export_with_profile(&value, "evm-gas-only");
        success(&output);
        let report = report(&dir);
        let input = input(&dir);
        assert_eq!(input.witnesses.len(), 1);
        assert_eq!(input.witnesses[0].block.body.transactions.len(), count + 1);
        assert_eq!(report["storage_state"]["initial_storage_entry_count"], 2);
        assert_eq!(report["storage_state"]["proof_slot_count"], 1);
        // Rebuild the expected full storage independently of the fixture's mutable trie.
        let expected_root = alloy_trie::root::storage_root_unhashed([
            (
                alloy_primitives::B256::ZERO,
                alloy_primitives::U256::from(count + 1),
            ),
            (
                alloy_primitives::B256::from(alloy_primitives::U256::from(9).to_be_bytes::<32>()),
                alloy_primitives::U256::from(11),
            ),
        ]);
        assert_eq!(report["storage_state"]["final_storage_entry_count"], 2);
        assert_eq!(
            report["storage_state"]["final_storage_root"],
            serde_json::to_value(expected_root).unwrap()
        );
        assert_eq!(report["observed_route_counts"]["opcode:0x54"], count);
        assert_eq!(report["observed_route_counts"]["opcode:0x55"], count);
        for (index, tx) in input.witnesses[0].block.body.transactions[1..]
            .iter()
            .enumerate()
        {
            assert_eq!(tx.nonce(), index as u64);
            assert_eq!(tx.to(), Some(alloy_primitives::Address::repeat_byte(0x66)));
            assert_eq!(report["receipts_by_block"][0][index + 1]["success"], true);
        }
        let quote = raiko2_zkgas_trace::quote_shasta_proposal(&input).unwrap();
        assert_eq!(
            serde_json::to_value(quote.public_output).unwrap(),
            report["public_output"]
        );
    }
}

#[test]
fn validation_only_output_matches_measured_quote_and_rejects_changed_canonical_block() {
    let (output, dir) = export(&storage_transaction_request(2));
    success(&output);
    let mut input = input(&dir);
    let profile = raiko2_zkgas_trace::ExecutionProfile::LegacyUnzen;
    let public_output =
        raiko2_zkgas_trace::validate_shasta_proposal_with_execution_profile(&input, profile)
            .unwrap();
    let quote = raiko2_zkgas_trace::quote_shasta_proposal(&input).unwrap();
    assert_eq!(quote.public_output, public_output);
    assert_eq!(
        serde_json::to_value(public_output).unwrap(),
        report(&dir)["public_output"]
    );
    assert!(quote.blocks[0].prove.mpt.path_node_visits > 0);
    input.witnesses[0].block.header.state_root = alloy_primitives::B256::ZERO;
    assert!(
        raiko2_zkgas_trace::validate_shasta_proposal_with_execution_profile(&input, profile)
            .is_err()
    );
    assert!(raiko2_zkgas_trace::quote_shasta_proposal(&input).is_err());
}

#[test]
fn retained_quote_preserves_every_measured_selector_and_price() {
    let Some(path) = std::env::var_os("ZKGAS_RETAINED_INPUT_JSON") else {
        return;
    };
    let quote_path = std::env::var_os("ZKGAS_RETAINED_QUOTE_JSONL").expect("retained quote path");
    let input: GuestInput = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let zkgas::contracts::QuoteRowV1::Complete(retained) =
        serde_json::from_slice(&fs::read(quote_path).unwrap()).unwrap()
    else {
        panic!("retained quote must be complete");
    };
    let profile = retained.execution_profile;
    let quote =
        raiko2_zkgas_trace::quote_shasta_proposal_with_execution_profile(&input, profile).unwrap();
    assert_eq!(quote, retained.quote);
    let public_output =
        raiko2_zkgas_trace::validate_shasta_proposal_with_execution_profile(&input, profile)
            .unwrap();
    assert_eq!(public_output, quote.public_output);
    assert!(
        quote
            .blocks
            .iter()
            .any(|block| block.prove.mpt.path_node_visits > 0)
    );
}

#[test]
fn storage_transaction_descriptor_preserves_gas_bounds_and_composition_exclusions() {
    let good = storage_transaction_request(8);
    for gas in [21_000, 15_000_000] {
        let mut value = good.clone();
        value["program"]["transactions"]["gas_limit"] = gas.into();
        assert!(zkgas::fixture::parse_request(&serde_json::to_vec(&value).unwrap()).is_ok());
    }
    for (field, invalid) in [
        ("count_per_block", json!(0)),
        ("count_per_block", json!(9)),
        ("gas_limit", json!(20_999)),
        ("gas_limit", json!(15_000_001)),
        ("kind", json!("create")),
    ] {
        let mut value = good.clone();
        value["program"]["transactions"][field] = invalid;
        let (output, dir) = export_with_profile(&value, "evm-gas-only");
        assert!(!output.status.success(), "accepted invalid {field}");
        absent_outputs(&dir);
    }
    for (field, invalid) in [
        ("block_count", json!(2)),
        ("authorizations", json!({"pattern":"compact","count":1})),
        ("access_list", json!({"pattern":"repeated_key","count":1})),
        (
            "prestate_contracts",
            json!([{"address":"0x0000000000000000000000000000000000010000","code_hex":"0x00"}]),
        ),
    ] {
        let mut value = good.clone();
        value["program"][field] = invalid;
        let (output, dir) = export_with_profile(&value, "evm-gas-only");
        assert!(!output.status.success(), "accepted invalid {field}");
        absent_outputs(&dir);
    }
}

#[test]
fn transaction_descriptor_create_deployment_has_honest_initial_and_rolling_code() {
    // Return byte 0x42 as code, without embedding speculative runtime into the witness.
    let initcode = "0x604260005360016000f3";
    let mut value = transaction_request(
        "create",
        2,
        100_000,
        initcode,
        json!({"opcode:0xf3":4}),
        Some("0x42"),
    );
    value["program"]["block_count"] = 2.into();
    let (output, dir) = export_with_profile(&value, "evm-gas-only");
    success(&output);
    let report = report(&dir);
    let input = input(&dir);
    assert_eq!(input.witnesses[0].witness.codes.len(), 2);
    assert!(
        !input.witnesses[0]
            .witness
            .codes
            .iter()
            .any(|c| c.as_ref() == hex::decode(&initcode[2..]).unwrap().as_slice())
    );
    assert!(
        input.witnesses[1]
            .witness
            .codes
            .iter()
            .any(|c| c.as_ref() == [0x42])
    );
    for (block_index, witness) in input.witnesses.iter().enumerate() {
        assert_eq!(witness.block.body.transactions.len(), 3);
        let users = report["transactions"]["blocks"][block_index]["users"]
            .as_array()
            .unwrap();
        for (index, tx) in witness.block.body.transactions[1..].iter().enumerate() {
            let nonce = (block_index * 2 + index) as u64;
            assert_eq!(tx.nonce(), nonce);
            assert_eq!(tx.to(), None);
            assert_eq!(tx.input().as_ref(), hex::decode(&initcode[2..]).unwrap());
            assert_eq!(users[index]["to"], Value::Null);
            assert_eq!(users[index]["poststate"]["nonce"], 1);
            assert_eq!(
                users[index]["poststate"]["code_hash"],
                serde_json::to_value(alloy_primitives::keccak256([0x42])).unwrap()
            );
        }
    }
}

#[test]
fn transaction_descriptor_empty_create_and_empty_return_preserve_real_stops() {
    for (code, opcode) in [
        ("0x", "opcode:0x00"),
        ("0x00", "opcode:0x00"),
        ("0x60006000f3", "opcode:0xf3"),
    ] {
        let value =
            transaction_request("create", 2, 100_000, code, json!({(opcode):2}), Some("0x"));
        let (output, dir) = export_with_profile(&value, "evm-gas-only");
        success(&output);
        let report = report(&dir);
        assert_eq!(report["observed_route_counts"][opcode], 2);
        for user in report["transactions"]["blocks"][0]["users"]
            .as_array()
            .unwrap()
        {
            assert_eq!(user["poststate"]["nonce"], 1);
            assert_eq!(
                user["poststate"]["code_hash"],
                serde_json::to_value(alloy_primitives::keccak256([])).unwrap()
            );
        }
    }
}

#[test]
fn transaction_descriptor_failed_create_and_same_tx_selfdestruct_report_absence() {
    for (code, opcode, expected_success) in [
        ("0x60006000fd", "opcode:0xfd", false),
        ("0xfe", "opcode:0xfe", false),
        ("0x5fff", "opcode:0xff", true),
    ] {
        let expected = if opcode == "opcode:0xfe" { 0 } else { 2 };
        let mut value =
            transaction_request("create", 2, 100_000, code, json!({(opcode):expected}), None);
        value["expected_user_success"] = expected_success.into();
        let (output, dir) = export_with_profile(&value, "evm-gas-only");
        success(&output);
        let report = report(&dir);
        for user in report["transactions"]["blocks"][0]["users"]
            .as_array()
            .unwrap()
        {
            assert_eq!(user["poststate"], Value::Null);
        }
        if !expected_success {
            assert_eq!(report["receipts_by_block"][0][1]["success"], false);
        }
        if opcode == "opcode:0xfe" {
            assert_eq!(report["attempted_route_counts"][opcode], 2);
        }
    }
}

#[test]
fn transaction_descriptor_malformed_bounds_and_unsupported_composition_are_rejected() {
    let good = transaction_request(
        "create",
        1,
        100_000,
        "0x00",
        json!({"opcode:0x00":1}),
        Some("0x"),
    );
    for descriptor in [
        json!({"kind":"unknown","count_per_block":1,"gas_limit":100_000}),
        json!({"kind":"call","count_per_block":0,"gas_limit":100_000}),
        json!({"kind":"create","count_per_block":2049,"gas_limit":100_000}),
        json!({"kind":"create","count_per_block":1,"gas_limit":20999}),
        json!({"kind":"create","count_per_block":1,"gas_limit":15000001}),
        json!({"kind":"create","count_per_block":true,"gas_limit":100_000}),
        json!({"kind":"call","count_per_block":1,"gas_limit":100_000,"expected_runtime_hex":"0x"}),
        json!({"kind":"create","count_per_block":1,"gas_limit":100_000,"expected_runtime_hex":"00"}),
    ] {
        let mut value = good.clone();
        value["program"]["transactions"] = descriptor;
        assert!(zkgas::fixture::parse_request(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    for (field, descriptor) in [
        ("authorizations", json!({"pattern":"compact","count":1})),
        ("access_list", json!({"pattern":"repeated_key","count":1})),
        (
            "storage_state",
            json!({"initial_storage":{},"proof_slots":["0x0"],"insert_slots":[],"delete_slots":[]}),
        ),
    ] {
        let mut value = good.clone();
        value["program"][field] = descriptor;
        assert!(zkgas::fixture::parse_request(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    for (kind, code, input) in [
        ("create", format!("0x{}", "00".repeat(49153)), "0x"),
        ("create", "0x00".into(), "0x01"),
        ("call", "0x".into(), "0x"),
    ] {
        let mut value =
            transaction_request(kind, 1, 100_000, &code, json!({"opcode:0x00":0}), None);
        value["program"]["input_hex"] = input.into();
        assert!(zkgas::fixture::parse_request(&serde_json::to_vec(&value).unwrap()).is_err());
    }
}

#[test]
fn transaction_descriptor_wrong_runtime_or_account_presence_reject_without_publication() {
    for expected in [None, Some("0x43")] {
        let value = transaction_request(
            "create",
            1,
            100_000,
            "0x604260005360016000f3",
            json!({"opcode:0xf3":1}),
            expected,
        );
        let (output, dir) = export_with_profile(&value, "evm-gas-only");
        assert!(!output.status.success());
        absent_outputs(&dir);
    }
    let mut value = transaction_request(
        "create",
        1,
        100_000,
        "0x60006000fd",
        json!({"opcode:0xfd":1}),
        Some("0x"),
    );
    value["expected_user_success"] = false.into();
    let (output, dir) = export_with_profile(&value, "evm-gas-only");
    assert!(!output.status.success());
    absent_outputs(&dir);
}

#[test]
fn transaction_descriptor_partial_inclusion_and_intrinsic_failure_reject_all_outputs() {
    // The first15M declared transaction fits, but the second cannot fit after consumed gas.
    let value = transaction_request(
        "call",
        3,
        15_000_000,
        "0x5f621fffff5200",
        json!({"opcode:0x52":3,"opcode:0x00":3}),
        None,
    );
    let (output, dir) = export_with_profile(&value, "evm-gas-only");
    assert!(!output.status.success());
    absent_outputs(&dir);
    let value = transaction_request(
        "create",
        1,
        21_000,
        "0x",
        json!({"opcode:0x00":1}),
        Some("0x"),
    );
    let (output, dir) = export_with_profile(&value, "evm-gas-only");
    assert!(!output.status.success());
    absent_outputs(&dir);
}

#[test]
fn transaction_descriptor_fee_cap_and_funding_allow_dense_high_basefee_blocks() {
    let mut value = transaction_request(
        "call",
        3,
        9_000_000,
        "0x5f621fffff5200",
        json!({"opcode:0x52":96,"opcode:0x00":96}),
        None,
    );
    value["program"]["block_count"] = 32.into();
    let (output, dir) = export_with_profile(&value, "evm-gas-only");
    success(&output);
    let report = report(&dir);
    assert!(
        report["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["block_base_fee_per_gas"].as_u64().unwrap() > 25_000_000)
    );
    assert_eq!(report["transaction_count"], 128);
}

#[test]
fn transaction_descriptor_blob_overflow_is_explicit_transport_failure() {
    let mut value = transaction_request(
        "call",
        1024,
        50_000,
        "0x00",
        json!({"opcode:0x00":3072}),
        None,
    );
    value["program"]["block_count"] = 3.into();
    let (output, dir) = export_with_profile(&value, "evm-gas-only");
    assert!(!output.status.success());
    absent_outputs(&dir);
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("controlled manifest does not fit one Kona blob"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn transaction_descriptor_route_bounds_cover_block_instruction_budget() {
    // JUMPDEST costs 1gas: 650 users execute 15,974,400 instructions while total
    // user gas 650*(21000+24576)=29,624,400 still fits the 31M block envelope.
    let mut value = transaction_request(
        "call",
        650,
        50_000,
        &format!("0x{}", "5b".repeat(24_576)),
        json!({"opcode:0x5b":15_974_400,"opcode:0x00":650}),
        None,
    );
    assert!(zkgas::fixture::parse_request(&serde_json::to_vec(&value).unwrap()).is_ok());
    value["program"]["expected_route_counts"]["opcode:0x5b"] = 31_000_001u64.into();
    assert!(zkgas::fixture::parse_request(&serde_json::to_vec(&value).unwrap()).is_err());
    value["program"]
        .as_object_mut()
        .unwrap()
        .remove("transactions");
    value["program"]["expected_route_counts"]["opcode:0x5b"] = 15_974_400u64.into();
    assert!(zkgas::fixture::parse_request(&serde_json::to_vec(&value).unwrap()).is_err());
}

fn multiaccount_request(codes: &[&str], call: bool) -> Value {
    let addresses = (0..codes.len())
        .map(|index| format!("0x{:040x}", 0x10000 + index))
        .collect::<Vec<_>>();
    let mut code = String::from("0x");
    if call {
        for address in addresses.iter().take(2) {
            code.push_str(&format!("5f5f5f5f5f73{}61fffff150", &address[2..]));
        }
    }
    code.push_str("00");
    let mut value = request(&code, json!({"opcode:0xf1":if call {2} else {0}}));
    value["program"]["transactions"] =
        json!({"kind":"call","count_per_block":1,"gas_limit":1000000});
    value["program"]["prestate_contracts"] = codes
        .iter()
        .zip(addresses)
        .map(|(code, address)| json!({"address":address,"code_hex":code}))
        .collect();
    value
}

#[test]
fn multiaccount_code_is_selected_from_native_reads_and_deduplicated() {
    for (codes, unique) in [
        (["0x305f5260205ff3", "0x305f5260205ff3", "0x604200"], 1),
        (["0x305f5260205ff3", "0x305f5260205ff300", "0x604200"], 2),
    ] {
        for call in [false, true] {
            let value = multiaccount_request(&codes, call);
            let (output, dir) = export_with_profile(&value, "evm-gas-only");
            success(&output);
            let input = input(&dir);
            let report = report(&dir);
            assert_eq!(report["prestate_contracts"]["configured_account_count"], 3);
            assert_eq!(
                report["prestate_contracts"]["witnessed_account_count"],
                if call { 2 } else { 0 }
            );
            assert_eq!(
                report["prestate_contracts"]["witnessed_unique_code_count"],
                if call { unique } else { 0 }
            );
            let code_bytes = &input.witnesses[0].witness.codes;
            assert!(
                !code_bytes
                    .iter()
                    .any(|code| code.as_ref() == hex::decode("604200").unwrap())
            );
            // N0's primary STOP shares the checkpoint runtime; full has a distinct parent.
            assert_eq!(code_bytes.len(), if call { 3 + unique } else { 2 });
            if call {
                let mut missing_code = input.clone();
                missing_code.witnesses[0]
                    .witness
                    .codes
                    .retain(|code| code.as_ref() != hex::decode("305f5260205ff3").unwrap());
                assert!(
                    raiko2_zkgas_trace::quote_shasta_proposal_with_execution_profile(
                        &missing_code,
                        raiko2_zkgas_trace::ExecutionProfile::EvmGasOnly
                    )
                    .is_err()
                );
                let mut missing_state = input.clone();
                // JSON decoding canonicalizes inline nodes into the proposal pool.
                missing_state.proposal_state_nodes.clear();
                missing_state.witnesses[0].witness.state.clear();
                missing_state.witnesses[0].witness.state_indices.clear();
                assert!(
                    raiko2_zkgas_trace::quote_shasta_proposal_with_execution_profile(
                        &missing_state,
                        raiko2_zkgas_trace::ExecutionProfile::EvmGasOnly
                    )
                    .is_err()
                );
            }
        }
    }
}

#[test]
fn multiaccount_rejects_invalid_configuration_before_publication() {
    let base = multiaccount_request(&["0x00"], false);
    for (field, value) in [
        ("address", json!("0x01")),
        ("address", json!(format!("0x{}", "00".repeat(20)))),
        ("address", json!(format!("0x{}", "66".repeat(20)))),
        ("code_hex", json!("0x")),
        ("code_hex", json!("00")),
        ("code_hex", json!("0xzz")),
        ("code_hex", json!(format!("0x{}", "00".repeat(24577)))),
        ("code_hex", json!("0xef00")),
    ] {
        let mut bad = base.clone();
        bad["program"]["prestate_contracts"][0][field] = value;
        let (output, dir) = export(&bad);
        assert!(!output.status.success());
        absent_outputs(&dir);
    }
    let mut duplicate = base.clone();
    duplicate["program"]["prestate_contracts"] = json!([
        base["program"]["prestate_contracts"][0],
        base["program"]["prestate_contracts"][0]
    ]);
    assert!(zkgas::fixture::parse_request(&serde_json::to_vec(&duplicate).unwrap()).is_err());
    for (field, value) in [
        ("block_count", json!(2)),
        (
            "transactions",
            json!({"kind":"create","count_per_block":1,"gas_limit":1000000}),
        ),
        (
            "storage_state",
            json!({"initial_storage":{},"proof_slots":["0x0"]}),
        ),
        ("prestate_contracts", json!([])),
    ] {
        let mut bad = base.clone();
        bad["program"][field] = value;
        assert!(zkgas::fixture::parse_request(&serde_json::to_vec(&bad).unwrap()).is_err());
    }
    let mut legacy = base.clone();
    legacy["program"]
        .as_object_mut()
        .unwrap()
        .remove("transactions");
    assert!(zkgas::fixture::parse_request(&serde_json::to_vec(&legacy).unwrap()).is_err());
}

#[test]
fn multiaccount_preserves_read_only_and_reverted_storage_reads() {
    for (code, reverts) in [
        ("0x6000545000", false),
        ("0x6001600055600054505f5ffd", true),
    ] {
        let value = multiaccount_request(&[code, code, "0x604200"], true);
        let (output, dir) = export_with_profile(&value, "evm-gas-only");
        success(&output);
        let report = report(&dir);
        assert!(
            report["prestate_contracts"]["loaded_storage_target_count"]
                .as_u64()
                .unwrap()
                >= 2
        );
        assert_eq!(report["prestate_contracts"]["witnessed_account_count"], 2);
        if reverts {
            assert_eq!(report["terminal_counts"]["revert"], 2);
        }
    }
}

#[test]
fn multiaccount_account_only_read_does_not_load_code() {
    let mut value = multiaccount_request(&["0x604200"], false);
    value["program"]["code_hex"] = json!(format!("0x73{:040x}3f5000", 0x10000));
    value["program"]["expected_route_counts"] = json!({"opcode:0x3f":1});
    let (output, dir) = export_with_profile(&value, "evm-gas-only");
    success(&output);
    let report = report(&dir);
    assert_eq!(report["prestate_contracts"]["witnessed_account_count"], 1);
    assert_eq!(
        report["prestate_contracts"]["witnessed_unique_code_count"],
        0
    );
}

#[test]
fn multiaccount_request_size_is_scoped_and_contract_count_bounded() {
    let code = format!("0x{}", "00".repeat(4096));
    let mut value = multiaccount_request(&[], false);
    value["program"]["prestate_contracts"] = (0..512)
        .map(|index| json!({"address":format!("0x{:040x}",0x10000+index),"code_hex":code}))
        .collect();
    let bytes = serde_json::to_vec(&value).unwrap();
    assert!(bytes.len() > 4 * 1024 * 1024);
    assert!(zkgas::fixture::parse_request(&bytes).is_ok());
    let duplicate = value["program"]["prestate_contracts"][0].clone();
    value["program"]["prestate_contracts"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    assert!(zkgas::fixture::parse_request(&serde_json::to_vec(&value).unwrap()).is_err());
    value["program"]
        .as_object_mut()
        .unwrap()
        .remove("prestate_contracts");
    let mut legacy = serde_json::to_vec(&value).unwrap();
    legacy.resize(4 * 1024 * 1024 + 1, b' ');
    assert!(zkgas::fixture::parse_request(&legacy).is_err());
}

#[test]
fn multiaccount_rejects_system_identities_from_canonical_sources() {
    let mut value = multiaccount_request(&["0x00"], false);
    let spec = raiko2_primitives::SupportedChainSpecs::default()
        .get_chain_spec_with_chain_id(167000)
        .unwrap();
    for address in [
        taiko_client_protocol::FixedKSigner::golden_touch()
            .unwrap()
            .address(),
        spec.l2_contract.unwrap(),
    ] {
        value["program"]["prestate_contracts"][0]["address"] = json!(address);
        assert!(zkgas::fixture::parse_request(&serde_json::to_vec(&value).unwrap()).is_err());
    }
}

#[test]
fn multiaccount_unused_background_accounts_do_not_pad_proof_targets() {
    let value = multiaccount_request(&["0x604200"; 32], false);
    let (output, dir) = export_with_profile(&value, "evm-gas-only");
    success(&output);
    let report = report(&dir);
    let diagnostics = &report["prestate_contracts"];
    assert_eq!(diagnostics["witnessed_account_count"], 0);
    assert_eq!(diagnostics["witnessed_unique_code_count"], 0);
    assert!(
        diagnostics["minimized_witness_node_count"]
            .as_u64()
            .unwrap()
            < diagnostics["full_witness_node_count"].as_u64().unwrap()
    );
    assert_eq!(diagnostics["configured_code_bytes"], 32 * 3);
    assert_eq!(diagnostics["configured_unique_code_bytes"], 3);
}

fn initialized_fanout_request(code: &str, opcode: &str) -> Value {
    let mut value = multiaccount_request(&[code, code], false);
    value["program"]["transactions"]["count_per_block"] = json!(3);
    value["program"]["transactions"]["recipients"] = json!([
        "0x0000000000000000000000000000000000010000",
        "0x0000000000000000000000000000000000010001",
        "0x0000000000000000000000000000000000010000"
    ]);
    value["program"]["prestate_contracts"][0]["initial_storage"] = json!({"0x0":"0x1","0x9":"0xb"});
    value["program"]["prestate_contracts"][1]["initial_storage"] = json!({"0x0":"0x5","0x9":"0xd"});
    value["program"]["expected_route_counts"] = json!({(opcode):3});
    value
}

fn independent_storage_root(values: &[(u64, u64)]) -> Value {
    use alloy_primitives::{B256, U256};
    serde_json::to_value(alloy_trie::root::storage_root_unhashed(values.iter().map(
        |(key, value)| {
            (
                B256::from(U256::from(*key).to_be_bytes::<32>()),
                U256::from(*value),
            )
        },
    )))
    .unwrap()
}

#[test]
fn initialized_fanout_commits_distinct_and_repeated_recipients_with_actual_roots() {
    use reth_primitives_traits::SignerRecoverable as _;
    // Increment raw slot0: A is visited twice and B once. Slot9 is untouched background.
    let value = initialized_fanout_request("0x5f546001015f5500", "opcode:0x55");
    let (output, dir) = export_with_profile(&value, "evm-gas-only");
    success(&output);
    let report = report(&dir);
    let input = input(&dir);
    let diagnostics = report["prestate_contracts"]["initialized_storage"]
        .as_array()
        .unwrap();
    assert_eq!(diagnostics.len(), 2);
    for (index, initial, final_values) in [
        (0, vec![(0, 1), (9, 11)], vec![(0, 3), (9, 11)]),
        (1, vec![(0, 5), (9, 13)], vec![(0, 6), (9, 13)]),
    ] {
        assert_eq!(
            diagnostics[index]["initial_storage_root"],
            independent_storage_root(&initial)
        );
        assert_eq!(
            diagnostics[index]["final_storage_root"],
            independent_storage_root(&final_values)
        );
        assert_eq!(diagnostics[index]["initial_storage_entry_count"], 2);
        assert_eq!(diagnostics[index]["final_storage_entry_count"], 2);
    }
    let mut sender = None;
    for (index, tx) in input.witnesses[0].block.body.transactions[1..]
        .iter()
        .enumerate()
    {
        let expected = value["program"]["transactions"]["recipients"][index]
            .as_str()
            .unwrap()
            .parse::<alloy_primitives::Address>()
            .unwrap();
        assert_eq!(tx.to(), Some(expected));
        assert_eq!(tx.nonce(), index as u64);
        let recovered = tx.recover_signer().unwrap();
        assert_eq!(*sender.get_or_insert(recovered), recovered);
        assert_eq!(
            report["transactions"]["blocks"][0]["users"][index]["to"],
            serde_json::to_value(expected).unwrap()
        );
        assert_eq!(report["receipts_by_block"][0][index + 1]["success"], true);
    }
    assert_eq!(report["observed_route_counts"]["opcode:0x54"], 3);
    assert_eq!(report["observed_route_counts"]["opcode:0x55"], 3);
    assert_eq!(report["observed_route_counts"].get("opcode:0xf1"), None);
    let quote = raiko2_zkgas_trace::quote_shasta_proposal_with_execution_profile(
        &input,
        raiko2_zkgas_trace::ExecutionProfile::EvmGasOnly,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(quote.public_output).unwrap(),
        report["public_output"]
    );
}

#[test]
fn initialized_fanout_delete_and_revert_report_committed_roots() {
    for (code, reverts) in [("0x5f5f5500", false), ("0x5f5f555f5ffd", true)] {
        let mut value = initialized_fanout_request(code, "opcode:0x55");
        value["expected_user_success"] = json!(!reverts);
        let (output, dir) = export_with_profile(&value, "evm-gas-only");
        success(&output);
        let report = report(&dir);
        let diagnostics = &report["prestate_contracts"]["initialized_storage"];
        for (index, original, background) in [(0, 1, 11), (1, 5, 13)] {
            let expected = if reverts {
                vec![(0, original), (9, background)]
            } else {
                vec![(9, background)]
            };
            assert_eq!(
                diagnostics[index]["final_storage_root"],
                independent_storage_root(&expected)
            );
            assert_eq!(
                diagnostics[index]["final_storage_entry_count"],
                expected.len()
            );
        }
    }
}

#[test]
fn initialized_fanout_validates_map_bounds_and_optional_empty_mode() {
    let mut value = initialized_fanout_request("0x00", "opcode:0x00");
    for contract in value["program"]["prestate_contracts"]
        .as_array_mut()
        .unwrap()
    {
        contract["initial_storage"] = json!({});
    }
    let (output, dir) = export_with_profile(&value, "evm-gas-only");
    success(&output);
    assert_eq!(
        report(&dir)["prestate_contracts"]["initialized_storage"][0]["final_storage_entry_count"],
        0
    );
    let entries = (0..8192)
        .map(|i| (format!("0x{i:x}"), json!("0x1")))
        .collect::<serde_json::Map<_, _>>();
    for contract in value["program"]["prestate_contracts"]
        .as_array_mut()
        .unwrap()
    {
        contract["initial_storage"] = Value::Object(entries.clone());
    }
    assert!(zkgas::fixture::parse_request(&serde_json::to_vec(&value).unwrap()).is_ok());
    value["program"]["prestate_contracts"][0]["initial_storage"]["0x2000"] = json!("0x1");
    assert!(zkgas::fixture::parse_request(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[test]
fn initialized_fanout_rejects_invalid_recipients_and_storage_without_publication() {
    let base = initialized_fanout_request("0x00", "opcode:0x00");
    let mut variants = Vec::new();
    for recipients in [
        json!([]),
        json!(["0x0000000000000000000000000000000000010000"]),
        json!([
            "0x0000000000000000000000000000000000010000",
            "0x0000000000000000000000000000000000010002",
            "0x0000000000000000000000000000000000010000"
        ]),
    ] {
        let mut bad = base.clone();
        bad["program"]["transactions"]["recipients"] = recipients;
        variants.push(bad);
    }
    for (field, content) in [
        ("block_count", json!(2)),
        (
            "storage_state",
            json!({"initial_storage":{},"proof_slots":["0x0"]}),
        ),
        (
            "access_list",
            json!({"pattern":"distinct_addresses","count":1}),
        ),
        ("authorizations", json!({"pattern":"distinct","count":1})),
    ] {
        let mut bad = base.clone();
        bad["program"][field] = content;
        variants.push(bad);
    }
    let mut create = base.clone();
    create["program"]["transactions"]["kind"] = json!("create");
    variants.push(create);
    let mut missing = base.clone();
    missing["program"]
        .as_object_mut()
        .unwrap()
        .remove("prestate_contracts");
    variants.push(missing);
    for map in [
        json!({"0x0":"0x0"}),
        json!({"garbage":"0x1"}),
        json!({"0x0":"garbage"}),
        json!({"0x0":"0x1", "0x00":"0x2"}),
    ] {
        let mut bad = base.clone();
        bad["program"]["prestate_contracts"][0]["initial_storage"] = map;
        variants.push(bad);
    }
    let duplicate_raw_slot = serde_json::to_string(&base)
        .unwrap()
        .replace("\"0x0\":\"0x1\"", "\"0x0\":\"0x1\",\"0x0\":\"0x2\"");
    assert!(zkgas::fixture::parse_request(duplicate_raw_slot.as_bytes()).is_err());
    for bad in variants {
        assert!(
            zkgas::fixture::parse_request(&serde_json::to_vec(&bad).unwrap()).is_err(),
            "{bad}"
        );
        let (output, dir) = export_with_profile(&bad, "evm-gas-only");
        assert!(!output.status.success());
        absent_outputs(&dir);
    }
}

#[test]
fn legacy_code_only_prestate_omits_initialized_storage_diagnostics() {
    let value = multiaccount_request(&["0x00"], false);
    let (output, dir) = export_with_profile(&value, "evm-gas-only");
    success(&output);
    assert!(
        report(&dir)["prestate_contracts"]
            .get("initialized_storage")
            .is_none()
    );
    let parsed = zkgas::fixture::parse_request(&serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(
        serde_json::to_value(&parsed.program.prestate_contracts.as_ref().unwrap()[0])
            .unwrap()
            .get("initial_storage")
            .is_none()
    );
    assert!(
        serde_json::to_value(parsed.program.transactions.unwrap())
            .unwrap()
            .get("recipients")
            .is_none()
    );
}

fn shared_access_list_request(code: &str, counts: Value) -> Value {
    let mut value = transaction_request("call", 2, 15_000_000, code, counts, None);
    value["program"]["access_list"] = json!({"pattern":"raw_keys","count":4,"storage_keys":[
        format!("0x{:064x}",0),format!("0x{}","ff".repeat(32)),
        format!("0x{:064x}",0),format!("0x{:064x}",1)
    ]});
    value
}

#[test]
fn shared_access_list_call_users_report_exact_signed_vectors_and_block_union() {
    let mut value = shared_access_list_request("0x60006000f3", json!({"opcode:0xf3":4}));
    value["program"]["block_count"] = json!(2);
    let (output, dir) = export_with_profile(&value, "evm-gas-only");
    success(&output);
    let actual = input(&dir);
    let report = report(&dir);
    assert_eq!(report["transaction_count"], 6);
    assert_eq!(report["access_list"]["transactions_per_block"], 2);
    assert_eq!(report["access_list"]["count_per_block"], 4);
    assert_eq!(report["access_list"]["blocks"].as_array().unwrap().len(), 2);
    for (block_index, witness) in actual.witnesses.iter().enumerate() {
        assert_eq!(witness.block.body.transactions.len(), 3);
        assert_eq!(
            report["receipts_by_block"][block_index]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        let block = &report["access_list"]["blocks"][block_index];
        assert_eq!(block["raw_address_items"], 2);
        assert_eq!(block["raw_storage_keys"], 8);
        assert_eq!(block["unique_addresses"], 1);
        assert_eq!(block["unique_storage_pairs"], 3);
        for (index, tx) in witness.block.body.transactions[1..].iter().enumerate() {
            assert_eq!(tx.nonce(), (block_index * 2 + index) as u64);
            assert_eq!(tx.gas_limit(), 15_000_000);
            assert_eq!(tx.max_fee_per_gas(), 1_000_000_000);
            assert_eq!(tx.to(), Some(alloy_primitives::Address::repeat_byte(0x66)));
            let list = tx.access_list().unwrap();
            assert_eq!(list.0.len(), 1);
            assert_eq!(
                serde_json::to_value(&list.0[0].storage_keys).unwrap(),
                value["program"]["access_list"]["storage_keys"]
            );
            let user = &block["transactions"][index];
            assert_eq!(user["nonce"], tx.nonce());
            assert_eq!(user["transaction_index"], index + 1);
            assert_eq!(
                user["transaction_hash"],
                serde_json::to_value(tx.hash()).unwrap()
            );
            assert_eq!(user["raw_storage_keys"], 4);
            assert_eq!(user["unique_storage_pairs"], 3);
            assert_eq!(
                report["receipts_by_block"][block_index][index + 1]["success"],
                true
            );
        }
    }
}

#[test]
fn shared_access_list_call_revert_and_halt_still_gate_every_included_user() {
    for (code, counts) in [
        ("0x60006000fd", json!({"opcode:0xfd":2})),
        ("0xfe", json!({"opcode:0xfe":0})),
    ] {
        let mut value = shared_access_list_request(code, counts);
        value["expected_user_success"] = json!(false);
        let (output, dir) = export_with_profile(&value, "evm-gas-only");
        success(&output);
        let report = report(&dir);
        assert_eq!(
            report["access_list"]["blocks"][0]["transactions"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        for receipt in report["receipts_by_block"][0]
            .as_array()
            .unwrap()
            .iter()
            .skip(1)
        {
            assert_eq!(receipt["success"], false);
        }
    }
}

#[test]
fn shared_access_list_call_keeps_intrinsic_gas_and_per_transaction_bounds() {
    let mut value = shared_access_list_request("0x60006000f3", json!({"opcode:0xf3":2}));
    // Four raw slots:21000+2400+4*1900 =31000 intrinsic; duplicates still count.
    value["program"]["transactions"]["gas_limit"] = json!(30_999);
    let (output, dir) = export_with_profile(&value, "evm-gas-only");
    assert!(!output.status.success());
    absent_outputs(&dir);
    for bad in [20_999, 15_000_001] {
        value["program"]["transactions"]["gas_limit"] = json!(bad);
        assert!(zkgas::fixture::parse_request(&serde_json::to_vec(&value).unwrap()).is_err());
    }
}
