mod common;

use common::trusted_temp_root;
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use zkgas::{
    arithmetic::ARITHMETIC_POLICY,
    compare::{comparison_kind, comparison_row_id, comparison_run_id, row_sequence_sha256},
    contracts::*,
    identity::{canonical_json, sha256_hex},
    output::CommitOutcome,
    report::{ReportArgs, build_report, run_report, verify_comparison_run},
};

struct TestDir(PathBuf);
impl TestDir {
    fn new() -> Self {
        let path = trusted_temp_root().join(format!("zkgas-report-test-{}", uuid::Uuid::new_v4()));
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
fn executable() -> ExecutableIdentity {
    ExecutableIdentity {
        sha256: "report-executable".into(),
        package_version: "test".into(),
        source_revision: None,
        dirty: None,
    }
}
fn config() -> ReportConfigV1 {
    ReportConfigV1 {
        schema_version: 1,
        filters: ReportFiltersV1::default(),
        quantiles: vec!["0".into(), "0.5".into(), "1".into()],
        buckets: vec![],
    }
}
fn invalid(id: &str) -> ComparisonResultV1 {
    ComparisonResultV1::InvalidObservation {
        observation_set_id: "set".into(),
        observation: FailedObservationV1 {
            observation_id: id.into(),
            attempt_id: "attempt".into(),
            quote_key: None,
            failure: FailureV1 {
                code: "failed".into(),
                message: "unavailable".into(),
            },
            source: None,
        },
        observation_row_sha256: "source".into(),
    }
}
fn fixture(dir: &TestDir, results: Vec<ComparisonResultV1>) -> PathBuf {
    let mut counts = BTreeMap::new();
    let mut digests = Vec::new();
    let mut rows = Vec::new();
    for result in results {
        *counts.entry(comparison_kind(&result)).or_insert(0) += 1;
        let id = comparison_row_id(&result).unwrap();
        let bytes = canonical_json(&ComparisonRowV1 {
            schema_version: 1,
            comparison_row_id: id.clone(),
            result,
        })
        .unwrap();
        digests.push(RowDigestV1 {
            comparison_row_id: id.clone(),
            row_sha256: sha256_hex(&bytes),
        });
        rows.push((id, bytes));
    }
    let status = if counts.keys().all(|k| {
        matches!(
            k,
            ComparisonKind::Joined | ComparisonKind::IncompatibleProfile
        )
    }) {
        RunStatus::Complete
    } else {
        RunStatus::Partial
    };
    let mut manifest = ComparisonManifestV1 {
        schema_version: 1,
        run_id: String::new(),
        executable: executable(),
        quote_file_sha256: vec![],
        observation_file_sha256: vec![],
        profiles_sha256: "profiles".into(),
        arithmetic_policy: ARITHMETIC_POLICY.into(),
        model_ids: vec![],
        profile_ids: vec![],
        row_sequence_sha256: row_sequence_sha256(&digests).unwrap(),
        rows: digests,
        status,
        counts,
    };
    manifest.run_id = comparison_run_id(&manifest).unwrap();
    let path = dir.path().join(&manifest.run_id);
    fs::create_dir_all(path.join("rows")).unwrap();
    fs::write(
        path.join("manifest.json"),
        canonical_json(&manifest).unwrap(),
    )
    .unwrap();
    for (id, bytes) in rows {
        fs::write(path.join("rows").join(format!("{id}.json")), bytes).unwrap();
    }
    path
}
fn manifest(path: &Path) -> ComparisonManifestV1 {
    serde_json::from_slice(&fs::read(path.join("manifest.json")).unwrap()).unwrap()
}
fn row_path(path: &Path, index: usize) -> PathBuf {
    path.join("rows").join(format!(
        "{}.json",
        manifest(path).rows[index].comparison_row_id
    ))
}
fn args(dir: &TestDir, comparisons: PathBuf) -> ReportArgs {
    let config_path = dir.path().join("report-config.json");
    fs::write(&config_path, canonical_json(&config()).unwrap()).unwrap();
    ReportArgs {
        comparisons,
        config: config_path,
        output: dir.path().join("report.json"),
    }
}

fn pair(index: usize, prediction: u128, actual: u128, compatible: bool) -> ComparisonResultV1 {
    let quote: CompleteQuoteV1 = serde_json::from_value(json!({
        "schema_version":1,"key":{"corpus_id":"corpus","sample_id":format!("sample-{index}"),"guest_input_sha256":"input"},
        "executable":executable(),"input_source":{"kind":"single","descriptor_sha256":"descriptor"},"guest_input_byte_length":12,
        "proposal":{"proposal_id":1,"chain":{"name":"sample-chain","chain_id":1},"proof_carry_chain_id":1,"proof_carry_proposal_id":1,"witness_chain_spec_sha256":["spec"],"first_block":2,"last_block":2,"block_count":1,"public_output":"output","fork_segments":[{"fork":"SHASTA","first_block":2,"last_block":2}]},
        "prediction_unit":"sp1_pgu","prediction_surface":"prove","calibration_profile":"model",
        "quote":{"schema_version":1,"model_id":"model","guest_input_sha256":"input","guest_input_bincode_length":12,"public_output":format!("0x{}", "00".repeat(32)),"blocks":[],
            "node":{"unzen_zkgas_sum":"2","block_local_known_scaled":"0","block_local_known_pgu":"0","sum_of_block_known_pgu":"0"},
            "prove":{"selectors":{"payload":{"total_transaction_count":0,"canonical_transaction_bytes":0,"nonempty_payload_transactions":0},"true_unattempted_transactions":0,"blobs":{"source_blob_counts":[],"proposal_blob_hash_count":0,"provider_blob_count":0,"provider_commitment_count":0,"provider_proof_count":0},"parent_mpt":{"path_node_visits":0,"materialization_work_units":0,"hydration_hash_blocks":0,"dirty_encoded_nodes":0,"dirty_hash_blocks":0,"structural_branch_extension_churn_nodes":0,"dropped_trie_nodes":0}},
                "block_local_scaled":"0","block_local_pgu":"0","sum_of_block_local_pgu":"0","proposal_once":{"access_list_scaled":"0","authorization_scaled":"0","fixed_scaled":"0","operations_scaled":"0","feature_error_scaled":"0","resources_scaled":"0","mpt_scaled":"0","startup_scaled":"0","transaction_payload_scaled":"0","blob_source_scaled":"0","parent_mpt_scaled":"0","total_scaled":"0","total_pgu":"0"},"proposal_once_pgu":"0","total_scaled":"0","total_pgu":prediction.to_string()}}
    })).unwrap();
    let observation = CompleteObservationV1 {
        observation_id: format!("observation-{index}"),
        attempt_id: "attempt".into(),
        quote_key: quote.key.clone(),
        backend: if compatible { "sp1" } else { "risc0" }.into(),
        workload: Workload {
            value: actual,
            unit: if compatible {
                "sp1_pgu"
            } else {
                "risc0_cycles"
            }
            .into(),
        },
        measurement_stage: "execute".into(),
        program: ProgramIdentity {
            elf_sha256: "elf".into(),
            serialized_vk_sha256: Some("vk".into()),
            sp1_vkey: None,
            risc0_image_id: None,
        },
        backend_version: Some("version".into()),
        software_version: None,
        replicate_group: None,
        source: None,
    };
    let observation_key = ObservationKey {
        observation_set_id: "set".into(),
        observation_id: observation.observation_id.clone(),
        attempt_id: observation.attempt_id.clone(),
    };
    if compatible {
        ComparisonResultV1::Joined {
            quote: Box::new(quote),
            observation,
            observation_key,
            profile_id: "profile".into(),
            metrics: zkgas::arithmetic::direct_metrics(&prediction.into(), &actual.into()).unwrap(),
            normalization: None,
        }
    } else {
        ComparisonResultV1::IncompatibleProfile {
            quote: Box::new(quote),
            observation,
            observation_key,
            profiles_sha256: "profiles".into(),
            reason: "no_matching_profile".into(),
            cross_program_ratio: None,
            normalization: None,
        }
    }
}
fn report_for(results: Vec<ComparisonResultV1>, config: &ReportConfigV1) -> ReportV1 {
    let dir = TestDir::new();
    let path = fixture(&dir, results);
    build_report(
        &verify_comparison_run(&path).unwrap(),
        config,
        &executable(),
    )
    .unwrap()
}

#[test]
fn integrity_accepts_the_exact_tree_and_rejects_every_row_mutation() {
    for mutation in [
        "missing",
        "extra",
        "renamed",
        "bytes",
        "symlink",
        "directory",
        "root-extra",
        "rows-symlink",
    ] {
        let dir = TestDir::new();
        let path = fixture(&dir, vec![invalid("a"), invalid("b")]);
        verify_comparison_run(&path).unwrap();
        let row = row_path(&path, 0);
        match mutation {
            "missing" => fs::remove_file(&row).unwrap(),
            "extra" => fs::write(path.join("rows/extra.json"), b"{}").unwrap(),
            "renamed" => fs::rename(&row, path.join("rows/renamed.json")).unwrap(),
            "bytes" => {
                let mut bytes = fs::read(&row).unwrap();
                bytes.push(b' ');
                fs::write(&row, bytes).unwrap();
            }
            "symlink" => {
                fs::remove_file(&row).unwrap();
                std::os::unix::fs::symlink("../manifest.json", &row).unwrap();
            }
            "directory" => {
                fs::remove_file(&row).unwrap();
                fs::create_dir(&row).unwrap();
            }
            "root-extra" => fs::write(path.join("extra"), b"extra").unwrap(),
            "rows-symlink" => {
                fs::rename(path.join("rows"), path.join("elsewhere")).unwrap();
                std::os::unix::fs::symlink("elsewhere", path.join("rows")).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(verify_comparison_run(&path).is_err(), "accepted {mutation}");
    }
}

#[test]
fn integrity_rejects_manifest_sequence_identity_counts_status_and_unknown_fields() {
    for mutation in ["order", "sequence", "count", "status", "run-id", "unknown"] {
        let dir = TestDir::new();
        let path = fixture(&dir, vec![invalid("a"), invalid("b")]);
        let mut value = serde_json::to_value(manifest(&path)).unwrap();
        match mutation {
            "order" => value["rows"].as_array_mut().unwrap().reverse(),
            "sequence" => value["row_sequence_sha256"] = json!("changed"),
            "count" => value["counts"]["invalid_observation"] = json!(3),
            "status" => value["status"] = json!("complete"),
            "run-id" => value["run_id"] = json!("changed"),
            "unknown" => value["unknown"] = json!(true),
            _ => unreachable!(),
        }
        fs::write(path.join("manifest.json"), canonical_json(&value).unwrap()).unwrap();
        assert!(verify_comparison_run(&path).is_err(), "accepted {mutation}");
    }
}

#[test]
fn integrity_report_identity_binds_raw_config_executable_and_verified_manifest() {
    let dir = TestDir::new();
    let path = fixture(&dir, vec![invalid("a")]);
    let args = args(&dir, path.clone());
    let first = run_report(&args, &executable()).unwrap();
    assert_eq!(first.commit, CommitOutcome::Created);
    assert_eq!(
        run_report(&args, &executable()).unwrap().commit,
        CommitOutcome::Reused
    );
    let bytes = fs::read(&args.output).unwrap();
    let mut different = executable();
    different.sha256 = "different".into();
    assert!(run_report(&args, &different).is_err());
    assert_eq!(fs::read(&args.output).unwrap(), bytes);
    let mut raw = fs::read(&args.config).unwrap();
    raw.push(b' ');
    fs::write(&args.config, raw).unwrap();
    let mut other = args.clone();
    other.output = dir.path().join("other-report.json");
    let whitespace = run_report(&other, &executable()).unwrap();
    assert_ne!(first.report.report_id, whitespace.report.report_id);
    let verified = verify_comparison_run(&path).unwrap();
    let library = build_report(&verified, &config(), &executable()).unwrap();
    assert_eq!(
        library,
        build_report(&verified, &config(), &executable()).unwrap()
    );
    let mut changed_config = config();
    changed_config.quantiles.clear();
    assert_ne!(
        library.report_id,
        build_report(&verified, &changed_config, &executable())
            .unwrap()
            .report_id
    );
    assert_ne!(
        library.report_id,
        build_report(&verified, &config(), &different)
            .unwrap()
            .report_id
    );
    let other_dir = TestDir::new();
    let other_path = fixture(&other_dir, vec![invalid("different-row")]);
    let changed = build_report(
        &verify_comparison_run(&other_path).unwrap(),
        &config(),
        &executable(),
    )
    .unwrap();
    assert_ne!(
        library.comparison_manifest_sha256,
        changed.comparison_manifest_sha256
    );
    assert_ne!(library.row_sequence_sha256, changed.row_sequence_sha256);
    assert_ne!(library.report_id, changed.report_id);
}

#[test]
fn aggregation_uses_exact_eligible_ratios_full_sums_and_nearest_rank() {
    // Positive-actual ratios: 1/2, 2, 0. Mean=5/6; underquote=2/3.
    // Full sums include the fourth row's positive prediction at zero actual: 180/90=2.
    let report = report_for(
        vec![
            pair(0, 10, 20, true),
            pair(1, 120, 60, true),
            pair(2, 0, 10, true),
            pair(3, 50, 0, true),
        ],
        &config(),
    );
    let s = &report.strata[0];
    assert_eq!(
        (
            s.row_count,
            s.eligible_count,
            s.zero_actual_count,
            s.infinite_underpricing_count
        ),
        (4, 3, 1, 1)
    );
    assert_eq!((&s.prediction_sum[..], &s.actual_sum[..]), ("180", "90"));
    assert_eq!(
        s.count_weighted_prediction_over_actual
            .as_ref()
            .unwrap()
            .display,
        "0.833333"
    );
    assert_eq!(
        s.workload_weighted_prediction_over_actual
            .as_ref()
            .unwrap()
            .display,
        "2.000000"
    );
    assert_eq!(s.underquote_rate.as_ref().unwrap().display, "0.666667");
    assert_eq!(s.quantiles["0"].display, "0.000000");
    assert_eq!(s.quantiles["0.5"].display, "0.500000");
    assert_eq!(s.quantiles["1"].display, "2.000000");
    assert!(s.quantile_method.contains("positive-actual"));
    assert!(s.quantile_method.contains("max(1,ceil(q*n))"));
    // Full sums and weighted arithmetic must exceed u128 without truncation.
    let big = report_for(
        vec![
            pair(0, u128::MAX, u128::MAX, true),
            pair(1, u128::MAX, u128::MAX, true),
        ],
        &config(),
    );
    assert_eq!(
        big.strata[0].prediction_sum,
        "680564733841876926926749214863536422910"
    );
    assert_eq!(
        big.strata[0]
            .count_weighted_prediction_over_actual
            .as_ref()
            .unwrap()
            .display,
        "1.000000"
    );
}

#[test]
fn aggregation_never_combines_any_distinct_stratum_identity_component() {
    let mut rows = vec![pair(0, 10, 20, true)];
    for index in 1..=15 {
        let mut row = pair(index, 10, 20, true);
        let ComparisonResultV1::Joined {
            quote,
            observation,
            profile_id,
            normalization,
            ..
        } = &mut row
        else {
            unreachable!()
        };
        match index {
            1 => quote.proposal.chain.chain_id = 2,
            2 => quote.proposal.fork_segments[0].fork = "UNZEN".into(),
            3 => quote.proposal.fork_segments[0].first_block = 3,
            4 => quote.proposal.fork_segments.push(ForkSegmentV1 {
                fork: "UNZEN".into(),
                first_block: 3,
                last_block: 3,
            }),
            5 => observation.backend = "other".into(),
            6 => observation.workload.unit = "other".into(),
            7 => observation.measurement_stage = "prove".into(),
            8 => observation.program.elf_sha256 = "other".into(),
            9 => observation.program.serialized_vk_sha256 = Some("other".into()),
            10 => observation.program.sp1_vkey = Some("native-vkey".into()),
            11 => observation.program.risc0_image_id = Some("image-id".into()),
            12 => observation.backend_version = None,
            13 => quote.quote.model_id = "other".into(),
            14 => *profile_id = "other".into(),
            15 => {
                *normalization = Some(NormalizedDiagnosticV1 {
                    normalization_profile_id: "normalization".into(),
                    normalization_sha256: "wrapper-sha".into(),
                    source_unit: "unzen_zkgas".into(),
                    target_unit: "sp1_pgu".into(),
                    converted_prediction: "1".into(),
                    diagnostic_only: true,
                    prediction_over_actual: None,
                })
            }
            _ => unreachable!(),
        }
        rows.push(row);
    }
    rows.push(pair(16, 10, 20, false));
    let report = report_for(rows, &config());
    assert_eq!(report.strata.len(), 17);
    assert!(report.strata.iter().all(|s| s.row_count == 1));
    let mut cross = pair(0, 1, 1, true);
    let ComparisonResultV1::Joined { quote, .. } = &mut cross else {
        unreachable!()
    };
    quote.proposal.fork_segments.push(ForkSegmentV1 {
        fork: "NEXT".into(),
        first_block: 3,
        last_block: 3,
    });
    let mut reversed = cross.clone();
    let ComparisonResultV1::Joined {
        quote,
        observation_key,
        ..
    } = &mut reversed
    else {
        unreachable!()
    };
    quote.key.sample_id = "reversed".into();
    observation_key.observation_id = "reversed".into();
    quote.proposal.fork_segments.reverse();
    assert_eq!(report_for(vec![cross, reversed], &config()).strata.len(), 2);
}

#[test]
fn aggregation_normalizes_absolute_fork_boundaries_but_preserves_crossing_topology() {
    let first = pair(0, 10, 20, true);
    let mut shifted = pair(1, 10, 20, true);
    let ComparisonResultV1::Joined {
        quote,
        observation_key,
        ..
    } = &mut shifted
    else {
        unreachable!()
    };
    quote.proposal.first_block = 200;
    quote.proposal.last_block = 200;
    quote.proposal.fork_segments[0].first_block = 200;
    quote.proposal.fork_segments[0].last_block = 200;
    observation_key.observation_id = "shifted".into();

    let report = report_for(vec![first.clone(), shifted], &config());
    assert_eq!(report.strata.len(), 1);
    assert_eq!(report.strata[0].row_count, 2);

    let mut crossing = first;
    let ComparisonResultV1::Joined { quote, .. } = &mut crossing else {
        unreachable!()
    };
    quote.proposal.last_block = 3;
    quote.proposal.block_count = 2;
    quote.proposal.fork_segments.push(ForkSegmentV1 {
        fork: "UNZEN".into(),
        first_block: 3,
        last_block: 3,
    });
    let report = report_for(vec![crossing, pair(2, 10, 20, true)], &config());
    assert_eq!(report.strata.len(), 2);
}

#[test]
fn aggregation_incompatible_series_have_named_exact_shape_diagnostics_only() {
    // x=[1,2,2], y=[1,2,3]: Pearson=sqrt(3)/2; tied rank Spearman=sqrt(3)/2;
    // tau-b=2/sqrt(2*3). These values must not depend on a platform floating point library.
    let report = report_for(
        vec![
            pair(0, 1, 1, false),
            pair(1, 2, 2, false),
            pair(2, 2, 3, false),
        ],
        &config(),
    );
    let s = &report.strata[0];
    assert!(!s.identity.compatible);
    assert_eq!(s.eligible_count, 0);
    assert!(s.count_weighted_prediction_over_actual.is_none());
    assert!(s.workload_weighted_prediction_over_actual.is_none());
    assert!(s.underquote_rate.is_none());
    assert!(s.quantiles.is_empty());
    for (key, value) in [
        ("raw_value_pearson_diagnostic", "0.866025"),
        ("raw_value_spearman_diagnostic", "0.866025"),
        ("raw_value_kendall_tau_b_diagnostic", "0.816497"),
    ] {
        assert_eq!(s.diagnostics[key].value.as_deref(), Some(value));
        assert!(!s.diagnostics[key].method.is_empty());
    }
    let negative = report_for(
        vec![
            pair(0, 1, 3, false),
            pair(1, 2, 2, false),
            pair(2, 3, 1, false),
        ],
        &config(),
    );
    for diagnostic in negative.strata[0]
        .diagnostics
        .values()
        .filter(|d| d.method.contains("correlation"))
    {
        assert_eq!(diagnostic.value.as_deref(), Some("-1.000000"));
    }
    let constant = report_for(vec![pair(0, 2, 1, false), pair(1, 2, 3, false)], &config());
    for key in [
        "raw_value_pearson_diagnostic",
        "raw_value_spearman_diagnostic",
        "raw_value_kendall_tau_b_diagnostic",
    ] {
        assert!(constant.strata[0].diagnostics[key].value.is_none());
        assert!(
            constant.strata[0].diagnostics[key]
                .unavailable_reason
                .is_some()
        );
    }
}

#[test]
fn aggregation_ties_in_both_marginals_keep_raw_pearson_and_tie_corrections_distinct() {
    let report = report_for(
        vec![
            pair(0, 1, 1, false),
            pair(1, 1, 1, false),
            pair(2, 2, 2, false),
            pair(3, 2, 3, false),
        ],
        &config(),
    );
    let d = &report.strata[0].diagnostics;
    assert_eq!(
        d["raw_value_pearson_diagnostic"].value.as_deref(),
        Some("0.904534")
    );
    assert_eq!(
        d["raw_value_spearman_diagnostic"].value.as_deref(),
        Some("0.942809")
    );
    assert_eq!(
        d["raw_value_kendall_tau_b_diagnostic"].value.as_deref(),
        Some("0.894427")
    );
}

#[test]
fn aggregation_buckets_are_prediction_ranges_and_zero_actual_makes_amplification_unavailable() {
    let mut c = config();
    c.buckets = vec![
        WorkloadBucketV1 {
            name: "low".into(),
            minimum: 0,
            maximum_exclusive: Some(10),
        },
        WorkloadBucketV1 {
            name: "high".into(),
            minimum: 10,
            maximum_exclusive: None,
        },
    ];
    let report = report_for(
        vec![
            pair(0, 1, 100, false),
            pair(1, 2, 400, false),
            pair(2, 10, 2, false),
            pair(3, 12, 0, false),
        ],
        &c,
    );
    let s = &report.strata[0];
    assert_eq!(s.buckets[0].count, 2);
    assert_eq!(
        s.buckets[0].amplification.as_ref().unwrap().display,
        "4.000000"
    );
    assert_eq!(s.buckets[1].count, 2);
    assert!(s.buckets[1].amplification.is_none());
    assert!(
        s.diagnostics["bucket:high:amplification_diagnostic"]
            .unavailable_reason
            .as_ref()
            .unwrap()
            .contains("zero")
    );
    let singleton = report_for(vec![pair(0, 1, 100, false)], &c);
    assert!(singleton.strata[0].buckets[0].amplification.is_none());
    assert!(
        singleton.strata[0].diagnostics["bucket:low:amplification_diagnostic"]
            .unavailable_reason
            .as_ref()
            .unwrap()
            .contains("two")
    );
}

#[test]
fn aggregation_filters_follow_whole_run_accounting_and_exclusions_are_explicit() {
    let mut other = pair(1, 10, 20, true);
    let ComparisonResultV1::Joined { quote, .. } = &mut other else {
        unreachable!()
    };
    quote.proposal.chain.chain_id = 2;
    let mut c = config();
    c.filters.chain_ids = vec![1];
    let report = report_for(vec![pair(0, 10, 20, true), other, invalid("failed")], &c);
    assert_eq!(report.whole_run_counts[&ComparisonKind::Joined], 2);
    assert_eq!(
        report.whole_run_counts[&ComparisonKind::InvalidObservation],
        1
    );
    assert_eq!(
        report.selected_counts.get(&ComparisonKind::Joined),
        Some(&1)
    );
    assert!(
        !report
            .selected_counts
            .contains_key(&ComparisonKind::InvalidObservation)
    );
    assert_eq!(report.strata.len(), 1);
    assert_eq!(report.excluded.len(), 2);
    assert!(report.excluded.iter().all(|e| !e.reason.is_empty()));
    let no_filter = report_for(vec![invalid("failed")], &config());
    assert_eq!(
        no_filter.selected_counts[&ComparisonKind::InvalidObservation],
        1
    );
    assert_eq!(no_filter.excluded[0].reason, "invalid_observation");
    let value = serde_json::to_string(&report).unwrap();
    for forbidden in ["compliance", "capacity", "admission", "promotion"] {
        assert!(!value.contains(forbidden));
    }
}

#[test]
fn aggregation_rejects_invalid_config_and_duplicate_numeric_quantiles() {
    let dir = TestDir::new();
    let path = fixture(&dir, vec![pair(0, 1, 1, true)]);
    let run = verify_comparison_run(&path).unwrap();
    for quantiles in [
        vec!["-0.1"],
        vec!["1.1"],
        vec!["NaN"],
        vec!["0.5", "0.50"],
        vec!["1e-1"],
        vec!["0.123456789012345678901234567890123"],
    ] {
        let mut c = config();
        c.quantiles = quantiles.into_iter().map(str::to_owned).collect();
        assert!(build_report(&run, &c, &executable()).is_err());
    }
    for buckets in [
        vec![WorkloadBucketV1 {
            name: "".into(),
            minimum: 0,
            maximum_exclusive: None,
        }],
        vec![WorkloadBucketV1 {
            name: "bad".into(),
            minimum: 1,
            maximum_exclusive: Some(1),
        }],
        vec![
            WorkloadBucketV1 {
                name: "a".into(),
                minimum: 0,
                maximum_exclusive: Some(10),
            },
            WorkloadBucketV1 {
                name: "b".into(),
                minimum: 5,
                maximum_exclusive: None,
            },
        ],
        vec![
            WorkloadBucketV1 {
                name: "a".into(),
                minimum: 0,
                maximum_exclusive: Some(10),
            },
            WorkloadBucketV1 {
                name: "a".into(),
                minimum: 10,
                maximum_exclusive: None,
            },
        ],
    ] {
        let mut c = config();
        c.buckets = buckets;
        assert!(build_report(&run, &c, &executable()).is_err());
    }
}

fn invoke(args: &ReportArgs) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_zkgas-report"))
        .arg("--comparisons")
        .arg(&args.comparisons)
        .arg("--config")
        .arg(&args.config)
        .arg("--output")
        .arg(&args.output)
        .output()
        .unwrap()
}

#[test]
fn exits_complete_incompatible_and_empty_runs_are_zero_partial_source_is_two() {
    for (results, filter, expected) in [
        (vec![pair(0, 10, 20, true)], false, 0),
        (vec![pair(0, 10, 20, false)], false, 0),
        (vec![], false, 0),
        (vec![pair(0, 10, 20, true), invalid("failed")], true, 2),
        (vec![invalid("failed")], true, 2),
    ] {
        let dir = TestDir::new();
        let path = fixture(&dir, results);
        let args = args(&dir, path);
        if filter {
            let mut c = config();
            c.filters.chain_ids = vec![999];
            fs::write(&args.config, canonical_json(&c).unwrap()).unwrap();
        }
        let output = invoke(&args);
        assert_eq!(
            output.status.code(),
            Some(expected),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: ReportV1 = serde_json::from_slice(&fs::read(&args.output).unwrap()).unwrap();
        if filter {
            assert!(report.selected_counts.is_empty());
            assert_eq!(report.status, RunStatus::Partial);
        }
    }
}

#[test]
fn exits_structural_config_and_output_errors_are_one_and_commit_no_new_report() {
    for mutation in [
        "bad-row",
        "invalid-config",
        "unknown-config",
        "conflict",
        "bad-args",
    ] {
        let dir = TestDir::new();
        let path = fixture(&dir, vec![pair(0, 10, 20, true)]);
        let mut args = args(&dir, path.clone());
        match mutation {
            "bad-row" => fs::write(row_path(&path, 0), b"{}").unwrap(),
            "invalid-config" => fs::write(&args.config, b"{}").unwrap(),
            "unknown-config" => {
                let mut c = serde_json::to_value(config()).unwrap();
                c["extra"] = json!(true);
                fs::write(&args.config, canonical_json(&c).unwrap()).unwrap();
            }
            "conflict" => fs::write(&args.output, b"existing").unwrap(),
            "bad-args" => args.config = dir.path().join("missing"),
            _ => unreachable!(),
        }
        let output = invoke(&args);
        assert_eq!(output.status.code(), Some(1), "accepted {mutation}");
        assert!(output.stdout.is_empty());
        if mutation == "conflict" {
            assert_eq!(fs::read(&args.output).unwrap(), b"existing");
        } else {
            assert!(!args.output.exists());
        }
    }
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_zkgas-report"))
        .arg("--unknown-argument")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
}

fn reseal(path: &Path, mut manifest: ComparisonManifestV1) -> PathBuf {
    manifest.run_id = comparison_run_id(&manifest).unwrap();
    let new_path = path.parent().unwrap().join(&manifest.run_id);
    fs::write(
        path.join("manifest.json"),
        canonical_json(&manifest).unwrap(),
    )
    .unwrap();
    if path != new_path {
        fs::rename(path, &new_path).unwrap();
    }
    new_path
}

#[test]
fn integrity_recomputes_counts_status_tagged_ids_and_sequence_even_for_resealed_manifests() {
    for mutation in ["count", "status", "tagged-id", "sequence", "duplicate"] {
        let dir = TestDir::new();
        let path = fixture(&dir, vec![invalid("a"), invalid("b")]);
        let mut m = manifest(&path);
        match mutation {
            "count" => {
                m.counts.insert(ComparisonKind::InvalidObservation, 1);
            }
            "status" => m.status = RunStatus::Complete,
            "sequence" => m.row_sequence_sha256 = "wrong".into(),
            "duplicate" => {
                m.rows[1] = m.rows[0].clone();
                m.row_sequence_sha256 = row_sequence_sha256(&m.rows).unwrap();
            }
            "tagged-id" => {
                let row_path = row_path(&path, 0);
                let mut row: ComparisonRowV1 =
                    serde_json::from_slice(&fs::read(&row_path).unwrap()).unwrap();
                let ComparisonResultV1::InvalidObservation { observation, .. } = &mut row.result
                else {
                    unreachable!()
                };
                observation.observation_id = "changed".into();
                let bytes = canonical_json(&row).unwrap();
                m.rows[0].row_sha256 = sha256_hex(&bytes);
                m.row_sequence_sha256 = row_sequence_sha256(&m.rows).unwrap();
                fs::write(row_path, bytes).unwrap();
            }
            _ => unreachable!(),
        }
        let path = reseal(&path, m);
        assert!(
            verify_comparison_run(&path).is_err(),
            "accepted resealed {mutation}"
        );
    }
}

#[test]
fn integrity_byte_and_array_limits_stop_before_parsing_untrusted_tails() {
    let dir = TestDir::new();
    let path = fixture(&dir, vec![]);
    let args = args(&dir, path.clone());
    // Sparse oversized files exercise the metadata gate without allocating their contents.
    let file = fs::OpenOptions::new()
        .write(true)
        .open(path.join("manifest.json"))
        .unwrap();
    file.set_len(zkgas::report::MAX_REPORT_MANIFEST_BYTES + 1)
        .unwrap();
    let error = verify_comparison_run(&path).unwrap_err();
    assert!(error.to_string().contains("byte limit"));
    let dir = TestDir::new();
    let path = fixture(&dir, vec![]);
    let raw = format!(
        r#"{{"rows":[{},malformed-tail]}}"#,
        vec!["{}"; 10_001].join(",")
    );
    fs::write(path.join("manifest.json"), raw).unwrap();
    let error = verify_comparison_run(&path).unwrap_err();
    assert!(error.to_string().contains("row/item limit"), "{error}");
    assert!(!args.output.exists());
}

#[test]
fn configs_json_and_toml_are_strict_bounded_and_semantically_equivalent() {
    let dir = TestDir::new();
    let path = fixture(&dir, vec![pair(0, 1, 1, true)]);
    let args = args(&dir, path);
    let json_report = run_report(&args, &executable()).unwrap().report;
    let toml = "schema_version = 1\nquantiles = [\"0\", \"0.5\", \"1\"]\nbuckets = []\n[filters]\n";
    fs::write(&args.config, toml).unwrap();
    let mut other = args.clone();
    other.output = dir.path().join("toml-report.json");
    let toml_report = run_report(&other, &executable()).unwrap().report;
    assert_eq!(json_report.strata, toml_report.strata);
    assert_ne!(json_report.report_id, toml_report.report_id);
    for raw in [
        format!(
            r#"{{"schema_version":1,"filters":{{}},"quantiles":[{},malformed-tail],"buckets":[]}}"#,
            vec!["\"0\""; 129].join(",")
        ),
        format!(
            "schema_version=1\nquantiles=[{}]\nbuckets=[]\n[filters]\n",
            vec!["\"0\""; 129].join(",")
        ),
        "schema_version=1\nquantiles=[]\nbuckets=[]\nunknown=true\n[filters]\n".into(),
        format!(
            r#"{{"schema_version":1,"filters":{{}},"quantiles":[],"buckets":[{{"name":"a","minimum":"{}"}}]}}"#,
            "0".repeat(40)
        ),
    ] {
        fs::write(&args.config, raw).unwrap();
        let output = invoke(&other);
        assert_eq!(output.status.code(), Some(1));
    }
    fs::OpenOptions::new()
        .write(true)
        .open(&args.config)
        .unwrap()
        .set_len(zkgas::report::MAX_REPORT_CONFIG_BYTES + 1)
        .unwrap();
    let error = run_report(&other, &executable()).unwrap_err();
    assert!(error.to_string().contains("byte limit"));
}

#[test]
fn aggregation_zero_actual_explains_every_unavailable_direct_metric() {
    let report = report_for(vec![pair(0, 10, 0, true), pair(1, 20, 0, true)], &config());
    let s = &report.strata[0];
    assert_eq!(s.zero_actual_count, 2);
    assert_eq!(s.eligible_count, 0);
    assert!(s.count_weighted_prediction_over_actual.is_none());
    assert!(s.workload_weighted_prediction_over_actual.is_none());
    assert!(s.underquote_rate.is_none());
    assert!(s.quantiles.is_empty());
    assert!(
        s.diagnostics["per_row_metrics_unavailable"]
            .unavailable_reason
            .as_ref()
            .unwrap()
            .contains("positive actual")
    );
    assert!(
        s.diagnostics["workload_weighted_ratio_unavailable"]
            .unavailable_reason
            .as_ref()
            .unwrap()
            .contains("zero")
    );
}

#[test]
fn config_toml_semantic_item_limits_allow_boundary_trailing_comma() {
    let dir = TestDir::new();
    let path = fixture(&dir, vec![]);
    let args = args(&dir, path);
    let names: Vec<_> = (0..129).map(|i| format!("\"backend-{i}\"")).collect();
    let raw = format!(
        "schema_version=1\nquantiles=[]\nbuckets=[]\n[filters]\nbackends=[{}]\n",
        names.join(",")
    );
    fs::write(&args.config, raw).unwrap();
    let error = run_report(&args, &executable()).unwrap_err();
    assert!(format!("{error:#}").contains("item limit"), "{error:#}");
    // TOML allows a trailing comma; a full permitted collection must retain that syntax.
    let names: Vec<_> = (0..128).map(|i| format!("\"backend-{i}\"")).collect();
    let boundary = format!(
        "schema_version=1\nquantiles=[]\nbuckets=[]\n[filters]\nbackends=[{},]\n",
        names.join(",")
    );
    fs::write(&args.config, boundary).unwrap();
    run_report(&args, &executable()).unwrap();
}

#[test]
fn config_toml_accepts_authoritative_multiline_four_and_five_quote_endings() {
    for quote in ['"', '\''] {
        for closing_count in [4, 5] {
            let dir = TestDir::new();
            let path = fixture(&dir, vec![]);
            let args = args(&dir, path);
            let opening = quote.to_string().repeat(3);
            let closing = quote.to_string().repeat(closing_count);
            let second = "b".repeat(200);
            let raw = format!(
                "schema_version=1\nquantiles=[]\nbuckets=[]\n[filters]\nbackends=[{opening}a{closing}, \"{second}\"]\n"
            );
            let expected: ReportConfigV1 = toml::from_str(&raw).unwrap();
            assert_eq!(
                expected.filters.backends,
                vec![
                    format!("a{}", quote.to_string().repeat(closing_count - 3)),
                    second
                ]
            );
            fs::write(&args.config, &raw).unwrap();
            let cli_report = run_report(&args, &executable()).unwrap().report;
            let typed_report = build_report(
                &verify_comparison_run(&args.comparisons).unwrap(),
                &expected,
                &executable(),
            )
            .unwrap();
            assert_eq!(cli_report.selected_counts, typed_report.selected_counts);
            assert_eq!(cli_report.config_sha256, sha256_hex(raw.as_bytes()));
        }
    }
}

#[test]
fn config_toml_accepts_long_header_comments_and_quoted_comment_or_bracket_characters() {
    let dir = TestDir::new();
    let path = fixture(&dir, vec![]);
    let args = args(&dir, path);
    let raw = format!(
        "# ordinary comment with [brackets] and quotes \"'\nschema_version=1\nquantiles=[\"0.5\"] # array comment\nbuckets=[]\n[filters] #{}\nbackends=[\"backend#[]\", 'literal#[]'] # trailing comment\n",
        "c".repeat(1100)
    );
    let expected: ReportConfigV1 = toml::from_str(&raw).unwrap();
    assert_eq!(expected.filters.backends, vec!["backend#[]", "literal#[]"]);
    fs::write(&args.config, &raw).unwrap();
    let report = run_report(&args, &executable()).unwrap().report;
    assert_eq!(report.config_sha256, sha256_hex(raw.as_bytes()));
}

#[test]
fn config_toml_exact_raw_byte_bound_precedes_document_parsing() {
    let dir = TestDir::new();
    let path = fixture(&dir, vec![]);
    let args = args(&dir, path);
    let mut raw = b"schema_version=1\nquantiles=[]\nbuckets=[]\n[filters]\n#".to_vec();
    raw.resize(zkgas::report::MAX_REPORT_CONFIG_BYTES as usize, b'c');
    fs::write(&args.config, &raw).unwrap();
    run_report(&args, &executable()).unwrap();
    raw[0] = b'?'; // At the excess-byte boundary, even an invalid TOML prefix is never parsed.
    raw.push(b'c');
    fs::write(&args.config, raw).unwrap();
    let mut other = args.clone();
    other.output = dir.path().join("over-limit-report.json");
    let error = run_report(&other, &executable()).unwrap_err();
    assert_eq!(error.to_string(), "file byte limit exceeded");
    assert!(!other.output.exists());
}
