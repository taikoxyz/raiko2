mod common;

use alethia_reth_evm::zk_gas::pricing::{
    MptSelectors, PricingBreakdown, ProposalPricingInput, RUNTIME_SPEC_SHA256,
    TransactionPayloadSelectors, ZkGasModel,
};
use common::trusted_temp_root;
use raiko2_zkgas_trace::PricingBreakdownSnapshot;
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};
use zkgas::{
    compare::{
        CompareArgs, ProfileResolution, comparison_row_id, embedded_profiles, resolve_profile,
        run_compare,
    },
    contracts::*,
    identity::{canonical_json, content_id},
};

struct TestDir(PathBuf);
impl TestDir {
    fn new() -> Self {
        let path = trusted_temp_root().join(format!("zkgas-compare-test-{}", uuid::Uuid::new_v4()));
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
fn snapshot(pricing: PricingBreakdown) -> PricingBreakdownSnapshot {
    PricingBreakdownSnapshot {
        fixed_scaled: pricing.fixed_scaled,
        operations_scaled: pricing.operations_scaled,
        feature_error_scaled: pricing.feature_error_scaled,
        resources_scaled: pricing.resources_scaled,
        mpt_scaled: pricing.mpt_scaled,
        startup_scaled: pricing.startup_scaled,
        transaction_payload_scaled: pricing.transaction_payload_scaled,
        blob_source_scaled: pricing.blob_source_scaled,
        parent_mpt_scaled: pricing.parent_mpt_scaled,
        total_scaled: pricing.total_scaled,
        total_pgu: pricing.total_pgu.unwrap(),
    }
}

// This synthetic profile exercises current-model validation. The unchanged historical program
// and normalization identities are fixtures for matching tests, not a new calibration claim.
fn current_model_profiles() -> CompatibilityProfilesV1 {
    let mut profiles = embedded_profiles().unwrap();
    let profile = &mut profiles.profiles[0];
    profile.profile_id = "current-model-test-profile".into();
    profile.model_id = RUNTIME_SPEC_SHA256.into();
    profile.runtime_spec_sha256 = Some(RUNTIME_SPEC_SHA256.into());
    profiles
}

fn quote() -> CompleteQuoteV1 {
    let public_output = format!("0x{}", "00".repeat(32));
    let mut quote: CompleteQuoteV1 = serde_json::from_value(json!({
        "schema_version":1,"key":{"corpus_id":"corpus","sample_id":"sample","guest_input_sha256":"input"},
        "executable":executable(),"input_source":{"kind":"single","descriptor_sha256":"descriptor"},"guest_input_byte_length":12,
        "proposal":{"proposal_id":1,"chain":{"name":"sample-chain","chain_id":167000},"proof_carry_chain_id":167000,"proof_carry_proposal_id":1,"witness_chain_spec_sha256":["spec"],"first_block":2,"last_block":2,"block_count":1,"public_output":public_output,"fork_segments":[{"fork":"SHASTA","first_block":2,"last_block":2}]},
        "prediction_unit":"sp1_pgu","prediction_surface":"prove","calibration_profile":RUNTIME_SPEC_SHA256,
        "quote":{"schema_version":1,"model_id":RUNTIME_SPEC_SHA256,"guest_input_sha256":"input","guest_input_bincode_length":12,"public_output":public_output,"blocks":[{
            "block_index":0,"block_number":2,
            "node":{"unzen_zkgas":2,"pricing":{"fixed_scaled":"0","operations_scaled":"0","feature_error_scaled":"0","evm_local_resources":{"unique_account_count":0,"unique_storage_slot_access_count":0,"unique_storage_slot_write_count":0,"unique_code_hash_count":0,"unique_code_bytes":0},"evm_local_resources_scaled":"0","transaction_payload":{"canonical_transaction_bytes":1,"nonempty_payload_transactions":0},"transaction_payload_scaled":"0","known_subtotal_scaled":"0","known_subtotal_pgu":"0"}},
            "prove":{"input_transaction_count":1,"consumed_candidate_count":1,"attempted_transaction_count":1,"true_unattempted_transactions":0,"committed_payload":{"total_transaction_count":1,"canonical_transaction_bytes":1,"nonempty_payload_transactions":0},"exact_resources":{"unique_account_count":0,"unique_storage_slot_access_count":0,"unique_storage_slot_write_count":0,"unique_code_hash_count":0,"unique_code_bytes":0},"mpt":{"path_node_visits":0,"materialization_work_units":0,"hydration_hash_blocks":0,"dirty_encoded_nodes":0,"dirty_hash_blocks":0,"structural_branch_extension_churn_nodes":0,"dropped_trie_nodes":0},"pricing":{"fixed_scaled":"0","operations_scaled":"0","feature_error_scaled":"0","resources_scaled":"0","mpt_scaled":"0","startup_scaled":"0","transaction_payload_scaled":"0","blob_source_scaled":"0","parent_mpt_scaled":"0","total_scaled":"0","total_pgu":"0"}}
        }],
            "node":{"unzen_zkgas_sum":"2","block_local_known_scaled":"0","block_local_known_pgu":"0","sum_of_block_known_pgu":"0"},
            "prove":{"selectors":{"payload":{"total_transaction_count":1,"canonical_transaction_bytes":1,"nonempty_payload_transactions":0},"true_unattempted_transactions":0,"blobs":{"source_blob_counts":[1],"proposal_blob_hash_count":1,"provider_blob_count":1,"provider_commitment_count":1,"provider_proof_count":1},"parent_mpt":{"path_node_visits":0,"materialization_work_units":0,"hydration_hash_blocks":0,"dirty_encoded_nodes":0,"dirty_hash_blocks":0,"structural_branch_extension_churn_nodes":0,"dropped_trie_nodes":0}},
                "block_local_scaled":"0","block_local_pgu":"0","sum_of_block_local_pgu":"0","proposal_once":{"fixed_scaled":"10000000000000000000","operations_scaled":"0","feature_error_scaled":"0","resources_scaled":"0","mpt_scaled":"0","startup_scaled":"0","transaction_payload_scaled":"0","blob_source_scaled":"0","parent_mpt_scaled":"0","total_scaled":"10000000000000000000","total_pgu":"10"},"proposal_once_pgu":"10","total_scaled":"10000000000000000000","total_pgu":"10"}}
    })).unwrap();
    let selectors = &quote.quote.prove.selectors;
    let parent = selectors.parent_mpt;
    let expected = ZkGasModel
        .price_proposal_once(&ProposalPricingInput {
            canonical_transaction_bytes: 0,
            nonempty_payload_transactions: 0,
            true_unattempted_transactions: selectors.true_unattempted_transactions,
            source_blob_counts: &selectors.blobs.source_blob_counts,
            proposal_blob_hash_count: selectors.blobs.proposal_blob_hash_count,
            provider_blob_count: selectors.blobs.provider_blob_count,
            provider_commitment_count: selectors.blobs.provider_commitment_count,
            provider_proof_count: selectors.blobs.provider_proof_count,
            parent_mpt: MptSelectors {
                path_node_visits: parent.path_node_visits,
                materialization_work_units: parent.materialization_work_units,
                hydration_hash_blocks: parent.hydration_hash_blocks,
                dirty_encoded_nodes: parent.dirty_encoded_nodes,
                dirty_hash_blocks: parent.dirty_hash_blocks,
                structural_branch_extension_churn_nodes: parent
                    .structural_branch_extension_churn_nodes,
                dropped_trie_nodes: parent.dropped_trie_nodes,
            },
        })
        .unwrap();
    quote.quote.prove.proposal_once = snapshot(expected);
    quote.quote.prove.proposal_once_pgu = expected.total_pgu.unwrap();
    let payload = ZkGasModel
        .price_transaction_payload(TransactionPayloadSelectors {
            canonical_transaction_bytes: 1,
            nonempty_payload_transactions: 0,
        })
        .unwrap();
    let node = &mut quote.quote.blocks[0].node.pricing;
    node.transaction_payload_scaled = payload.transaction_payload_scaled;
    node.known_subtotal_scaled = payload.total_scaled;
    node.known_subtotal_pgu = payload.total_pgu.unwrap();
    quote.quote.blocks[0].prove.pricing = snapshot(payload);
    quote.quote.node.block_local_known_scaled = payload.total_scaled;
    quote.quote.node.block_local_known_pgu = payload.total_pgu.unwrap();
    quote.quote.node.sum_of_block_known_pgu = payload.total_pgu.unwrap();
    quote.quote.prove.block_local_scaled = payload.total_scaled;
    quote.quote.prove.block_local_pgu = payload.total_pgu.unwrap();
    quote.quote.prove.sum_of_block_local_pgu = payload.total_pgu.unwrap();
    quote.quote.prove.total_scaled = expected.total_scaled + payload.total_scaled;
    quote.quote.prove.total_pgu = expected.total_pgu.unwrap() + payload.total_pgu.unwrap();
    quote
}
fn observation() -> CompleteObservationV1 {
    let quote = quote();
    CompleteObservationV1 {
        observation_id: "measurement".into(),
        attempt_id: "attempt-1".into(),
        quote_key: quote.key,
        backend: "sp1".into(),
        workload: Workload {
            value: quote.quote.prove.total_pgu * 2,
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
    let profiles = current_model_profiles();
    let profiles_path = dir.path().join("current-model-profiles.toml");
    for profile in &profiles.profiles {
        if let Some(normalization) = &profile.normalization {
            let artifact = dir.path().join(&normalization.artifact_path);
            fs::create_dir_all(artifact.parent().unwrap()).unwrap();
            fs::write(
                artifact,
                include_bytes!("../profiles/artifacts/unzen-sp1-block-comparison-v1.json"),
            )
            .unwrap();
        }
    }
    fs::write(&profiles_path, toml::to_string(&profiles).unwrap()).unwrap();
    CompareArgs {
        quotes: vec![quote_file],
        observations: vec![observation_file],
        output: dir.path().join("store"),
        profiles: Some(profiles_path),
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
fn quote_fixture_uses_current_model_for_selector_price_validation() {
    assert_eq!(quote().quote.model_id, RUNTIME_SPEC_SHA256);
}

#[test]
fn profile_resolution_matches_named_algorithms_and_exactly_one_profile() {
    let q = quote();
    let o = observation();
    let mut p = current_model_profiles();
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
    let mut p = current_model_profiles();
    let mut second = p.profiles[0].clone();
    second.profile_id = "second".into();
    p.profiles.push(second);
    assert_eq!(
        resolve_profile(&q, &o, &p),
        ProfileResolution::Ambiguous(vec!["current-model-test-profile".into(), "second".into()])
    );
    let mut q = q.clone();
    q.prediction_surface = "node".into();
    assert_eq!(resolve_profile(&q, &o, &p), ProfileResolution::NoMatch);
}

#[test]
fn ambiguous_profile_rows_use_their_own_identity_domain() {
    let quote = quote();
    let observation = observation();
    let observation_key = ObservationKey {
        observation_set_id: "set".into(),
        observation_id: observation.observation_id.clone(),
        attempt_id: observation.attempt_id.clone(),
    };
    let matching_profile_ids = vec!["first".to_owned(), "second".to_owned()];
    let result = ComparisonResultV1::AmbiguousProfile {
        quote: Box::new(quote),
        observation,
        observation_key: observation_key.clone(),
        matching_profile_ids: matching_profile_ids.clone(),
    };
    assert_eq!(
        comparison_row_id(&result).unwrap(),
        content_id(
            "zkgas-comparison/ambiguous-profile-v1",
            &(
                match &result {
                    ComparisonResultV1::AmbiguousProfile { quote, .. } => &quote.key,
                    _ => unreachable!(),
                },
                &observation_key,
                &matching_profile_ids,
            ),
        )
        .unwrap()
    );
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
                    q.executable.sha256 = "different-executable".into();
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
fn complete_quotes_reject_every_inconsistent_embedded_identity_before_joining() {
    for mutation in [
        "input-hash",
        "input-length",
        "proof-chain",
        "proof-proposal",
        "range",
        "block-count",
        "witness-count",
        "public-output",
        "empty-forks",
        "fork-start",
        "fork-end",
        "fork-gap",
        "fork-name",
        "block-index",
        "block-number",
        "quote-schema",
        "prediction-unit",
        "prediction-surface",
        "calibration-model",
        "quote-model",
        "execution-profile",
        "prediction-total",
    ] {
        let dir = TestDir::new();
        let mut q = quote();
        match mutation {
            "input-hash" => q.quote.guest_input_sha256 = "other".into(),
            "input-length" => q.quote.guest_input_bincode_length += 1,
            "proof-chain" => q.proposal.proof_carry_chain_id += 1,
            "proof-proposal" => q.proposal.proof_carry_proposal_id += 1,
            "range" => q.proposal.last_block += 1,
            "block-count" => q.proposal.block_count += 1,
            "witness-count" => q.proposal.witness_chain_spec_sha256.push("extra".into()),
            "public-output" => q.proposal.public_output = "other".into(),
            "empty-forks" => q.proposal.fork_segments.clear(),
            "fork-start" => q.proposal.fork_segments[0].first_block += 1,
            "fork-end" => q.proposal.fork_segments[0].last_block -= 1,
            "fork-gap" => {
                q.proposal.last_block = 4;
                q.proposal.block_count = 3;
                q.proposal.witness_chain_spec_sha256 = vec!["a".into(), "b".into(), "c".into()];
                q.quote.blocks[0].block_number = 2;
                let mut second = q.quote.blocks[0].clone();
                second.block_index = 1;
                second.block_number = 3;
                let mut third = second.clone();
                third.block_index = 2;
                third.block_number = 4;
                q.quote.blocks.extend([second, third]);
                q.proposal.fork_segments = vec![
                    ForkSegmentV1 {
                        fork: "SHASTA".into(),
                        first_block: 2,
                        last_block: 2,
                    },
                    ForkSegmentV1 {
                        fork: "UNZEN".into(),
                        first_block: 4,
                        last_block: 4,
                    },
                ];
            }
            "fork-name" => q.proposal.fork_segments[0].fork.clear(),
            "block-index" => q.quote.blocks[0].block_index = 1,
            "block-number" => q.quote.blocks[0].block_number += 1,
            "quote-schema" => q.quote.schema_version = 2,
            "prediction-unit" => q.prediction_unit = "other".into(),
            "prediction-surface" => q.prediction_surface = "node".into(),
            "calibration-model" => q.calibration_profile = "other".into(),
            "quote-model" => q.quote.model_id = "other".into(),
            "execution-profile" => {
                q.execution_profile = raiko2_zkgas_trace::ExecutionProfile::EvmGasOnly;
            }
            "prediction-total" => q.quote.prove.total_pgu += 1,
            _ => unreachable!(),
        }
        let args = write_inputs(
            &dir,
            &[QuoteRowV1::Complete(Box::new(q))],
            &[complete_observation()],
        );
        let error = run_compare(&args, &executable()).unwrap_err();
        assert!(
            error.to_string().contains("complete quote invariant")
                || error.to_string().contains("embedded quote schema_version"),
            "wrong error for {mutation}: {error:#}"
        );
        assert!(!args.output.exists(), "committed output for {mutation}");
    }
}

fn quote_with_feature_error(
    operations_scaled: i128,
    feature_error_scaled: i128,
) -> CompleteQuoteV1 {
    const SCALE: i128 = 1_000_000_000_000_000_000;
    let mut quote = quote();
    let payload_scaled = quote.quote.blocks[0]
        .node
        .pricing
        .transaction_payload_scaled;
    let payload_pgu = u128::try_from(payload_scaled / SCALE).unwrap();
    let operation_pgu = u128::try_from(operations_scaled / SCALE).unwrap();
    let node = &mut quote.quote.blocks[0].node.pricing;
    node.operations_scaled = operations_scaled;
    node.feature_error_scaled = feature_error_scaled;
    node.known_subtotal_scaled = operations_scaled + payload_scaled;
    node.known_subtotal_pgu = operation_pgu + payload_pgu;
    let prove = &mut quote.quote.blocks[0].prove.pricing;
    prove.operations_scaled = operations_scaled;
    prove.feature_error_scaled = feature_error_scaled;
    prove.total_scaled = operations_scaled + payload_scaled;
    prove.total_pgu = operation_pgu + payload_pgu;
    quote.quote.node.block_local_known_scaled = operations_scaled + payload_scaled;
    quote.quote.node.block_local_known_pgu = operation_pgu + payload_pgu;
    quote.quote.node.sum_of_block_known_pgu = operation_pgu + payload_pgu;
    quote.quote.prove.block_local_scaled = operations_scaled + payload_scaled;
    quote.quote.prove.block_local_pgu = operation_pgu + payload_pgu;
    quote.quote.prove.sum_of_block_local_pgu = operation_pgu + payload_pgu;
    quote.quote.prove.total_scaled =
        operations_scaled + payload_scaled + quote.quote.prove.proposal_once.total_scaled;
    quote.quote.prove.total_pgu =
        operation_pgu + payload_pgu + quote.quote.prove.proposal_once.total_pgu;
    quote
}

#[test]
fn feature_error_is_validated_as_operations_attribution_not_an_additive_price() {
    const SCALE: i128 = 1_000_000_000_000_000_000;
    for feature_error_scaled in [SCALE, SCALE / 2] {
        let dir = TestDir::new();
        let q = quote_with_feature_error(2 * SCALE, feature_error_scaled);
        let args = write_inputs(
            &dir,
            &[QuoteRowV1::Complete(Box::new(q))],
            &[complete_observation()],
        );
        let run = run_compare(&args, &executable()).unwrap();
        assert_eq!(run.manifest.counts.get(&ComparisonKind::Joined), Some(&1));
    }

    let dir = TestDir::new();
    let q = quote_with_feature_error(2 * SCALE, 2 * SCALE + 1);
    let args = write_inputs(
        &dir,
        &[QuoteRowV1::Complete(Box::new(q))],
        &[complete_observation()],
    );
    let error = run_compare(&args, &executable()).unwrap_err();
    assert!(format!("{error:#}").contains("feature_error attribution"));
    assert!(!args.output.exists());
}

fn add_block_price(quote: &mut CompleteQuoteV1, scaled: i128) {
    let pricing = &mut quote.quote.blocks[0].prove.pricing;
    pricing.total_scaled += scaled;
    pricing.total_pgu += 1;
    quote.quote.prove.block_local_scaled += scaled;
    quote.quote.prove.block_local_pgu += 1;
    quote.quote.prove.sum_of_block_local_pgu += 1;
    quote.quote.prove.total_scaled += scaled;
    quote.quote.prove.total_pgu += 1;
}

#[test]
fn complete_quote_accepts_attempted_pricing_above_the_committed_node_prefix() {
    const SCALE: i128 = 1_000_000_000_000_000_000;
    let dir = TestDir::new();
    let mut q = quote();
    q.quote.blocks[0].prove.pricing.fixed_scaled = SCALE;
    q.quote.blocks[0].prove.pricing.operations_scaled = 2 * SCALE;
    q.quote.blocks[0].prove.pricing.feature_error_scaled = SCALE;
    for _ in 0..3 {
        add_block_price(&mut q, SCALE);
    }

    let args = write_inputs(
        &dir,
        &[QuoteRowV1::Complete(Box::new(q))],
        &[complete_observation()],
    );
    let run = run_compare(&args, &executable()).unwrap();
    assert_eq!(run.manifest.counts.get(&ComparisonKind::Joined), Some(&1));
}

#[test]
fn proposal_payload_prices_only_candidates_outside_the_committed_block() {
    let dir = TestDir::new();
    let mut q = quote();
    q.quote.blocks[0].prove.input_transaction_count = 2;
    q.quote.blocks[0].prove.consumed_candidate_count = 2;
    q.quote.prove.selectors.payload.total_transaction_count = 2;
    q.quote.prove.selectors.payload.canonical_transaction_bytes = 101;
    q.quote
        .prove
        .selectors
        .payload
        .nonempty_payload_transactions = 1;
    let proposal = ZkGasModel
        .price_proposal_once(&ProposalPricingInput {
            canonical_transaction_bytes: 100,
            nonempty_payload_transactions: 1,
            true_unattempted_transactions: 0,
            source_blob_counts: &[1],
            proposal_blob_hash_count: 1,
            provider_blob_count: 1,
            provider_commitment_count: 1,
            provider_proof_count: 1,
            parent_mpt: MptSelectors::default(),
        })
        .unwrap();
    q.quote.prove.proposal_once = snapshot(proposal);
    q.quote.prove.proposal_once_pgu = proposal.total_pgu.unwrap();
    q.quote.prove.total_scaled = q.quote.prove.block_local_scaled + proposal.total_scaled;
    q.quote.prove.total_pgu = q.quote.prove.block_local_pgu + proposal.total_pgu.unwrap();

    let all_payload = ZkGasModel
        .price_transaction_payload(TransactionPayloadSelectors {
            canonical_transaction_bytes: 101,
            nonempty_payload_transactions: 1,
        })
        .unwrap();
    assert_eq!(
        q.quote.blocks[0].prove.pricing.transaction_payload_scaled
            + proposal.transaction_payload_scaled,
        all_payload.transaction_payload_scaled
    );
    let args = write_inputs(
        &dir,
        &[QuoteRowV1::Complete(Box::new(q))],
        &[complete_observation()],
    );
    assert_eq!(run_compare(&args, &executable()).unwrap().exit_code(), 0);
}

#[test]
fn committed_payload_selectors_are_required_in_the_initial_schema() {
    for path in ["node", "prove"] {
        let mut value = serde_json::to_value(quote()).unwrap();
        let block = &mut value["quote"]["blocks"][0];
        if path == "node" {
            block["node"]["pricing"]
                .as_object_mut()
                .unwrap()
                .remove("transaction_payload");
        } else {
            block["prove"]
                .as_object_mut()
                .unwrap()
                .remove("committed_payload");
        }
        assert!(serde_json::from_value::<CompleteQuoteV1>(value).is_err());
    }
}

#[test]
fn complete_quote_rejects_cross_surface_underquote_and_selector_domain_forgery() {
    const SCALE: i128 = 1_000_000_000_000_000_000;
    for mutation in [
        "prove-fixed-below-node",
        "prove-operations-below-node",
        "shared-feature-attribution",
        "exact-below-local",
        "zero-input-transactions",
        "zero-consumed-candidates",
        "attempted-exceeds-consumed",
        "consumed-exceeds-input",
        "block-proposal-component",
        "proposal-block-component",
        "empty-blob-sources",
        "proposal-selector-price",
        "negative-components",
        "node-resource-price",
        "prove-resource-price",
        "prove-mpt-price",
        "node-payload-selectors",
        "committed-payload-selectors",
        "committed-payload-count",
        "payload-price",
        "proposal-payload-price",
    ] {
        let dir = TestDir::new();
        let mut q = if mutation == "shared-feature-attribution" {
            quote_with_feature_error(2 * SCALE, SCALE)
        } else {
            quote()
        };
        match mutation {
            "prove-fixed-below-node" => {
                let node = &mut q.quote.blocks[0].node.pricing;
                node.fixed_scaled = SCALE;
                node.known_subtotal_scaled = SCALE;
                node.known_subtotal_pgu = 1;
                q.quote.node.block_local_known_scaled = SCALE;
                q.quote.node.block_local_known_pgu = 1;
                q.quote.node.sum_of_block_known_pgu = 1;
            }
            "prove-operations-below-node" => {
                let node = &mut q.quote.blocks[0].node.pricing;
                node.operations_scaled = SCALE;
                node.known_subtotal_scaled = SCALE;
                node.known_subtotal_pgu = 1;
                q.quote.node.block_local_known_scaled = SCALE;
                q.quote.node.block_local_known_pgu = 1;
                q.quote.node.sum_of_block_known_pgu = 1;
            }
            "shared-feature-attribution" => {
                q.quote.blocks[0].prove.pricing.feature_error_scaled /= 2;
            }
            "exact-below-local" => {
                q.quote.blocks[0]
                    .node
                    .pricing
                    .evm_local_resources
                    .unique_account_count = 1;
            }
            "zero-input-transactions" => {
                q.quote.blocks[0].prove.input_transaction_count = 0;
                q.quote.blocks[0].prove.consumed_candidate_count = 0;
                q.quote.blocks[0].prove.attempted_transaction_count = 0;
                q.quote.prove.selectors.payload.total_transaction_count = 0;
            }
            "zero-consumed-candidates" => {
                q.quote.blocks[0].prove.consumed_candidate_count = 0;
                q.quote.blocks[0].prove.attempted_transaction_count = 0;
                q.quote.blocks[0].prove.true_unattempted_transactions = 1;
                q.quote.prove.selectors.true_unattempted_transactions = 1;
            }
            "attempted-exceeds-consumed" => {
                q.quote.blocks[0].prove.attempted_transaction_count = 2;
            }
            "consumed-exceeds-input" => {
                q.quote.blocks[0].prove.consumed_candidate_count = 2;
            }
            "block-proposal-component" => {
                q.quote.blocks[0].prove.pricing.startup_scaled = SCALE;
                add_block_price(&mut q, SCALE);
            }
            "proposal-block-component" => {
                q.quote.prove.proposal_once.startup_scaled -= SCALE;
                q.quote.prove.proposal_once.fixed_scaled = SCALE;
            }
            "empty-blob-sources" => {
                q.quote.prove.selectors.blobs.source_blob_counts.clear();
            }
            "proposal-selector-price" => {
                q.quote.prove.selectors.payload.canonical_transaction_bytes += 1;
            }
            "negative-components" => {
                let node = &mut q.quote.blocks[0].node.pricing;
                node.fixed_scaled = -1;
                node.operations_scaled = 1;
                let prove = &mut q.quote.blocks[0].prove.pricing;
                prove.fixed_scaled = -1;
                prove.operations_scaled = 1;
            }
            "node-resource-price" => {
                let node = &mut q.quote.blocks[0].node.pricing;
                node.evm_local_resources_scaled = SCALE;
                node.known_subtotal_scaled += SCALE;
                node.known_subtotal_pgu += 1;
                q.quote.node.block_local_known_scaled += SCALE;
                q.quote.node.block_local_known_pgu += 1;
                q.quote.node.sum_of_block_known_pgu += 1;
            }
            "prove-resource-price" => {
                q.quote.blocks[0].prove.pricing.resources_scaled = SCALE;
                add_block_price(&mut q, SCALE);
            }
            "prove-mpt-price" => {
                q.quote.blocks[0].prove.pricing.mpt_scaled = SCALE;
                add_block_price(&mut q, SCALE);
            }
            "node-payload-selectors" => {
                q.quote.blocks[0]
                    .node
                    .pricing
                    .transaction_payload
                    .canonical_transaction_bytes += 1;
            }
            "committed-payload-selectors" => {
                q.quote.blocks[0]
                    .prove
                    .committed_payload
                    .canonical_transaction_bytes += 1;
            }
            "committed-payload-count" => {
                q.quote.blocks[0]
                    .prove
                    .committed_payload
                    .total_transaction_count += 1;
            }
            "payload-price" => {
                let node = &mut q.quote.blocks[0].node.pricing;
                node.transaction_payload_scaled += SCALE;
                node.known_subtotal_scaled += SCALE;
                node.known_subtotal_pgu += 1;
                q.quote.node.block_local_known_scaled += SCALE;
                q.quote.node.block_local_known_pgu += 1;
                q.quote.node.sum_of_block_known_pgu += 1;
                q.quote.blocks[0].prove.pricing.transaction_payload_scaled += SCALE;
                add_block_price(&mut q, SCALE);
            }
            "proposal-payload-price" => {
                let proposal = &mut q.quote.prove.proposal_once;
                proposal.transaction_payload_scaled += SCALE;
                proposal.total_scaled += SCALE;
                proposal.total_pgu += 1;
                q.quote.prove.proposal_once_pgu += 1;
                q.quote.prove.total_scaled += SCALE;
                q.quote.prove.total_pgu += 1;
            }
            _ => unreachable!(),
        }
        let args = write_inputs(
            &dir,
            &[QuoteRowV1::Complete(Box::new(q))],
            &[complete_observation()],
        );
        let error = run_compare(&args, &executable()).unwrap_err();
        assert!(
            error.to_string().contains("complete quote invariant"),
            "wrong error for {mutation}: {error:#}"
        );
        let expected_model_diagnostic = match mutation {
            "node-resource-price" => Some("EVM-local resource price mismatch"),
            "prove-resource-price" | "prove-mpt-price" => {
                Some("exact block resource or MPT price mismatch")
            }
            "payload-price" => Some("committed payload price mismatch"),
            "proposal-payload-price" => {
                Some("proposal-once pricing does not match sealed selectors")
            }
            _ => None,
        };
        if let Some(diagnostic) = expected_model_diagnostic {
            assert!(
                format!("{error:#}").contains(diagnostic),
                "wrong model diagnostic for {mutation}: {error:#}"
            );
        }
        assert!(!args.output.exists());
    }
}

#[test]
fn schema_v1_quote_from_an_explicit_different_model_remains_comparable() {
    const SCALE: i128 = 1_000_000_000_000_000_000;
    let dir = TestDir::new();
    let mut q = quote();
    q.quote.model_id = "model-m2".into();
    q.calibration_profile = q.quote.model_id.clone();

    let payload_scaled = q.quote.blocks[0].node.pricing.transaction_payload_scaled;
    let payload_pgu = u128::try_from(payload_scaled / SCALE).unwrap();
    let block = &mut q.quote.blocks[0];
    block.node.pricing.evm_local_resources.unique_account_count = 1;
    block.node.pricing.evm_local_resources_scaled = SCALE;
    block.node.pricing.known_subtotal_scaled = SCALE + payload_scaled;
    block.node.pricing.known_subtotal_pgu = 1 + payload_pgu;
    block.prove.exact_resources.unique_account_count = 1;
    block.prove.pricing.resources_scaled = SCALE;
    block.prove.pricing.total_scaled = SCALE + payload_scaled;
    block.prove.pricing.total_pgu = 1 + payload_pgu;
    q.quote.node.block_local_known_scaled = SCALE + payload_scaled;
    q.quote.node.block_local_known_pgu = 1 + payload_pgu;
    q.quote.node.sum_of_block_known_pgu = 1 + payload_pgu;
    q.quote.prove.block_local_scaled = SCALE + payload_scaled;
    q.quote.prove.block_local_pgu = 1 + payload_pgu;
    q.quote.prove.sum_of_block_local_pgu = 1 + payload_pgu;

    q.quote.prove.proposal_once.startup_scaled += SCALE;
    q.quote.prove.proposal_once.total_scaled += SCALE;
    q.quote.prove.proposal_once.total_pgu += 1;
    q.quote.prove.proposal_once_pgu += 1;
    q.quote.prove.total_scaled += 2 * SCALE;
    q.quote.prove.total_pgu += 2;

    let mut o = observation();
    o.workload.value = q.quote.prove.total_pgu * 2;
    let mut profiles = current_model_profiles();
    let profile = &mut profiles.profiles[0];
    profile.profile_id = "model-m2-profile".into();
    profile.model_id = q.quote.model_id.clone();
    profile.runtime_spec_sha256 = Some(q.quote.model_id.clone());
    profile.normalization = None;

    let mut args = write_inputs(
        &dir,
        &[QuoteRowV1::Complete(Box::new(q))],
        &[ObservationRowV1::Complete(Box::new(o))],
    );
    let profiles_path = dir.path().join("profiles.toml");
    fs::write(&profiles_path, toml::to_string(&profiles).unwrap()).unwrap();
    args.profiles = Some(profiles_path);

    let run = run_compare(&args, &executable()).unwrap();
    assert_eq!(run.exit_code(), 0);
    assert!(matches!(
        &rows(&run.run_path)[0].result,
        ComparisonResultV1::Joined { profile_id, .. } if profile_id == "model-m2-profile"
    ));
}

#[test]
fn empty_quote_or_observation_populations_are_structural_collection_failures() {
    for (quotes, observations) in [
        (vec![], vec![complete_observation()]),
        (vec![complete_quote()], vec![]),
        (vec![], vec![]),
    ] {
        let dir = TestDir::new();
        let args = write_inputs(&dir, &quotes, &observations);
        let error = run_compare(&args, &executable()).unwrap_err();
        assert!(
            error.to_string().contains("population is empty"),
            "{error:#}"
        );
        assert!(!args.output.exists());
    }
}

#[test]
fn incompatibility_and_ambiguity_emit_one_row_with_the_correct_metric_eligibility() {
    for mode in ["program", "unit", "no-profile", "ambiguous", "custom"] {
        let dir = TestDir::new();
        let mut o = observation();
        let mut profiles = current_model_profiles();
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
        sha256_hex(&fs::read(args.profiles.as_ref().unwrap()).unwrap())
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
        let mut p = current_model_profiles();
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
            .arg("--profiles")
            .arg(args.profiles.as_ref().unwrap())
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
    let args = write_inputs(&dir, &[failed.clone(), failed], &[complete_observation()]);
    assert!(
        run_compare(&args, &executable())
            .unwrap_err()
            .to_string()
            .contains("duplicate comparison row ID")
    );
    assert!(!args.output.exists());
}
