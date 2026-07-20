use super::*;

#[tokio::test]
async fn e2e_v4_invalidate_artifacts_range_removes_canonical_root_ref() {
    let (app, engine, state) = v4_sp1_acl_state_app_with_clear_rate_limit(None);
    complete_v4_sp1_proposal(&app, &engine, 21).await;

    let records = state.runtime.list_tasks().await.expect("list tasks");
    let record = records.first().expect("runtime task").clone();
    let metadata: TaskMetadata =
        serde_json::from_value(record.metadata.clone()).expect("parse task metadata");

    let root_refs = root_proof_artifact_refs(&metadata, record.pipeline_key)
        .expect("proposal root refs")
        .refs;
    assert_eq!(root_refs.len(), 1, "expected one canonical root ref");

    let request = json!({
        "proof_type": "sp1",
        "proposal_id_start": 21,
        "proposal_id_end": 21
    });
    let (status, invalidated) = post_json_with_api_key(
        &app,
        "/v4/prover/invalidate-artifacts",
        "clear-secret",
        request,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{invalidated}");
    assert_eq!(
        invalidated["data"]["artifacts"]["removed"],
        root_refs.len(),
        "{invalidated}"
    );
    for proof_ref in root_refs {
        assert!(
            state
                .runtime
                .get_proof_artifact(
                    &metadata.network_pair,
                    record.pipeline_key,
                    record.route,
                    &proof_ref,
                )
                .await
                .expect("get proof artifact")
                .is_none(),
            "stale root proof ref remained: {proof_ref}"
        );
    }
}

#[tokio::test]
async fn e2e_v4_invalidate_artifacts_range_removes_aggregate_child_refs() {
    let (app, engine, state) = v4_sp1_acl_state_app_with_clear_rate_limit(None);
    complete_v4_sp1_aggregation(&app, &engine, 31, 32).await;

    let records = state.runtime.list_tasks().await.expect("list tasks");
    let record = records.first().expect("runtime task");
    let metadata: TaskMetadata =
        serde_json::from_value(record.metadata.clone()).expect("parse task metadata");
    let proposal_refs = metadata
        .proposals
        .iter()
        .flat_map(|proposal| proposal_proof_artifact_refs(record.pipeline_key, proposal))
        .collect::<Vec<_>>();
    let root_refs = root_proof_artifact_refs(&metadata, record.pipeline_key)
        .expect("aggregate root refs")
        .refs;
    assert!(!proposal_refs.is_empty(), "expected child proposal refs");
    for proof_ref in proposal_refs.iter().chain(root_refs.iter()) {
        write_e2e_proof_artifact(
            &state,
            &metadata.network_pair,
            proof_ref,
            record.pipeline_key,
            record.route,
            &fixture_external_aggregate_proof(),
        )
        .await;
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
        "proposal_id_start": 31,
        "proposal_id_end": 32
    });
    let (status, invalidated) = post_json_with_api_key(
        &app,
        "/v4/prover/invalidate-artifacts",
        "clear-secret",
        request,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{invalidated}");
    for proof_ref in proposal_refs.into_iter().chain(root_refs) {
        assert!(
            state
                .runtime
                .get_proof_artifact(
                    &metadata.network_pair,
                    record.pipeline_key,
                    record.route,
                    &proof_ref,
                )
                .await
                .expect("get proof artifact")
                .is_none(),
            "stale child proposal proof ref remained: {proof_ref}"
        );
        assert_e2e_pending_publication(
            &state,
            &metadata.network_pair,
            &proof_ref,
            record.pipeline_key,
            record.route,
            false,
        )
        .await;
    }
}
