mod common;

use std::{
    fs,
    path::{Path, PathBuf},
};

use common::trusted_temp_root;
use raiko2_primitives_shasta::GuestInput;
use raiko2_zkgas_trace::quote_shasta_proposal;
use zkgas::{
    compare::MAX_COMPARE_FILE_BYTES,
    contracts::*,
    identity::sha256_hex,
    quote::{MAX_QUOTE_OUTPUT_BYTES, QuoteArgs, run_quote},
};

struct TestDir(PathBuf);
impl TestDir {
    fn new() -> Self {
        let path = trusted_temp_root().join(format!("zkgas-quote-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn fixture() -> GuestInput {
    serde_json::from_slice(&fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_41455_l2_12556056_12556247.json")).unwrap()).unwrap()
}
fn executable() -> ExecutableIdentity {
    ExecutableIdentity {
        sha256: "test-executable".into(),
        package_version: "test-version".into(),
        source_revision: None,
        dirty: None,
    }
}
fn single_args(dir: &TestDir, json: bool) -> QuoteArgs {
    let input = fixture();
    let path = dir
        .path()
        .join(if json { "input.json" } else { "input.bin" });
    fs::write(
        &path,
        if json {
            serde_json::to_vec(&input).unwrap()
        } else {
            bincode::serialize(&input).unwrap()
        },
    )
    .unwrap();
    QuoteArgs {
        input_bincode: (!json).then_some(path.clone()),
        input_json: json.then_some(path),
        corpus_id: Some("corpus".into()),
        sample_id: Some("sample".into()),
        output: dir.path().join("quotes.jsonl"),
        ..Default::default()
    }
}
fn rows(path: &Path) -> Vec<QuoteRowV1> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn create_fifo(path: &Path) {
    let output = std::process::Command::new("mkfifo")
        .arg(path)
        .output()
        .expect("run mkfifo");
    assert!(
        output.status.success(),
        "mkfifo failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn single_input_matches_quote_core_and_canonical_identity() {
    let dir = TestDir::new();
    let args = single_args(&dir, false);
    let input = fixture();
    let canonical = bincode::serialize(&input).unwrap();
    let expected = quote_shasta_proposal(&input).unwrap();
    assert_eq!(run_quote(&args, &executable()).unwrap().exit_code(), 0);
    let [QuoteRowV1::Complete(row)]: [QuoteRowV1; 1] = rows(&args.output).try_into().unwrap()
    else {
        panic!("complete row expected")
    };
    assert_eq!(row.quote, expected);
    assert_eq!(row.key.guest_input_sha256, sha256_hex(&canonical));
    assert_eq!(row.guest_input_byte_length, canonical.len() as u64);
    assert_eq!(row.proposal.first_block, 12_556_056);
    assert_eq!(row.proposal.last_block, 12_556_247);
    assert_eq!(row.proposal.block_count, 192);
    assert_eq!(
        row.proposal.fork_segments,
        vec![ForkSegmentV1 {
            fork: "UNZEN".into(),
            first_block: 12_556_056,
            last_block: 12_556_247
        }]
    );
    assert_eq!(row.prediction_unit, "sp1_pgu");
    assert_eq!(row.prediction_surface, "prove");
    assert_eq!(row.calibration_profile, row.quote.model_id);
    assert_eq!(row.proposal.witness_chain_spec_sha256.len(), 192);
    assert!(matches!(row.input_source, InputSourceV1::Single { .. }));
    assert!(
        !fs::read_to_string(&args.output)
            .unwrap()
            .contains(&*dir.path().to_string_lossy())
    );
}

#[test]
fn json_missing_proof_carry_data_is_not_backfilled() {
    let dir = TestDir::new();
    let args = single_args(&dir, true);
    let mut value = serde_json::to_value(fixture()).unwrap();
    value.as_object_mut().unwrap().remove("proof_carry_data");
    fs::write(
        args.input_json.as_ref().unwrap(),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    assert_eq!(run_quote(&args, &executable()).unwrap().exit_code(), 2);
    assert_eq!(
        failure_code(&rows(&args.output)[0]),
        "proposal_identity_mismatch"
    );
    let raw = fs::read(args.input_json.unwrap()).unwrap();
    let decoded: GuestInput = serde_json::from_slice(&raw).unwrap();
    let canonical = bincode::serialize(&decoded).unwrap();
    let QuoteRowV1::Failed(row) = &rows(&args.output)[0] else {
        unreachable!()
    };
    assert_eq!(
        row.observed_guest_input_sha256.as_deref(),
        Some(sha256_hex(&raw).as_str())
    );
    assert_eq!(
        row.verified_key.as_ref().unwrap().guest_input_sha256,
        sha256_hex(&canonical)
    );
}

#[test]
fn open_manifest_input_rejects_symlink_swap_after_validation() {
    use std::{fs::File, io::Read};
    use zkgas::quote::open_manifest_input;
    let dir = TestDir::new();
    let outside = TestDir::new();
    fs::create_dir(dir.path().join("data")).unwrap();
    fs::write(dir.path().join("data/input.bin"), b"trusted").unwrap();
    fs::write(outside.path().join("input.bin"), b"outside").unwrap();
    let root = File::open(dir.path()).unwrap();
    assert!(
        fs::canonicalize(dir.path().join("data/input.bin"))
            .unwrap()
            .starts_with(dir.path())
    );
    let mut pinned = open_manifest_input(&root, Path::new("data/input.bin")).unwrap();
    fs::rename(dir.path().join("data"), dir.path().join("old-data")).unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("data")).unwrap();
    assert!(open_manifest_input(&root, Path::new("data/input.bin")).is_err());
    let mut contents = String::new();
    pinned.read_to_string(&mut contents).unwrap();
    assert_eq!(contents, "trusted");
}

fn cli() -> std::process::Command {
    std::process::Command::new(env!("CARGO_BIN_EXE_zkgas-quote"))
}

#[test]
fn cli_single_exit_classes_reuse_and_conflict() {
    let dir = TestDir::new();
    let args = single_args(&dir, false);
    let invoke = || {
        cli()
            .arg("--input-bincode")
            .arg(args.input_bincode.as_ref().unwrap())
            .args(["--corpus-id", "corpus", "--sample-id", "sample", "--output"])
            .arg(&args.output)
            .output()
            .unwrap()
    };
    assert_eq!(invoke().status.code(), Some(0));
    let expected = fs::read(&args.output).unwrap();
    assert_eq!(invoke().status.code(), Some(0));
    assert_eq!(fs::read(&args.output).unwrap(), expected);
    fs::write(&args.output, b"conflicting output").unwrap();
    assert_eq!(invoke().status.code(), Some(1));
    assert_eq!(fs::read(&args.output).unwrap(), b"conflicting output");

    let partial_output = dir.path().join("partial.jsonl");
    let result = cli()
        .arg("--input-bincode")
        .arg(dir.path().join("missing.bin"))
        .args(["--corpus-id", "corpus", "--sample-id", "sample", "--output"])
        .arg(&partial_output)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(failure_code(&rows(&partial_output)[0]), "input_open");

    let invalid_output = dir.path().join("invalid.jsonl");
    let result = cli()
        .arg("--input-bincode")
        .arg(args.input_bincode.unwrap())
        .arg("--output")
        .arg(&invalid_output)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(!invalid_output.exists());
}

#[test]
fn cli_rejects_invalid_input_modes_with_exit_one() {
    let dir = TestDir::new();
    for flags in [
        vec![],
        vec!["--input-json", "input.json", "--input-bincode", "input.bin"],
        vec!["--manifest", "manifest.json", "--input-json", "input.json"],
        vec!["--manifest", "manifest.json", "--corpus-id", "corpus"],
        vec!["--manifest", "manifest.json", "--concurrency", "2"],
    ] {
        let result = cli()
            .args(flags)
            .arg("--output")
            .arg(dir.path().join("output.jsonl"))
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(1));
        assert!(!dir.path().join("output.jsonl").exists());
    }
}

#[test]
fn cli_batch_uses_raw_manifest_identity_and_partial_exit() {
    let dir = TestDir::new();
    let bytes = bincode::serialize(&GuestInput::default()).unwrap();
    fs::write(dir.path().join("empty.bin"), &bytes).unwrap();
    let args = batch_args(&dir, vec![sample("empty", "empty.bin", &bytes)]);
    let manifest = args.manifest.as_ref().unwrap();
    let invoke = |output: &Path| {
        cli()
            .arg("--manifest")
            .arg(manifest)
            .arg("--output")
            .arg(output)
            .output()
            .unwrap()
    };
    assert_eq!(invoke(&args.output).status.code(), Some(2));
    let first = fs::read(&args.output).unwrap();
    assert_eq!(invoke(&args.output).status.code(), Some(2));
    assert_eq!(fs::read(&args.output).unwrap(), first);
    let mut raw = fs::read(manifest).unwrap();
    raw.push(b'\n');
    fs::write(manifest, raw).unwrap();
    let other = dir.path().join("other.jsonl");
    assert_eq!(invoke(&other).status.code(), Some(2));
    assert_ne!(fs::read(other).unwrap(), first);
    fs::write(manifest, b"invalid manifest").unwrap();
    let invalid = dir.path().join("invalid.jsonl");
    assert_eq!(invoke(&invalid).status.code(), Some(1));
    assert!(!invalid.exists());
}

#[test]
fn cli_help_documents_modes_and_resource_defaults() {
    let result = cli().arg("--help").output().unwrap();
    assert!(result.status.success());
    let help = String::from_utf8(result.stdout).unwrap();
    for text in [
        "--input-bincode",
        "--input-json",
        "--manifest",
        "--max-manifest-bytes",
        "16777216",
        "--max-input-bytes",
        "536870912",
        "--max-total-bytes",
        "8589934592",
        "--max-rows",
        "10000",
        "--max-elapsed-seconds",
        "3600",
        "--concurrency",
        "[default: 1]",
    ] {
        assert!(help.contains(text), "{text}: {help}");
    }
}

#[test]
fn single_resource_limits_have_distinct_failure_classes() {
    for (field, code) in [("input", "input_byte_limit"), ("total", "total_byte_limit")] {
        let dir = TestDir::new();
        let mut args = single_args(&dir, false);
        if field == "input" {
            args.limits.max_input_bytes = 1;
        } else {
            args.limits.max_total_bytes = 1;
        }
        assert_eq!(run_quote(&args, &executable()).unwrap().exit_code(), 2);
        assert_eq!(failure_code(&rows(&args.output)[0]), code);
    }
    let dir = TestDir::new();
    let mut args = single_args(&dir, false);
    args.limits.max_rows = 0;
    assert!(run_quote(&args, &executable()).is_err());
    assert!(!args.output.exists());
}

#[test]
fn manifest_partial_keeps_good_rows_and_all_ordinals() {
    let dir = TestDir::new();
    let bytes = bincode::serialize(&fixture()).unwrap();
    fs::write(dir.path().join("good.bin"), &bytes).unwrap();
    let args = batch_args(
        &dir,
        vec![
            sample("good", "good.bin", &bytes),
            sample("missing", "missing.bin", &bytes),
        ],
    );
    assert_eq!(run_quote(&args, &executable()).unwrap().exit_code(), 2);
    let rows = rows(&args.output);
    assert_eq!(rows.len(), 2);
    assert!(matches!(rows[0], QuoteRowV1::Complete(_)));
    assert_eq!(failure_code(&rows[1]), "input_open");
    let QuoteRowV1::Failed(second) = &rows[1] else {
        unreachable!()
    };
    assert!(matches!(
        second.input_source,
        InputSourceV1::Manifest { row_ordinal: 1, .. }
    ));
}

#[test]
fn cli_complete_batch_and_json_modes() {
    let dir = TestDir::new();
    let json_args = single_args(&dir, true);
    let result = cli()
        .arg("--input-json")
        .arg(json_args.input_json.unwrap())
        .args(["--corpus-id", "corpus", "--sample-id", "sample", "--output"])
        .arg(&json_args.output)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(0));
    let bytes = bincode::serialize(&fixture()).unwrap();
    fs::write(dir.path().join("input.bin"), &bytes).unwrap();
    let mut args = batch_args(
        &dir,
        vec![
            sample("one", "input.bin", &bytes),
            sample("two", "input.bin", &bytes),
        ],
    );
    args.output = dir.path().join("batch.jsonl");
    let result = cli()
        .arg("--manifest")
        .arg(args.manifest.unwrap())
        .arg("--output")
        .arg(&args.output)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(0));
    assert_eq!(rows(&args.output).len(), 2);
}

#[test]
fn single_input_json_has_same_identity_and_output_as_bincode() {
    let binary_dir = TestDir::new();
    let json_dir = TestDir::new();
    let binary_args = single_args(&binary_dir, false);
    let json_args = single_args(&json_dir, true);
    assert_eq!(
        run_quote(&binary_args, &executable()).unwrap().exit_code(),
        0
    );
    assert_eq!(run_quote(&json_args, &executable()).unwrap().exit_code(), 0);
    assert_eq!(
        fs::read(binary_args.output).unwrap(),
        fs::read(json_args.output).unwrap()
    );
}

#[test]
fn single_descriptor_binds_resource_limits() {
    let dir = TestDir::new();
    let mut args = single_args(&dir, false);
    run_quote(&args, &executable()).unwrap();
    let first = rows(&args.output);
    args.output = dir.path().join("other.jsonl");
    args.limits.max_input_bytes -= 1;
    run_quote(&args, &executable()).unwrap();
    let second = rows(&args.output);
    let (QuoteRowV1::Complete(first), QuoteRowV1::Complete(second)) = (&first[0], &second[0])
    else {
        panic!("expected complete rows")
    };
    assert_eq!(first.key, second.key);
    assert_ne!(first.input_source, second.input_source);
}

#[test]
fn api_rejects_every_raised_ceiling_before_input_io() {
    for (field, ceiling) in [
        ("max_manifest_bytes", 16 * 1024 * 1024u64),
        ("max_input_bytes", 512 * 1024 * 1024),
        ("max_total_bytes", 8 * 1024 * 1024 * 1024),
        ("max_rows", 10_000),
        ("max_elapsed_seconds", 3_600),
        ("max_output_bytes", MAX_QUOTE_OUTPUT_BYTES),
    ] {
        let dir = TestDir::new();
        let mut args = QuoteArgs {
            manifest: Some(dir.path().join("must-not-be-opened.json")),
            output: dir.path().join("quotes.jsonl"),
            ..Default::default()
        };
        match field {
            "max_manifest_bytes" => args.limits.max_manifest_bytes = ceiling + 1,
            "max_input_bytes" => args.limits.max_input_bytes = ceiling + 1,
            "max_total_bytes" => args.limits.max_total_bytes = ceiling + 1,
            "max_rows" => args.limits.max_rows = (ceiling + 1) as usize,
            "max_elapsed_seconds" => args.limits.max_elapsed_seconds = ceiling + 1,
            "max_output_bytes" => args.limits.max_output_bytes = ceiling + 1,
            _ => unreachable!(),
        }
        let error = run_quote(&args, &executable()).unwrap_err().to_string();
        assert!(
            error.contains(&format!("{field} exceeds hard ceiling")),
            "{field}: {error}"
        );
        assert!(!args.output.exists(), "{field}");
    }
}

#[test]
fn cli_rejects_every_raised_ceiling_with_structural_exit() {
    for (flag, field, value) in [
        ("--max-manifest-bytes", "max_manifest_bytes", "16777217"),
        ("--max-input-bytes", "max_input_bytes", "536870913"),
        ("--max-total-bytes", "max_total_bytes", "8589934593"),
        ("--max-rows", "max_rows", "10001"),
        ("--max-elapsed-seconds", "max_elapsed_seconds", "3601"),
        ("--max-output-bytes", "max_output_bytes", "67108865"),
    ] {
        let dir = TestDir::new();
        let output = dir.path().join("quotes.jsonl");
        let result = cli()
            .arg("--manifest")
            .arg(dir.path().join("must-not-be-opened.json"))
            .args([flag, value, "--output"])
            .arg(&output)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(1), "{field}");
        let error = String::from_utf8(result.stderr).unwrap();
        assert!(
            error.contains("ERROR") && error.contains("zkgas quote failed"),
            "the product CLI must install a subscriber that writes runtime errors to stderr: {error}"
        );
        assert!(
            error.contains(&format!("{field} exceeds hard ceiling")),
            "{field}: {error}"
        );
        assert!(!output.exists(), "{field}");
    }
}

#[test]
fn callers_can_tighten_every_resource_ceiling() {
    let dir = TestDir::new();
    let mut args = single_args(&dir, false);
    args.limits.max_manifest_bytes -= 1;
    args.limits.max_input_bytes -= 1;
    args.limits.max_total_bytes -= 1;
    args.limits.max_rows -= 1;
    args.limits.max_elapsed_seconds -= 1;
    args.limits.max_output_bytes -= 1;
    assert_eq!(run_quote(&args, &executable()).unwrap().exit_code(), 0);
    assert!(matches!(rows(&args.output)[0], QuoteRowV1::Complete(_)));
}

#[test]
fn quote_output_is_streamed_under_the_downstream_file_ceiling_and_never_partially_published() {
    assert_eq!(MAX_QUOTE_OUTPUT_BYTES, MAX_COMPARE_FILE_BYTES);
    let baseline_dir = TestDir::new();
    let baseline = batch_args(
        &baseline_dir,
        vec![sample("one", "missing.bin", b"missing")],
    );
    run_quote(&baseline, &executable()).unwrap();
    let one_row_bytes = fs::metadata(&baseline.output).unwrap().len();

    let dir = TestDir::new();
    let mut args = batch_args(
        &dir,
        vec![
            sample("one", "missing.bin", b"missing"),
            sample("two", "missing.bin", b"missing"),
        ],
    );
    args.limits.max_output_bytes = one_row_bytes;
    let error = run_quote(&args, &executable()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("quote output byte limit exceeded")
    );
    assert!(!args.output.exists());

    fs::write(&args.output, b"existing-output").unwrap();
    assert!(run_quote(&args, &executable()).is_err());
    assert_eq!(fs::read(&args.output).unwrap(), b"existing-output");
    assert_eq!(
        fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(".zkgas-file-"))
            .count(),
        0
    );
}

fn batch_args(dir: &TestDir, samples: Vec<InputSampleV1>) -> QuoteArgs {
    let path = dir.path().join("manifest.json");
    fs::write(
        &path,
        serde_json::to_vec(&InputManifestV1 {
            schema_version: 1,
            corpus_id: "batch".into(),
            samples,
        })
        .unwrap(),
    )
    .unwrap();
    QuoteArgs {
        manifest: Some(path),
        output: dir.path().join("quotes.jsonl"),
        ..Default::default()
    }
}
fn sample(id: &str, path: &str, bytes: &[u8]) -> InputSampleV1 {
    InputSampleV1 {
        sample_id: id.into(),
        guest_input: InputFile {
            path: path.into(),
            sha256: sha256_hex(bytes),
            byte_length: bytes.len() as u64,
        },
        metadata: None,
    }
}
fn failure_code(row: &QuoteRowV1) -> &str {
    let QuoteRowV1::Failed(row) = row else {
        panic!("expected failed row")
    };
    &row.failure.code
}

#[test]
fn manifest_preserves_order_raw_hash_and_metadata() {
    let dir = TestDir::new();
    let bytes = bincode::serialize(&fixture()).unwrap();
    fs::write(dir.path().join("input.bin"), &bytes).unwrap();
    let mut second = sample("alpha", "input.bin", &bytes);
    second.metadata = Some(serde_json::from_str(r#"{"producer":"fixture"}"#).unwrap());
    let args = batch_args(&dir, vec![sample("zeta", "input.bin", &bytes), second]);
    assert_eq!(run_quote(&args, &executable()).unwrap().exit_code(), 0);
    let manifest_hash = sha256_hex(&fs::read(args.manifest.unwrap()).unwrap());
    let rows = rows(&args.output);
    for (ordinal, id) in ["zeta", "alpha"].into_iter().enumerate() {
        let QuoteRowV1::Complete(row) = &rows[ordinal] else {
            panic!("expected complete")
        };
        assert_eq!(row.key.sample_id, id);
        assert_eq!(
            row.input_source,
            InputSourceV1::Manifest {
                manifest_sha256: manifest_hash.clone(),
                row_ordinal: ordinal as u64
            }
        );
    }
    let QuoteRowV1::Complete(second) = &rows[1] else {
        unreachable!()
    };
    assert_eq!(
        second.metadata.as_ref().unwrap().as_map()["producer"],
        "fixture"
    );
}

#[test]
fn invalid_manifest_and_configuration_do_not_publish() {
    for case in [
        "duplicate",
        "schema",
        "rows",
        "manifest_bytes",
        "empty",
        "bad_hash",
        "missing_id",
        "concurrency",
    ] {
        let dir = TestDir::new();
        let mut samples = vec![sample("sample", "never-opened", b"input")];
        if case == "duplicate" {
            samples.push(samples[0].clone());
        }
        if case == "empty" {
            samples.clear();
        }
        if case == "bad_hash" {
            samples[0].guest_input.sha256 = "invalid".into();
        }
        if case == "missing_id" {
            samples[0].sample_id.clear();
        }
        let mut args = batch_args(&dir, samples);
        match case {
            "schema" => {
                fs::write(args.manifest.as_ref().unwrap(), b"{\"schema_version\":2}").unwrap()
            }
            "rows" => args.limits.max_rows = 0,
            "manifest_bytes" => args.limits.max_manifest_bytes = 1,
            "concurrency" => args.limits.concurrency = 2,
            _ => (),
        }
        assert!(run_quote(&args, &executable()).is_err(), "{case}");
        assert!(!args.output.exists(), "{case}");
    }
}

#[test]
fn manifest_semantic_failures_commit_typed_rows() {
    for (case, code) in [
        ("hash", "input_hash_mismatch"),
        ("length", "input_length_mismatch"),
        ("absolute", "input_path"),
        ("parent", "input_path"),
        ("missing", "input_open"),
        ("symlink", "input_open"),
        ("nested_symlink", "input_open"),
        ("directory", "input_not_regular"),
        ("fifo", "input_not_regular"),
        ("per_file", "input_byte_limit"),
        ("total", "total_byte_limit"),
        ("decode", "input_decode"),
        ("empty", "empty_proposal"),
        ("trailing", "noncanonical_bincode"),
        ("elapsed", "elapsed_time_limit"),
    ] {
        let dir = TestDir::new();
        let bytes = if case == "elapsed" {
            bincode::serialize(&fixture()).unwrap()
        } else if case == "decode" {
            b"invalid".to_vec()
        } else {
            bincode::serialize(&GuestInput::default()).unwrap()
        };
        fs::write(dir.path().join("input.bin"), &bytes).unwrap();
        let mut row = sample("sample", "input.bin", &bytes);
        match case {
            "hash" => row.guest_input.sha256 = "00".repeat(32),
            "length" => row.guest_input.byte_length += 1,
            "absolute" => {
                row.guest_input.path = dir.path().join("input.bin").to_str().unwrap().into()
            }
            "parent" => row.guest_input.path = "../input.bin".into(),
            "missing" => row.guest_input.path = "missing.bin".into(),
            "symlink" => {
                std::os::unix::fs::symlink("input.bin", dir.path().join("link")).unwrap();
                row.guest_input.path = "link".into();
            }
            "nested_symlink" => {
                std::os::unix::fs::symlink(".", dir.path().join("link")).unwrap();
                row.guest_input.path = "link/input.bin".into();
            }
            "directory" => row.guest_input.path = ".".into(),
            "fifo" => {
                create_fifo(&dir.path().join("pipe"));
                row.guest_input.path = "pipe".into();
            }
            "trailing" => {
                let mut extra = bytes.clone();
                extra.push(0);
                fs::write(dir.path().join("input.bin"), &extra).unwrap();
                row = sample("sample", "input.bin", &extra);
            }
            _ => (),
        }
        let mut args = batch_args(&dir, vec![row]);
        match case {
            "per_file" => args.limits.max_input_bytes = 1,
            "total" => args.limits.max_total_bytes = 1,
            "elapsed" => args.limits.max_elapsed_seconds = 0,
            _ => (),
        }
        assert_eq!(
            run_quote(&args, &executable()).unwrap().exit_code(),
            2,
            "{case}"
        );
        assert_eq!(failure_code(&rows(&args.output)[0]), code, "{case}");
    }
}

#[test]
fn single_input_identity_mismatch_is_a_typed_partial_failure() {
    for case in [
        "carry_chain",
        "carry_proposal",
        "manifest_proposal",
        "witness_chain",
    ] {
        let dir = TestDir::new();
        let mut args = single_args(&dir, false);
        let mut input = fixture();
        let code = match case {
            "carry_chain" => {
                input.proof_carry_data.chain_id += 1;
                "proposal_identity_mismatch"
            }
            "carry_proposal" => {
                input.proof_carry_data.transition_input.proposal_id += 1;
                "proposal_identity_mismatch"
            }
            "manifest_proposal" => {
                input.taiko.proposal_id += 1;
                "proposal_identity_mismatch"
            }
            _ => {
                input.witnesses[0].chain_spec.chain_id += 1;
                "chain_identity_mismatch"
            }
        };
        fs::write(
            args.input_bincode.as_ref().unwrap(),
            bincode::serialize(&input).unwrap(),
        )
        .unwrap();
        args.output = dir.path().join("failure.jsonl");
        assert_eq!(run_quote(&args, &executable()).unwrap().exit_code(), 2);
        assert_eq!(failure_code(&rows(&args.output)[0]), code, "{case}");
    }
}

#[test]
fn failed_quote_only_claims_identity_after_canonical_decode() {
    for (case, observed, verified) in [
        ("missing", false, false),
        ("oversized", false, false),
        ("decode", true, false),
        ("noncanonical", true, false),
        ("semantic", true, true),
    ] {
        let dir = TestDir::new();
        let path = dir.path().join("input.bin");
        let mut bytes = bincode::serialize(&GuestInput::default()).unwrap();
        if case == "decode" {
            bytes = b"invalid".to_vec();
        }
        if case == "noncanonical" {
            bytes.push(0);
        }
        if case != "missing" {
            fs::write(&path, &bytes).unwrap();
        }
        let mut args = QuoteArgs {
            input_bincode: Some(path),
            corpus_id: Some("corpus".into()),
            sample_id: Some("sample".into()),
            output: dir.path().join("quotes.jsonl"),
            ..Default::default()
        };
        if case == "oversized" {
            args.limits.max_input_bytes = 1;
        }
        assert_eq!(run_quote(&args, &executable()).unwrap().exit_code(), 2);
        let row = &rows(&args.output)[0];
        let value = serde_json::to_value(row).unwrap();
        assert_eq!(value["corpus_id"], "corpus");
        assert_eq!(value["sample_id"], "sample");
        assert!(value.get("key").is_none());
        assert!(value.get("guest_input_byte_length").is_none());
        assert_eq!(value.get("verified_key").is_some(), verified, "{case}");
        assert_eq!(
            value.get("observed_guest_input_sha256").is_some(),
            observed,
            "{case}"
        );
        assert_eq!(
            value.get("observed_guest_input_byte_length").is_some(),
            observed,
            "{case}"
        );
        if observed {
            assert_eq!(value["observed_guest_input_sha256"], sha256_hex(&bytes));
            assert_eq!(
                value["observed_guest_input_byte_length"],
                bytes.len() as u64
            );
        }
        if verified {
            assert_eq!(value["verified_key"]["corpus_id"], "corpus");
            assert_eq!(value["verified_key"]["sample_id"], "sample");
            assert_eq!(
                value["verified_key"]["guest_input_sha256"],
                sha256_hex(&bytes)
            );
        }
    }
    let dir = TestDir::new();
    let bytes = bincode::serialize(&GuestInput::default()).unwrap();
    fs::write(dir.path().join("input.bin"), &bytes).unwrap();
    let mut sample = sample("sample", "input.bin", &bytes);
    sample.guest_input.sha256 = "00".repeat(32);
    let args = batch_args(&dir, vec![sample]);
    assert_eq!(run_quote(&args, &executable()).unwrap().exit_code(), 2);
    let value = serde_json::to_value(&rows(&args.output)[0]).unwrap();
    assert!(value.get("verified_key").is_none());
    assert_eq!(value["observed_guest_input_sha256"], sha256_hex(&bytes));
}
