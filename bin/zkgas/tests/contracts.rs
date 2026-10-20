use std::collections::BTreeMap;

use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use zkgas::{contracts::*, identity::*};

fn assert_strict<T: DeserializeOwned>(value: Value) {
    serde_json::from_value::<T>(value.clone()).expect("schema version one");
    let mut wrong_version = value.clone();
    wrong_version["schema_version"] = json!(2);
    assert!(serde_json::from_value::<T>(wrong_version).is_err());
    let mut unknown = value;
    unknown["unexpected"] = json!(true);
    assert!(serde_json::from_value::<T>(unknown).is_err());
}

fn executable() -> Value {
    json!({"sha256":"executable", "package_version":"0.1.0"})
}

fn failure() -> Value {
    json!({"code":"producer_failed", "message":"unavailable"})
}

#[test]
fn all_eight_contracts_have_independent_strict_versions() {
    assert_strict::<InputManifestV1>(json!({"schema_version":1,"corpus_id":"corpus","samples":[]}));
    assert_strict::<ObservationManifestV1>(
        json!({"schema_version":1,"observation_set_id":"set","observations":[]}),
    );
    assert_strict::<QuoteRowV1>(
        json!({"schema_version":1,"status":"failed","corpus_id":"corpus","sample_id":"sample","executable":executable(),"input_source":{"kind":"single","descriptor_sha256":"descriptor"},"failure":failure()}),
    );
    assert_strict::<CompatibilityProfilesV1>(json!({"schema_version":1,"profiles":[]}));
    assert_strict::<ComparisonManifestV1>(
        json!({"schema_version":1,"run_id":"run","executable":executable(),"quote_file_sha256":[],"observation_file_sha256":[],"profiles_sha256":"profiles","arithmetic_policy":"exact-v1","model_ids":[],"profile_ids":[],"rows":[],"row_sequence_sha256":"sequence","status":"complete","counts":{}}),
    );
    assert_strict::<ComparisonRowV1>(
        json!({"schema_version":1,"comparison_row_id":"row","result":{"kind":"invalid_observation","observation_set_id":"set","observation":{"observation_id":"observation","attempt_id":"attempt","failure":failure()},"observation_row_sha256":"hash"}}),
    );
    assert_strict::<ReportConfigV1>(
        json!({"schema_version":1,"filters":{},"quantiles":["0.5"],"buckets":[]}),
    );
    assert_strict::<ReportV1>(
        json!({"schema_version":1,"report_id":"report","comparison_manifest_sha256":"manifest","row_sequence_sha256":"sequence","config_sha256":"config","executable":executable(),"arithmetic_policy":"exact-v1","status":"complete","whole_run_counts":{},"selected_counts":{},"strata":[],"excluded":[]}),
    );
}

#[test]
fn failed_quote_distinguishes_unavailable_observed_and_verified_identity() {
    let minimal = json!({"schema_version":1,"status":"failed","corpus_id":"corpus","sample_id":"sample","executable":executable(),"input_source":{"kind":"single","descriptor_sha256":"descriptor"},"failure":failure()});
    let QuoteRowV1::Failed(row) = serde_json::from_value::<QuoteRowV1>(minimal.clone()).unwrap()
    else {
        unreachable!()
    };
    let encoded = serde_json::to_value(row).unwrap();
    assert!(encoded.get("verified_key").is_none());
    assert!(encoded.get("observed_guest_input_sha256").is_none());
    assert!(encoded.get("observed_guest_input_byte_length").is_none());
    assert!(encoded.get("key").is_none());
    assert!(encoded.get("guest_input_byte_length").is_none());
    let mut observed = minimal.clone();
    observed["observed_guest_input_sha256"] = json!("observed");
    observed["observed_guest_input_byte_length"] = json!(7);
    serde_json::from_value::<QuoteRowV1>(observed.clone()).unwrap();
    observed["verified_key"] =
        json!({"corpus_id":"corpus","sample_id":"sample","guest_input_sha256":"canonical"});
    serde_json::from_value::<QuoteRowV1>(observed).unwrap();
    let mut legacy = minimal.clone();
    legacy["key"] = json!({"corpus_id":"corpus","sample_id":"sample","guest_input_sha256":""});
    assert!(serde_json::from_value::<QuoteRowV1>(legacy).is_err());
    for id in ["corpus_id", "sample_id"] {
        let mut missing = minimal.clone();
        missing.as_object_mut().unwrap().remove(id);
        assert!(serde_json::from_value::<QuoteRowV1>(missing).is_err());
    }
}

#[test]
fn nested_structures_reject_unknown_fields_and_failed_observations_remain_valid() {
    let mut value = json!({"schema_version":1,"corpus_id":"c","samples":[{"sample_id":"s","guest_input":{"path":"missing.bin","sha256":"hash","byte_length":1}}]});
    value["samples"][0]["guest_input"]["unexpected"] = json!(1);
    assert!(serde_json::from_value::<InputManifestV1>(value).is_err());
    let failed = json!({"schema_version":1,"observation_set_id":"set","observations":[{"status":"failed","observation_id":"o","attempt_id":"a","failure":failure()}]});
    serde_json::from_value::<ObservationManifestV1>(failed.clone()).unwrap();
    let mut unknown = failed;
    unknown["observations"][0]["unexpected"] = json!(0);
    assert!(serde_json::from_value::<ObservationManifestV1>(unknown).is_err());
}

#[test]
fn metadata_and_source_are_sorted_and_preserved() {
    let raw = r#"{"z":{"b":2,"a":1},"a":[true,"value"]}"#;
    let opaque: OpaqueObject = serde_json::from_str(raw).unwrap();
    assert_eq!(
        String::from_utf8(canonical_json(&opaque).unwrap()).unwrap(),
        r#"{"a":[true,"value"],"z":{"a":1,"b":2}}"#
    );
    let value = json!({"schema_version":1,"corpus_id":"c","samples":[{"sample_id":"s","guest_input":{"path":"input.bin","sha256":"hash","byte_length":1},"metadata":opaque}]});
    let input: InputManifestV1 = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(input).unwrap(), value);
    let observation = json!({"status":"failed","observation_id":"o","attempt_id":"a","failure":failure(),"source":opaque});
    let parsed: ObservationRowV1 = serde_json::from_value(observation.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), observation);
}

#[test]
fn opaque_byte_depth_and_entry_limits_reject_excess_values() {
    assert!(
        serde_json::from_value::<OpaqueObject>(json!({"value":"a".repeat(MAX_OPAQUE_BYTES)}))
            .is_err()
    );
    let entries: BTreeMap<_, _> = (0..=MAX_OPAQUE_ENTRIES)
        .map(|i| (i.to_string(), json!(0)))
        .collect();
    assert!(serde_json::from_value::<OpaqueObject>(json!(entries)).is_err());
    let mut deep = json!(0);
    for _ in 0..=MAX_OPAQUE_DEPTH {
        deep = json!({"next":deep});
    }
    assert!(serde_json::from_value::<OpaqueObject>(deep).is_err());
    assert!(serde_json::from_value::<OpaqueObject>(json!([1, 2])).is_err());
}

#[test]
fn excessive_manifest_rows_are_rejected_before_any_input_is_opened() {
    let samples: Vec<_> = (0..=MAX_INPUT_ROWS).map(|i| json!({"sample_id":format!("sample-{i}"),"guest_input":{"path":"does-not-exist.bin","sha256":"hash","byte_length":1}})).collect();
    let error = serde_json::from_value::<InputManifestV1>(
        json!({"schema_version":1,"corpus_id":"c","samples":samples}),
    )
    .unwrap_err();
    assert!(error.to_string().contains("row limit"), "{error}");
}

#[test]
fn workload_u128_wire_values_are_decimal_strings() {
    let workload = Workload {
        value: u128::MAX,
        unit: "sp1_pgu".into(),
    };
    let value = serde_json::to_value(&workload).unwrap();
    assert_eq!(
        value["value"],
        json!("340282366920938463463374607431768211455")
    );
    assert_eq!(serde_json::from_value::<Workload>(value).unwrap(), workload);
    assert!(serde_json::from_value::<Workload>(json!({"value":42,"unit":"sp1_pgu"})).is_err());
    assert!(serde_json::from_value::<Workload>(json!({"value":"-1","unit":"sp1_pgu"})).is_err());
}

#[test]
fn canonical_identity_sorts_nested_maps_and_separates_domains() {
    #[derive(Serialize)]
    struct Descriptor {
        values: BTreeMap<String, u64>,
    }
    let first = Descriptor {
        values: BTreeMap::from([("b".into(), 2), ("a".into(), 1)]),
    };
    let second = Descriptor {
        values: BTreeMap::from([("a".into(), 1), ("b".into(), 2)]),
    };
    assert_eq!(
        content_id("quote-row-v1", &first).unwrap(),
        content_id("quote-row-v1", &second).unwrap()
    );
    let ids: std::collections::BTreeSet<_> = ["quote-row-v1", "joined-row-v1", "unmatched-row-v1"]
        .into_iter()
        .map(|domain| content_id(domain, &first).unwrap())
        .collect();
    assert_eq!(ids.len(), 3);
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn executable_identity_binds_executable_bytes() {
    let identity = executable_identity().unwrap();
    let actual = std::fs::read(std::env::current_exe().unwrap()).unwrap();
    assert_eq!(identity.sha256, sha256_hex(&actual));
    assert_eq!(identity.package_version, env!("CARGO_PKG_VERSION"));
}

#[test]
fn complete_quote_preserves_full_width_embedded_pricing_and_rejects_malformed_values() {
    use raiko2_zkgas_trace::*;
    let quote = CompleteQuoteV1 {
        schema_version: 1,
        key: QuoteKey {
            corpus_id: "corpus".into(),
            sample_id: "sample".into(),
            guest_input_sha256: "input".into(),
        },
        executable: serde_json::from_value(executable()).unwrap(),
        input_source: InputSourceV1::Single {
            descriptor_sha256: "descriptor".into(),
        },
        guest_input_byte_length: 1,
        metadata: None,
        proposal: ProposalIdentityV1 {
            proposal_id: 1,
            chain: ChainIdentityV1 {
                name: "chain".into(),
                chain_id: 1,
            },
            proof_carry_chain_id: 1,
            proof_carry_proposal_id: 1,
            witness_chain_spec_sha256: vec!["spec".into()],
            first_block: 1,
            last_block: 1,
            block_count: 1,
            public_output: "output".into(),
            fork_segments: vec![],
        },
        prediction_unit: "sp1_pgu".into(),
        prediction_surface: "prove".into(),
        calibration_profile: "profile".into(),
        quote: ProposalWorkloadQuote {
            schema_version: 1,
            model_id: "model".into(),
            guest_input_sha256: "input".into(),
            guest_input_bincode_length: 1,
            public_output: Default::default(),
            blocks: vec![],
            node: NodeProposalWorkloadQuote {
                unzen_zkgas_sum: u128::MAX,
                block_local_known_scaled: i128::MIN,
                block_local_known_pgu: u128::MAX,
                sum_of_block_known_pgu: u128::MAX,
            },
            prove: ProveProposalWorkloadQuote {
                selectors: ProposalPricingSelectorsSnapshot {
                    payload: ProposalPayloadSelectors {
                        total_transaction_count: 0,
                        canonical_transaction_bytes: 0,
                        nonempty_payload_transactions: 0,
                    },
                    true_unattempted_transactions: 0,
                    blobs: ProposalBlobSelectors {
                        source_blob_counts: vec![],
                        proposal_blob_hash_count: 0,
                        provider_blob_count: 0,
                        provider_commitment_count: 0,
                        provider_proof_count: 0,
                    },
                    parent_mpt: Default::default(),
                },
                block_local_scaled: i128::MAX,
                block_local_pgu: u128::MAX,
                sum_of_block_admission_pgu: u128::MAX,
                proposal_once: PricingBreakdownSnapshot {
                    total_scaled: i128::MIN,
                    total_pgu: u128::MAX,
                    ..Default::default()
                },
                proposal_once_pgu: u128::MAX,
                total_scaled: i128::MAX,
                total_pgu: u128::MAX,
            },
        },
    };
    let row = QuoteRowV1::Complete(Box::new(quote));
    let value = serde_json::to_value(&row).unwrap();
    assert_eq!(
        value["quote"]["prove"]["total_pgu"],
        json!("340282366920938463463374607431768211455")
    );
    assert_eq!(
        value["quote"]["node"]["block_local_known_scaled"],
        json!("-170141183460469231731687303715884105728")
    );
    assert_eq!(
        serde_json::from_value::<QuoteRowV1>(value.clone()).unwrap(),
        row
    );
    assert_strict::<QuoteRowV1>(value.clone());
    let mut bad = value.clone();
    bad["quote"]["prove"]["total_pgu"] = json!(1);
    assert!(serde_json::from_value::<QuoteRowV1>(bad).is_err());
    let mut bad = value.clone();
    bad["quote"]["prove"]["total_pgu"] = json!("340282366920938463463374607431768211456");
    assert!(serde_json::from_value::<QuoteRowV1>(bad).is_err());
    let mut bad = value.clone();
    bad["quote"]["schema_version"] = json!(2);
    assert!(serde_json::from_value::<QuoteRowV1>(bad).is_err());
    let mut bad = value;
    bad["quote"]["prove"]["unknown"] = json!(1);
    assert!(serde_json::from_value::<QuoteRowV1>(bad).is_err());
}
