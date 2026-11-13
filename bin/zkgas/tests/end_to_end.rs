//! Real binary composition; observations are synthetic examples, never measured SP1 truth.
mod common;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use common::trusted_temp_root;
use raiko2_primitives_shasta::GuestInput;
use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
use serde_json::json;
use zkgas::{
    compare::{comparison_row_id, comparison_run_id, row_sequence_sha256},
    contracts::*,
    identity::{canonical_json, sha256_hex},
    report::{build_report, verify_comparison_run},
};

struct TestDir(PathBuf);
impl TestDir {
    fn new() -> Self {
        let path = trusted_temp_root().join(format!("zkgas-e2e-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn invoke(binary: &str, args: &[&str], paths: &[(&str, &Path)], expected: i32) -> Output {
    let mut command = Command::new(binary);
    command.args(args);
    for (flag, path) in paths {
        command.arg(flag).arg(path);
    }
    let output = command.output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(expected),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    output
}

fn observation(quote: &CompleteQuoteV1, backend: &str, unit: &str) -> ObservationRowV1 {
    ObservationRowV1::Complete(Box::new(CompleteObservationV1 {
        observation_id: format!("synthetic-{backend}"),
        attempt_id: "example-1".into(),
        quote_key: quote.key.clone(),
        backend: backend.into(),
        workload: Workload {
            value: 100,
            unit: unit.into(),
        },
        measurement_stage: "execute".into(),
        program: ProgramIdentity {
            elf_sha256: "ba5ab7026643730c4416550a069fd0776f0e11ae09155ec5fce413acbe5e59c7".into(),
            serialized_vk_sha256: Some(
                "bd71c1f1cbd72387aa057ae6e936afd6dcb0bc170c6a91193c6d1a0b47507102".into(),
            ),
            sp1_vkey: None,
            risc0_image_id: (backend == "risc0").then(|| "synthetic-image-id".into()),
        },
        backend_version: Some("synthetic-test-fixture".into()),
        software_version: None,
        replicate_group: None,
        source: Some(
            OpaqueObject::new(BTreeMap::from([(
                "kind".into(),
                json!("synthetic test fixture; no guest execution or measurement"),
            )]))
            .unwrap(),
        ),
    }))
}

fn verify_store(path: &Path, expected_status: RunStatus) -> ComparisonManifestV1 {
    verify_comparison_run(path).expect("public integrity verification must accept the whole store");
    let manifest_bytes = fs::read(path.join("manifest.json")).unwrap();
    let manifest: ComparisonManifestV1 = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest.status, expected_status);
    assert_eq!(
        manifest.executable.sha256,
        sha256_hex(&fs::read(env!("CARGO_BIN_EXE_zkgas-compare")).unwrap())
    );
    assert_eq!(manifest.run_id, comparison_run_id(&manifest).unwrap());
    assert_eq!(
        manifest.row_sequence_sha256,
        row_sequence_sha256(&manifest.rows).unwrap()
    );
    for digest in &manifest.rows {
        let bytes = fs::read(
            path.join("rows")
                .join(format!("{}.json", digest.comparison_row_id)),
        )
        .unwrap();
        assert_eq!(sha256_hex(&bytes), digest.row_sha256);
        let row: ComparisonRowV1 = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(row.comparison_row_id, digest.comparison_row_id);
        assert_eq!(
            row.comparison_row_id,
            comparison_row_id(&row.result).unwrap()
        );
        match row.result {
            ComparisonResultV1::Joined {
                quote,
                observation,
                metrics,
                normalization,
                ..
            } => {
                assert_eq!(observation.quote_key, quote.key);
                let ratio = metrics.prediction_over_actual.unwrap();
                assert_eq!(ratio.numerator, quote.quote.prove.total_pgu.to_string());
                assert_eq!(ratio.denominator, "100");
                assert_eq!(metrics.underquoted, quote.quote.prove.total_pgu < 100);
                assert!(metrics.actual_over_prediction.is_some());
                assert!(normalization.is_none());
            }
            ComparisonResultV1::IncompatibleProfile {
                quote,
                observation,
                cross_program_ratio,
                normalization,
                ..
            } => {
                assert_eq!(observation.quote_key, quote.key);
                assert_eq!(observation.workload.unit, "risc0_cycles");
                assert!(
                    cross_program_ratio.is_none(),
                    "different units have no raw ratio"
                );
                assert!(normalization.is_none());
            }
            ComparisonResultV1::UnmatchedObservation { .. } => {}
            other => panic!("unexpected comparison result: {other:?}"),
        }
    }
    manifest
}

fn verify_report(path: &Path, run: &Path, config: &ReportConfigV1) -> ReportV1 {
    let bytes = fs::read(path).unwrap();
    let report: ReportV1 = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        report.executable.sha256,
        sha256_hex(&fs::read(env!("CARGO_BIN_EXE_zkgas-report")).unwrap())
    );
    let verified = verify_comparison_run(run).unwrap();
    let rebuilt = build_report(&verified, config, &report.executable).unwrap();
    assert_eq!(
        report, rebuilt,
        "public verifier/builder must reproduce report identity and metrics"
    );
    assert_eq!(
        report.config_sha256,
        sha256_hex(&canonical_json(config).unwrap())
    );
    assert_eq!(
        report.comparison_manifest_sha256,
        sha256_hex(&fs::read(run.join("manifest.json")).unwrap())
    );
    assert_eq!(bytes, canonical_json(&report).unwrap());
    report
}

#[test]
fn binaries_compose_complete_and_partial_runs_with_verified_identity_and_reuse() {
    // Removing unit gating, ignoring unmatched rows, or losing canonical input identity breaks
    // these assertions. Nothing in this test invokes a guest or an observation producer.
    let dir = TestDir::new();
    let root = &dir.0;
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_41455_l2_12556056_12556247.json",
    );
    let input: GuestInput = serde_json::from_slice(&fs::read(fixture).unwrap()).unwrap();
    let expected_output = hash_shasta_subproof_input(&input.proof_carry_data);
    let canonical = bincode::serialize(&input).unwrap();
    let input_path = root.join("input.bin");
    fs::write(&input_path, &canonical).unwrap();
    let quotes_path = root.join("quotes.jsonl");
    let quote = || {
        invoke(
            env!("CARGO_BIN_EXE_zkgas-quote"),
            &[
                "--corpus-id",
                "synthetic-e2e",
                "--sample-id",
                "mainnet-proposal-41455",
            ],
            &[("--input-bincode", &input_path), ("--output", &quotes_path)],
            0,
        )
    };
    quote();
    let quote_bytes = fs::read(&quotes_path).unwrap();
    let quote_rows: Vec<QuoteRowV1> = std::str::from_utf8(&quote_bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(quote_rows.len(), 1);
    let QuoteRowV1::Complete(quoted) = &quote_rows[0] else {
        panic!("complete quote required")
    };
    assert_eq!(quoted.quote.public_output, expected_output);
    assert_eq!(quoted.proposal.public_output, expected_output.to_string());
    assert_eq!(quoted.key.guest_input_sha256, sha256_hex(&canonical));
    assert_eq!(quoted.quote.guest_input_sha256, sha256_hex(&canonical));
    assert_eq!(quoted.guest_input_byte_length, canonical.len() as u64);
    assert_eq!(quoted.quote.guest_input_bincode_length, canonical.len());
    assert_eq!(
        quoted.executable.sha256,
        sha256_hex(&fs::read(env!("CARGO_BIN_EXE_zkgas-quote")).unwrap())
    );
    assert_eq!(quoted.proposal.proposal_id, 41455);
    assert_eq!(quoted.proposal.block_count, 192);
    assert_eq!(quoted.proposal.chain.chain_id, 167000);
    assert!(
        !std::str::from_utf8(&quote_bytes)
            .unwrap()
            .contains(root.to_str().unwrap())
    );
    quote();
    assert_eq!(quote_bytes, fs::read(&quotes_path).unwrap());

    let sp1 = observation(quoted, "sp1", "sp1_pgu");
    let risc0 = observation(quoted, "risc0", "risc0_cycles");
    let ObservationRowV1::Complete(sp1_row) = &sp1 else {
        unreachable!()
    };
    let sp1_row = sp1_row.as_ref().clone();
    let profiles = CompatibilityProfilesV1 {
        schema_version: 1,
        profiles: vec![CompatibilityProfileV1 {
            profile_id: "synthetic-calibration-example".into(),
            model_id: quoted.quote.model_id.clone(),
            backend: "sp1".into(),
            prediction_unit: "sp1_pgu".into(),
            observation_unit: "sp1_pgu".into(),
            measurement_stage: "execute".into(),
            program: sp1_row.program.clone(),
            backend_version: sp1_row.backend_version.clone(),
            runtime_spec_sha256: Some(quoted.quote.model_id.clone()),
            normalization: None,
        }],
    };
    let profiles_path = root.join("profiles.toml");
    fs::write(&profiles_path, toml::to_string(&profiles).unwrap()).unwrap();
    let mut observations = ObservationManifestV1 {
        schema_version: 1,
        observation_set_id: "synthetic-test-fixtures".into(),
        observations: vec![sp1, risc0],
    };
    let observations_path = root.join("observations.json");
    fs::write(&observations_path, canonical_json(&observations).unwrap()).unwrap();
    let store = root.join("comparisons");
    let compare = |expected| {
        invoke(
            env!("CARGO_BIN_EXE_zkgas-compare"),
            &[],
            &[
                ("--quotes", &quotes_path),
                ("--observations", &observations_path),
                ("--profiles", &profiles_path),
                ("--output", &store),
            ],
            expected,
        )
    };
    let output = compare(0);
    let run_id = String::from_utf8(output.stdout).unwrap().trim().to_owned();
    let run = store.join("runs").join(&run_id);
    let manifest = verify_store(&run, RunStatus::Complete);
    assert_eq!(
        manifest.counts,
        BTreeMap::from([
            (ComparisonKind::Joined, 1),
            (ComparisonKind::IncompatibleProfile, 1)
        ])
    );
    assert_eq!(manifest.quote_file_sha256, vec![sha256_hex(&quote_bytes)]);
    assert_eq!(
        manifest.observation_file_sha256,
        vec![sha256_hex(&fs::read(&observations_path).unwrap())]
    );
    assert_eq!(
        manifest.profiles_sha256,
        sha256_hex(&fs::read(&profiles_path).unwrap())
    );
    assert_eq!(String::from_utf8(compare(0).stdout).unwrap().trim(), run_id);
    assert_eq!(verify_store(&run, RunStatus::Complete), manifest);

    let mut config = ReportConfigV1 {
        schema_version: 1,
        filters: ReportFiltersV1::default(),
        quantiles: vec!["0.5".into(), "1".into()],
        buckets: vec![],
    };
    let config_path = root.join("report-config.json");
    fs::write(&config_path, canonical_json(&config).unwrap()).unwrap();
    let report_path = root.join("report.json");
    let report_cli = |run: &Path, path: &Path, expected| {
        invoke(
            env!("CARGO_BIN_EXE_zkgas-report"),
            &[],
            &[
                ("--comparisons", run),
                ("--config", &config_path),
                ("--output", path),
            ],
            expected,
        )
    };
    report_cli(&run, &report_path, 0);
    let report = verify_report(&report_path, &run, &config);
    assert_eq!(report.status, RunStatus::Complete);
    assert_eq!(report.whole_run_counts, manifest.counts);
    assert_eq!(report.selected_counts, manifest.counts);
    assert_eq!(report.strata.len(), 2);
    let compatible = report
        .strata
        .iter()
        .find(|s| s.identity.compatible)
        .unwrap();
    assert_eq!((compatible.row_count, compatible.eligible_count), (1, 1));
    assert_eq!(compatible.identity.backend, "sp1");
    assert!(compatible.underquote_rate.is_some());
    assert_eq!(compatible.quantiles.len(), 2);
    assert!(
        compatible
            .workload_weighted_prediction_over_actual
            .is_some()
    );
    let incompatible = report
        .strata
        .iter()
        .find(|s| !s.identity.compatible)
        .unwrap();
    assert_eq!(
        (incompatible.row_count, incompatible.eligible_count),
        (1, 0)
    );
    assert_eq!(incompatible.identity.workload_unit, "risc0_cycles");
    assert!(incompatible.count_weighted_prediction_over_actual.is_none());
    assert!(incompatible.quantiles.is_empty());
    assert!(
        incompatible
            .diagnostics
            .contains_key("direct_metrics_unavailable")
    );
    for stratum in &report.strata {
        assert_eq!(stratum.identity.chain_id, quoted.proposal.chain.chain_id);
        assert_eq!(
            stratum.identity.fork_topology,
            quoted
                .proposal
                .fork_segments
                .iter()
                .map(|segment| ForkTopologySegmentV1 {
                    fork: segment.fork.clone(),
                    first_block_offset: segment.first_block - quoted.proposal.first_block,
                })
                .collect::<Vec<_>>()
        );
        assert_eq!(stratum.identity.model_id, quoted.quote.model_id);
        assert_eq!(stratum.identity.measurement_stage, "execute");
        assert!(stratum.identity.normalization_sha256.is_none());
    }
    assert!(
        incompatible
            .workload_weighted_prediction_over_actual
            .is_none()
    );
    assert!(incompatible.underquote_rate.is_none());
    let report_bytes = fs::read(&report_path).unwrap();
    report_cli(&run, &report_path, 0);
    assert_eq!(report_bytes, fs::read(&report_path).unwrap());

    let mut unmatched = sp1_row.clone();
    unmatched.observation_id = "synthetic-unmatched".into();
    unmatched.quote_key.sample_id = "absent-sample".into();
    observations
        .observations
        .push(ObservationRowV1::Complete(Box::new(unmatched)));
    fs::write(&observations_path, canonical_json(&observations).unwrap()).unwrap();
    let partial_id = String::from_utf8(compare(2).stdout)
        .unwrap()
        .trim()
        .to_owned();
    assert_ne!(partial_id, run_id);
    let partial_run = store.join("runs").join(&partial_id);
    let partial_manifest = verify_store(&partial_run, RunStatus::Partial);
    assert_eq!(
        partial_manifest
            .counts
            .get(&ComparisonKind::UnmatchedObservation),
        Some(&1)
    );
    let partial_report_path = root.join("partial-report.json");
    report_cli(&partial_run, &partial_report_path, 2);
    let partial_report = verify_report(&partial_report_path, &partial_run, &config);
    assert_eq!(partial_report.status, RunStatus::Partial);
    assert_eq!(partial_report.whole_run_counts, partial_manifest.counts);
    assert_eq!(partial_report.selected_counts, partial_manifest.counts);
    assert_eq!(partial_report.strata.len(), 2);
    assert_eq!(partial_report.excluded.len(), 1);
    assert_eq!(partial_report.excluded[0].reason, "unmatched_observation");

    config.filters.backends = vec!["sp1".into()];
    fs::write(&config_path, canonical_json(&config).unwrap()).unwrap();
    let selected_path = root.join("selected-report.json");
    report_cli(&partial_run, &selected_path, 2);
    let selected = verify_report(&selected_path, &partial_run, &config);
    assert_eq!(selected.whole_run_counts, partial_manifest.counts);
    assert_eq!(
        selected.selected_counts,
        BTreeMap::from([
            (ComparisonKind::Joined, 1),
            (ComparisonKind::UnmatchedObservation, 1)
        ])
    );
    assert_eq!(selected.strata.len(), 1);
    assert_eq!(selected.excluded.len(), 2);
    assert_ne!(selected.report_id, partial_report.report_id);
    assert_eq!(fs::read_dir(store.join("runs")).unwrap().count(), 2);
    assert_eq!(verify_store(&run, RunStatus::Complete), manifest);
}
