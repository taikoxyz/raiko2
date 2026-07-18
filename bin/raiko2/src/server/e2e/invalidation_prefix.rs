use super::*;

#[tokio::test]
async fn e2e_v4_invalidate_artifacts_prefix_matches_aggregate_root() {
    let (app, engine, state) = v4_sp1_acl_state_app_with_clear_rate_limit(None);
    let aggregation_payload = v4_sp1_aggregation_request(41, 42);
    complete_v4_sp1_aggregation(&app, &engine, 41, 42).await;

    let records = state.runtime.list_tasks().await.expect("list tasks");
    let record = records
        .iter()
        .find(|record| {
            let metadata: TaskMetadata =
                serde_json::from_value(record.metadata.clone()).expect("parse task metadata");
            metadata.aggregate_request.is_some()
        })
        .expect("aggregate runtime task")
        .clone();
    assert_eq!(
        record.runner_status,
        RunnerStatus::Completed,
        "aggregate runtime task must be terminal before invalidation: {record:?}"
    );
    let metadata: TaskMetadata =
        serde_json::from_value(record.metadata.clone()).expect("parse task metadata");
    assert_eq!(
        metadata.requested_proof_type.as_deref(),
        Some("sp1"),
        "unexpected proof-type scope metadata: {metadata:?}"
    );
    let root_refs = root_proof_artifact_refs(&metadata, record.pipeline_key)
        .expect("aggregate root refs")
        .refs;
    let proposal_refs = metadata
        .proposals
        .iter()
        .flat_map(|proposal| proposal_proof_artifact_refs(record.pipeline_key, proposal))
        .collect::<Vec<_>>();
    assert!(!proposal_refs.is_empty(), "expected child proposal refs");

    let root_ref = root_refs.first().expect("aggregate root ref");
    replace_e2e_proof_artifact(
        &state,
        &metadata.network_pair,
        root_ref,
        record.pipeline_key,
        record.route,
        &Proof {
            proof: Some(format!("0x{}", "aa".repeat(32))),
            input: Some(alloy_primitives::B256::ZERO),
            ..Proof::default()
        },
    )
    .await;
    write_e2e_pending_publication(
        &state,
        &metadata.network_pair,
        root_ref,
        record.pipeline_key,
        record.route,
    )
    .await;
    for proof_ref in &proposal_refs {
        write_e2e_pending_publication(
            &state,
            &metadata.network_pair,
            proof_ref,
            record.pipeline_key,
            record.route,
        )
        .await;
    }

    let request = json!({
        "proof_type": "sp1",
        "proof_prefix": "0xAAAA",
        "proposal_id_start": 41,
        "proposal_id_end": 42
    });
    let (status, invalidated) = post_json_with_api_key(
        &app,
        "/v4/prover/invalidate-artifacts",
        "clear-secret",
        request,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{invalidated}");
    assert_eq!(invalidated["data"]["tasks"]["matched"], 1, "{invalidated}");
    assert_eq!(invalidated["data"]["tasks"]["removed"], 1, "{invalidated}");

    for proof_ref in &root_refs {
        assert!(
            state
                .runtime
                .get_proof_artifact(
                    &metadata.network_pair,
                    record.pipeline_key,
                    record.route,
                    proof_ref,
                )
                .await
                .expect("get proof artifact")
                .is_none(),
            "stale proof ref remained: {proof_ref}"
        );
        assert_e2e_pending_publication(
            &state,
            &metadata.network_pair,
            proof_ref,
            record.pipeline_key,
            record.route,
            false,
        )
        .await;
    }
    for proof_ref in &proposal_refs {
        assert!(
            state
                .runtime
                .get_proof_artifact(
                    &metadata.network_pair,
                    record.pipeline_key,
                    record.route,
                    proof_ref,
                )
                .await
                .expect("get shared proposal proof artifact")
                .is_some(),
            "shared proposal proof was removed: {proof_ref}"
        );
        assert_e2e_pending_publication(
            &state,
            &metadata.network_pair,
            proof_ref,
            record.pipeline_key,
            record.route,
            true,
        )
        .await;
    }

    let (status, resubmitted) = post_json_with_api_key(
        &app,
        "/v4/proof/proposal",
        "submit-secret",
        aggregation_payload,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{resubmitted}");
    assert_eq!(resubmitted["data"]["status"], "registered", "{resubmitted}");
    assert!(resubmitted["data"]["proof"].is_null(), "{resubmitted}");
}
#[tokio::test]
async fn e2e_v4_invalidate_artifacts_prefix_keeps_unmatched_aggregate() {
    let (app, engine, state) = v4_sp1_acl_state_app_with_clear_rate_limit(None);
    let aggregation_payload = v4_sp1_aggregation_request(51, 52);
    complete_v4_sp1_aggregation(&app, &engine, 51, 52).await;

    let record = state
        .runtime
        .list_tasks()
        .await
        .expect("list tasks")
        .into_iter()
        .find(|record| {
            serde_json::from_value::<TaskMetadata>(record.metadata.clone())
                .expect("parse task metadata")
                .aggregate_request
                .is_some()
        })
        .expect("aggregate runtime task");
    let metadata: TaskMetadata =
        serde_json::from_value(record.metadata.clone()).expect("parse task metadata");
    let root_ref = root_proof_artifact_refs(&metadata, record.pipeline_key)
        .expect("aggregate root refs")
        .refs
        .into_iter()
        .next()
        .expect("aggregate root ref");
    replace_e2e_proof_artifact(
        &state,
        &metadata.network_pair,
        &root_ref,
        record.pipeline_key,
        record.route,
        &Proof {
            proof: Some(format!("0x{}", "aa".repeat(32))),
            input: Some(alloy_primitives::B256::ZERO),
            ..Proof::default()
        },
    )
    .await;
    let (status, invalidated) = post_json_with_api_key(
        &app,
        "/v4/prover/invalidate-artifacts",
        "clear-secret",
        json!({
            "proof_type": "sp1",
            "proof_prefix": "0xBBBB",
            "proposal_id_start": 51,
            "proposal_id_end": 52
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{invalidated}");
    assert_eq!(invalidated["data"]["tasks"]["matched"], 0, "{invalidated}");
    assert_eq!(invalidated["data"]["tasks"]["removed"], 0, "{invalidated}");
    assert_eq!(
        invalidated["data"]["artifacts"]["matched"], 0,
        "{invalidated}"
    );
    assert!(
        state
            .runtime
            .get_task(&record.task_id)
            .await
            .expect("get aggregate task")
            .is_some()
    );
    assert!(
        state
            .runtime
            .get_proof_artifact(
                &metadata.network_pair,
                record.pipeline_key,
                record.route,
                &root_ref,
            )
            .await
            .expect("get aggregate proof artifact")
            .is_some()
    );

    let (status, completed) = post_json_with_api_key(
        &app,
        "/v4/proof/proposal",
        "submit-secret",
        aggregation_payload,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    assert_eq!(completed["data"]["status"], "completed", "{completed}");
}
#[tokio::test]
async fn e2e_v4_invalidate_artifacts_prefix_removes_only_matched_aggregate_input() {
    let (app, engine, state) = v4_sp1_acl_state_app_with_clear_rate_limit(None);
    complete_v4_sp1_aggregation(&app, &engine, 71, 72).await;

    let record = state
        .runtime
        .list_tasks()
        .await
        .expect("list tasks")
        .into_iter()
        .find(|record| {
            serde_json::from_value::<TaskMetadata>(record.metadata.clone())
                .expect("parse task metadata")
                .aggregate_request
                .is_some()
        })
        .expect("aggregate runtime task");
    let metadata: TaskMetadata =
        serde_json::from_value(record.metadata.clone()).expect("parse task metadata");
    let root_ref = root_proof_artifact_refs(&metadata, record.pipeline_key)
        .expect("aggregate root refs")
        .refs
        .into_iter()
        .next()
        .expect("aggregate root ref");
    let proposal_refs = metadata
        .proposals
        .iter()
        .flat_map(|proposal| proposal_proof_artifact_refs(record.pipeline_key, proposal))
        .collect::<Vec<_>>();
    assert_eq!(proposal_refs.len(), 2, "expected two proposal refs");
    replace_e2e_proof_artifact(
        &state,
        &metadata.network_pair,
        &proposal_refs[0],
        record.pipeline_key,
        record.route,
        &Proof {
            proof: Some(format!("0x{}", "bb".repeat(32))),
            input: Some(alloy_primitives::B256::ZERO),
            ..Proof::default()
        },
    )
    .await;
    for proof_ref in std::iter::once(&root_ref).chain(proposal_refs.iter()) {
        write_e2e_pending_publication(
            &state,
            &metadata.network_pair,
            proof_ref,
            record.pipeline_key,
            record.route,
        )
        .await;
    }

    let (status, invalidated) = post_json_with_api_key(
        &app,
        "/v4/prover/invalidate-artifacts",
        "clear-secret",
        json!({
            "proof_type": "sp1",
            "proof_prefix": "0xBBBB",
            "proposal_id_start": 71,
            "proposal_id_end": 72
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{invalidated}");
    assert_eq!(invalidated["data"]["tasks"]["matched"], 1, "{invalidated}");
    assert_eq!(invalidated["data"]["tasks"]["removed"], 1, "{invalidated}");
    assert!(
        state
            .runtime
            .get_proof_artifact(
                &metadata.network_pair,
                record.pipeline_key,
                record.route,
                &root_ref,
            )
            .await
            .expect("get aggregate proof artifact")
            .is_none(),
        "aggregate output depending on the invalidated input remained"
    );
    assert!(
        state
            .runtime
            .get_proof_artifact(
                &metadata.network_pair,
                record.pipeline_key,
                record.route,
                &proposal_refs[0],
            )
            .await
            .expect("get matched proposal proof artifact")
            .is_none(),
        "matched proposal input remained"
    );
    assert_e2e_pending_publication(
        &state,
        &metadata.network_pair,
        &root_ref,
        record.pipeline_key,
        record.route,
        false,
    )
    .await;
    assert_e2e_pending_publication(
        &state,
        &metadata.network_pair,
        &proposal_refs[0],
        record.pipeline_key,
        record.route,
        false,
    )
    .await;
    assert!(
        state
            .runtime
            .get_proof_artifact(
                &metadata.network_pair,
                record.pipeline_key,
                record.route,
                &proposal_refs[1],
            )
            .await
            .expect("get unmatched proposal proof artifact")
            .is_some(),
        "unmatched proposal input was removed"
    );
    assert_e2e_pending_publication(
        &state,
        &metadata.network_pair,
        &proposal_refs[1],
        record.pipeline_key,
        record.route,
        true,
    )
    .await;
}

#[tokio::test]
async fn e2e_v4_invalidate_external_aggregate_input_removes_dependent_output() {
    let (app, engine, state) = v4_sp1_acl_state_app_with_clear_rate_limit(None);
    let matched_proof = format!("0x{}", "aa".repeat(32));
    let unmatched_proof = format!("0x{}", "cc".repeat(32));
    let (status, submitted) = post_json(
        &app,
        "/v3/proof/aggregate",
        json!({
            "aggregation_ids": [81, 82],
            "proofs": [
                sp1_external_proof(matched_proof.clone()),
                sp1_external_proof(unmatched_proof)
            ],
            "proof_type": "sp1",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{submitted}");
    Box::pin(drive_engine_to_idle(&engine)).await;

    let record = state
        .runtime
        .list_tasks()
        .await
        .expect("list tasks")
        .into_iter()
        .find(|record| {
            serde_json::from_value::<TaskMetadata>(record.metadata.clone())
                .expect("parse task metadata")
                .aggregate_input_artifacts
                .len()
                == 2
        })
        .expect("external aggregate runtime task");
    assert_eq!(record.runner_status, RunnerStatus::Completed);
    let metadata: TaskMetadata =
        serde_json::from_value(record.metadata.clone()).expect("parse task metadata");
    let root_ref = root_proof_artifact_refs(&metadata, record.pipeline_key)
        .expect("external aggregate root ref")
        .refs
        .into_iter()
        .next()
        .expect("external aggregate root ref");
    let input_refs = metadata
        .aggregate_input_artifacts
        .iter()
        .map(|artifact| artifact.proof_ref.clone())
        .collect::<Vec<_>>();

    let (status, invalidated) = post_json_with_api_key(
        &app,
        "/v4/prover/invalidate-artifacts",
        "clear-secret",
        json!({
            "proof_type": "sp1",
            "proposal_id_start": 81,
            "proposal_id_end": 81
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{invalidated}");
    assert_eq!(invalidated["data"]["tasks"]["removed"], 1, "{invalidated}");
    assert!(
        state
            .runtime
            .get_task(&record.task_id)
            .await
            .expect("get external aggregate task")
            .is_none(),
        "dependent external aggregate root remained registered"
    );
    for removed_ref in [&root_ref, &input_refs[0]] {
        assert!(
            state
                .runtime
                .get_proof_artifact(
                    &metadata.network_pair,
                    record.pipeline_key,
                    record.route,
                    removed_ref,
                )
                .await
                .expect("get invalidated artifact")
                .is_none(),
            "dependent artifact {removed_ref} remained active"
        );
    }
    assert!(
        state
            .runtime
            .get_proof_artifact(
                &metadata.network_pair,
                record.pipeline_key,
                record.route,
                &input_refs[1],
            )
            .await
            .expect("get unmatched external aggregate input")
            .is_some(),
        "unmatched external aggregate input was removed"
    );
}

#[tokio::test]
async fn e2e_v4_invalidate_artifacts_keeps_shared_artifact_for_nonterminal_root() {
    let (app, engine, state) = v4_sp1_acl_state_app_with_clear_rate_limit(None);
    complete_v4_sp1_proposal(&app, &engine, 61).await;

    let aggregation_payload = v4_sp1_aggregation_request(61, 61);
    let (status, submitted) = post_json_with_api_key(
        &app,
        "/v4/proof/proposal",
        "submit-secret",
        aggregation_payload.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{submitted}");
    assert_eq!(submitted["data"]["status"], "registered", "{submitted}");
    let aggregate_task_id = submitted["data"]["task_id"]
        .as_str()
        .expect("aggregate task id")
        .to_string();

    let records = state.runtime.list_tasks().await.expect("list tasks");
    let proposal_record = records
        .iter()
        .find(|record| {
            let metadata: TaskMetadata =
                serde_json::from_value(record.metadata.clone()).expect("parse task metadata");
            metadata.aggregate_request.is_none()
                && metadata
                    .proposals
                    .iter()
                    .any(|proposal| proposal.proposal_id == 61)
        })
        .expect("completed proposal task");
    let proposal_metadata: TaskMetadata =
        serde_json::from_value(proposal_record.metadata.clone()).expect("parse proposal metadata");
    let proposal_ref = proposal_proof_artifact_refs(
        proposal_record.pipeline_key,
        proposal_metadata.proposals.first().expect("proposal"),
    )
    .into_iter()
    .next()
    .expect("proposal artifact ref");
    write_e2e_pending_publication(
        &state,
        &proposal_metadata.network_pair,
        &proposal_ref,
        proposal_record.pipeline_key,
        proposal_record.route,
    )
    .await;

    let (status, invalidated) = post_json_with_api_key(
        &app,
        "/v4/prover/invalidate-artifacts",
        "clear-secret",
        json!({
            "proof_type": "sp1",
            "proposal_id_start": 61,
            "proposal_id_end": 61
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{invalidated}");
    assert_eq!(invalidated["data"]["tasks"]["matched"], 1, "{invalidated}");
    assert_eq!(invalidated["data"]["tasks"]["removed"], 1, "{invalidated}");
    assert_eq!(
        invalidated["data"]["tasks"]["skipped_non_terminal"], 1,
        "{invalidated}"
    );
    assert!(
        state
            .runtime
            .get_task(&aggregate_task_id)
            .await
            .expect("get aggregate task")
            .is_some(),
        "nonterminal aggregate root was removed"
    );
    assert!(
        state
            .runtime
            .get_proof_artifact(
                &proposal_metadata.network_pair,
                proposal_record.pipeline_key,
                proposal_record.route,
                &proposal_ref,
            )
            .await
            .expect("get shared proposal artifact")
            .is_some(),
        "artifact needed by the nonterminal aggregate root was removed"
    );
    assert_e2e_pending_publication(
        &state,
        &proposal_metadata.network_pair,
        &proposal_ref,
        proposal_record.pipeline_key,
        proposal_record.route,
        true,
    )
    .await;

    drive_engine_to_idle(&engine).await;
    let (status, completed) = post_json_with_api_key(
        &app,
        "/v4/proof/proposal",
        "submit-secret",
        aggregation_payload,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    assert_eq!(completed["data"]["status"], "completed", "{completed}");
}
