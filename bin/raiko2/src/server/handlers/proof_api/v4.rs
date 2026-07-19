use axum::{
    Json,
    body::Body,
    extract::{
        FromRequest, Path, Query, State, rejection::JsonRejection, rejection::QueryRejection,
    },
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
};
use raiko2_pipeline::{PipelineKey, PipelineRoute};
use raiko2_runtime::{RuntimeManager, RuntimeMutationOutcome};
use std::{collections::HashSet, sync::Arc};

use super::super::proof_types::v4 as wire;
use super::{
    ApiData, ApiError, ApiOk, AppState, BatchProofType, BatchShastaRequest,
    CanonicalBatchSubmission, ClearProverStatus, EngineHandle, ProofStatus, ProverStatus,
    ProverTaskScope, PublicProverArgs, ServerAclFeature, ShastaProposal, TaskData, TaskMetadata,
    authorize_acl_feature_with_rate_limit, authorize_optional_acl_feature_with_rate_limit,
    build_canonical_batch_submission, build_submission_plan, clear_prover_tasks,
    collect_prover_status, handle_created_batch_task, handle_existing_batch_task,
    is_terminal_runtime_status, load_task_data, operation_status, parse_task_metadata,
    proposal_proof_artifact_refs, register_batch_task, replace_existing_batch_task, resolve_engine,
    root_proof_artifact_refs,
};
use crate::server::request_identity::{FingerprintSink, RequestFingerprint, RequestIdentity};

// Bound client-supplied inclusive ranges before materializing them into Vecs.
const MAX_RANGE_LEN: u64 = 100_000;
const MAX_PROPOSALS_PER_REQUEST: usize = 1_024;
const MAX_TOTAL_L2_BLOCKS_PER_REQUEST: u64 = MAX_RANGE_LEN;
const PROOF_PREFIX_SCAN_LIMIT: usize = 64 * 1024;

pub(crate) async fn request_proposal_proof(
    State(state): State<AppState>,
    headers: HeaderMap,
    req: Request<Body>,
) -> Result<Json<wire::TaskResponse<wire::ProofTaskData>>, ProofRequestError> {
    authorize_optional_acl_feature_with_rate_limit(
        &state,
        &headers,
        ServerAclFeature::ProverSubmit,
    )
    .map_err(Error::from_api_error)?;
    let Json(req) = Json::<wire::ProofRequest>::from_request(req, &state)
        .await
        .map_err(|err| Error::from_json_rejection(&err))?;
    let proof_type = req.proof_type;
    validate_proof_request_shape(&req)?;
    let (proposal_id_start, proposal_id_end) = proposal_id_range(&req.proposals)?;
    let mut submission = proposal_submission(&state, &req)?;
    let request_fingerprint = proposal_request_fingerprint(
        state.runtime.environment(),
        state.runtime.namespace(),
        &submission,
    )
    .map_err(Error::from_api_error)?;
    submission.public_task_id = request_fingerprint.public_task_id();
    let task_id = submit_submission(&state, &submission, request_fingerprint.as_str()).await?;
    let data = load_task_data(&state, &task_id)
        .await
        .map_err(Error::from_api_error)?;
    Ok(Json(wire::TaskResponse {
        status: "ok",
        proof_type: proof_type.as_str().to_string(),
        proposal_id_start,
        proposal_id_end,
        data: proof_task_data(data),
    }))
}

pub(crate) async fn get_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<ApiData<TaskData>>, Error> {
    authorize_optional_acl_feature_with_rate_limit(
        &state,
        &headers,
        ServerAclFeature::ProverSubmit,
    )
    .map_err(Error::from_api_error)?;
    let data = load_task_data(&state, &id)
        .await
        .map_err(|err| match err.status {
            StatusCode::NOT_FOUND => Error::task_not_found(err.message),
            _ => Error::from_api_error(err),
        })?;
    Ok(Json(ApiData { status: "ok", data }))
}

pub(crate) async fn get_prover_status(
    State(state): State<AppState>,
    query: Result<Query<wire::ProverStatusQuery>, QueryRejection>,
) -> Result<Json<ApiOk<ProverStatus>>, Error> {
    let Query(query) = query.map_err(|err| Error::from_query_rejection(&err))?;
    let (tasks, network, skipped) = collect_prover_status(
        &state,
        ProverTaskScope::ProofType(batch_proof_type(query.proof_type)),
    )
    .await
    .map_err(Error::from_api_error)?;
    let data = ProverStatus {
        clean: tasks.is_clean() && network.is_clean() && skipped.is_clean(),
        tasks,
        network,
        skipped,
    };
    Ok(Json(ApiOk {
        status: "ok",
        proof_type: query.proof_type.as_str().to_string(),
        data,
    }))
}

pub(crate) async fn clear_prover(
    State(state): State<AppState>,
    headers: HeaderMap,
    req: Request<Body>,
) -> Result<Json<ApiOk<wire::ClearProverData>>, Error> {
    authorize_acl_feature_with_rate_limit(&state, &headers, ServerAclFeature::ProverClear)
        .map_err(Error::from_api_error)?;
    let Json(req) = Json::<wire::ProverClearRequest>::from_request(req, &state)
        .await
        .map_err(|err| Error::from_json_rejection(&err))?;
    let ClearProverStatus {
        status,
        cancelled,
        skipped,
        failed,
    } = clear_prover_tasks(
        &state,
        ProverTaskScope::ProofType(batch_proof_type(req.proof_type)),
    )
    .await
    .map_err(Error::from_api_error)?;
    let data = wire::ClearProverData {
        cancelled,
        skipped,
        failed,
    };
    Ok(Json(ApiOk {
        status,
        proof_type: req.proof_type.as_str().to_string(),
        data,
    }))
}

pub(crate) async fn invalidate_artifacts(
    State(state): State<AppState>,
    headers: HeaderMap,
    req: Request<Body>,
) -> Result<Json<ApiOk<wire::InvalidateArtifactsData>>, Error> {
    authorize_acl_feature_with_rate_limit(&state, &headers, ServerAclFeature::ProverClear)
        .map_err(Error::from_api_error)?;
    let Json(req) = Json::<wire::InvalidateArtifactsRequest>::from_request(req, &state)
        .await
        .map_err(|err| Error::from_json_rejection(&err))?;
    validate_invalidate_artifacts_request(&req)?;

    let data = invalidate_artifacts_inner(&state, &req)
        .await
        .map_err(Error::from_api_error)?;
    let failed = data.artifacts.failed.saturating_add(data.tasks.failed);
    Ok(Json(ApiOk {
        status: operation_status(failed),
        proof_type: req.proof_type.as_str().to_string(),
        data,
    }))
}

async fn invalidate_artifacts_inner(
    state: &AppState,
    req: &wire::InvalidateArtifactsRequest,
) -> Result<wire::InvalidateArtifactsData, ApiError> {
    let mut data = wire::InvalidateArtifactsData {
        dry_run: req.dry_run,
        ..wire::InvalidateArtifactsData::default()
    };
    let pipeline_keys = pipeline_keys_for_invalidate(req.proof_type);
    let proposal_range = invalidate_proposal_range(req);
    let scope = ProverTaskScope::ProofType(batch_proof_type(req.proof_type));

    let candidate_tasks = collect_invalidation_task_candidates(
        state,
        &pipeline_keys,
        scope,
        proposal_range,
        req.proof_prefix.as_deref(),
        &mut data,
    )
    .await?;
    let mut matched_artifacts = collect_invalidation_artifacts(
        state,
        &pipeline_keys,
        proposal_range,
        &candidate_tasks.artifact_refs,
        req.proof_prefix.as_deref(),
        &mut data,
    )
    .await?;
    let matched_artifact_refs = proof_artifact_identities(&matched_artifacts);
    let (matched_tasks, protected_artifact_refs) = select_invalidation_tasks(
        candidate_tasks,
        req.proof_prefix.as_deref(),
        &matched_artifact_refs,
        &mut data,
    );
    let root_artifact_refs = matched_tasks
        .iter()
        .flat_map(|task| task.root_artifact_refs.iter().cloned())
        .collect::<HashSet<_>>();
    extend_matched_artifacts_by_refs(
        state,
        &pipeline_keys,
        &root_artifact_refs,
        &mut matched_artifacts,
        &mut data,
    )
    .await?;
    matched_artifacts.retain(|artifact| {
        !protected_artifact_refs.contains(&ProofArtifactIdentity::from_artifact(artifact))
    });

    if !req.dry_run {
        let blocked_artifact_refs = remove_invalidated_tasks(state, matched_tasks, &mut data).await;
        if !blocked_artifact_refs.is_empty() {
            matched_artifacts.retain(|artifact| {
                !blocked_artifact_refs.contains(&ProofArtifactIdentity::from_artifact(artifact))
            });
        }
        remove_invalidated_artifacts(state, matched_artifacts, &mut data).await;
    }

    Ok(data)
}

struct CandidateInvalidationTasks {
    records: Vec<CandidateInvalidationTask>,
    artifact_refs: HashSet<ProofArtifactIdentity>,
    protected_artifact_refs: HashSet<ProofArtifactIdentity>,
}

struct CandidateInvalidationTask {
    record: raiko2_runtime::RuntimeTaskRecord,
    metadata: TaskMetadata,
    artifact_refs: HashSet<ProofArtifactIdentity>,
    root_artifact_refs: HashSet<ProofArtifactIdentity>,
    root_proof_matches_prefix: bool,
}

struct MatchedInvalidationTask {
    record: raiko2_runtime::RuntimeTaskRecord,
    metadata: TaskMetadata,
    artifact_refs: HashSet<ProofArtifactIdentity>,
    root_artifact_refs: HashSet<ProofArtifactIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProofArtifactIdentity {
    network_pair: String,
    pipeline_key: PipelineKey,
    route: PipelineRoute,
    proof_ref: String,
}

impl ProofArtifactIdentity {
    fn from_artifact(artifact: &raiko2_runtime::ProofArtifactRecord) -> Self {
        Self {
            network_pair: artifact.network_pair.clone(),
            pipeline_key: artifact.pipeline_key,
            route: artifact.route,
            proof_ref: artifact.proof_ref.clone(),
        }
    }
}

struct InvalidationTaskLogContext {
    task_kind: &'static str,
    aggregate: bool,
    proposal_ids: String,
    proposal_count: usize,
}

fn invalidation_task_log_context(metadata: &TaskMetadata) -> InvalidationTaskLogContext {
    let aggregate = metadata.aggregate_request.is_some() || metadata.aggregate_task_id.is_some();
    InvalidationTaskLogContext {
        task_kind: if aggregate {
            "aggregate"
        } else if metadata.proposals.len() == 1 {
            "proposal"
        } else {
            "proposal_batch"
        },
        aggregate,
        proposal_ids: format_invalidation_proposal_ids(metadata),
        proposal_count: metadata.proposals.len(),
    }
}

fn format_invalidation_proposal_ids(metadata: &TaskMetadata) -> String {
    let ids = metadata
        .proposals
        .iter()
        .map(|proposal| proposal.proposal_id)
        .collect::<Vec<_>>();
    match ids.as_slice() {
        [] => "none".to_string(),
        [id] => id.to_string(),
        [first, .., last] if ids.windows(2).all(|window| window[1] == window[0] + 1) => {
            format!("{first}..{last}")
        }
        _ => ids.iter().map(u64::to_string).collect::<Vec<_>>().join(","),
    }
}

async fn collect_invalidation_task_candidates(
    state: &AppState,
    pipeline_keys: &[PipelineKey],
    scope: ProverTaskScope,
    proposal_range: Option<(u64, u64)>,
    proof_prefix: Option<&str>,
    data: &mut wire::InvalidateArtifactsData,
) -> Result<CandidateInvalidationTasks, ApiError> {
    let tasks = state
        .runtime
        .list_tasks()
        .await
        .map_err(|err| ApiError::internal(format!("failed to list runtime tasks: {err}")))?;
    let mut candidates = CandidateInvalidationTasks {
        records: Vec::new(),
        artifact_refs: HashSet::new(),
        protected_artifact_refs: HashSet::new(),
    };
    for record in tasks {
        if !pipeline_keys.contains(&record.pipeline_key) {
            continue;
        }
        let metadata = match parse_task_metadata(&record) {
            Ok(metadata) => metadata,
            Err(err) => {
                data.tasks.invalid_metadata = data.tasks.invalid_metadata.saturating_add(1);
                tracing::warn!(
                    task_id = %record.task_id,
                    error = %err.message,
                    "skipping artifact invalidation record with invalid metadata"
                );
                continue;
            }
        };
        let root_artifact_refs = root_invalidation_artifact_refs(
            &record.network_pair,
            &metadata,
            record.pipeline_key,
            record.route,
        );
        let artifact_refs = invalidation_artifact_refs(
            &record.network_pair,
            &metadata,
            record.pipeline_key,
            record.route,
            proposal_range,
        );
        if !scope.matches(&metadata) || !metadata_matches_proposal_range(&metadata, proposal_range)
        {
            candidates.protected_artifact_refs.extend(artifact_refs);
            continue;
        }
        if !is_terminal_runtime_status(record.runner_status) {
            data.tasks.skipped_non_terminal = data.tasks.skipped_non_terminal.saturating_add(1);
            candidates.protected_artifact_refs.extend(artifact_refs);
            continue;
        }
        candidates
            .artifact_refs
            .extend(artifact_refs.iter().cloned());
        let root_proof_matches_prefix =
            task_matches_proof_prefix(state.runtime.as_ref(), &record, &metadata, proof_prefix)
                .await?;
        candidates.records.push(CandidateInvalidationTask {
            record,
            metadata,
            artifact_refs,
            root_artifact_refs,
            root_proof_matches_prefix,
        });
    }
    Ok(candidates)
}

fn select_invalidation_tasks(
    candidates: CandidateInvalidationTasks,
    proof_prefix: Option<&str>,
    matched_artifact_refs: &HashSet<ProofArtifactIdentity>,
    data: &mut wire::InvalidateArtifactsData,
) -> (Vec<MatchedInvalidationTask>, HashSet<ProofArtifactIdentity>) {
    let mut matched = Vec::new();
    let mut protected_artifact_refs = candidates.protected_artifact_refs;
    for task in candidates.records {
        let task_matches = if proof_prefix.is_none() {
            true
        } else {
            task.root_proof_matches_prefix
                || task
                    .artifact_refs
                    .iter()
                    .any(|artifact_ref| matched_artifact_refs.contains(artifact_ref))
        };
        if task_matches {
            data.tasks.matched = data.tasks.matched.saturating_add(1);
            matched.push(MatchedInvalidationTask {
                record: task.record,
                metadata: task.metadata,
                artifact_refs: task.artifact_refs,
                root_artifact_refs: task.root_artifact_refs,
            });
        } else {
            protected_artifact_refs.extend(task.artifact_refs.iter().cloned());
        }
    }
    (matched, protected_artifact_refs)
}

fn proof_artifact_identities(
    artifacts: &[raiko2_runtime::ProofArtifactRecord],
) -> HashSet<ProofArtifactIdentity> {
    artifacts
        .iter()
        .map(ProofArtifactIdentity::from_artifact)
        .collect()
}

async fn extend_matched_artifacts_by_refs(
    state: &AppState,
    pipeline_keys: &[PipelineKey],
    refs: &HashSet<ProofArtifactIdentity>,
    matched_artifacts: &mut Vec<raiko2_runtime::ProofArtifactRecord>,
    data: &mut wire::InvalidateArtifactsData,
) -> Result<(), ApiError> {
    if refs.is_empty() {
        return Ok(());
    }
    let mut seen_artifacts = proof_artifact_identities(matched_artifacts);
    let artifacts = state
        .runtime
        .list_proof_artifacts()
        .await
        .map_err(|err| ApiError::internal(format!("failed to list proof artifacts: {err}")))?;
    for artifact in artifacts {
        if !pipeline_keys.contains(&artifact.pipeline_key) {
            continue;
        }
        let identity = ProofArtifactIdentity {
            network_pair: artifact.network_pair.clone(),
            pipeline_key: artifact.pipeline_key,
            route: artifact.route,
            proof_ref: artifact.proof_ref.clone(),
        };
        if refs.contains(&identity) && seen_artifacts.insert(identity) {
            data.artifacts.matched = data.artifacts.matched.saturating_add(1);
            matched_artifacts.push(artifact);
        }
    }
    Ok(())
}

fn root_invalidation_artifact_refs(
    network_pair: &str,
    metadata: &TaskMetadata,
    pipeline_key: PipelineKey,
    route: PipelineRoute,
) -> HashSet<ProofArtifactIdentity> {
    root_proof_artifact_refs(metadata, pipeline_key)
        .map(|root_refs| {
            root_refs
                .refs
                .into_iter()
                .map(|proof_ref| ProofArtifactIdentity {
                    network_pair: network_pair.to_string(),
                    pipeline_key,
                    route,
                    proof_ref,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn invalidation_artifact_refs(
    network_pair: &str,
    metadata: &TaskMetadata,
    pipeline_key: PipelineKey,
    route: PipelineRoute,
    proposal_range: Option<(u64, u64)>,
) -> HashSet<ProofArtifactIdentity> {
    let mut refs = root_invalidation_artifact_refs(network_pair, metadata, pipeline_key, route);
    refs.extend(
        metadata
            .aggregate_input_artifacts
            .iter()
            .enumerate()
            .filter_map(|(index, artifact)| {
                let proposal_matches = match proposal_range {
                    None => true,
                    Some(_) => metadata
                        .aggregate_request
                        .as_ref()
                        .and_then(|request| request.proposal_ids.get(index))
                        .is_some_and(|proposal_id| {
                            proposal_matches_range(*proposal_id, proposal_range)
                        }),
                };
                proposal_matches.then(|| ProofArtifactIdentity {
                    network_pair: network_pair.to_string(),
                    pipeline_key,
                    route,
                    proof_ref: artifact.proof_ref.clone(),
                })
            }),
    );
    for proposal in &metadata.proposals {
        if !proposal_matches_range(proposal.proposal_id, proposal_range) {
            continue;
        }
        refs.extend(
            proposal_proof_artifact_refs(pipeline_key, proposal)
                .into_iter()
                .map(|proof_ref| ProofArtifactIdentity {
                    network_pair: network_pair.to_string(),
                    pipeline_key,
                    route,
                    proof_ref,
                }),
        );
    }
    refs
}

fn proposal_matches_range(proposal_id: u64, range: Option<(u64, u64)>) -> bool {
    match range {
        Some((start, end)) => (start..=end).contains(&proposal_id),
        None => true,
    }
}

async fn collect_invalidation_artifacts(
    state: &AppState,
    pipeline_keys: &[PipelineKey],
    proposal_range: Option<(u64, u64)>,
    matched_task_artifact_refs: &HashSet<ProofArtifactIdentity>,
    proof_prefix: Option<&str>,
    data: &mut wire::InvalidateArtifactsData,
) -> Result<Vec<raiko2_runtime::ProofArtifactRecord>, ApiError> {
    let artifacts = state
        .runtime
        .list_proof_artifacts()
        .await
        .map_err(|err| ApiError::internal(format!("failed to list proof artifacts: {err}")))?;
    let mut matched_artifacts = Vec::new();
    let mut seen_artifacts = HashSet::new();
    for artifact in artifacts {
        if !pipeline_keys.contains(&artifact.pipeline_key) {
            continue;
        }
        if !artifact_matches_proposal_scope(&artifact, proposal_range, matched_task_artifact_refs) {
            continue;
        }
        if !artifact_matches_proof_prefix(state.runtime.as_ref(), &artifact, proof_prefix).await? {
            continue;
        }
        if seen_artifacts.insert(ProofArtifactIdentity::from_artifact(&artifact)) {
            data.artifacts.matched = data.artifacts.matched.saturating_add(1);
            matched_artifacts.push(artifact);
        }
    }
    Ok(matched_artifacts)
}

#[allow(clippy::too_many_lines)]
async fn remove_invalidated_tasks(
    state: &AppState,
    matched_tasks: Vec<MatchedInvalidationTask>,
    data: &mut wire::InvalidateArtifactsData,
) -> HashSet<ProofArtifactIdentity> {
    let mut blocked_artifact_refs = HashSet::new();
    for task in matched_tasks {
        let MatchedInvalidationTask {
            record,
            metadata,
            artifact_refs,
            root_artifact_refs,
        } = task;
        let log_context = invalidation_task_log_context(&metadata);
        let mut cleanup_failed = false;
        match state
            .lifecycle
            .remove(&record, raiko2_queue::DetachMode::Remove)
            .await
        {
            Ok((RuntimeMutationOutcome::Applied, _)) => {}
            Ok((outcome, _)) => {
                data.tasks.failed = data.tasks.failed.saturating_add(1);
                tracing::warn!(
                    task_id = %record.task_id,
                    task_kind = log_context.task_kind,
                    aggregate = log_context.aggregate,
                    proposal_ids = %log_context.proposal_ids,
                    proposal_count = log_context.proposal_count,
                    network_pair = %record.network_pair,
                    proof_type = %metadata.proof_type,
                    pipeline_key = %record.pipeline_key.as_str(),
                    ?outcome,
                    "invalidated task changed before conditional retirement"
                );
                cleanup_failed = true;
            }
            Err(err) => {
                data.tasks.failed = data.tasks.failed.saturating_add(1);
                tracing::warn!(
                    task_id = %record.task_id,
                    task_kind = log_context.task_kind,
                    aggregate = log_context.aggregate,
                    proposal_ids = %log_context.proposal_ids,
                    proposal_count = log_context.proposal_count,
                    network_pair = %record.network_pair,
                    proof_type = %metadata.proof_type,
                    pipeline_key = %record.pipeline_key.as_str(),
                    error = %err,
                    "failed to retire invalidated task"
                );
                cleanup_failed = true;
            }
        }
        if cleanup_failed {
            blocked_artifact_refs.extend(artifact_refs);
            blocked_artifact_refs.extend(root_artifact_refs);
            continue;
        }

        data.tasks.removed = data.tasks.removed.saturating_add(1);
    }
    blocked_artifact_refs
}

async fn remove_invalidated_artifacts(
    state: &AppState,
    matched_artifacts: Vec<raiko2_runtime::ProofArtifactRecord>,
    data: &mut wire::InvalidateArtifactsData,
) {
    for artifact in matched_artifacts {
        let identity = ProofArtifactIdentity {
            network_pair: artifact.network_pair.clone(),
            pipeline_key: artifact.pipeline_key,
            route: artifact.route,
            proof_ref: artifact.proof_ref.clone(),
        };
        let blocked_artifact_refs = recheck_artifact_consumers(state, &identity, data).await;
        if blocked_artifact_refs.contains(&identity) {
            continue;
        }

        let delete_result = match state
            .runtime
            .invalidate_proof_artifact_descriptor_if_unowned(
                &artifact.network_pair,
                artifact.pipeline_key,
                artifact.route,
                &artifact.proof_ref,
                &artifact.descriptor(),
            )
            .await
        {
            Ok(raiko2_runtime::ProofArtifactInvalidationResult::Invalidated(result)) => result,
            Ok(raiko2_runtime::ProofArtifactInvalidationResult::BlockedByLiveTask) => {
                data.tasks.skipped_non_terminal = data.tasks.skipped_non_terminal.saturating_add(1);
                continue;
            }
            Ok(raiko2_runtime::ProofArtifactInvalidationResult::MissingOrChanged) => continue,
            Err(err) => {
                data.artifacts.failed = data.artifacts.failed.saturating_add(1);
                tracing::warn!(
                    network_pair = %artifact.network_pair,
                    proof_ref = %artifact.proof_ref,
                    error = %err,
                    "failed to invalidate proof artifact"
                );
                continue;
            }
        };

        data.artifacts.removed = data.artifacts.removed.saturating_add(1);
        match delete_result {
            raiko2_runtime::ProofArtifactDeleteResult::Removed => {
                data.artifacts.manifests_removed =
                    data.artifacts.manifests_removed.saturating_add(1);
            }
            raiko2_runtime::ProofArtifactDeleteResult::Missing => {
                data.artifacts.manifests_missing =
                    data.artifacts.manifests_missing.saturating_add(1);
            }
        }
    }
}

async fn recheck_artifact_consumers(
    state: &AppState,
    artifact: &ProofArtifactIdentity,
    data: &mut wire::InvalidateArtifactsData,
) -> HashSet<ProofArtifactIdentity> {
    let records = state.runtime.tasks_referencing(&artifact.proof_ref).await;
    let mut matched_tasks = Vec::new();
    let mut blocked_artifact_refs = HashSet::new();
    for record in records {
        if record.pipeline_key != artifact.pipeline_key || record.route != artifact.route {
            continue;
        }
        let metadata = match parse_task_metadata(&record) {
            Ok(metadata) => metadata,
            Err(err) => {
                data.tasks.invalid_metadata = data.tasks.invalid_metadata.saturating_add(1);
                tracing::warn!(
                    task_id = %record.task_id,
                    error = %err.message,
                    "skipping late invalidation task with invalid metadata"
                );
                return HashSet::from([artifact.clone()]);
            }
        };
        if record.network_pair != artifact.network_pair {
            continue;
        }
        let root_artifact_refs = root_invalidation_artifact_refs(
            &record.network_pair,
            &metadata,
            record.pipeline_key,
            record.route,
        );
        let artifact_refs = invalidation_artifact_refs(
            &record.network_pair,
            &metadata,
            record.pipeline_key,
            record.route,
            None,
        );
        if !artifact_refs.contains(artifact) {
            continue;
        }
        if !is_terminal_runtime_status(record.runner_status) {
            data.tasks.skipped_non_terminal = data.tasks.skipped_non_terminal.saturating_add(1);
            blocked_artifact_refs.insert(artifact.clone());
            continue;
        }
        data.tasks.matched = data.tasks.matched.saturating_add(1);
        matched_tasks.push(MatchedInvalidationTask {
            record,
            metadata,
            artifact_refs,
            root_artifact_refs,
        });
    }
    blocked_artifact_refs.extend(remove_invalidated_tasks(state, matched_tasks, data).await);
    blocked_artifact_refs
}

fn validate_invalidate_artifacts_request(
    req: &wire::InvalidateArtifactsRequest,
) -> Result<(), Error> {
    if let Some(prefix) = req.proof_prefix.as_deref() {
        if !prefix.starts_with("0x") {
            return Err(Error::invalid_request("proof_prefix must start with 0x"));
        }
        if prefix.len() <= 2 {
            return Err(Error::invalid_request("proof_prefix must not be empty"));
        }
        if prefix.len() > 130 {
            return Err(Error::invalid_request("proof_prefix is too long"));
        }
        if !prefix[2..].bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(Error::invalid_request(
                "proof_prefix must contain hex characters",
            ));
        }
    }
    match (req.proposal_id_start, req.proposal_id_end) {
        (Some(start), Some(end)) if start > end => Err(Error::invalid_request(
            "proposal_id_start must be <= proposal_id_end",
        )),
        (Some(_), None) | (None, Some(_)) => Err(Error::invalid_request(
            "proposal_id_start and proposal_id_end must be provided together",
        )),
        _ => Ok(()),
    }
}

fn invalidate_proposal_range(req: &wire::InvalidateArtifactsRequest) -> Option<(u64, u64)> {
    req.proposal_id_start.zip(req.proposal_id_end)
}

fn pipeline_keys_for_invalidate(proof_type: wire::ProofType) -> Vec<PipelineKey> {
    match proof_type {
        wire::ProofType::Risc0 => vec![PipelineKey::ShastaRisc0, PipelineKey::ShastaRisc0Network],
        wire::ProofType::Sp1 => vec![PipelineKey::ShastaSp1],
        wire::ProofType::Sgx => vec![PipelineKey::ShastaSgx],
        wire::ProofType::SgxGeth => vec![PipelineKey::ShastaSgxGeth],
    }
}

fn metadata_matches_proposal_range(metadata: &TaskMetadata, range: Option<(u64, u64)>) -> bool {
    let Some((start, end)) = range else {
        return true;
    };
    metadata
        .proposals
        .iter()
        .any(|proposal| (start..=end).contains(&proposal.proposal_id))
        || metadata.aggregate_request.as_ref().is_some_and(|request| {
            request
                .proposal_ids
                .iter()
                .any(|proposal_id| (start..=end).contains(proposal_id))
        })
}

async fn task_matches_proof_prefix(
    runtime: &RuntimeManager,
    record: &raiko2_runtime::RuntimeTaskRecord,
    metadata: &TaskMetadata,
    proof_prefix: Option<&str>,
) -> Result<bool, ApiError> {
    let Some(prefix) = proof_prefix else {
        return Ok(true);
    };
    let Some(root_refs) = root_proof_artifact_refs(metadata, record.pipeline_key) else {
        return Ok(false);
    };
    for proof_ref in root_refs.refs {
        if proof_artifact_starts_with(
            runtime,
            &record.network_pair,
            record.pipeline_key,
            record.route,
            &proof_ref,
            prefix,
        )
        .await?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn artifact_matches_proposal_scope(
    artifact: &raiko2_runtime::ProofArtifactRecord,
    proposal_range: Option<(u64, u64)>,
    matched_task_artifact_refs: &HashSet<ProofArtifactIdentity>,
) -> bool {
    proposal_range.is_none()
        || matched_task_artifact_refs.contains(&ProofArtifactIdentity {
            network_pair: artifact.network_pair.clone(),
            pipeline_key: artifact.pipeline_key,
            route: artifact.route,
            proof_ref: artifact.proof_ref.clone(),
        })
}

async fn artifact_matches_proof_prefix(
    runtime: &RuntimeManager,
    artifact: &raiko2_runtime::ProofArtifactRecord,
    proof_prefix: Option<&str>,
) -> Result<bool, ApiError> {
    match proof_prefix {
        Some(prefix) => {
            proof_artifact_starts_with(
                runtime,
                &artifact.network_pair,
                artifact.pipeline_key,
                artifact.route,
                &artifact.proof_ref,
                prefix,
            )
            .await
        }
        None => Ok(true),
    }
}

async fn proof_artifact_starts_with(
    runtime: &RuntimeManager,
    network_pair: &str,
    pipeline_key: PipelineKey,
    route: PipelineRoute,
    proof_ref: &str,
    prefix: &str,
) -> Result<bool, ApiError> {
    let object = runtime
        .read_proof_artifact_prefix(
            network_pair,
            pipeline_key,
            route,
            proof_ref,
            PROOF_PREFIX_SCAN_LIMIT,
        )
        .await
        .map_err(|err| {
            ApiError::internal(format!(
                "failed to read proof artifact {proof_ref} for prefix match: {err}"
            ))
        })?;
    let Some(object) = object else {
        return Ok(false);
    };
    proof_json_prefix_starts_with(&object.bytes, prefix).map_err(|err| {
        ApiError::internal(format!(
            "failed to inspect proof artifact {proof_ref} prefix within {PROOF_PREFIX_SCAN_LIMIT} bytes: {err}"
        ))
    })
}

fn proof_json_prefix_starts_with(bytes: &[u8], prefix: &str) -> Result<bool, &'static str> {
    let mut pos = 0;
    skip_json_ws(bytes, &mut pos);
    if bytes.get(pos) != Some(&b'{') {
        return Err("proof artifact is not a JSON object");
    }
    pos += 1;

    loop {
        skip_json_ws(bytes, &mut pos);
        match bytes.get(pos) {
            Some(b'}') => return Ok(false),
            Some(b'"') => {}
            Some(_) => return Err("expected JSON object key"),
            None => return Err("proof field not found in prefix scan window"),
        }
        let key = parse_json_string(bytes, &mut pos)?;
        skip_json_ws(bytes, &mut pos);
        if bytes.get(pos) != Some(&b':') {
            return Err("expected JSON object colon");
        }
        pos += 1;

        if key == b"proof" {
            return proof_json_string_starts_with(bytes, &mut pos, prefix);
        }
        skip_json_value(bytes, &mut pos)?;
        skip_json_ws(bytes, &mut pos);
        match bytes.get(pos) {
            Some(b',') => pos += 1,
            Some(b'}') => return Ok(false),
            Some(_) => return Err("expected JSON object separator"),
            None => return Err("proof field not found in prefix scan window"),
        }
    }
}

fn proof_json_string_starts_with(
    bytes: &[u8],
    pos: &mut usize,
    prefix: &str,
) -> Result<bool, &'static str> {
    skip_json_ws(bytes, pos);
    if bytes.get(*pos..(*pos + 4)) == Some(b"null") {
        *pos += 4;
        return Ok(false);
    }
    if bytes.get(*pos) != Some(&b'"') {
        return Err("proof field is not a JSON string or null");
    }
    *pos += 1;
    for expected in prefix.as_bytes() {
        let Some(actual) = bytes.get(*pos).copied() else {
            return Err("proof prefix exceeds scan window");
        };
        if actual == b'"' {
            return Ok(false);
        }
        if actual == b'\\' {
            return Err("proof prefix contains an escaped byte");
        }
        if !actual.eq_ignore_ascii_case(expected) {
            return Ok(false);
        }
        *pos += 1;
    }
    Ok(true)
}

fn skip_json_ws(bytes: &[u8], pos: &mut usize) {
    while bytes
        .get(*pos)
        .is_some_and(|byte| matches!(byte, b' ' | b'\n' | b'\r' | b'\t'))
    {
        *pos += 1;
    }
}

fn parse_json_string(bytes: &[u8], pos: &mut usize) -> Result<Vec<u8>, &'static str> {
    if bytes.get(*pos) != Some(&b'"') {
        return Err("expected JSON string");
    }
    *pos += 1;
    let mut out = Vec::new();
    loop {
        let Some(byte) = bytes.get(*pos).copied() else {
            return Err("unterminated JSON string in prefix scan window");
        };
        *pos += 1;
        match byte {
            b'"' => return Ok(out),
            b'\\' => {
                out.push(consume_json_escape(bytes, pos)?.unwrap_or(b'?'));
            }
            0..=0x1f => return Err("invalid control byte in JSON string"),
            _ => out.push(byte),
        }
    }
}

fn skip_json_string(bytes: &[u8], pos: &mut usize) -> Result<(), &'static str> {
    if bytes.get(*pos) != Some(&b'"') {
        return Err("expected JSON string");
    }
    *pos += 1;
    loop {
        let Some(byte) = bytes.get(*pos).copied() else {
            return Err("unterminated JSON string in prefix scan window");
        };
        *pos += 1;
        match byte {
            b'"' => return Ok(()),
            b'\\' => {
                let _ = consume_json_escape(bytes, pos)?;
            }
            0..=0x1f => return Err("invalid control byte in JSON string"),
            _ => {}
        }
    }
}

fn consume_json_escape(bytes: &[u8], pos: &mut usize) -> Result<Option<u8>, &'static str> {
    let Some(escaped) = bytes.get(*pos).copied() else {
        return Err("unterminated JSON escape in prefix scan window");
    };
    *pos += 1;
    match escaped {
        b'"' | b'\\' | b'/' => Ok(Some(escaped)),
        b'b' => Ok(Some(0x08)),
        b'f' => Ok(Some(0x0c)),
        b'n' => Ok(Some(b'\n')),
        b'r' => Ok(Some(b'\r')),
        b't' => Ok(Some(b'\t')),
        b'u' => {
            if bytes.len().saturating_sub(*pos) < 4 {
                return Err("unterminated JSON unicode escape in prefix scan window");
            }
            if !bytes[*pos..(*pos + 4)].iter().all(u8::is_ascii_hexdigit) {
                return Err("invalid JSON unicode escape in prefix scan");
            }
            *pos += 4;
            Ok(None)
        }
        _ => Err("invalid JSON escape in prefix scan"),
    }
}

fn skip_json_value(bytes: &[u8], pos: &mut usize) -> Result<(), &'static str> {
    skip_json_ws(bytes, pos);
    match bytes.get(*pos).copied() {
        Some(b'"') => skip_json_string(bytes, pos),
        Some(b'{' | b'[') => skip_nested_json(bytes, pos),
        Some(b'n') if bytes.get(*pos..(*pos + 4)) == Some(b"null") => {
            *pos += 4;
            Ok(())
        }
        Some(b't') if bytes.get(*pos..(*pos + 4)) == Some(b"true") => {
            *pos += 4;
            Ok(())
        }
        Some(b'f') if bytes.get(*pos..(*pos + 5)) == Some(b"false") => {
            *pos += 5;
            Ok(())
        }
        Some(b'-' | b'0'..=b'9') => {
            while bytes.get(*pos).is_some_and(|byte| {
                !matches!(byte, b',' | b'}' | b']' | b' ' | b'\n' | b'\r' | b'\t')
            }) {
                *pos += 1;
            }
            Ok(())
        }
        Some(_) => Err("unsupported JSON value in prefix scan"),
        None => Err("missing JSON value in prefix scan window"),
    }
}

fn skip_nested_json(bytes: &[u8], pos: &mut usize) -> Result<(), &'static str> {
    let mut expected_closers = Vec::new();
    loop {
        let Some(byte) = bytes.get(*pos).copied() else {
            return Err("unterminated nested JSON value in prefix scan window");
        };
        match byte {
            b'"' => {
                skip_json_string(bytes, pos)?;
            }
            b'{' => {
                expected_closers.push(b'}');
                *pos += 1;
            }
            b'[' => {
                expected_closers.push(b']');
                *pos += 1;
            }
            b'}' | b']' => {
                let Some(expected) = expected_closers.pop() else {
                    return Err("unbalanced nested JSON value");
                };
                if byte != expected {
                    return Err("mismatched nested JSON closer");
                }
                *pos += 1;
                if expected_closers.is_empty() {
                    return Ok(());
                }
            }
            _ => *pos += 1,
        }
    }
}

// Handler-level error plumbing stays here; only the v4 wire payload lives in proof_types::v4.
#[derive(Debug)]
pub(crate) struct Error {
    pub(super) status: StatusCode,
    pub(super) code: &'static str,
    message: String,
}

impl Error {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request", message)
    }

    fn unsupported_proof_type(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "unsupported_proof_type", message)
    }

    fn request_conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "request_conflict", message)
    }

    fn task_not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "task_not_found", message)
    }

    fn from_json_rejection(err: &JsonRejection) -> Self {
        let message = err.to_string();
        if message.contains("missing field `proof_type`") {
            Self::new(
                StatusCode::BAD_REQUEST,
                "missing_proof_type",
                "missing required field `proof_type`",
            )
        } else if message.contains("unknown variant") && message.contains("proof_type") {
            Self::new(StatusCode::BAD_REQUEST, "invalid_proof_type", message)
        } else {
            Self::invalid_request(message)
        }
    }

    fn from_query_rejection(err: &QueryRejection) -> Self {
        let message = err.to_string();
        if message.contains("missing field `proof_type`") {
            Self::new(
                StatusCode::BAD_REQUEST,
                "missing_proof_type",
                "missing required query parameter `proof_type`",
            )
        } else if message.contains("unknown variant") && message.contains("proof_type") {
            Self::new(StatusCode::BAD_REQUEST, "invalid_proof_type", message)
        } else {
            Self::invalid_request(message)
        }
    }

    fn from_api_error(err: ApiError) -> Self {
        let code = match err.status {
            StatusCode::BAD_REQUEST
                if err.message.contains("proof_type=") && err.message.contains("not supported") =>
            {
                "unsupported_proof_type"
            }
            StatusCode::BAD_REQUEST => "invalid_request",
            StatusCode::NOT_FOUND if err.message == "ACL feature is not enabled" => "not_found",
            StatusCode::NOT_FOUND => "task_not_found",
            StatusCode::UNAUTHORIZED => "unauthorized",
            StatusCode::FORBIDDEN => "forbidden",
            StatusCode::CONFLICT => "request_conflict",
            StatusCode::TOO_MANY_REQUESTS => "rate_limited",
            StatusCode::SERVICE_UNAVAILABLE => "unsupported_proof_type",
            _ => "internal_error",
        };
        Self::new(err.status, code, err.message)
    }

    const fn proof_request_http_status(&self) -> StatusCode {
        match self.status {
            StatusCode::BAD_REQUEST | StatusCode::CONFLICT => StatusCode::OK,
            status => status,
        }
    }

    fn into_response_with_status(self, status: StatusCode) -> Response {
        (
            status,
            Json(wire::ApiErrorBody {
                status: "error",
                error: self.code,
                message: self.message,
            }),
        )
            .into_response()
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = self.status;
        self.into_response_with_status(status)
    }
}

#[derive(Debug)]
pub(crate) struct ProofRequestError(Error);

impl From<Error> for ProofRequestError {
    fn from(err: Error) -> Self {
        Self(err)
    }
}

impl std::ops::Deref for ProofRequestError {
    type Target = Error;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl IntoResponse for ProofRequestError {
    fn into_response(self) -> Response {
        let status = self.0.proof_request_http_status();
        self.0.into_response_with_status(status)
    }
}

const fn batch_proof_type(proof_type: wire::ProofType) -> BatchProofType {
    match proof_type {
        wire::ProofType::Risc0 => BatchProofType::Risc0,
        wire::ProofType::Sp1 => BatchProofType::Sp1,
        wire::ProofType::Sgx => BatchProofType::Sgx,
        wire::ProofType::SgxGeth => BatchProofType::SgxGeth,
    }
}

fn proposal_id_range(proposals: &[wire::ProposalRequest]) -> Result<(u64, u64), Error> {
    let Some(first) = proposals.first() else {
        return Err(Error::invalid_request("proposals must not be empty"));
    };
    let mut expected = first.proposal_id;
    for proposal in proposals {
        if proposal.proposal_id != expected {
            return Err(Error::invalid_request(
                "proposals[].proposal_id must be strictly increasing and contiguous",
            ));
        }
        expected = expected
            .checked_add(1)
            .ok_or_else(|| Error::invalid_request("proposals[].proposal_id range overflows u64"))?;
    }
    Ok((first.proposal_id, expected - 1))
}

fn validate_proof_request_shape(req: &wire::ProofRequest) -> Result<(), Error> {
    if req.proposals.is_empty() {
        return Err(Error::invalid_request("proposals must not be empty"));
    }
    if req.proposals.len() > MAX_PROPOSALS_PER_REQUEST {
        return Err(Error::invalid_request(format!(
            "proposals length {} exceeds maximum {MAX_PROPOSALS_PER_REQUEST}",
            req.proposals.len()
        )));
    }
    if !req.aggregate && req.proposals.len() != 1 {
        return Err(Error::invalid_request(
            "aggregate=false requires exactly one proposal",
        ));
    }
    let mut total_l2_blocks = 0u64;
    for proposal in &req.proposals {
        let len = inclusive_range_len(
            proposal.l2_block_number_start,
            proposal.l2_block_number_end,
            "proposals[].l2_block_number_start",
            "proposals[].l2_block_number_end",
        )?;
        validate_inclusive_range_len(
            len,
            "proposals[].l2_block_number_start",
            "proposals[].l2_block_number_end",
        )?;
        total_l2_blocks = total_l2_blocks.checked_add(len).ok_or_else(|| {
            Error::invalid_request("total proposals[].l2 block range length overflows u64")
        })?;
        if total_l2_blocks > MAX_TOTAL_L2_BLOCKS_PER_REQUEST {
            return Err(Error::invalid_request(format!(
                "total proposals[].l2 block range length {total_l2_blocks} exceeds maximum {MAX_TOTAL_L2_BLOCKS_PER_REQUEST}"
            )));
        }
    }
    Ok(())
}

fn inclusive_range_len(
    start: u64,
    end: u64,
    start_field: &'static str,
    end_field: &'static str,
) -> Result<u64, Error> {
    if end < start {
        return Err(Error::invalid_request(format!(
            "{end_field} must be greater than or equal to {start_field}"
        )));
    }

    end.checked_sub(start)
        .and_then(|delta| delta.checked_add(1))
        .ok_or_else(|| {
            Error::invalid_request(format!(
                "{start_field}..={end_field} range length overflows u64"
            ))
        })
}

fn validate_inclusive_range_len(
    len: u64,
    start_field: &'static str,
    end_field: &'static str,
) -> Result<(), Error> {
    if len > MAX_RANGE_LEN {
        return Err(Error::invalid_request(format!(
            "{start_field}..={end_field} range length {len} exceeds maximum {MAX_RANGE_LEN}"
        )));
    }
    Ok(())
}

fn collect_inclusive_range(
    start: u64,
    end: u64,
    start_field: &'static str,
    end_field: &'static str,
) -> Result<Vec<u64>, Error> {
    // V4 accepts compact inclusive ranges; internal batch paths consume explicit IDs.
    let len = inclusive_range_len(start, end, start_field, end_field)?;
    validate_inclusive_range_len(len, start_field, end_field)?;

    Ok((start..=end).collect())
}

fn proposal_submission(
    state: &AppState,
    req: &wire::ProofRequest,
) -> Result<CanonicalBatchSubmission, Error> {
    // Translate v4 proof requests into the canonical batch path so routing,
    // metadata, proposal dependencies, and aggregate requeue stay single-sourced.
    let proof_type = req.proof_type;
    let batch_req = BatchShastaRequest {
        proposals: req
            .proposals
            .iter()
            .map(|proposal| {
                let l2_block_numbers = collect_inclusive_range(
                    proposal.l2_block_number_start,
                    proposal.l2_block_number_end,
                    "proposals[].l2_block_number_start",
                    "proposals[].l2_block_number_end",
                )?;
                Ok(ShastaProposal {
                    proposal_id: proposal.proposal_id,
                    checkpoint: proposal.checkpoint,
                    l1_inclusion_block_number: proposal.l1_inclusion_block_number,
                    l2_block_numbers,
                    last_anchor_block_number: proposal.last_anchor_block_number,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?,
        proof_type: batch_proof_type(proof_type),
        aggregate: req.aggregate,
        prover: req.prover.map(|addr| addr.to_string()),
        network: None,
        l1_network: None,
        blob_proof_type: None,
        graffiti: None,
        prover_args: PublicProverArgs::default(),
    };

    let submission = build_canonical_batch_submission(state, batch_req)
        .map_err(Error::from_api_error)?
        .ok_or_else(|| Error::unsupported_proof_type("proof type was not selected"))?;
    Ok(submission)
}

struct ProposalIdentity<'a> {
    environment: &'a str,
    namespace: &'a str,
    submission: &'a CanonicalBatchSubmission,
    prover_config_json: String,
}

impl<'a> ProposalIdentity<'a> {
    fn new(
        environment: &'a str,
        namespace: &'a str,
        submission: &'a CanonicalBatchSubmission,
    ) -> Result<Self, ApiError> {
        let prover_config_json =
            serde_json::to_string(&submission.prover_config).map_err(|err| {
                ApiError::internal(format!("failed to serialize prover config identity: {err}"))
            })?;
        Ok(Self {
            environment,
            namespace,
            submission,
            prover_config_json,
        })
    }
}

impl RequestIdentity for ProposalIdentity<'_> {
    const DOMAIN: &'static str = "proof/proposal:v1";

    fn write_identity(&self, sink: &mut FingerprintSink) {
        let submission = self.submission;
        sink.str("environment", self.environment);
        sink.str("namespace", self.namespace);
        sink.str("network_pair", &submission.pair.key);
        sink.str("route", &submission.route.route.to_string());
        sink.str("proof_type", &submission.route.proof_type().to_string());
        sink.opt_str(
            "prover_type",
            submission
                .prover_type
                .map(crate::server::task_metadata::ProverType::as_str),
        );
        sink.str("prover_config", &self.prover_config_json);
        sink.str(
            "requested_proof_type",
            submission.requested_proof_type.as_str(),
        );
        sink.opt_str("prover", submission.prover.as_deref());
        sink.bool("aggregate_requested", submission.aggregate_requested);
        sink.u64("proposals.len", submission.proposals.len() as u64);
        for (index, proposal) in submission.proposals.iter().enumerate() {
            let prefix = format!("proposals[{index}]");
            sink.u64(format!("{prefix}.proposal_id"), proposal.proposal_id);
            sink.u64(
                format!("{prefix}.l1_inclusion_block_number"),
                proposal.l1_inclusion_block_number,
            );
            sink.u64(
                format!("{prefix}.l2_block_number_start"),
                proposal.l2_block_range.start,
            );
            sink.u64(
                format!("{prefix}.l2_block_number_end"),
                proposal.l2_block_range.end,
            );
            sink.u64(
                format!("{prefix}.last_anchor_block_number"),
                proposal.last_anchor_block_number,
            );
            sink.bool(
                format!("{prefix}.checkpoint.present"),
                proposal.checkpoint.is_some(),
            );
            if let Some(checkpoint) = proposal.checkpoint {
                sink.u64(
                    format!("{prefix}.checkpoint.block_number"),
                    checkpoint.block_number,
                );
                sink.b256(
                    format!("{prefix}.checkpoint.block_hash"),
                    &checkpoint.block_hash,
                );
                sink.b256(
                    format!("{prefix}.checkpoint.state_root"),
                    &checkpoint.state_root,
                );
            }
        }
    }
}

fn proposal_request_fingerprint(
    environment: &str,
    namespace: &str,
    submission: &CanonicalBatchSubmission,
) -> Result<RequestFingerprint, ApiError> {
    Ok(ProposalIdentity::new(environment, namespace, submission)?.fingerprint())
}

async fn submit_submission(
    state: &AppState,
    submission: &CanonicalBatchSubmission,
    request_fingerprint: &str,
) -> Result<String, Error> {
    // The authoritative runtime transition provides the linearization point. Queue and artifact
    // effects are exact, idempotent follow-ups rather than one long-lived lifecycle lock.
    // Deterministic v4 task IDs are reusable only when the normalized request fingerprint matches.
    if let Some(existing) = state
        .runtime
        .get_task(&submission.public_task_id)
        .await
        .map_err(|err| Error::from_api_error(ApiError::internal(err.to_string())))?
    {
        if existing.request_fingerprint != request_fingerprint {
            // New task ids are fingerprint-derived, so this branch should only be reachable for
            // stale/manual rows or an actual hash collision. Failed/cancelled rows may be replaced;
            // active/completed rows must not be silently overwritten.
            if matches!(
                existing.runner_status,
                raiko2_runtime::RunnerStatus::Failed | raiko2_runtime::RunnerStatus::Cancelled
            ) {
                // Resolve backend availability before attempting a terminal replacement so an
                // unsupported pipeline cannot mutate the authoritative slot and is reported
                // consistently as unsupported_proof_type.
                ensure_engine_available(
                    state,
                    &submission.pair.key,
                    submission.route.pipeline_key(),
                    submission.requested_proof_type.as_str(),
                )?;
                replace_existing_batch_task(
                    state,
                    submission,
                    &existing,
                    Some(request_fingerprint),
                )
                .await
                .map_err(Error::from_api_error)?;
                return Ok(submission.public_task_id.clone());
            }
            return Err(Error::request_conflict(
                "same proof task id was submitted with different proof input",
            ));
        }
        handle_existing_batch_task(state, submission, existing, Some(request_fingerprint))
            .await
            .map_err(Error::from_api_error)?;
        return Ok(submission.public_task_id.clone());
    }

    ensure_engine_available(
        state,
        &submission.pair.key,
        submission.route.pipeline_key(),
        submission.requested_proof_type.as_str(),
    )?;
    let plan =
        build_submission_plan(submission, request_fingerprint).map_err(Error::from_api_error)?;
    match register_batch_task(state, submission, &plan, request_fingerprint)
        .await
        .map_err(Error::from_api_error)?
    {
        raiko2_runtime::TaskRegistrationOutcome::Created(record) => {
            handle_created_batch_task(state, submission, &plan, &record)
                .await
                .map_err(Error::from_api_error)?;
        }
        raiko2_runtime::TaskRegistrationOutcome::Existing(existing) => {
            if existing.request_fingerprint != request_fingerprint {
                return Err(Error::request_conflict(
                    "same proof task id was submitted with different proof input",
                ));
            }
            handle_existing_batch_task(state, submission, existing, Some(request_fingerprint))
                .await
                .map_err(Error::from_api_error)?;
        }
    }
    Ok(submission.public_task_id.clone())
}

fn ensure_engine_available(
    state: &AppState,
    pair_key: &str,
    pipeline_key: raiko2_pipeline::PipelineKey,
    proof_type: impl std::fmt::Display,
) -> Result<Arc<dyn EngineHandle>, Error> {
    // Single source of truth for "backend not served here": callers that need the engine handle
    // (aggregation) and callers that only gate availability (proposal) share this NOT_FOUND mapping
    // so an unavailable pipeline is always a 400 unsupported_proof_type, never a 404 task_not_found.
    resolve_engine(state, pair_key, pipeline_key).map_err(|err| match err.status {
        StatusCode::NOT_FOUND => Error::unsupported_proof_type(format!(
            "proof_type={proof_type} is not supported by this server route"
        )),
        _ => Error::from_api_error(err),
    })
}

fn proof_task_data(data: TaskData) -> wire::ProofTaskData {
    wire::ProofTaskData {
        task_id: data.task_id,
        status: proof_status_string(&data.status),
        proof: data.proof,
        error: data.error,
    }
}

fn proof_status_string(status: &ProofStatus) -> String {
    match status {
        ProofStatus::Pending => "registered",
        ProofStatus::Proving => "work_in_progress",
        ProofStatus::Completed => "completed",
        ProofStatus::Failed => "failed",
        ProofStatus::Cancelled => "cancelled",
    }
    .to_string()
}

#[cfg(test)]
pub(super) fn proposal_request_fingerprint_for_test(
    submission: &CanonicalBatchSubmission,
) -> Result<String, Error> {
    Ok(
        proposal_request_fingerprint("test", "raiko2-test", submission)
            .map_err(Error::from_api_error)?
            .as_str()
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::super::{AggregateStatus, ProposalStatus, RootRuntime, RuntimeRunnerStatus};
    use super::*;
    use crate::config::Config;
    use crate::server::proof_artifact::load_proof_artifact_material;
    use crate::server::state::{EngineQueueTaskView, EngineStatusView, StaticPipelineFactory};
    use crate::server::task_metadata::{
        ProposalTask, RuntimeMetadata, aggregate_task_ref, proposal_task_ref,
        publication_proof_artifact_refs,
    };
    use raiko2_engine::{
        AggregationTaskRequest, EngineTaskId, ProposalTaskRequest, ProverTaskConfig,
    };
    use raiko2_primitives::L2BlockRange;
    use raiko2_primitives::Proof;
    use raiko2_queue::TaskStoreError;
    use raiko2_runtime::test_support::{
        ExactInvalidationResult, ProofObjectStore, RuntimeStateObject, RuntimeStateStore,
        RuntimeStateWriteResult, RuntimeStore, RuntimeStoreScope,
    };
    use raiko2_runtime::{
        ProofArtifactKey, ProofArtifactObject, ProofArtifactPutResult, ProofArtifactRegistration,
        RunnerStatus as RuntimeTaskRunnerStatus, RuntimeManager, TaskRegistration,
    };
    use std::{future::Future, path::PathBuf, pin::Pin, process, sync::Arc, time::SystemTime};

    type TestBoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

    struct TestRemoveEngine {
        fail_remove: bool,
    }

    #[derive(Debug)]
    struct DeleteFailArtifactStore;

    #[async_trait::async_trait]
    impl RuntimeStoreScope for DeleteFailArtifactStore {
        fn environment(&self) -> &'static str {
            "test"
        }

        fn namespace(&self) -> &'static str {
            "delete-failure"
        }

        fn backend_name(&self) -> &'static str {
            "test"
        }
    }

    #[async_trait::async_trait]
    impl ProofObjectStore for DeleteFailArtifactStore {
        async fn put_if_absent(
            &self,
            _key: &ProofArtifactKey,
            _bytes: &[u8],
        ) -> anyhow::Result<ProofArtifactPutResult> {
            panic!("unexpected proof artifact publication")
        }

        async fn get(&self, key: &ProofArtifactKey) -> anyhow::Result<Option<ProofArtifactObject>> {
            Ok(Some(ProofArtifactObject {
                proof_uri: format!("delete-fails://{}", key.proof_ref),
                content_hash: "content-hash".to_string(),
                generation: Some(7),
                bytes: serde_json::to_vec(&Proof {
                    proof: Some("0xproof".to_string()),
                    ..Proof::default()
                })?,
            }))
        }

        async fn get_descriptor(
            &self,
            key: &ProofArtifactKey,
        ) -> anyhow::Result<Option<raiko2_runtime::ProofArtifactDescriptor>> {
            Ok(self.get(key).await?.map(|object| object.descriptor()))
        }

        async fn get_prefix(
            &self,
            _key: &ProofArtifactKey,
            _max_bytes: usize,
        ) -> anyhow::Result<Option<raiko2_runtime::ProofArtifactPrefix>> {
            Ok(None)
        }

        async fn invalidate_exact(
            &self,
            _key: &ProofArtifactKey,
            _descriptor: &raiko2_runtime::ProofArtifactDescriptor,
        ) -> anyhow::Result<ExactInvalidationResult> {
            anyhow::bail!("injected artifact deletion failure")
        }

        async fn is_invalidated(
            &self,
            _key: &ProofArtifactKey,
            _descriptor: &raiko2_runtime::ProofArtifactDescriptor,
        ) -> anyhow::Result<bool> {
            Ok(false)
        }

        async fn delete_exact(
            &self,
            _key: &ProofArtifactKey,
            _descriptor: &raiko2_runtime::ProofArtifactDescriptor,
        ) -> anyhow::Result<raiko2_runtime::ProofArtifactDeleteResult> {
            Ok(raiko2_runtime::ProofArtifactDeleteResult::Missing)
        }
    }

    #[async_trait::async_trait]
    impl RuntimeStateStore for DeleteFailArtifactStore {
        async fn load_runtime_state(&self) -> anyhow::Result<Option<RuntimeStateObject>> {
            Ok(None)
        }

        async fn store_runtime_state(
            &self,
            _bytes: &[u8],
            expected_generation: Option<i64>,
        ) -> anyhow::Result<RuntimeStateWriteResult> {
            Ok(RuntimeStateWriteResult::Stored {
                generation: Some(expected_generation.unwrap_or(0).saturating_add(1)),
            })
        }
    }

    impl EngineHandle for TestRemoveEngine {
        fn get_status(
            &self,
            _id: EngineTaskId,
        ) -> TestBoxFuture<'_, Result<Option<EngineStatusView>, TaskStoreError>> {
            Box::pin(async { Ok(None) })
        }

        fn get_task_state(
            &self,
            _id: EngineTaskId,
        ) -> TestBoxFuture<'_, Result<Option<EngineQueueTaskView>, TaskStoreError>> {
            Box::pin(async { Ok(None) })
        }

        fn has_active_execution(
            &self,
            _owner: raiko2_queue::RootOwner,
        ) -> TestBoxFuture<'_, Result<bool, TaskStoreError>> {
            Box::pin(async { Ok(false) })
        }

        fn attach_execution_plan(
            &self,
            _owner: raiko2_queue::RootOwner,
            _plan: raiko2_engine::EngineExecutionPlan,
        ) -> TestBoxFuture<'_, Result<raiko2_queue::AttachOutcome, TaskStoreError>> {
            Box::pin(async { Ok(raiko2_queue::AttachOutcome::Attached) })
        }

        fn detach_execution(
            &self,
            _owner: raiko2_queue::RootOwner,
            mode: raiko2_queue::DetachMode,
        ) -> TestBoxFuture<
            '_,
            Result<raiko2_queue::DetachOutcome<raiko2_engine::EngineTaskKey>, TaskStoreError>,
        > {
            Box::pin(async move {
                if self.fail_remove {
                    Err(TaskStoreError::backend(std::io::Error::other(
                        "engine detach failed",
                    )))
                } else {
                    Ok(raiko2_queue::DetachOutcome::not_attached(mode))
                }
            })
        }
    }

    #[test]
    fn invalidation_task_log_context_describes_single_proposal() {
        let metadata = task_log_metadata(&[21], false);

        let context = invalidation_task_log_context(&metadata);

        assert_eq!(context.task_kind, "proposal");
        assert_eq!(context.proposal_ids, "21");
        assert_eq!(context.proposal_count, 1);
        assert!(!context.aggregate);
    }

    #[test]
    fn invalidation_task_log_context_describes_aggregate_range() {
        let metadata = task_log_metadata(&[31, 32], true);

        let context = invalidation_task_log_context(&metadata);

        assert_eq!(context.task_kind, "aggregate");
        assert_eq!(context.proposal_ids, "31..32");
        assert_eq!(context.proposal_count, 2);
        assert!(context.aggregate);
    }

    #[tokio::test]
    async fn existing_submission_does_not_depend_on_a_global_lifecycle_operation() {
        let runtime = Arc::new(
            RuntimeManager::new(test_runtime_root("existing-submit-lifecycle"))
                .expect("runtime manager"),
        );
        let state = AppState::from_parts(
            Arc::new(Config::default()),
            Arc::new(StaticPipelineFactory::default()),
            Arc::clone(&runtime),
        );
        let req = wire::ProofRequest {
            proof_type: wire::ProofType::Risc0,
            aggregate: false,
            prover: None,
            proposals: vec![wire::ProposalRequest {
                proposal_id: 7,
                checkpoint: None,
                l1_inclusion_block_number: 11,
                l2_block_number_start: 7,
                l2_block_number_end: 7,
                last_anchor_block_number: 6,
            }],
        };
        let mut submission = proposal_submission(&state, &req).expect("canonical submission");
        let fingerprint =
            proposal_request_fingerprint(runtime.environment(), runtime.namespace(), &submission)
                .expect("request fingerprint");
        submission.public_task_id = fingerprint.public_task_id();
        let metadata = TaskMetadata {
            network_pair: submission.pair.key.clone(),
            network: submission.pair.network.clone(),
            l1_network: submission.pair.l1_network.clone(),
            proof_type: submission.route.proof_type(),
            requested_proof_type: Some(submission.requested_proof_type.as_str().to_string()),
            prover_type: submission.prover_type,
            execution_mode: submission.execution_mode,
            aggregate_requested: false,
            proposals: Vec::new(),
            aggregate_task_id: None,
            aggregate_request: None,
            aggregate_input_artifacts: Vec::new(),
            runtime: RuntimeMetadata::default(),
        };
        runtime
            .register_task(TaskRegistration {
                task_id: submission.public_task_id.clone(),
                pipeline_key: submission.route.pipeline_key(),
                route: submission.route.route,
                task_kind: "hoodi_proposal".to_string(),
                network_pair: metadata.network_pair.clone(),
                artifact_refs: Vec::new(),
                metadata: serde_json::to_value(metadata).expect("serialize metadata"),
                request_fingerprint: fingerprint.as_str().to_string(),
            })
            .await
            .expect("register existing task");
        let existing = runtime
            .get_task(&submission.public_task_id)
            .await
            .expect("get existing task")
            .expect("registered task");
        runtime
            .cancel_task_if_current(&existing.lifetime(), None)
            .await
            .expect("cancel existing task");

        let result = submit_submission(&state, &submission, fingerprint.as_str()).await;
        assert!(
            result.is_err(),
            "empty test factory should reject submission"
        );
    }

    #[tokio::test]
    async fn remove_invalidated_tasks_keeps_runtime_row_when_child_cleanup_fails() {
        let mut factory = StaticPipelineFactory::default();
        factory.insert(
            "taiko_dev/taiko_dev_l1",
            PipelineKey::ShastaSp1,
            Arc::new(TestRemoveEngine { fail_remove: true }),
        );
        let runtime = Arc::new(
            RuntimeManager::new(test_runtime_root("invalidate-child-cleanup-fails"))
                .expect("runtime manager"),
        );
        let state = AppState::from_parts(
            Arc::new(Config::default()),
            Arc::new(factory),
            Arc::clone(&runtime),
        );
        let metadata = task_log_metadata(&[55], false);
        let artifact_refs = publication_proof_artifact_refs(&metadata, PipelineKey::ShastaSp1);
        let mut record = runtime
            .register_task(TaskRegistration {
                task_id: "task_child_cleanup_failure".to_string(),
                pipeline_key: PipelineKey::ShastaSp1,
                route: PipelineKey::ShastaSp1.route(),
                task_kind: "hoodi_proposal".to_string(),
                network_pair: metadata.network_pair.clone(),
                artifact_refs,
                metadata: serde_json::to_value(&metadata).expect("serialize metadata"),
                request_fingerprint: "task-child-cleanup-failure".into(),
            })
            .await
            .expect("register runtime task");
        record.runner_status = RuntimeTaskRunnerStatus::Completed;
        runtime.upsert_task(&record).await.expect("upsert task");

        let artifact_refs = invalidation_artifact_refs(
            &record.network_pair,
            &metadata,
            record.pipeline_key,
            record.route,
            None,
        );
        let root_artifact_refs = root_invalidation_artifact_refs(
            &record.network_pair,
            &metadata,
            record.pipeline_key,
            record.route,
        );
        let expected_blocked_refs = artifact_refs
            .iter()
            .chain(root_artifact_refs.iter())
            .cloned()
            .collect::<HashSet<_>>();
        let mut data = wire::InvalidateArtifactsData::default();
        let blocked_refs = remove_invalidated_tasks(
            &state,
            vec![MatchedInvalidationTask {
                record,
                metadata,
                artifact_refs,
                root_artifact_refs,
            }],
            &mut data,
        )
        .await;

        assert_eq!(data.tasks.failed, 1);
        assert_eq!(data.tasks.removed, 0);
        assert_eq!(blocked_refs, expected_blocked_refs);
        assert!(
            runtime
                .get_task("task_child_cleanup_failure")
                .await
                .expect("get runtime task")
                .is_some(),
            "runtime task row was removed before child cleanup could complete"
        );
    }

    #[tokio::test]
    async fn remove_invalidated_tasks_cannot_remove_a_reopened_root() {
        let engine: Arc<dyn EngineHandle> = Arc::new(TestRemoveEngine { fail_remove: false });
        let mut factory = StaticPipelineFactory::default();
        factory.insert(
            "taiko_dev/taiko_dev_l1",
            PipelineKey::ShastaSp1,
            Arc::clone(&engine),
        );
        let runtime = Arc::new(
            RuntimeManager::new(test_runtime_root("invalidate-reopened-root"))
                .expect("runtime manager"),
        );
        let state = AppState::from_parts(
            Arc::new(Config::default()),
            Arc::new(factory),
            Arc::clone(&runtime),
        );
        let metadata = task_log_metadata(&[56], false);
        let artifact_refs = publication_proof_artifact_refs(&metadata, PipelineKey::ShastaSp1);
        let mut stale = runtime
            .register_task(TaskRegistration {
                task_id: "task_reopened_before_invalidation".to_string(),
                pipeline_key: PipelineKey::ShastaSp1,
                route: PipelineKey::ShastaSp1.route(),
                task_kind: "hoodi_proposal".to_string(),
                network_pair: metadata.network_pair.clone(),
                artifact_refs,
                metadata: serde_json::to_value(&metadata).expect("serialize metadata"),
                request_fingerprint: "task-reopened-before-invalidation".into(),
            })
            .await
            .expect("register runtime task");
        stale.runner_status = RuntimeTaskRunnerStatus::Failed;
        stale.error = Some("retryable failure".to_string());
        runtime
            .upsert_task(&stale)
            .await
            .expect("fail runtime task");

        let reopened = runtime
            .prepare_task_for_recovery_if_unchanged(&stale)
            .await
            .expect("reopen runtime task")
            .expect("reopened task");
        state
            .lifecycle
            .attach(
                &reopened,
                &engine,
                raiko2_engine::EngineExecutionPlan {
                    proposals: Vec::new(),
                    aggregate: None,
                },
            )
            .await
            .expect("attach reopened task");

        let artifact_refs = invalidation_artifact_refs(
            &stale.network_pair,
            &metadata,
            stale.pipeline_key,
            stale.route,
            None,
        );
        let root_artifact_refs = root_invalidation_artifact_refs(
            &stale.network_pair,
            &metadata,
            stale.pipeline_key,
            stale.route,
        );
        let expected_blocked_refs = artifact_refs
            .iter()
            .chain(root_artifact_refs.iter())
            .cloned()
            .collect::<HashSet<_>>();
        let mut data = wire::InvalidateArtifactsData::default();
        let blocked_refs = remove_invalidated_tasks(
            &state,
            vec![MatchedInvalidationTask {
                record: stale.clone(),
                metadata,
                artifact_refs,
                root_artifact_refs,
            }],
            &mut data,
        )
        .await;

        assert_eq!(data.tasks.failed, 1);
        assert_eq!(data.tasks.removed, 0);
        assert_eq!(blocked_refs, expected_blocked_refs);
        let current = runtime
            .get_task(&stale.task_id)
            .await
            .expect("load current task")
            .expect("current task");
        assert_eq!(current.incarnation_id, stale.incarnation_id);
        assert_eq!(current.runner_status, RuntimeTaskRunnerStatus::Allocated);
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn invalidate_artifacts_keeps_artifact_record_when_child_cleanup_fails() {
        let mut factory = StaticPipelineFactory::default();
        factory.insert(
            "taiko_dev/taiko_dev_l1",
            PipelineKey::ShastaSp1,
            Arc::new(TestRemoveEngine { fail_remove: true }),
        );
        let runtime = Arc::new(
            RuntimeManager::new(test_runtime_root("invalidate-artifact-cleanup-fails"))
                .expect("runtime manager"),
        );
        let state = AppState::from_parts(
            Arc::new(Config::default()),
            Arc::new(factory),
            Arc::clone(&runtime),
        );
        let mut metadata = task_log_metadata(&[56], false);
        metadata.requested_proof_type = Some("sp1".to_string());
        let artifact_refs = publication_proof_artifact_refs(&metadata, PipelineKey::ShastaSp1);
        let mut record = runtime
            .register_task(TaskRegistration {
                task_id: "task_artifact_cleanup_failure".to_string(),
                pipeline_key: PipelineKey::ShastaSp1,
                route: PipelineKey::ShastaSp1.route(),
                task_kind: "hoodi_proposal".to_string(),
                network_pair: metadata.network_pair.clone(),
                artifact_refs,
                metadata: serde_json::to_value(&metadata).expect("serialize metadata"),
                request_fingerprint: "task-artifact-cleanup-failure".into(),
            })
            .await
            .expect("register runtime task");
        record.runner_status = RuntimeTaskRunnerStatus::Completed;
        runtime.upsert_task(&record).await.expect("upsert task");

        let root_refs = root_invalidation_artifact_refs(
            &record.network_pair,
            &metadata,
            record.pipeline_key,
            record.route,
        );
        let proof_ref = root_refs
            .iter()
            .next()
            .expect("root proof ref")
            .proof_ref
            .clone();
        let publication = runtime
            .publish_proof_artifact_bytes(
                &metadata.network_pair,
                record.pipeline_key,
                record.route,
                &proof_ref,
                serde_json::to_vec(&Proof {
                    proof: Some("0xroot".to_string()),
                    ..Proof::default()
                })
                .expect("serialize proof")
                .as_slice(),
            )
            .await
            .expect("write proof artifact");
        let artifact = publication
            .try_object()
            .expect("proof publication should materialize content");
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: metadata.network_pair.clone(),
                proof_ref: proof_ref.clone(),
                pipeline_key: record.pipeline_key,
                route: record.route,
                proof_uri: artifact.proof_uri.clone(),
                content_hash: artifact.content_hash.clone(),
                generation: artifact.generation,
            })
            .await
            .expect("register proof artifact");

        let data = invalidate_artifacts_inner(
            &state,
            &wire::InvalidateArtifactsRequest {
                proof_type: wire::ProofType::Sp1,
                proof_prefix: None,
                proposal_id_start: Some(56),
                proposal_id_end: Some(56),
                dry_run: false,
            },
        )
        .await
        .expect("invalidate artifacts");

        assert_eq!(data.tasks.failed, 1);
        assert_eq!(data.tasks.removed, 0);
        assert_eq!(data.artifacts.matched, 1);
        assert_eq!(data.artifacts.removed, 0);
        assert!(
            runtime
                .get_task("task_artifact_cleanup_failure")
                .await
                .expect("get runtime task")
                .is_some(),
            "runtime task row was removed before child cleanup could complete"
        );
        assert!(
            runtime
                .get_proof_artifact(
                    &metadata.network_pair,
                    record.pipeline_key,
                    record.route,
                    &proof_ref,
                )
                .await
                .expect("get proof artifact")
                .is_some(),
            "proof artifact record was removed while task cleanup failed"
        );
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn invalidation_preserves_proposal_artifact_used_by_active_root() {
        let network_pair = "taiko_dev/taiko_dev_l1";
        let pipeline = PipelineKey::ShastaSp1;
        let route = pipeline.route();
        let mut factory = StaticPipelineFactory::default();
        factory.insert(
            network_pair,
            pipeline,
            Arc::new(TestRemoveEngine { fail_remove: false }),
        );
        let runtime = Arc::new(
            RuntimeManager::new(test_runtime_root("invalidate-shared-active-proposal"))
                .expect("runtime manager"),
        );
        let state = AppState::from_parts(
            Arc::new(Config::default()),
            Arc::new(factory),
            Arc::clone(&runtime),
        );
        let mut completed_metadata = task_log_metadata(&[58], false);
        completed_metadata.requested_proof_type = Some("sp1".to_string());
        let mut active_metadata = task_log_metadata(&[58], true);
        active_metadata.requested_proof_type = Some("sp1".to_string());
        let proposal_ref = proposal_proof_artifact_refs(
            pipeline,
            completed_metadata.proposals.first().expect("proposal"),
        )
        .into_iter()
        .next()
        .expect("proposal artifact ref");

        for (task_id, metadata, status) in [
            (
                "completed-proposal-root",
                &completed_metadata,
                RuntimeTaskRunnerStatus::Completed,
            ),
            (
                "active-aggregate-root",
                &active_metadata,
                RuntimeTaskRunnerStatus::Running,
            ),
        ] {
            let mut record = runtime
                .register_task(TaskRegistration {
                    task_id: task_id.to_string(),
                    pipeline_key: pipeline,
                    route,
                    task_kind: "hoodi_proposal".to_string(),
                    network_pair: metadata.network_pair.clone(),
                    artifact_refs: vec![proposal_ref.clone()],
                    metadata: serde_json::to_value(metadata).expect("serialize metadata"),
                    request_fingerprint: format!("request-{task_id}"),
                })
                .await
                .expect("register runtime task");
            record.runner_status = status;
            runtime.upsert_task(&record).await.expect("upsert task");
        }
        register_test_artifact(
            runtime.as_ref(),
            network_pair,
            pipeline,
            route,
            &proposal_ref,
            "0xproposal",
        )
        .await;
        let data = invalidate_artifacts_inner(
            &state,
            &wire::InvalidateArtifactsRequest {
                proof_type: wire::ProofType::Sp1,
                proof_prefix: None,
                proposal_id_start: Some(58),
                proposal_id_end: Some(58),
                dry_run: false,
            },
        )
        .await
        .expect("invalidate artifacts");

        assert_eq!(data.tasks.removed, 1);
        assert!(
            runtime
                .get_task("active-aggregate-root")
                .await
                .expect("get active root")
                .is_some()
        );
        assert!(
            runtime
                .get_proof_artifact(network_pair, pipeline, route, &proposal_ref)
                .await
                .expect("get shared proposal artifact")
                .is_some()
        );
    }

    #[tokio::test]
    async fn remove_invalidated_artifacts_keeps_retryable_record_when_backing_delete_fails() {
        let store: Arc<dyn RuntimeStore> = Arc::new(DeleteFailArtifactStore);
        let runtime = Arc::new(RuntimeManager::with_store(store));
        let state = AppState::from_parts(
            Arc::new(Config::default()),
            Arc::new(StaticPipelineFactory::default()),
            Arc::clone(&runtime),
        );
        let network_pair = "taiko_dev/taiko_dev_l1";
        let pipeline_key = PipelineKey::ShastaSp1;
        let proof_ref = "proposal-delete-failure";
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: network_pair.to_string(),
                proof_ref: proof_ref.to_string(),
                pipeline_key,
                route: pipeline_key.route(),
                proof_uri: format!("delete-fails://{proof_ref}"),
                content_hash: "content-hash".to_string(),
                generation: Some(7),
            })
            .await
            .expect("register proof artifact");
        let artifact = runtime
            .get_proof_artifact(network_pair, pipeline_key, pipeline_key.route(), proof_ref)
            .await
            .expect("get proof artifact")
            .expect("proof artifact record");
        let mut data = wire::InvalidateArtifactsData::default();
        data.artifacts.matched = 1;

        remove_invalidated_artifacts(&state, vec![artifact], &mut data).await;

        assert_eq!(data.artifacts.matched, 1);
        assert_eq!(data.artifacts.removed, 0);
        assert_eq!(data.artifacts.manifests_removed, 0);
        assert_eq!(data.artifacts.failed, 1);
        assert!(
            runtime
                .get_proof_artifact(network_pair, pipeline_key, pipeline_key.route(), proof_ref)
                .await
                .expect("get proof artifact")
                .is_none(),
            "invalidated proof artifact remained active after backing delete failed"
        );
        let retryable = runtime
            .list_proof_artifacts()
            .await
            .expect("list retryable proof artifacts");
        assert_eq!(retryable.len(), 1);
        assert_eq!(retryable[0].proof_ref, proof_ref);
        assert!(retryable[0].invalidated_at.is_some());
        assert!(
            load_proof_artifact_material(
                runtime.as_ref(),
                network_pair,
                pipeline_key,
                pipeline_key.route(),
                proof_ref,
            )
            .await
            .expect("load invalidated proof artifact")
            .is_none(),
            "tombstoned backing object was rediscovered"
        );
    }

    #[tokio::test]
    async fn remove_invalidated_artifacts_counts_missing_manifest() {
        let runtime = Arc::new(
            RuntimeManager::new(test_runtime_root("invalidate-missing-manifest"))
                .expect("runtime manager"),
        );
        let state = AppState::from_parts(
            Arc::new(Config::default()),
            Arc::new(StaticPipelineFactory::default()),
            Arc::clone(&runtime),
        );
        let network_pair = "taiko_dev/taiko_dev_l1";
        let pipeline_key = PipelineKey::ShastaSp1;
        let route = pipeline_key.route();
        let proof_ref = "proposal-missing-manifest";
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: network_pair.to_string(),
                proof_ref: proof_ref.to_string(),
                pipeline_key,
                route,
                proof_uri: format!("memory://missing/{proof_ref}"),
                content_hash: "missing-content-hash".to_string(),
                generation: Some(7),
            })
            .await
            .expect("register dangling artifact");
        let artifact = runtime
            .get_proof_artifact(network_pair, pipeline_key, route, proof_ref)
            .await
            .expect("get artifact")
            .expect("artifact record");
        let mut data = wire::InvalidateArtifactsData::default();
        data.artifacts.matched = 1;

        remove_invalidated_artifacts(&state, vec![artifact], &mut data).await;

        assert_eq!(data.artifacts.removed, 1);
        assert_eq!(data.artifacts.manifests_removed, 0);
        assert_eq!(data.artifacts.manifests_missing, 1);
        assert_eq!(data.artifacts.failed, 0);
    }

    #[tokio::test]
    async fn invalidation_rechecks_root_that_completed_after_candidate_snapshot() {
        let mut factory = StaticPipelineFactory::default();
        factory.insert(
            "taiko_dev/taiko_dev_l1",
            PipelineKey::ShastaSp1,
            Arc::new(TestRemoveEngine { fail_remove: true }),
        );
        let runtime = Arc::new(
            RuntimeManager::new(test_runtime_root("invalidate-late-terminal-root"))
                .expect("runtime manager"),
        );
        let state = AppState::from_parts(
            Arc::new(Config::default()),
            Arc::new(factory),
            Arc::clone(&runtime),
        );
        let metadata = task_log_metadata(&[57], false);
        let pipeline = PipelineKey::ShastaSp1;
        let route = pipeline.route();
        let proof_ref =
            root_invalidation_artifact_refs(&metadata.network_pair, &metadata, pipeline, route)
                .into_iter()
                .next()
                .expect("root artifact ref")
                .proof_ref;
        let publication = runtime
            .publish_proof_artifact_bytes(
                &metadata.network_pair,
                pipeline,
                route,
                &proof_ref,
                &serde_json::to_vec(&Proof {
                    proof: Some("0xlate".to_string()),
                    ..Proof::default()
                })
                .expect("serialize proof"),
            )
            .await
            .expect("publish proof artifact");
        let object = publication
            .try_object()
            .expect("proof publication should materialize content");
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: metadata.network_pair.clone(),
                proof_ref: proof_ref.clone(),
                pipeline_key: pipeline,
                route,
                proof_uri: object.proof_uri.clone(),
                content_hash: object.content_hash.clone(),
                generation: object.generation,
            })
            .await
            .expect("register proof artifact");
        let artifact = runtime
            .get_proof_artifact(&metadata.network_pair, pipeline, route, &proof_ref)
            .await
            .expect("get proof artifact")
            .expect("proof artifact record");

        // The artifact candidate was captured while the root was non-terminal. It completes
        // before deletion starts, so the post-marker task lookup must discover it.
        let mut record = runtime
            .register_task(TaskRegistration {
                task_id: "task_late_terminal_root".to_string(),
                pipeline_key: pipeline,
                route,
                task_kind: "hoodi_proposal".to_string(),
                network_pair: metadata.network_pair.clone(),
                artifact_refs: vec![proof_ref.clone()],
                metadata: serde_json::to_value(&metadata).expect("serialize metadata"),
                request_fingerprint: "task-late-terminal-root".into(),
            })
            .await
            .expect("register runtime task");
        record.runner_status = RuntimeTaskRunnerStatus::Completed;
        runtime.upsert_task(&record).await.expect("complete task");

        let mut data = wire::InvalidateArtifactsData::default();
        data.artifacts.matched = 1;
        remove_invalidated_artifacts(&state, vec![artifact], &mut data).await;

        assert_eq!(data.tasks.matched, 1);
        assert_eq!(data.tasks.failed, 1);
        assert_eq!(data.artifacts.removed, 0);
        assert!(
            runtime
                .get_proof_artifact_including_invalidated(
                    &metadata.network_pair,
                    pipeline,
                    route,
                    &proof_ref,
                )
                .await
                .expect("get tombstoned artifact")
                .is_some(),
            "artifact deletion proceeded after late root cleanup failed"
        );
    }

    #[tokio::test]
    async fn invalidation_keeps_artifact_for_late_nonterminal_consumer() {
        let runtime = Arc::new(
            RuntimeManager::new(test_runtime_root("invalidate-late-nonterminal-root"))
                .expect("runtime manager"),
        );
        let state = AppState::from_parts(
            Arc::new(Config::default()),
            Arc::new(StaticPipelineFactory::default()),
            Arc::clone(&runtime),
        );
        let metadata = task_log_metadata(&[58], false);
        let pipeline = PipelineKey::ShastaSp1;
        let route = pipeline.route();
        let proof_ref =
            root_invalidation_artifact_refs(&metadata.network_pair, &metadata, pipeline, route)
                .into_iter()
                .next()
                .expect("root artifact ref")
                .proof_ref;
        let publication = runtime
            .publish_proof_artifact_bytes(
                &metadata.network_pair,
                pipeline,
                route,
                &proof_ref,
                &serde_json::to_vec(&Proof {
                    proof: Some("0xlate".to_string()),
                    ..Proof::default()
                })
                .expect("serialize proof"),
            )
            .await
            .expect("publish proof artifact");
        let object = publication
            .try_object()
            .expect("proof publication should materialize content");
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: metadata.network_pair.clone(),
                proof_ref: proof_ref.clone(),
                pipeline_key: pipeline,
                route,
                proof_uri: object.proof_uri.clone(),
                content_hash: object.content_hash.clone(),
                generation: object.generation,
            })
            .await
            .expect("register proof artifact");
        let artifact = runtime
            .get_proof_artifact(&metadata.network_pair, pipeline, route, &proof_ref)
            .await
            .expect("get proof artifact")
            .expect("proof artifact record");

        runtime
            .register_task(TaskRegistration {
                task_id: "task_late_nonterminal_root".to_string(),
                pipeline_key: pipeline,
                route,
                task_kind: "hoodi_proposal".to_string(),
                network_pair: metadata.network_pair.clone(),
                artifact_refs: vec![proof_ref.clone()],
                metadata: serde_json::to_value(&metadata).expect("serialize metadata"),
                request_fingerprint: "task-late-nonterminal-root".into(),
            })
            .await
            .expect("register runtime task");

        let mut data = wire::InvalidateArtifactsData::default();
        data.artifacts.matched = 1;
        remove_invalidated_artifacts(&state, vec![artifact], &mut data).await;

        assert_eq!(data.tasks.skipped_non_terminal, 1);
        assert_eq!(data.artifacts.removed, 0);
        assert!(
            runtime
                .get_proof_artifact(&metadata.network_pair, pipeline, route, &proof_ref)
                .await
                .expect("get retained proof artifact")
                .is_some(),
            "artifact invalidation ignored a late non-terminal consumer"
        );
    }

    #[tokio::test]
    async fn invalidation_keeps_sp1_routes_distinct() {
        let runtime = Arc::new(
            RuntimeManager::new(test_runtime_root("invalidate-sp1-routes"))
                .expect("runtime manager"),
        );
        let state = AppState::from_parts(
            Arc::new(Config::default()),
            Arc::new(StaticPipelineFactory::default()),
            Arc::clone(&runtime),
        );
        let network_pair = "taiko_dev/taiko_dev_l1";
        let pipeline = PipelineKey::ShastaSp1;
        let proof_ref = "proposal-shared-ref";
        for route in [
            "sp1/local".parse::<PipelineRoute>().expect("local route"),
            "sp1/network"
                .parse::<PipelineRoute>()
                .expect("network route"),
        ] {
            let bytes = serde_json::to_vec(&Proof {
                proof: Some("0xproof".to_string()),
                ..Proof::default()
            })
            .expect("serialize proof");
            let publication = runtime
                .publish_proof_artifact_bytes(network_pair, pipeline, route, proof_ref, &bytes)
                .await
                .expect("publish proof artifact");
            let object = publication
                .try_object()
                .expect("proof publication should materialize content");
            runtime
                .upsert_proof_artifact(ProofArtifactRegistration {
                    network_pair: network_pair.to_string(),
                    proof_ref: proof_ref.to_string(),
                    pipeline_key: pipeline,
                    route,
                    proof_uri: object.proof_uri.clone(),
                    content_hash: object.content_hash.clone(),
                    generation: object.generation,
                })
                .await
                .expect("register proof artifact");
        }
        let mut data = wire::InvalidateArtifactsData::default();
        let matched = collect_invalidation_artifacts(
            &state,
            &[pipeline],
            None,
            &HashSet::new(),
            None,
            &mut data,
        )
        .await
        .expect("collect artifacts");

        assert_eq!(matched.len(), 2);
        assert_eq!(data.artifacts.matched, 2);
        remove_invalidated_artifacts(&state, matched, &mut data).await;
        assert_eq!(data.artifacts.removed, 2);
        assert!(
            runtime
                .list_proof_artifacts()
                .await
                .expect("list")
                .is_empty()
        );
    }

    #[test]
    fn proof_json_prefix_starts_with_matches_large_proof_prefix() {
        let proof = format!(
            "0x{}{}",
            "aa".repeat(80),
            "bb".repeat(PROOF_PREFIX_SCAN_LIMIT)
        );
        let json = format!("{{\n  \"proof\": \"{proof}\",\n  \"input\": null\n}}");

        assert!(
            proof_json_prefix_starts_with(json.as_bytes(), "0xAAAA").expect("scan proof prefix")
        );
        assert!(
            !proof_json_prefix_starts_with(json.as_bytes(), "0xAABB").expect("scan proof prefix")
        );
    }

    #[test]
    fn proof_json_prefix_starts_with_skips_prior_fields() {
        let json = br#"{"input":null,"extra_data":{"ignored":"proof"},"proof":"0xabcdef"}"#;

        assert!(proof_json_prefix_starts_with(json, "0xABCD").expect("scan proof prefix"));
    }

    #[test]
    fn proof_json_prefix_starts_with_skips_large_prior_string_value() {
        let large_input = "a".repeat(16 * 1024);
        let json = format!(r#"{{"input":"{large_input}","proof":"0xabcdef"}}"#);

        assert!(
            proof_json_prefix_starts_with(json.as_bytes(), "0xABCD").expect("scan proof prefix")
        );
    }

    #[test]
    fn proof_json_prefix_starts_with_rejects_invalid_unicode_escape() {
        let err =
            proof_json_prefix_starts_with(br#"{"input":"\u12xx","proof":"0xabcdef"}"#, "0xABCD")
                .expect_err("invalid unicode escape should fail scan");

        assert_eq!(err, "invalid JSON unicode escape in prefix scan");
    }

    #[test]
    fn proof_json_prefix_starts_with_rejects_mismatched_nested_closer() {
        let err = proof_json_prefix_starts_with(br#"{"input":{],"proof":"0xabcdef"}"#, "0xABCD")
            .expect_err("mismatched nested closer should fail scan");

        assert_eq!(err, "mismatched nested JSON closer");
    }

    #[test]
    fn proof_json_prefix_starts_with_treats_null_or_missing_proof_as_mismatch() {
        assert!(
            !proof_json_prefix_starts_with(br#"{"proof":null}"#, "0xaaaa")
                .expect("scan null proof")
        );
        assert!(
            !proof_json_prefix_starts_with(br#"{"input":null}"#, "0xaaaa")
                .expect("scan missing proof")
        );
    }

    #[tokio::test]
    async fn proof_artifact_starts_with_matches_bounded_prefix_window() {
        let root = test_runtime_root("proof-prefix-window");
        let runtime = RuntimeManager::new(root).expect("runtime manager");
        let proof = format!("0x{}", "aa".repeat(PROOF_PREFIX_SCAN_LIMIT));
        runtime
            .publish_proof_artifact_bytes(
                "taiko_dev/ethereum",
                PipelineKey::ShastaSp1,
                PipelineKey::ShastaSp1.route(),
                "proof-ref",
                &serde_json::to_vec_pretty(&Proof {
                    proof: Some(proof),
                    ..Proof::default()
                })
                .expect("serialize proof"),
            )
            .await
            .expect("write proof artifact");

        assert!(
            proof_artifact_starts_with(
                &runtime,
                "taiko_dev/ethereum",
                PipelineKey::ShastaSp1,
                PipelineKey::ShastaSp1.route(),
                "proof-ref",
                "0xAAAA",
            )
            .await
            .expect("scan proof artifact prefix"),
            "proof prefix did not match"
        );
    }

    #[tokio::test]
    async fn prefix_invalidation_propagates_corrupt_artifact_error() {
        let runtime = Arc::new(
            RuntimeManager::new(test_runtime_root("invalidate-corrupt-prefix"))
                .expect("runtime manager"),
        );
        let state = AppState::from_parts(
            Arc::new(Config::default()),
            Arc::new(StaticPipelineFactory::default()),
            Arc::clone(&runtime),
        );
        let network_pair = "taiko_dev/taiko_dev_l1";
        let pipeline_key = PipelineKey::ShastaSp1;
        let route = pipeline_key.route();
        let proof_ref = "proposal-corrupt-prefix";
        let publication = runtime
            .publish_proof_artifact_bytes(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
                br#"{"proof":42}"#,
            )
            .await
            .expect("publish corrupt artifact");
        let object = publication
            .try_object()
            .expect("proof publication should materialize content");
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: network_pair.to_string(),
                proof_ref: proof_ref.to_string(),
                pipeline_key,
                route,
                proof_uri: object.proof_uri.clone(),
                content_hash: object.content_hash.clone(),
                generation: object.generation,
            })
            .await
            .expect("register corrupt artifact");

        let result = invalidate_artifacts_inner(
            &state,
            &wire::InvalidateArtifactsRequest {
                proof_type: wire::ProofType::Sp1,
                proof_prefix: Some("0xAA".to_string()),
                proposal_id_start: None,
                proposal_id_end: None,
                dry_run: false,
            },
        )
        .await;
        let Err(error) = result else {
            panic!("corrupt prefix scan must fail the invalidation request");
        };

        assert!(error.message.contains("failed to inspect proof artifact"));
        assert!(
            runtime
                .get_proof_artifact(network_pair, pipeline_key, route, proof_ref)
                .await
                .expect("get retained artifact")
                .is_some()
        );
    }

    #[test]
    fn proof_request_error_preserves_internal_http_status() {
        let err = ProofRequestError(Error::from_api_error(ApiError::internal("db is down")));

        let response = err.into_response();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn proposal_id_range_requires_contiguous_ids() {
        let proposals = vec![
            wire::ProposalRequest {
                proposal_id: 7,
                checkpoint: None,
                l1_inclusion_block_number: 11,
                l2_block_number_start: 7,
                l2_block_number_end: 7,
                last_anchor_block_number: 6,
            },
            wire::ProposalRequest {
                proposal_id: 9,
                checkpoint: None,
                l1_inclusion_block_number: 12,
                l2_block_number_start: 8,
                l2_block_number_end: 8,
                last_anchor_block_number: 7,
            },
        ];

        let err = proposal_id_range(&proposals).expect_err("gap should be rejected");

        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "invalid_request");
        assert_eq!(
            err.message,
            "proposals[].proposal_id must be strictly increasing and contiguous"
        );
    }

    #[test]
    fn v4_non_aggregate_request_requires_single_proposal() {
        let req = wire::ProofRequest {
            proof_type: wire::ProofType::Sp1,
            aggregate: false,
            prover: None,
            proposals: vec![
                wire::ProposalRequest {
                    proposal_id: 7,
                    checkpoint: None,
                    l1_inclusion_block_number: 11,
                    l2_block_number_start: 7,
                    l2_block_number_end: 7,
                    last_anchor_block_number: 6,
                },
                wire::ProposalRequest {
                    proposal_id: 8,
                    checkpoint: None,
                    l1_inclusion_block_number: 12,
                    l2_block_number_start: 8,
                    l2_block_number_end: 8,
                    last_anchor_block_number: 7,
                },
            ],
        };

        let err = validate_proof_request_shape(&req)
            .expect_err("batch proposal should require aggregate=true");

        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "invalid_request");
        assert_eq!(err.message, "aggregate=false requires exactly one proposal");
    }

    #[test]
    fn v4_aggregate_request_rejects_too_many_proposals() {
        let proposals = (0..=MAX_PROPOSALS_PER_REQUEST as u64)
            .map(|proposal_id| wire::ProposalRequest {
                proposal_id,
                checkpoint: None,
                l1_inclusion_block_number: proposal_id,
                l2_block_number_start: proposal_id,
                l2_block_number_end: proposal_id,
                last_anchor_block_number: proposal_id.saturating_sub(1),
            })
            .collect();
        let req = wire::ProofRequest {
            proof_type: wire::ProofType::Sp1,
            aggregate: true,
            prover: None,
            proposals,
        };

        let err = validate_proof_request_shape(&req)
            .expect_err("oversized proposal list should be rejected");

        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "invalid_request");
        assert_eq!(
            err.message,
            format!(
                "proposals length {} exceeds maximum {MAX_PROPOSALS_PER_REQUEST}",
                MAX_PROPOSALS_PER_REQUEST + 1
            )
        );
    }

    #[test]
    fn v4_aggregate_request_rejects_too_many_total_l2_blocks() {
        let req = wire::ProofRequest {
            proof_type: wire::ProofType::Sp1,
            aggregate: true,
            prover: None,
            proposals: vec![
                wire::ProposalRequest {
                    proposal_id: 1,
                    checkpoint: None,
                    l1_inclusion_block_number: 1,
                    l2_block_number_start: 1,
                    l2_block_number_end: MAX_TOTAL_L2_BLOCKS_PER_REQUEST,
                    last_anchor_block_number: 0,
                },
                wire::ProposalRequest {
                    proposal_id: 2,
                    checkpoint: None,
                    l1_inclusion_block_number: 2,
                    l2_block_number_start: MAX_TOTAL_L2_BLOCKS_PER_REQUEST + 1,
                    l2_block_number_end: MAX_TOTAL_L2_BLOCKS_PER_REQUEST + 1,
                    last_anchor_block_number: MAX_TOTAL_L2_BLOCKS_PER_REQUEST,
                },
            ],
        };

        let err = validate_proof_request_shape(&req)
            .expect_err("total expanded L2 blocks should be capped");

        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "invalid_request");
        assert_eq!(
            err.message,
            format!(
                "total proposals[].l2 block range length {} exceeds maximum {MAX_TOTAL_L2_BLOCKS_PER_REQUEST}",
                MAX_TOTAL_L2_BLOCKS_PER_REQUEST + 1
            )
        );
    }

    #[test]
    fn v4_request_shape_rejects_single_oversized_l2_range_with_range_error() {
        let req = wire::ProofRequest {
            proof_type: wire::ProofType::Sp1,
            aggregate: true,
            prover: None,
            proposals: vec![wire::ProposalRequest {
                proposal_id: 1,
                checkpoint: None,
                l1_inclusion_block_number: 1,
                l2_block_number_start: 1,
                l2_block_number_end: MAX_RANGE_LEN + 1,
                last_anchor_block_number: 0,
            }],
        };

        let err =
            validate_proof_request_shape(&req).expect_err("oversized L2 range should be rejected");

        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "invalid_request");
        assert_eq!(
            err.message,
            format!(
                "proposals[].l2_block_number_start..=proposals[].l2_block_number_end range length {} exceeds maximum {MAX_RANGE_LEN}",
                MAX_RANGE_LEN + 1
            )
        );
    }

    #[test]
    fn v4_l2_block_range_rejects_descending_bounds() {
        let err = collect_inclusive_range(
            22,
            20,
            "proposals[].l2_block_number_start",
            "proposals[].l2_block_number_end",
        )
        .expect_err("descending range should be rejected");

        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "invalid_request");
        assert_eq!(
            err.message,
            "proposals[].l2_block_number_end must be greater than or equal to proposals[].l2_block_number_start"
        );
    }

    #[test]
    fn v4_l2_block_range_rejects_oversized_ranges() {
        let err = collect_inclusive_range(
            1,
            MAX_RANGE_LEN + 1,
            "proposals[].l2_block_number_start",
            "proposals[].l2_block_number_end",
        )
        .expect_err("oversized range should be rejected");

        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "invalid_request");
        assert_eq!(
            err.message,
            format!(
                "proposals[].l2_block_number_start..=proposals[].l2_block_number_end range length {} exceeds maximum {MAX_RANGE_LEN}",
                MAX_RANGE_LEN + 1
            )
        );
    }

    #[test]
    fn v4_l2_block_range_rejects_overflowing_ranges() {
        let err = collect_inclusive_range(
            0,
            u64::MAX,
            "proposals[].l2_block_number_start",
            "proposals[].l2_block_number_end",
        )
        .expect_err("overflowing range should be rejected");

        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "invalid_request");
        assert_eq!(
            err.message,
            "proposals[].l2_block_number_start..=proposals[].l2_block_number_end range length overflows u64"
        );
    }

    #[test]
    fn v4_proof_task_data_projects_root_status_and_proof() {
        let data = TaskData {
            task_id: "task_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            route: "shasta/sp1".to_string(),
            prover_type: None,
            execution_mode: None,
            status: ProofStatus::Completed,
            network: "taiko_dev".to_string(),
            l1_network: "ethereum".to_string(),
            runtime: RootRuntime {
                runner_status: RuntimeRunnerStatus::Completed,
                active_stage: Some("aggregate".to_string()),
                last_event: None,
                updated_at: 1,
                engine_state_present: false,
            },
            current_index: Some(1),
            proposals: vec![ProposalStatus {
                index: 0,
                proposal_id: 10,
                checkpoint: None,
                task_id: "proposal-task-10".to_string(),
                status: ProofStatus::Completed,
                l1_inclusion_block_number: 100,
                l2_block_numbers: vec![20, 21],
                last_anchor_block_number: 19,
                proof: Some("0xproposal".to_string()),
                proof_ref: Some("proposal-ref".to_string()),
                proof_uri: Some("proposal-path".to_string()),
                error: None,
                runtime: None,
                extra_data: None,
            }],
            aggregate: Some(AggregateStatus {
                task_id: "aggregate-task".to_string(),
                status: ProofStatus::Completed,
                proof: Some("0xaggregate".to_string()),
                proof_ref: Some("aggregate-ref".to_string()),
                proof_uri: Some("aggregate-path".to_string()),
                error: None,
                runtime: None,
                extra_data: None,
            }),
            proof: Some("0xroot".to_string()),
            proof_ref: Some("root-ref".to_string()),
            proof_uri: Some("root-path".to_string()),
            error: None,
        };

        let response = proof_task_data(data);

        assert_eq!(response.status, "completed");
        assert_eq!(
            response.task_id,
            "task_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(response.proof.as_deref(), Some("0xroot"));
        assert!(response.error.is_none());
    }

    fn task_log_metadata(proposal_ids: &[u64], aggregate: bool) -> TaskMetadata {
        let pipeline_key = PipelineKey::ShastaSp1;
        let proposals = proposal_ids
            .iter()
            .map(|proposal_id| {
                let l2_block_number = proposal_id + 1;
                let request = ProposalTaskRequest {
                    proposal_id: *proposal_id,
                    l2_block_range: Some(L2BlockRange {
                        start: l2_block_number,
                        end: l2_block_number,
                    }),
                    l1_inclusion_block_number: proposal_id + 100,
                    last_anchor_block_number: proposal_id.saturating_sub(1),
                    checkpoint: None,
                    blob_proof_type: None,
                    prover: None,
                    graffiti: None,
                    prover_config: ProverTaskConfig::default(),
                };
                ProposalTask {
                    proposal_id: *proposal_id,
                    checkpoint: None,
                    l1_inclusion_block_number: proposal_id + 100,
                    l2_block_numbers: vec![l2_block_number],
                    last_anchor_block_number: proposal_id.saturating_sub(1),
                    task_id: proposal_task_ref(pipeline_key, &request),
                    request,
                }
            })
            .collect::<Vec<_>>();
        let aggregate_request = aggregate.then(|| AggregationTaskRequest {
            request_id: "aggregate-request".to_string(),
            proposal_ids: proposal_ids.to_vec(),
            prover_config: ProverTaskConfig::default(),
        });
        TaskMetadata {
            network_pair: "taiko_dev/taiko_dev_l1".to_string(),
            network: "taiko_dev".to_string(),
            l1_network: "taiko_dev_l1".to_string(),
            proof_type: raiko2_primitives::ProofType::Sp1,
            requested_proof_type: None,
            prover_type: None,
            execution_mode: None,
            aggregate_requested: aggregate,
            proposals,
            aggregate_task_id: aggregate_request
                .as_ref()
                .map(|request| aggregate_task_ref(pipeline_key, request)),
            aggregate_request,
            aggregate_input_artifacts: Vec::new(),
            runtime: RuntimeMetadata::default(),
        }
    }

    async fn register_test_artifact(
        runtime: &RuntimeManager,
        network_pair: &str,
        pipeline: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        proof: &str,
    ) {
        let bytes = serde_json::to_vec(&Proof {
            proof: Some(proof.to_string()),
            ..Proof::default()
        })
        .expect("serialize proof");
        let publication = runtime
            .publish_proof_artifact_bytes(network_pair, pipeline, route, proof_ref, &bytes)
            .await
            .expect("publish proof artifact");
        let artifact = publication
            .try_object()
            .expect("proof publication should materialize content");
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: network_pair.to_string(),
                proof_ref: proof_ref.to_string(),
                pipeline_key: pipeline,
                route,
                proof_uri: artifact.proof_uri.clone(),
                content_hash: artifact.content_hash.clone(),
                generation: artifact.generation,
            })
            .await
            .expect("register proof artifact");
    }

    fn test_runtime_root(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        std::env::temp_dir().join(format!("raiko2-{label}-{}-{nanos}", process::id()))
    }
}
