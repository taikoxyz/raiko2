use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::HeaderMap,
    response::Response,
};
use raiko2_runtime::{RunnerStatus as RuntimeRunnerStatus, TaskRegistrationOutcome};
use std::collections::HashSet;
use tracing::{debug, info};

use super::{
    AggregateProofRequest, ApiData, ApiError, ApiOk, AppState, BatchShastaRequest,
    ClearProverStatus, ProofStatus, ProverStatus, ProverTaskScope, PruneStatus, ServerAclFeature,
    TaskData, TaskLookup, TaskMetadata, authorize_acl_feature_with_rate_limit,
    batch_request_fingerprint, build_canonical_batch_submission,
    build_external_aggregate_submission, build_submission_plan, cancel_registered_tasks,
    clear_prover_tasks, clear_task_publication_outboxes, collect_prover_status,
    handle_created_batch_task, handle_created_external_aggregate_task, handle_existing_batch_task,
    handle_existing_external_aggregate_task, legacy_api_error_response, load_all_task_data,
    load_task_data, load_task_lookup, planned_external_aggregate_task, prover_type_label,
    public_task_id_from_fingerprint, register_batch_task, register_external_aggregate_task,
    remove_task_children, resolve_engine, zk_any_not_drawn_response,
};

pub(crate) async fn request_batch_shasta_proof(
    state: State<AppState>,
    req: Result<Json<BatchShastaRequest>, JsonRejection>,
) -> Response {
    match request_batch_shasta_proof_inner(state, req).await {
        Ok(response) => response,
        Err(err) => legacy_api_error_response(err),
    }
}

async fn request_batch_shasta_proof_inner(
    State(state): State<AppState>,
    req: Result<Json<BatchShastaRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let Json(req) = req.map_err(|err| ApiError::bad_request(err.to_string()))?;
    let requested_proof_type = req.proof_type;
    let requested_aggregate = req.aggregate;
    let requested_network = req.network.clone();
    let requested_l1_network = req.l1_network.clone();
    let proposal_ids = req
        .proposals
        .iter()
        .map(|proposal| proposal.proposal_id)
        .collect::<Vec<_>>();
    let not_drawn_batch_id = req.proposals.first().map(|proposal| proposal.proposal_id);
    let Some(mut submission) = build_canonical_batch_submission(&state, req)? else {
        info!(
            proof_type = requested_proof_type.as_str(),
            aggregate = requested_aggregate,
            network = requested_network.as_deref().unwrap_or("default"),
            l1_network = requested_l1_network.as_deref().unwrap_or("default"),
            proposal_count = proposal_ids.len(),
            "received hoodi shasta batch request not drawn"
        );
        debug!(
            proposal_ids = ?proposal_ids,
            "received hoodi shasta batch request not drawn proposal ids"
        );
        return Ok(zk_any_not_drawn_response(not_drawn_batch_id));
    };
    let request_fingerprint = batch_request_fingerprint(
        state.runtime.environment(),
        state.runtime.namespace(),
        &submission,
    )?;
    submission.public_task_id = public_task_id_from_fingerprint(&request_fingerprint);
    resolve_engine(
        &state,
        &submission.pair.key,
        submission.route.pipeline_key(),
    )?;
    let plan =
        build_submission_plan(state.runtime.as_ref(), &submission, &request_fingerprint).await?;

    info!(
        task_id = submission.public_task_id.as_str(),
        proof_type = requested_proof_type.as_str(),
        selected_proof_type = %submission.route.proof_type(),
        prover_type = prover_type_label(submission.prover_type),
        aggregate = submission.aggregate_requested,
        pair = submission.pair.key.as_str(),
        route = %submission.route.route,
        proposal_count = proposal_ids.len(),
        "received hoodi shasta batch request"
    );
    debug!(
        task_id = submission.public_task_id.as_str(),
        proposal_ids = ?proposal_ids,
        "received hoodi shasta batch request proposal ids"
    );
    let _lifecycle_operation = state
        .runtime
        .acquire_lifecycle_operation()
        .await
        .map_err(|error| ApiError::internal(format!("runtime lifecycle unavailable: {error}")))?;
    match register_batch_task(&state, &submission, &plan, &request_fingerprint).await? {
        TaskRegistrationOutcome::Existing(existing) => {
            handle_existing_batch_task(&state, &submission, existing, None).await
        }
        TaskRegistrationOutcome::Created(_) => {
            handle_created_batch_task(&state, &submission, &plan).await
        }
    }
}

pub(crate) async fn request_aggregation_proof(
    state: State<AppState>,
    req: Result<Json<AggregateProofRequest>, JsonRejection>,
) -> Response {
    match request_aggregation_proof_inner(state, req).await {
        Ok(response) => response,
        Err(err) => legacy_api_error_response(err),
    }
}

async fn request_aggregation_proof_inner(
    State(state): State<AppState>,
    req: Result<Json<AggregateProofRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let Json(req) = req.map_err(|err| ApiError::bad_request(err.to_string()))?;
    let requested_proof_type = req.proof_type;
    let proof_count = req.proofs.len();
    let aggregation_ids = req.aggregation_ids.clone();
    let submission = build_external_aggregate_submission(&state, req).await?;
    let engine = resolve_engine(
        &state,
        &submission.pair.key,
        submission.route.pipeline_key(),
    )?;
    let aggregate = planned_external_aggregate_task(&state.runtime, &submission).await?;
    let _lifecycle_operation = state
        .runtime
        .acquire_lifecycle_operation()
        .await
        .map_err(|error| ApiError::internal(format!("runtime lifecycle unavailable: {error}")))?;

    info!(
        task_id = submission.public_task_id.as_str(),
        proof_type = requested_proof_type.as_str(),
        selected_proof_type = %submission.route.proof_type(),
        prover_type = prover_type_label(submission.prover_type),
        pair = submission.pair.key.as_str(),
        route = %submission.route.route,
        proofs = proof_count,
        aggregation_ids = ?aggregation_ids,
        "received hoodi aggregate request"
    );

    match register_external_aggregate_task(&state, &submission, &aggregate).await? {
        TaskRegistrationOutcome::Existing(existing) => {
            handle_existing_external_aggregate_task(&state, &engine, &submission, existing).await
        }
        TaskRegistrationOutcome::Created(_) => {
            handle_created_external_aggregate_task(&state, &engine, &submission, &aggregate).await
        }
    }
}

pub(crate) async fn get_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ApiOk<TaskData>>, ApiError> {
    let data = load_task_data(&state, &id).await?;
    let lookup = load_task_lookup(&state, &id).await?;

    Ok(Json(ApiOk {
        status: "ok",
        proof_type: lookup.metadata.proof_type.to_string(),
        data,
    }))
}

pub(crate) async fn cancel_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<ApiOk<TaskData>>, ApiError> {
    authorize_acl_feature_with_rate_limit(&state, &headers, ServerAclFeature::Admin)?;
    let _lifecycle_operation = state
        .runtime
        .acquire_lifecycle_operation()
        .await
        .map_err(|error| ApiError::internal(format!("runtime lifecycle unavailable: {error}")))?;

    let TaskLookup {
        record,
        metadata,
        engine,
    } = load_task_lookup(&state, &id).await?;

    if matches!(
        record.runner_status,
        RuntimeRunnerStatus::Completed
            | RuntimeRunnerStatus::Failed
            | RuntimeRunnerStatus::Cancelled
    ) {
        return get_task(State(state), Path(id)).await;
    }

    cancel_registered_tasks(&state.runtime, &engine, &id, record.pipeline_key, &metadata)
        .await
        .map_err(|err| ApiError::internal(err.to_string()))?;

    state
        .runtime
        .sync_status(&id, RuntimeRunnerStatus::Cancelled, None, None)
        .await
        .map_err(|err| ApiError::internal(format!("failed to sync runtime cancellation: {err}")))?;

    get_task(State(state), Path(id)).await
}

pub(crate) async fn get_prover_status(
    State(state): State<AppState>,
) -> Result<Json<ApiData<ProverStatus>>, ApiError> {
    let (tasks, network, skipped) = collect_prover_status(&state, ProverTaskScope::ZkAny).await?;
    Ok(Json(ApiData {
        status: "ok",
        data: ProverStatus {
            clean: tasks.is_clean() && network.is_clean() && skipped.is_clean(),
            tasks,
            network,
            skipped,
        },
    }))
}

pub(crate) async fn clear_prover(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ClearProverStatus>, ApiError> {
    authorize_acl_feature_with_rate_limit(&state, &headers, ServerAclFeature::ProverClear)?;
    Ok(Json(
        clear_prover_tasks(&state, ProverTaskScope::ZkAny).await?,
    ))
}

pub(crate) async fn report_proofs(
    State(state): State<AppState>,
) -> Result<Json<Vec<TaskData>>, ApiError> {
    let tasks = load_all_task_data(&state).await?;
    Ok(Json(tasks))
}

pub(crate) async fn list_proofs(
    State(state): State<AppState>,
) -> Result<Json<Vec<TaskData>>, ApiError> {
    let tasks = load_all_task_data(&state)
        .await?
        .into_iter()
        .filter(|task| matches!(task.status, ProofStatus::Completed) && task.proof.is_some())
        .collect::<Vec<_>>();
    Ok(Json(tasks))
}

pub(crate) async fn prune_proofs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<PruneStatus>, ApiError> {
    authorize_acl_feature_with_rate_limit(&state, &headers, ServerAclFeature::Admin)?;
    let _lifecycle_operation = state
        .runtime
        .acquire_lifecycle_operation()
        .await
        .map_err(|error| ApiError::internal(format!("runtime lifecycle unavailable: {error}")))?;

    let records = state
        .runtime
        .list_tasks()
        .await
        .map_err(|err| ApiError::internal(format!("failed to list runtime tasks: {err}")))?;
    let mut removed_engine_task_ids = HashSet::new();

    for record in records {
        let metadata: TaskMetadata = serde_json::from_value(record.metadata.clone())
            .map_err(|err| ApiError::internal(format!("failed to parse task metadata: {err}")))?;
        let engine = resolve_engine(&state, &metadata.network_pair, record.pipeline_key)?;

        remove_task_children(
            &engine,
            record.pipeline_key,
            &metadata,
            &mut removed_engine_task_ids,
        )
        .await
        .map_err(|err| ApiError::internal(err.to_string()))?;

        clear_task_publication_outboxes(&state.runtime, &record, &metadata).await?;

        state
            .runtime
            .remove_task_if_incarnation(&record.task_id, record.incarnation_id)
            .await
            .map_err(|err| {
                ApiError::internal(format!("failed to prune task {}: {err}", record.task_id))
            })?;
    }

    Ok(Json(PruneStatus { status: "ok" }))
}
