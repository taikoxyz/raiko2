use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};
use zkgas::{
    compare::{CompareArgs, ProfileResolution, embedded_profiles, resolve_profile, run_compare},
    contracts::*,
    identity::canonical_json,
};

struct TestDir(PathBuf);
impl TestDir {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("zkgas-compare-test-{}", uuid::Uuid::new_v4()));
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
        sha256: "compare-executable".into(),
        package_version: "test".into(),
        source_revision: None,
        dirty: None,
    }
}
fn quote() -> CompleteQuoteV1 {
    let p = embedded_profiles().unwrap().profiles.remove(0);
    serde_json::from_value(json!({
        "schema_version":1,"key":{"corpus_id":"corpus","sample_id":"sample","guest_input_sha256":"input"},
        "executable":executable(),"input_source":{"kind":"single","descriptor_sha256":"descriptor"},"guest_input_byte_length":12,
        "proposal":{"proposal_id":1,"chain":{"name":"sample-chain","chain_id":167000},"proof_carry_chain_id":167000,"proof_carry_proposal_id":1,"witness_chain_spec_sha256":["spec"],"first_block":2,"last_block":2,"block_count":1,"public_output":"output","fork_segments":[{"fork":"SHASTA","first_block":2,"last_block":2}]},
        "prediction_unit":"sp1_pgu","prediction_surface":"prove","calibration_profile":p.model_id,
        "quote":{"schema_version":1,"model_id":p.model_id,"guest_input_sha256":"input","guest_input_bincode_length":12,"public_output":format!("0x{}", "00".repeat(32)),"blocks":[],
            "node":{"unzen_zkgas_sum":"2","block_local_known_scaled":"0","block_local_known_pgu":"0","sum_of_block_known_pgu":"0"},
            "prove":{"selectors":{"payload":{"total_transaction_count":0,"canonical_transaction_bytes":0,"nonempty_payload_transactions":0},"true_unattempted_transactions":0,"blobs":{"source_blob_counts":[],"proposal_blob_hash_count":0,"provider_blob_count":0,"provider_commitment_count":0,"provider_proof_count":0},"parent_mpt":{"path_node_visits":0,"materialization_work_units":0,"hydration_hash_blocks":0,"dirty_encoded_nodes":0,"dirty_hash_blocks":0,"structural_branch_extension_churn_nodes":0,"dropped_trie_nodes":0}},
                "block_local_scaled":"0","block_local_pgu":"0","sum_of_block_admission_pgu":"0","proposal_once":{"fixed_scaled":"0","operations_scaled":"0","feature_error_scaled":"0","resources_scaled":"0","mpt_scaled":"0","startup_scaled":"0","transaction_payload_scaled":"0","blob_source_scaled":"0","parent_mpt_scaled":"0","total_scaled":"0","total_pgu":"0"},"proposal_once_pgu":"0","total_scaled":"0","total_pgu":"10"}}
    })).unwrap()
}
fn observation() -> CompleteObservationV1 {
    CompleteObservationV1 {
        observation_id: "measurement".into(),
        attempt_id: "attempt-1".into(),
        quote_key: quote().key,
        backend: "sp1".into(),
        workload: Workload {
            value: 20,
            unit: "sp1_pgu".into(),
        },
        measurement_stage: "execute".into(),
        program: embedded_profiles().unwrap().profiles.remove(0).program,
        backend_version: None,
        software_version: None,
        replicate_group: None,
        source: None,
    }
}
fn write_inputs(
    dir: &TestDir,
    quotes: &[QuoteRowV1],
    observations: &[ObservationRowV1],
) -> CompareArgs {
    let quote_file = dir.path().join("quotes.jsonl");
    let mut bytes = Vec::new();
    for q in quotes {
        bytes.extend(canonical_json(q).unwrap());
        bytes.push(b'\n');
    }
    fs::write(&quote_file, bytes).unwrap();
    let observation_file = dir.path().join("observations.json");
    fs::write(
        &observation_file,
        canonical_json(&ObservationManifestV1 {
            schema_version: 1,
            observation_set_id: "set".into(),
            observations: observations.to_vec(),
        })
        .unwrap(),
    )
    .unwrap();
    CompareArgs {
        quotes: vec![quote_file],
        observations: vec![observation_file],
        output: dir.path().join("store"),
        ..Default::default()
    }
}
fn complete_quote() -> QuoteRowV1 {
    QuoteRowV1::Complete(Box::new(quote()))
}
fn complete_observation() -> ObservationRowV1 {
    ObservationRowV1::Complete(Box::new(observation()))
}
fn rows(path: &Path) -> Vec<ComparisonRowV1> {
    let manifest: ComparisonManifestV1 =
        serde_json::from_slice(&fs::read(path.join("manifest.json")).unwrap()).unwrap();
    manifest
        .rows
        .iter()
        .map(|row| {
            serde_json::from_slice(
                &fs::read(
                    path.join("rows")
                        .join(format!("{}.json", row.comparison_row_id)),
                )
                .unwrap(),
            )
            .unwrap()
        })
        .collect()
}

#[test]
fn profile_resolution_matches_named_algorithms_and_exactly_one_profile() {
    let q = quote();
    let o = observation();
    let mut p = embedded_profiles().unwrap();
    assert!(matches!(
        resolve_profile(&q, &o, &p),
        ProfileResolution::Matched(_)
    ));
    let mut alternate = o.clone();
    alternate.program.serialized_vk_sha256 = None;
    alternate.program.sp1_vkey = o.program.serialized_vk_sha256.clone();
    assert_eq!(
        resolve_profile(&q, &alternate, &p),
        ProfileResolution::NoMatch
    );
    p.profiles.clear();
    assert_eq!(resolve_profile(&q, &o, &p), ProfileResolution::NoMatch);
    let mut p = embedded_profiles().unwrap();
    let mut second = p.profiles[0].clone();
    second.profile_id = "second".into();
    p.profiles.push(second);
    assert_eq!(
        resolve_profile(&q, &o, &p),
        ProfileResolution::Ambiguous(vec!["sp1-pricing-calibration-v1".into(), "second".into()])
    );
    let mut q = q.clone();
    q.prediction_surface = "node".into();
    assert_eq!(resolve_profile(&q, &o, &p), ProfileResolution::NoMatch);
}

#[test]
fn repeated_attempts_join_once_each_and_normalize_the_proposal_sum() {
    let dir = TestDir::new();
    let mut second = observation();
    second.attempt_id = "attempt-2".into();
    let args = write_inputs(
        &dir,
        &[complete_quote()],
        &[
            complete_observation(),
            ObservationRowV1::Complete(Box::new(second)),
        ],
    );
    let run = run_compare(&args, &executable()).unwrap();
    assert_eq!(run.exit_code(), 0);
    let result = rows(&run.run_path);
    assert_eq!(result.len(), 2);
    assert_ne!(result[0].comparison_row_id, result[1].comparison_row_id);
    for row in result {
        let ComparisonResultV1::Joined {
            metrics,
            normalization,
            ..
        } = row.result
        else {
            panic!("joined")
        };
        assert!(metrics.underquoted);
        assert_eq!(metrics.prediction_over_actual.unwrap().display, "0.500000");
        assert_eq!(normalization.unwrap().converted_prediction, "169");
    }
}

#[test]
fn unmatched_and_upstream_failures_are_retained_without_using_observed_hash_as_a_key() {
    let dir = TestDir::new();
    let mut o = observation();
    o.quote_key.sample_id = "missing".into();
    let failed:QuoteRowV1=serde_json::from_value(json!({"schema_version":1,"status":"failed","corpus_id":"corpus","sample_id":"missing","observed_guest_input_sha256":"input","executable":executable(),"input_source":{"kind":"single","descriptor_sha256":"descriptor"},"failure":{"code":"decode","message":"invalid"}})).unwrap();
    let failed_o:ObservationRowV1=serde_json::from_value(json!({"status":"failed","observation_id":"failed","attempt_id":"a","failure":{"code":"execute","message":"failed"}})).unwrap();
    let args = write_inputs(
        &dir,
        &[complete_quote(), failed],
        &[ObservationRowV1::Complete(Box::new(o)), failed_o],
    );
    let run = run_compare(&args, &executable()).unwrap();
    assert_eq!(run.exit_code(), 2);
    let result = rows(&run.run_path);
    assert_eq!(result.len(), 4);
    for kind in [
        ComparisonKind::UnmatchedQuote,
        ComparisonKind::UnmatchedObservation,
        ComparisonKind::InvalidQuote,
        ComparisonKind::InvalidObservation,
    ] {
        assert_eq!(run.manifest.counts.get(&kind), Some(&1));
    }
}

#[test]
fn duplicate_primary_keys_within_and_across_files_are_structural_even_when_identical() {
    for conflicting in [false, true] {
        for quotes in [false, true] {
            for across_files in [false, true] {
                let dir = TestDir::new();
                let mut q = quote();
                let mut o = observation();
                if conflicting {
                    q.quote.prove.total_pgu += 1;
                    o.workload.value += 1;
                }
                let mut args = write_inputs(&dir, &[complete_quote()], &[complete_observation()]);
                if across_files {
                    let path = dir.path().join("second");
                    if quotes {
                        fs::write(
                            &path,
                            canonical_json(&QuoteRowV1::Complete(Box::new(q))).unwrap(),
                        )
                        .unwrap();
                        args.quotes.push(path);
                    } else {
                        fs::write(
                            &path,
                            canonical_json(&ObservationManifestV1 {
                                schema_version: 1,
                                observation_set_id: "set".into(),
                                observations: vec![ObservationRowV1::Complete(Box::new(o))],
                            })
                            .unwrap(),
                        )
                        .unwrap();
                        args.observations.push(path);
                    }
                } else {
                    args = if quotes {
                        write_inputs(
                            &dir,
                            &[complete_quote(), QuoteRowV1::Complete(Box::new(q))],
                            &[complete_observation()],
                        )
                    } else {
                        write_inputs(
                            &dir,
                            &[complete_quote()],
                            &[
                                complete_observation(),
                                ObservationRowV1::Complete(Box::new(o)),
                            ],
                        )
                    };
                }
                assert!(
                    run_compare(&args, &executable())
                        .unwrap_err()
                        .to_string()
                        .contains("duplicate")
                );
                assert!(!args.output.exists());
            }
        }
    }
}

#[test]
fn incompatibility_and_ambiguity_emit_one_row_with_the_correct_metric_eligibility() {
    for mode in ["program", "unit", "no-profile", "ambiguous", "custom"] {
        let dir = TestDir::new();
        let mut o = observation();
        let mut profiles = embedded_profiles().unwrap();
        match mode {
            "program" => o.program.elf_sha256 = "other".into(),
            "unit" => {
                o.backend = "risc0".into();
                o.workload.unit = "risc0_cycles".into();
            }
            "no-profile" => profiles.profiles.clear(),
            "ambiguous" => {
                let mut p = profiles.profiles[0].clone();
                p.profile_id = "second".into();
                profiles.profiles.push(p);
            }
            "custom" => {
                o.program.elf_sha256 = "custom".into();
                profiles.profiles[0].program = o.program.clone();
                profiles.profiles[0].profile_id = "custom".into();
            }
            _ => unreachable!(),
        };
        let mut args = write_inputs(
            &dir,
            &[complete_quote()],
            &[ObservationRowV1::Complete(Box::new(o))],
        );
        // Custom fixtures do not need normalization artifacts except to check identity gating.
        for p in &mut profiles.profiles {
            p.normalization = None;
        }
        let path = dir.path().join("profiles.toml");
        fs::write(&path, toml::to_string(&profiles).unwrap()).unwrap();
        args.profiles = Some(path);
        let run = run_compare(&args, &executable()).unwrap();
        assert_eq!(run.exit_code(), if mode == "ambiguous" { 2 } else { 0 });
        let result = rows(&run.run_path);
        assert_eq!(result.len(), 1);
        match (&result[0].result, mode) {
            (ComparisonResultV1::Joined { profile_id, .. }, "custom") => {
                assert_eq!(profile_id, "custom")
            }
            (
                ComparisonResultV1::AmbiguousProfile {
                    matching_profile_ids,
                    ..
                },
                "ambiguous",
            ) => assert_eq!(matching_profile_ids.len(), 2),
            (
                ComparisonResultV1::IncompatibleProfile {
                    cross_program_ratio,
                    normalization,
                    reason,
                    ..
                },
                _,
            ) => {
                assert_eq!(cross_program_ratio.is_some(), mode != "unit");
                assert!(normalization.is_none());
                assert_eq!(reason, "no_matching_profile");
            }
            _ => panic!("unexpected row for {mode}"),
        }
    }
}

#[test]
fn immutable_runs_reuse_only_byte_identical_complete_trees() {
    use zkgas::output::CommitOutcome;
    for tamper in ["row", "manifest", "extra", "missing", "symlink"] {
        let dir = TestDir::new();
        let args = write_inputs(&dir, &[complete_quote()], &[complete_observation()]);
        let run = run_compare(&args, &executable()).unwrap();
        assert_eq!(run.commit, CommitOutcome::Created);
        let reused = run_compare(&args, &executable()).unwrap();
        assert_eq!(reused.commit, CommitOutcome::Reused);
        assert_eq!(reused.manifest, run.manifest);
        let row_path = run
            .run_path
            .join("rows")
            .join(format!("{}.json", run.manifest.rows[0].comparison_row_id));
        match tamper {
            "row" => fs::write(&row_path, b"tampered").unwrap(),
            "manifest" => fs::write(run.run_path.join("manifest.json"), b"conflicting").unwrap(),
            "extra" => fs::write(run.run_path.join("rows/extra.json"), b"extra").unwrap(),
            "missing" => fs::remove_file(&row_path).unwrap(),
            "symlink" => {
                fs::remove_file(&row_path).unwrap();
                std::os::unix::fs::symlink(args.quotes.first().unwrap(), &row_path).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(run_compare(&args, &executable()).is_err());
        assert_eq!(
            fs::read_dir(args.output.join("runs")).unwrap().count(),
            1,
            "no abandoned staging tree"
        );
        if tamper == "row" {
            assert_eq!(fs::read(row_path).unwrap(), b"tampered");
        }
    }
}

#[test]
fn manifest_binds_all_ordered_inputs_executable_profiles_arithmetic_and_row_bytes() {
    use zkgas::{
        compare::{comparison_row_id, comparison_run_id, row_sequence_sha256},
        identity::sha256_hex,
    };
    let dir = TestDir::new();
    let args = write_inputs(&dir, &[complete_quote()], &[complete_observation()]);
    let run = run_compare(&args, &executable()).unwrap();
    let manifest = &run.manifest;
    assert_eq!(
        manifest.quote_file_sha256,
        vec![sha256_hex(&fs::read(&args.quotes[0]).unwrap())]
    );
    assert_eq!(
        manifest.observation_file_sha256,
        vec![sha256_hex(&fs::read(&args.observations[0]).unwrap())]
    );
    assert_eq!(
        manifest.profiles_sha256,
        sha256_hex(zkgas::compare::EMBEDDED_PROFILES.as_bytes())
    );
    assert_eq!(
        manifest.row_sequence_sha256,
        row_sequence_sha256(&manifest.rows).unwrap()
    );
    assert_eq!(manifest.run_id, comparison_run_id(manifest).unwrap());
    for row in rows(&run.run_path) {
        assert_eq!(
            row.comparison_row_id,
            comparison_row_id(&row.result).unwrap()
        );
        let bytes = fs::read(
            run.run_path
                .join("rows")
                .join(format!("{}.json", row.comparison_row_id)),
        )
        .unwrap();
        assert_eq!(
            manifest
                .rows
                .iter()
                .find(|r| r.comparison_row_id == row.comparison_row_id)
                .unwrap()
                .row_sha256,
            sha256_hex(&bytes)
        );
    }
    for field in [
        "quote_file_sha256",
        "observation_file_sha256",
        "profiles_sha256",
        "arithmetic_policy",
        "model_ids",
        "profile_ids",
        "row_sequence_sha256",
        "executable",
        "rows",
    ] {
        let mut value = serde_json::to_value(manifest).unwrap();
        match field {
            "executable" => value[field]["sha256"] = json!("other"),
            "rows" => value[field][0]["row_sha256"] = json!("other"),
            "quote_file_sha256" | "observation_file_sha256" | "model_ids" | "profile_ids" => {
                value[field][0] = json!("other")
            }
            _ => value[field] = json!("other"),
        };
        let changed: ComparisonManifestV1 = serde_json::from_value(value).unwrap();
        assert_ne!(
            manifest.run_id,
            comparison_run_id(&changed).unwrap(),
            "{field} must bind run identity"
        );
    }
    let mut exe = executable();
    exe.sha256 = "different-executable".into();
    assert_ne!(
        run_compare(&args, &exe).unwrap().manifest.run_id,
        manifest.run_id
    );
}

#[test]
fn ordered_file_hashes_and_ordered_row_digests_are_part_of_the_run_identity() {
    use zkgas::compare::{comparison_run_id, row_sequence_sha256};
    let dir = TestDir::new();
    let mut second = observation();
    second.attempt_id = "second".into();
    let mut args = write_inputs(
        &dir,
        &[complete_quote()],
        &[
            complete_observation(),
            ObservationRowV1::Complete(Box::new(second)),
        ],
    );
    let first = run_compare(&args, &executable()).unwrap();
    let mut changed = first.manifest.clone();
    changed.rows.reverse();
    changed.row_sequence_sha256 = row_sequence_sha256(&changed.rows).unwrap();
    assert_ne!(
        changed.row_sequence_sha256,
        first.manifest.row_sequence_sha256
    );
    assert_ne!(comparison_run_id(&changed).unwrap(), first.manifest.run_id);
    let path = dir.path().join("second-quotes.jsonl");
    let mut q = quote();
    q.key.sample_id = "second".into();
    fs::write(
        &path,
        canonical_json(&QuoteRowV1::Complete(Box::new(q))).unwrap(),
    )
    .unwrap();
    args.quotes.push(path);
    let run = run_compare(&args, &executable()).unwrap();
    args.quotes.reverse();
    let reversed = run_compare(&args, &executable()).unwrap();
    assert_ne!(run.manifest.run_id, reversed.manifest.run_id);
    let mut hashes = run.manifest.quote_file_sha256.clone();
    hashes.reverse();
    assert_eq!(hashes, reversed.manifest.quote_file_sha256);
}

#[test]
fn normalization_requires_every_named_identity_and_both_independent_artifact_hashes() {
    use zkgas::compare::verify_normalization_artifact;
    let bytes = include_bytes!("../profiles/artifacts/unzen-sp1-block-comparison-v1.json");
    let n = embedded_profiles().unwrap().profiles[0]
        .normalization
        .clone()
        .unwrap();
    verify_normalization_artifact(&n, bytes).unwrap();
    for field in [
        "artifact_sha256",
        "artifact_identity",
        "numerator",
        "denominator",
        "diagnostic_only",
    ] {
        let mut value = serde_json::to_value(&n).unwrap();
        value[field] = if field == "diagnostic_only" {
            json!(false)
        } else if field == "denominator" {
            json!("0")
        } else {
            json!("1")
        };
        let bad: NormalizationV1 = serde_json::from_value(value).unwrap();
        assert!(
            verify_normalization_artifact(&bad, bytes).is_err(),
            "{field}"
        );
    }
    for identity in [
        "source_program",
        "target_program",
        "source_unit",
        "target_unit",
    ] {
        let dir = TestDir::new();
        let mut p = embedded_profiles().unwrap();
        let n = p.profiles[0].normalization.as_mut().unwrap();
        match identity {
            "source_program" => n.source_program.elf_sha256 = "other".into(),
            "target_program" => n.target_program.serialized_vk_sha256 = Some("other".into()),
            "source_unit" => n.source_unit = "unknown".into(),
            "target_unit" => n.target_unit = "unknown".into(),
            _ => unreachable!(),
        };
        fs::create_dir(dir.path().join("artifacts")).unwrap();
        fs::write(dir.path().join(&n.artifact_path), bytes).unwrap();
        let path = dir.path().join("profiles.toml");
        fs::write(&path, toml::to_string(&p).unwrap()).unwrap();
        let mut args = write_inputs(&dir, &[complete_quote()], &[complete_observation()]);
        args.profiles = Some(path);
        let run = run_compare(&args, &executable()).unwrap();
        let ComparisonResultV1::Joined { normalization, .. } = &rows(&run.run_path)[0].result
        else {
            panic!("direct metrics remain compatible")
        };
        assert!(normalization.is_none(), "{identity}");
    }
}

#[test]
fn hard_input_byte_and_file_count_bounds_precede_content_loading() {
    use zkgas::compare::{MAX_COMPARE_FILE_BYTES, MAX_COMPARE_FILES};
    let dir = TestDir::new();
    let mut args = write_inputs(&dir, &[complete_quote()], &[complete_observation()]);
    args.quotes = vec![dir.path().join("missing"); MAX_COMPARE_FILES];
    assert!(
        run_compare(&args, &executable())
            .unwrap_err()
            .to_string()
            .contains("file count limit")
    );
    let invalid = dir.path().join("invalid.jsonl");
    fs::write(&invalid, b"not-json").unwrap();
    let large = dir.path().join("oversized");
    fs::File::create(&large)
        .unwrap()
        .set_len(MAX_COMPARE_FILE_BYTES + 1)
        .unwrap();
    args.quotes = vec![invalid.clone(), large.clone()];
    assert!(
        run_compare(&args, &executable())
            .unwrap_err()
            .to_string()
            .contains("per-file byte limit")
    );
    fs::File::create(&large)
        .unwrap()
        .set_len(MAX_COMPARE_FILE_BYTES)
        .unwrap();
    args.quotes = vec![invalid, large.clone(), large.clone(), large.clone(), large];
    assert!(
        run_compare(&args, &executable())
            .unwrap_err()
            .to_string()
            .contains("aggregate input byte limit")
    );
    assert!(!args.output.exists());
}

#[test]
fn excessive_parsed_rows_and_failed_verified_duplicate_keys_are_structural() {
    use zkgas::compare::MAX_COMPARE_ROWS;
    let dir = TestDir::new();
    let mut args = write_inputs(&dir, &[complete_quote()], &[complete_observation()]);
    let observations:Vec<_>=(0..MAX_COMPARE_ROWS).map(|i| json!({"status":"failed","observation_id":format!("failed-{i}"),"attempt_id":"a","failure":{"code":"unavailable","message":"missing"}})).collect();
    fs::write(
        &args.observations[0],
        serde_json::to_vec(
            &json!({"schema_version":1,"observation_set_id":"set","observations":observations}),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(
        run_compare(&args, &executable())
            .unwrap_err()
            .to_string()
            .contains("row count limit")
    );
    assert!(!args.output.exists());
    let failed:QuoteRowV1=serde_json::from_value(json!({"schema_version":1,"status":"failed","corpus_id":"corpus","sample_id":"sample","verified_key":quote().key,"executable":executable(),"input_source":{"kind":"single","descriptor_sha256":"descriptor"},"failure":{"code":"quote","message":"failed"}})).unwrap();
    args = write_inputs(&dir, &[complete_quote(), failed], &[complete_observation()]);
    assert!(
        run_compare(&args, &executable())
            .unwrap_err()
            .to_string()
            .contains("duplicate quote primary key")
    );
}

#[test]
fn observation_row_limit_stops_before_malformed_tail_without_output() {
    let dir = TestDir::new();
    let args = write_inputs(&dir, &[complete_quote()], &[]);
    let observation_rows: Vec<_> = (0..10_001).map(|i| serde_json::to_string(&json!({"status":"failed","observation_id":format!("observation-{i}"),"attempt_id":"attempt","failure":{"code":"unavailable","message":"missing"}})).unwrap()).collect();
    for tail in ["]}", r#",{"status":"invalid-tail"}]}"#, ",malformed-tail]}"] {
        let raw = format!(
            r#"{{"schema_version":1,"observation_set_id":"set","observations":[{}{}"#,
            observation_rows.join(","),
            tail
        );
        fs::write(&args.observations[0], raw).unwrap();
        let error = run_compare(&args, &executable()).unwrap_err();
        assert!(
            error.to_string().contains("observation manifest row limit"),
            "{error}"
        );
        assert!(!args.output.exists());
    }
}

#[test]
fn observation_files_below_individual_limits_still_obey_the_aggregate_row_limit() {
    let dir = TestDir::new();
    let mut args = write_inputs(&dir, &[complete_quote()], &[]);
    for index in 0..2 {
        let path = dir.path().join(format!("observations-{index}.json"));
        let observations: Vec<_> = (0..5_000).map(|i| json!({"status":"failed","observation_id":format!("observation-{i}"),"attempt_id":"attempt","failure":{"code":"unavailable","message":"missing"}})).collect();
        fs::write(&path, serde_json::to_vec(&json!({"schema_version":1,"observation_set_id":format!("set-{index}"),"observations":observations})).unwrap()).unwrap();
        args.observations.push(path);
    }
    let error = run_compare(&args, &executable()).unwrap_err();
    assert!(
        error.to_string().contains("total input row count limit"),
        "{error}"
    );
    assert!(!args.output.exists());
}

#[test]
fn binary_exit_codes_cover_complete_partial_incompatible_and_structural_errors() {
    use std::process::Command;
    for mode in ["complete", "partial", "incompatible", "structural"] {
        let dir = TestDir::new();
        let mut o = observation();
        match mode {
            "partial" => o.quote_key.sample_id = "absent".into(),
            "incompatible" => o.program.elf_sha256 = "other".into(),
            _ => {}
        };
        let args = write_inputs(
            &dir,
            &[complete_quote()],
            &[ObservationRowV1::Complete(Box::new(o))],
        );
        if mode == "structural" {
            fs::write(&args.quotes[0], b"invalid").unwrap();
        }
        let output = Command::new(env!("CARGO_BIN_EXE_zkgas-compare"))
            .arg("--quotes")
            .arg(&args.quotes[0])
            .arg("--observations")
            .arg(&args.observations[0])
            .arg("--output")
            .arg(&args.output)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(match mode {
                "partial" => 2,
                "structural" => 1,
                _ => 0,
            }),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if mode == "structural" {
            assert!(!args.output.exists());
        } else {
            let id = String::from_utf8(output.stdout).unwrap();
            assert!(
                args.output
                    .join("runs")
                    .join(id.trim())
                    .join("manifest.json")
                    .is_file()
            );
        }
    }
}

#[test]
fn unverified_failed_rows_cannot_alias_one_immutable_row_path() {
    let failed:QuoteRowV1=serde_json::from_value(json!({"schema_version":1,"status":"failed","corpus_id":"corpus","sample_id":"failed","observed_guest_input_sha256":"claimed-input","executable":executable(),"input_source":{"kind":"single","descriptor_sha256":"descriptor"},"failure":{"code":"decode","message":"invalid"}})).unwrap();
    let dir = TestDir::new();
    let args = write_inputs(&dir, &[failed.clone(), failed], &[]);
    assert!(
        run_compare(&args, &executable())
            .unwrap_err()
            .to_string()
            .contains("duplicate comparison row ID")
    );
    assert!(!args.output.exists());
}
