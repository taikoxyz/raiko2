#![allow(missing_docs)]

use alloy_primitives::{Address, B256};
use assert_cmd::cargo::cargo_bin_cmd;
use flate2::read::GzDecoder;
use predicates::prelude::*;
use raiko2_primitives::{ProofType, SupportedChainSpecs};
use raiko2_primitives_shasta::{GuestInput, build_proof_carry_data_from_witness_spec};

fn input_without_proof_carry_data() -> GuestInput {
    let mut input = GuestInput::default();
    input.taiko.proposal_id = 7;
    input.taiko.chain_spec.name = "taiko_mainnet".to_string();
    input.taiko.chain_spec.chain_id = 167_000;
    input.taiko.chain_spec.is_taiko = true;
    input.taiko.prover_data.actual_prover = Address::from([0x11; 20]);
    input.taiko.proposal_event.proposal.id = 7u64.try_into().expect("fits in uint48");
    input.taiko.proposal_event.proposal.proposer = Address::from([0x22; 20]);
    input.taiko.proposal_event.proposal.timestamp = 123u64.try_into().expect("fits in uint48");
    input.taiko.proposal_event.proposal.parentProposalHash = B256::from([0x33; 32]);

    let chain_spec = SupportedChainSpecs::default()
        .get_chain_spec_with_chain_id(167_000)
        .expect("supported chain");
    let mut witness = raiko2_primitives::StatelessInput {
        chain_spec,
        ..Default::default()
    };
    witness.block.header.number = 1;
    witness.block.header.timestamp = u64::MAX / 2;
    witness.block.header.parent_hash = B256::from([0x44; 32]);
    witness.block.header.state_root = B256::from([0x55; 32]);
    input.witnesses.push(witness);
    input
}

#[test]
fn proposal_trace_requires_an_input() {
    cargo_bin_cmd!("guest-launcher")
        .args(["--stage", "proposal-trace"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("missing --input"));
}

#[test]
fn proposal_trace_rejects_non_native_backends_and_prove_mode() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let input = directory.path().join("input.json");
    std::fs::write(
        &input,
        serde_json::to_vec(&GuestInput::default()).expect("serialize default input"),
    )
    .expect("write input");

    cargo_bin_cmd!("guest-launcher")
        .args([
            "--stage",
            "proposal-trace",
            "--input",
            input.to_str().expect("UTF-8 path"),
            "--proof-type",
            "sp1",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "supports only --proof-type native",
        ));

    cargo_bin_cmd!("guest-launcher")
        .args([
            "--stage",
            "proposal-trace",
            "--input",
            input.to_str().expect("UTF-8 path"),
            "--mode",
            "prove",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("supports only --mode execute"));
}

#[test]
fn proposal_trace_emits_normalized_failure_json_for_invalid_proposal() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let input = directory.path().join("input.json");
    let output = directory.path().join("trace.json.gz");
    let summary = directory.path().join("trace.summary.json");
    std::fs::write(
        &input,
        serde_json::to_vec(&GuestInput::default()).expect("serialize default input"),
    )
    .expect("write input");

    cargo_bin_cmd!("guest-launcher")
        .args([
            "--stage",
            "proposal-trace",
            "--input",
            input.to_str().expect("UTF-8 input path"),
            "--json-out",
            output.to_str().expect("UTF-8 output path"),
        ])
        .assert()
        .success();

    let compressed = std::fs::read(output).expect("read compressed proposal trace output");
    assert_eq!(&compressed[..2], &[0x1f, 0x8b], "gzip magic");
    let trace: serde_json::Value =
        serde_json::from_slice(&std::fs::read(summary).expect("read proposal trace summary"))
            .expect("parse proposal trace summary");
    assert_eq!(trace["status"], "failed");
    assert_eq!(trace["parity_passed"], false);
    assert_eq!(trace["failure_stage"], "ordinary");
    assert_eq!(
        trace["guest_input_sha256"]
            .as_str()
            .expect("input hash")
            .len(),
        66
    );
    assert!(
        trace["guest_input_bincode_length"]
            .as_u64()
            .expect("input length")
            > 0
    );
}

#[test]
fn proposal_trace_cli_runs_repository_fixture_through_the_ab_gate() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let output = directory.path().join("trace.json.gz");
    let summary = directory.path().join("trace.summary.json");
    let input = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_23077_l2_9051439_9051630.json",
    );

    cargo_bin_cmd!("guest-launcher")
        .args([
            "--stage",
            "proposal-trace",
            "--input",
            input.to_str().expect("UTF-8 input path"),
            "--json-out",
            output.to_str().expect("UTF-8 output path"),
        ])
        .assert()
        .success();

    let compressed_size = std::fs::metadata(&output)
        .expect("compressed proposal trace metadata")
        .len();
    assert!(
        compressed_size < 256 * 1024 * 1024,
        "compressed trace bound"
    );
    let decompressed_size = std::io::copy(
        &mut GzDecoder::new(std::fs::File::open(&output).expect("open compressed trace")),
        &mut std::io::sink(),
    )
    .expect("stream-decompress complete trace");
    assert!(
        decompressed_size > compressed_size,
        "full operation trace must remain present behind compression"
    );
    let summary_bytes = std::fs::read(summary).expect("read proposal trace summary");
    assert!(
        summary_bytes.len() < 4 * 1024,
        "summary must remain bounded"
    );
    let trace: serde_json::Value =
        serde_json::from_slice(&summary_bytes).expect("parse proposal trace summary");
    assert_eq!(trace["status"], "complete");
    assert_eq!(trace["parity_passed"], true);
    assert!(trace["public_output"].as_str().is_some());
    assert!(trace["block_count"].as_u64().is_some_and(|count| count > 0));
    assert_eq!(trace["partial_block_count"], 0);
    assert!(
        trace["operation_count"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );
}

#[test]
fn proposal_trace_backfills_missing_carry_with_sp1_identity() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let input_path = directory.path().join("input.json");
    let output = directory.path().join("trace.json.gz");
    let summary = directory.path().join("trace.summary.json");
    let input = input_without_proof_carry_data();
    std::fs::write(
        &input_path,
        serde_json::to_vec(&input).expect("serialize input"),
    )
    .expect("write input");

    let mut sp1_input = input;
    sp1_input.proof_carry_data =
        build_proof_carry_data_from_witness_spec(&sp1_input, ProofType::Sp1)
            .expect("build SP1 carry data");
    let (expected_hash, expected_length) =
        raiko2_zkgas_trace::guest_input_identity(&sp1_input).expect("SP1 input identity");

    cargo_bin_cmd!("guest-launcher")
        .args([
            "--stage",
            "proposal-trace",
            "--input",
            input_path.to_str().expect("UTF-8 input path"),
            "--json-out",
            output.to_str().expect("UTF-8 output path"),
        ])
        .assert()
        .success();

    let trace: serde_json::Value =
        serde_json::from_slice(&std::fs::read(summary).expect("read proposal trace summary"))
            .expect("parse proposal trace summary");
    assert_eq!(trace["guest_input_sha256"], expected_hash);
    assert_eq!(trace["guest_input_bincode_length"], expected_length);
}
