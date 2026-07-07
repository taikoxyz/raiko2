use axum::{
    Json,
    body::Body,
    extract::{
        FromRequest, Path, Query, State, rejection::JsonRejection, rejection::QueryRejection,
    },
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
};
use raiko2_pipeline::PipelineKey;
use raiko2_primitives::Proof;
use std::{collections::HashSet, sync::Arc};
use tokio::fs;

use super::super::proof_types::v4 as wire;
use super::{
    ApiData, ApiError, ApiOk, AppState, BatchProofType, BatchShastaRequest,
    CanonicalBatchSubmission, ClearProverStatus, EngineHandle, ProofStatus, ProverStatus,
    ProverTaskScope, PublicProverArgs, ServerAclFeature, ShastaProposal, TaskData, TaskMetadata,
    authorize_acl_feature_with_rate_limit, authorize_optional_acl_feature_with_rate_limit,
    build_canonical_batch_submission, build_submission_plan, clear_prover_tasks,
    collect_prover_status, handle_created_batch_task, handle_existing_batch_task,
    is_terminal_runtime_status, load_task_data, parse_task_metadata, proposal_proof_artifact_refs,
    register_batch_task, remove_task_children_if_unreferenced, replace_existing_batch_task,
    resolve_engine, root_proof_artifact_refs,
};
use crate::server::request_identity::{FingerprintSink, RequestFingerprint, RequestIdentity};

// Bound client-supplied inclusive ranges before materializing them into Vecs.
const MAX_RANGE_LEN: u64 = 100_000;
const MAX_PROPOSALS_PER_REQUEST: usize = 1_024;
const MAX_TOTAL_L2_BLOCKS_PER_REQUEST: u64 = MAX_RANGE_LEN;

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
    let request_fingerprint = proposal_request_fingerprint(&submission);
    submission.public_task_id = request_fingerprint.public_task_id();
    submit_submission(&state, &submission, request_fingerprint.as_str()).await?;
    let data = load_task_data(&state, &submission.public_task_id)
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
        status: _,
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
        status: "ok",
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
    Ok(Json(ApiOk {
        status: "ok",
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
    let matched_tasks = select_invalidation_tasks(
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

    if !req.dry_run {
        let blocked_artifact_refs = remove_invalidated_tasks(state, matched_tasks, &mut data).await;
        if !blocked_artifact_refs.is_empty() {
            matched_artifacts.retain(|artifact| {
                !blocked_artifact_refs.contains(&ProofArtifactIdentity {
                    network_pair: artifact.network_pair.clone(),
                    proof_ref: artifact.proof_ref.clone(),
                })
            });
        }
        remove_invalidated_artifacts(state, matched_artifacts, &mut data).await;
    }

    Ok(data)
}

struct CandidateInvalidationTasks {
    records: Vec<CandidateInvalidationTask>,
    artifact_refs: HashSet<ProofArtifactIdentity>,
}

struct CandidateInvalidationTask {
    record: raiko2_runtime::RuntimeTaskRecord,
    metadata: TaskMetadata,
    artifact_refs: HashSet<ProofArtifactIdentity>,
    root_artifact_refs: HashSet<ProofArtifactIdentity>,
    root_proof_matches_prefix: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProofArtifactIdentity {
    network_pair: String,
    proof_ref: String,
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
        if !scope.matches(&metadata) {
            continue;
        }
        if !metadata_matches_proposal_range(&metadata, proposal_range) {
            continue;
        }
        if !is_terminal_runtime_status(record.runner_status) {
            data.tasks.skipped_non_terminal = data.tasks.skipped_non_terminal.saturating_add(1);
            continue;
        }
        let root_artifact_refs = root_invalidation_artifact_refs(&metadata, record.pipeline_key);
        let artifact_refs =
            invalidation_artifact_refs(&metadata, record.pipeline_key, proposal_range);
        candidates
            .artifact_refs
            .extend(artifact_refs.iter().cloned());
        let root_proof_matches_prefix = task_matches_proof_prefix(&record, proof_prefix).await;
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
) -> Vec<CandidateInvalidationTask> {
    let mut matched = Vec::new();
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
            matched.push(task);
        }
    }
    matched
}

fn proof_artifact_identities(
    artifacts: &[raiko2_runtime::ProofArtifactRecord],
) -> HashSet<ProofArtifactIdentity> {
    artifacts
        .iter()
        .map(|artifact| ProofArtifactIdentity {
            network_pair: artifact.network_pair.clone(),
            proof_ref: artifact.proof_ref.clone(),
        })
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
    metadata: &TaskMetadata,
    pipeline_key: PipelineKey,
) -> HashSet<ProofArtifactIdentity> {
    root_proof_artifact_refs(metadata, pipeline_key)
        .map(|root_refs| {
            root_refs
                .refs
                .into_iter()
                .map(|proof_ref| ProofArtifactIdentity {
                    network_pair: metadata.network_pair.clone(),
                    proof_ref,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn invalidation_artifact_refs(
    metadata: &TaskMetadata,
    pipeline_key: PipelineKey,
    proposal_range: Option<(u64, u64)>,
) -> HashSet<ProofArtifactIdentity> {
    let mut refs = root_invalidation_artifact_refs(metadata, pipeline_key);
    for proposal in &metadata.proposals {
        if !proposal_matches_range(proposal.proposal_id, proposal_range) {
            continue;
        }
        refs.extend(
            proposal_proof_artifact_refs(pipeline_key, proposal)
                .into_iter()
                .map(|proof_ref| ProofArtifactIdentity {
                    network_pair: metadata.network_pair.clone(),
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
        if !artifact_matches_proof_prefix(&artifact, proof_prefix).await {
            continue;
        }
        if seen_artifacts.insert((artifact.network_pair.clone(), artifact.proof_ref.clone())) {
            data.artifacts.matched = data.artifacts.matched.saturating_add(1);
            matched_artifacts.push(artifact);
        }
    }
    Ok(matched_artifacts)
}

async fn remove_invalidated_tasks(
    state: &AppState,
    matched_tasks: Vec<CandidateInvalidationTask>,
    data: &mut wire::InvalidateArtifactsData,
) -> HashSet<ProofArtifactIdentity> {
    let mut blocked_artifact_refs = HashSet::new();
    for task in matched_tasks {
        let CandidateInvalidationTask {
            record,
            metadata,
            artifact_refs,
            root_artifact_refs,
            ..
        } = task;
        let log_context = invalidation_task_log_context(&metadata);
        let mut cleanup_failed = false;
        match resolve_engine(state, &metadata.network_pair, record.pipeline_key) {
            Ok(engine) => {
                if let Err(err) = remove_task_children_if_unreferenced(
                    &state.runtime,
                    &engine,
                    &record.task_id,
                    record.pipeline_key,
                    &metadata,
                )
                .await
                {
                    data.tasks.failed = data.tasks.failed.saturating_add(1);
                    tracing::warn!(
                        task_id = %record.task_id,
                        task_kind = log_context.task_kind,
                        aggregate = log_context.aggregate,
                        proposal_ids = %log_context.proposal_ids,
                        proposal_count = log_context.proposal_count,
                        network_pair = %metadata.network_pair,
                        proof_type = %metadata.proof_type,
                        pipeline_key = %record.pipeline_key.as_str(),
                        error = %err,
                        "failed to remove invalidated task children"
                    );
                    cleanup_failed = true;
                }
            }
            Err(err) => {
                data.tasks.failed = data.tasks.failed.saturating_add(1);
                tracing::warn!(
                    task_id = %record.task_id,
                    task_kind = log_context.task_kind,
                    aggregate = log_context.aggregate,
                    proposal_ids = %log_context.proposal_ids,
                    proposal_count = log_context.proposal_count,
                    network_pair = %metadata.network_pair,
                    proof_type = %metadata.proof_type,
                    pipeline_key = %record.pipeline_key.as_str(),
                    status = %err.status,
                    error = %err.message,
                    "skipping engine child cleanup for invalidated task"
                );
                cleanup_failed = true;
            }
        }
        if cleanup_failed {
            blocked_artifact_refs.extend(artifact_refs);
            blocked_artifact_refs.extend(root_artifact_refs);
            continue;
        }

        match state.runtime.remove_task(&record.task_id).await {
            Ok(true) => data.tasks.removed = data.tasks.removed.saturating_add(1),
            Ok(false) => {}
            Err(err) => {
                data.tasks.failed = data.tasks.failed.saturating_add(1);
                tracing::warn!(
                    task_id = %record.task_id,
                    task_kind = log_context.task_kind,
                    aggregate = log_context.aggregate,
                    proposal_ids = %log_context.proposal_ids,
                    proposal_count = log_context.proposal_count,
                    network_pair = %metadata.network_pair,
                    proof_type = %metadata.proof_type,
                    pipeline_key = %record.pipeline_key.as_str(),
                    error = %err,
                    "failed to remove invalidated runtime task"
                );
                blocked_artifact_refs.extend(artifact_refs);
                blocked_artifact_refs.extend(root_artifact_refs);
            }
        }
    }
    blocked_artifact_refs
}

enum ProofArtifactFileRemoval {
    Removed,
    Missing,
    Failed,
}

async fn remove_invalidated_artifacts(
    state: &AppState,
    matched_artifacts: Vec<raiko2_runtime::ProofArtifactRecord>,
    data: &mut wire::InvalidateArtifactsData,
) {
    for artifact in matched_artifacts {
        let file_removal = match fs::remove_file(&artifact.proof_path).await {
            Ok(()) => ProofArtifactFileRemoval::Removed,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                ProofArtifactFileRemoval::Missing
            }
            Err(err) => {
                data.artifacts.failed = data.artifacts.failed.saturating_add(1);
                tracing::warn!(
                    network_pair = %artifact.network_pair,
                    proof_ref = %artifact.proof_ref,
                    proof_path = %artifact.proof_path,
                    error = %err,
                    "failed to remove invalidated proof artifact file; removing cache record"
                );
                ProofArtifactFileRemoval::Failed
            }
        };
        match state
            .runtime
            .remove_proof_artifact(&artifact.network_pair, &artifact.proof_ref)
            .await
        {
            Ok(Some(_record)) => {
                data.artifacts.removed = data.artifacts.removed.saturating_add(1);
                match file_removal {
                    ProofArtifactFileRemoval::Removed => {
                        data.artifacts.files_removed =
                            data.artifacts.files_removed.saturating_add(1);
                    }
                    ProofArtifactFileRemoval::Missing => {
                        data.artifacts.files_missing =
                            data.artifacts.files_missing.saturating_add(1);
                    }
                    ProofArtifactFileRemoval::Failed => {}
                }
            }
            Ok(None) => {}
            Err(err) => {
                data.artifacts.failed = data.artifacts.failed.saturating_add(1);
                tracing::warn!(
                    network_pair = %artifact.network_pair,
                    proof_ref = %artifact.proof_ref,
                    error = %err,
                    "failed to remove invalidated proof artifact record"
                );
            }
        }
    }
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
}

async fn task_matches_proof_prefix(
    record: &raiko2_runtime::RuntimeTaskRecord,
    proof_prefix: Option<&str>,
) -> bool {
    let Some(prefix) = proof_prefix else {
        return true;
    };
    let Some(path) = record.proof_path.as_deref() else {
        return false;
    };
    proof_file_starts_with(path, prefix).await
}

fn artifact_matches_proposal_scope(
    artifact: &raiko2_runtime::ProofArtifactRecord,
    proposal_range: Option<(u64, u64)>,
    matched_task_artifact_refs: &HashSet<ProofArtifactIdentity>,
) -> bool {
    proposal_range.is_none()
        || matched_task_artifact_refs.contains(&ProofArtifactIdentity {
            network_pair: artifact.network_pair.clone(),
            proof_ref: artifact.proof_ref.clone(),
        })
}

async fn artifact_matches_proof_prefix(
    artifact: &raiko2_runtime::ProofArtifactRecord,
    proof_prefix: Option<&str>,
) -> bool {
    match proof_prefix {
        Some(prefix) => proof_file_starts_with(&artifact.proof_path, prefix).await,
        None => true,
    }
}

async fn proof_file_starts_with(path: &str, prefix: &str) -> bool {
    let bytes = match fs::read(path).await {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::warn!(
                proof_path = %path,
                error = %err,
                "failed to read proof artifact for prefix match"
            );
            return false;
        }
    };
    let proof = match serde_json::from_slice::<Proof>(&bytes) {
        Ok(proof) => proof,
        Err(err) => {
            tracing::warn!(
                proof_path = %path,
                error = %err,
                "failed to parse proof artifact for prefix match"
            );
            return false;
        }
    };
    proof
        .proof
        .as_deref()
        .is_some_and(|proof| proof_hex_starts_with(proof, prefix))
}

fn proof_hex_starts_with(proof: &str, prefix: &str) -> bool {
    proof
        .as_bytes()
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
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
    submission: &'a CanonicalBatchSubmission,
}

impl<'a> ProposalIdentity<'a> {
    const fn new(submission: &'a CanonicalBatchSubmission) -> Self {
        Self { submission }
    }
}

impl RequestIdentity for ProposalIdentity<'_> {
    const DOMAIN: &'static str = "proof/proposal:v1";

    fn write_identity(&self, sink: &mut FingerprintSink) {
        let submission = self.submission;
        // Only client-supplied request data belongs in the idempotency key. `route` and
        // `prover_type` are server-derived and must not affect poll-by-re-POST behavior.
        sink.str("network_pair", &submission.pair.key);
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

fn proposal_request_fingerprint(submission: &CanonicalBatchSubmission) -> RequestFingerprint {
    ProposalIdentity::new(submission).fingerprint()
}

async fn submit_submission(
    state: &AppState,
    submission: &CanonicalBatchSubmission,
    request_fingerprint: &str,
) -> Result<(), Error> {
    // Deterministic v4 task IDs are reusable only when the normalized request fingerprint matches.
    if let Some(existing) = state
        .runtime
        .get_task(&submission.public_task_id)
        .await
        .map_err(|err| Error::from_api_error(ApiError::internal(err.to_string())))?
    {
        if existing.request_fingerprint.as_deref() != Some(request_fingerprint) {
            // New task ids are fingerprint-derived, so this branch should only be reachable for
            // stale/manual rows or an actual hash collision. Failed/cancelled rows may be replaced;
            // active/completed rows must not be silently overwritten.
            if matches!(
                existing.runner_status,
                raiko2_runtime::RunnerStatus::Failed | raiko2_runtime::RunnerStatus::Cancelled
            ) {
                // Gate on backend availability BEFORE replacing the row: replace_existing_batch_task
                // mutates runtime state and only resolves the engine late (in
                // handle_created_batch_task), so without this an unavailable backend would mutate the
                // terminal slot and return 404 task_not_found instead of 400 unsupported_proof_type —
                // reopening F5 through the F3 replacement path.
                ensure_engine_available(
                    state,
                    &submission.pair.key,
                    submission.route.pipeline_key(),
                    submission.requested_proof_type.as_str(),
                )?;
                let existing_metadata =
                    parse_task_metadata(&existing).map_err(Error::from_api_error)?;
                replace_existing_batch_task(
                    state,
                    submission,
                    &existing,
                    &existing_metadata,
                    Some(request_fingerprint),
                )
                .await
                .map_err(Error::from_api_error)?;
                return Ok(());
            }
            return Err(Error::request_conflict(
                "same proof task id was submitted with different proof input",
            ));
        }
        handle_existing_batch_task(state, submission, existing, Some(request_fingerprint))
            .await
            .map_err(Error::from_api_error)?;
        return Ok(());
    }

    ensure_engine_available(
        state,
        &submission.pair.key,
        submission.route.pipeline_key(),
        submission.requested_proof_type.as_str(),
    )?;
    let plan = build_submission_plan(&state.runtime, submission, request_fingerprint)
        .await
        .map_err(Error::from_api_error)?;
    match register_batch_task(state, submission, &plan, request_fingerprint)
        .await
        .map_err(Error::from_api_error)?
    {
        raiko2_runtime::TaskRegistrationOutcome::Created(_) => {
            handle_created_batch_task(state, submission, &plan)
                .await
                .map_err(Error::from_api_error)?;
        }
        raiko2_runtime::TaskRegistrationOutcome::Existing(existing) => {
            if existing.request_fingerprint.as_deref() != Some(request_fingerprint) {
                return Err(Error::request_conflict(
                    "same proof task id was submitted with different proof input",
                ));
            }
            handle_existing_batch_task(state, submission, existing, Some(request_fingerprint))
                .await
                .map_err(Error::from_api_error)?;
        }
    }
    Ok(())
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
mod tests {
    use super::super::{AggregateStatus, ProposalStatus, RootRuntime, RuntimeRunnerStatus};
    use super::*;
    use crate::config::Config;
    use crate::server::state::{EngineQueueTaskView, EngineStatusView, StaticPipelineFactory};
    use crate::server::task_metadata::ProposalTask;
    use raiko2_engine::{
        AggregateProofInput, AggregationTaskRequest, EngineTaskId, ProposalTaskRequest,
        ProverTaskConfig,
    };
    use raiko2_primitives::L2BlockRange;
    use raiko2_queue::TaskStoreError;
    use raiko2_runtime::{
        ProofArtifactRegistration, RunnerStatus as RuntimeTaskRunnerStatus, RuntimeManager,
        TaskRegistration,
    };
    use std::{future::Future, path::PathBuf, pin::Pin, process, sync::Arc, time::SystemTime};

    type TestBoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

    struct RemoveFailEngine;

    impl EngineHandle for RemoveFailEngine {
        fn submit_proposal_proof_with_dependencies(
            &self,
            _request: ProposalTaskRequest,
            _dependencies: Vec<EngineTaskId>,
        ) -> TestBoxFuture<'_, Result<EngineTaskId, TaskStoreError>> {
            Box::pin(async { panic!("unexpected proposal submission") })
        }

        fn submit_aggregation_proof_from_inputs(
            &self,
            _request: AggregationTaskRequest,
            _inputs: Vec<AggregateProofInput>,
        ) -> TestBoxFuture<'_, Result<EngineTaskId, TaskStoreError>> {
            Box::pin(async { panic!("unexpected aggregate submission") })
        }

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

        fn cancel(&self, _id: EngineTaskId) -> TestBoxFuture<'_, Result<(), TaskStoreError>> {
            Box::pin(async { Ok(()) })
        }

        fn remove(&self, _id: EngineTaskId) -> TestBoxFuture<'_, Result<(), TaskStoreError>> {
            Box::pin(async {
                Err(TaskStoreError::backend(std::io::Error::other(
                    "engine remove failed",
                )))
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
    async fn remove_invalidated_tasks_keeps_runtime_row_when_child_cleanup_fails() {
        let mut factory = StaticPipelineFactory::default();
        factory.insert(
            "taiko_dev/taiko_dev_l1",
            PipelineKey::ShastaSp1,
            Arc::new(RemoveFailEngine),
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
        let metadata = task_log_metadata_with_requests(&[55], false);
        let mut record = runtime
            .register_task(TaskRegistration {
                task_id: "task_child_cleanup_failure".to_string(),
                pipeline_key: Some(PipelineKey::ShastaSp1),
                route: PipelineKey::ShastaSp1.route(),
                task_kind: "hoodi_proposal".to_string(),
                proposal_id: Some(55),
                proof_ids: vec![metadata.proposals[0].task_id.clone()],
                metadata: serde_json::to_value(&metadata).expect("serialize metadata"),
                request_fingerprint: None,
            })
            .await
            .expect("register runtime task");
        record.runner_status = RuntimeTaskRunnerStatus::Completed;
        runtime.upsert_task(&record).await.expect("upsert task");

        let artifact_refs = invalidation_artifact_refs(&metadata, record.pipeline_key, None);
        let root_artifact_refs = root_invalidation_artifact_refs(&metadata, record.pipeline_key);
        let expected_blocked_refs = artifact_refs
            .iter()
            .chain(root_artifact_refs.iter())
            .cloned()
            .collect::<HashSet<_>>();
        let mut data = wire::InvalidateArtifactsData::default();
        let blocked_refs = remove_invalidated_tasks(
            &state,
            vec![CandidateInvalidationTask {
                record,
                metadata,
                artifact_refs,
                root_artifact_refs,
                root_proof_matches_prefix: false,
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
    async fn invalidate_artifacts_keeps_artifact_record_when_child_cleanup_fails() {
        let mut factory = StaticPipelineFactory::default();
        factory.insert(
            "taiko_dev/taiko_dev_l1",
            PipelineKey::ShastaSp1,
            Arc::new(RemoveFailEngine),
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
        let mut metadata = task_log_metadata_with_requests(&[56], false);
        metadata.requested_proof_type = Some("sp1".to_string());
        let mut record = runtime
            .register_task(TaskRegistration {
                task_id: "task_artifact_cleanup_failure".to_string(),
                pipeline_key: Some(PipelineKey::ShastaSp1),
                route: PipelineKey::ShastaSp1.route(),
                task_kind: "hoodi_proposal".to_string(),
                proposal_id: Some(56),
                proof_ids: vec![metadata.proposals[0].task_id.clone()],
                metadata: serde_json::to_value(&metadata).expect("serialize metadata"),
                request_fingerprint: None,
            })
            .await
            .expect("register runtime task");
        record.runner_status = RuntimeTaskRunnerStatus::Completed;
        runtime.upsert_task(&record).await.expect("upsert task");

        let root_refs = root_invalidation_artifact_refs(&metadata, record.pipeline_key);
        let proof_ref = root_refs
            .iter()
            .next()
            .expect("root proof ref")
            .proof_ref
            .clone();
        let proof_path = runtime.proof_artifact_path(&metadata.network_pair, &proof_ref);
        tokio::fs::create_dir_all(proof_path.parent().expect("proof dir"))
            .await
            .expect("create proof dir");
        tokio::fs::write(
            &proof_path,
            serde_json::to_vec(&Proof {
                proof: Some("0xroot".to_string()),
                ..Proof::default()
            })
            .expect("serialize proof"),
        )
        .await
        .expect("write proof artifact");
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: metadata.network_pair.clone(),
                proof_ref: proof_ref.clone(),
                pipeline_key: record.pipeline_key,
                route: record.route,
                proof_path: proof_path.display().to_string(),
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
                .get_proof_artifact(&metadata.network_pair, &proof_ref)
                .await
                .expect("get proof artifact")
                .is_some(),
            "proof artifact record was removed while task cleanup failed"
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
                proof_path: Some("proposal-path".to_string()),
                error: None,
                runtime: None,
                extra_data: None,
            }],
            aggregate: Some(AggregateStatus {
                task_id: "aggregate-task".to_string(),
                status: ProofStatus::Completed,
                proof: Some("0xaggregate".to_string()),
                proof_ref: Some("aggregate-ref".to_string()),
                proof_path: Some("aggregate-path".to_string()),
                error: None,
                runtime: None,
                extra_data: None,
            }),
            proof: Some("0xroot".to_string()),
            proof_ref: Some("root-ref".to_string()),
            proof_path: Some("root-path".to_string()),
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
        task_log_metadata_inner(proposal_ids, aggregate, false)
    }

    fn task_log_metadata_with_requests(proposal_ids: &[u64], aggregate: bool) -> TaskMetadata {
        task_log_metadata_inner(proposal_ids, aggregate, true)
    }

    fn task_log_metadata_inner(
        proposal_ids: &[u64],
        aggregate: bool,
        include_requests: bool,
    ) -> TaskMetadata {
        TaskMetadata {
            network_pair: "taiko_dev/taiko_dev_l1".to_string(),
            network: "taiko_dev".to_string(),
            l1_network: "taiko_dev_l1".to_string(),
            proof_type: raiko2_primitives::ProofType::Sp1,
            requested_proof_type: None,
            prover_type: None,
            execution_mode: None,
            aggregate_requested: aggregate,
            proposals: proposal_ids
                .iter()
                .map(|proposal_id| ProposalTask {
                    proposal_id: *proposal_id,
                    checkpoint: None,
                    l1_inclusion_block_number: proposal_id + 100,
                    l2_block_numbers: vec![proposal_id + 1],
                    last_anchor_block_number: proposal_id.saturating_sub(1),
                    task_id: format!("proposal-task-{proposal_id}"),
                    request: include_requests.then(|| ProposalTaskRequest {
                        proposal_id: *proposal_id,
                        l2_block_range: Some(L2BlockRange {
                            start: *proposal_id,
                            end: *proposal_id,
                        }),
                        l1_inclusion_block_number: proposal_id + 100,
                        last_anchor_block_number: proposal_id.saturating_sub(1),
                        checkpoint: None,
                        blob_proof_type: None,
                        prover: None,
                        graffiti: None,
                        prover_config: ProverTaskConfig::default(),
                    }),
                })
                .collect(),
            aggregate_task_id: aggregate.then(|| "aggregate-task".to_string()),
            aggregate_request: None,
            aggregate_input_artifacts: Vec::new(),
            runtime: Default::default(),
        }
    }

    fn test_runtime_root(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        std::env::temp_dir().join(format!("raiko2-{label}-{}-{nanos}", process::id()))
    }
}

#[cfg(test)]
pub(super) fn proposal_request_fingerprint_for_test(
    submission: &CanonicalBatchSubmission,
) -> Result<String, Error> {
    Ok(proposal_request_fingerprint(submission)
        .as_str()
        .to_string())
}
