use alloy_primitives::{hex, keccak256};
use axum::{
    Json,
    body::Body,
    extract::{
        FromRequest, Path, Query, State, rejection::JsonRejection, rejection::QueryRejection,
    },
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
};
use raiko2_prover::sp1_config::Sp1RequestContext;
use raiko2_runtime::RuntimeTaskRecord;
use std::collections::HashMap;

use super::super::proof_types::v4 as wire;
use super::{
    AggregateProofRequest, ApiData, ApiError, ApiOk, AppState, BatchProofType, BatchShastaRequest,
    CanonicalBatchSubmission, CanonicalProofRoute, ClearProverStatus, ExternalAggregateSubmission,
    PlannedAggregateTask, ProofArtifactMaterial, ProofStatus, ProverStatus, ProverTaskScope,
    ProverType, PublicProverArgs, ResolvedNetworkPair, ServerAclFeature, ShastaProposal, TaskData,
    aggregate_task_ref, augment_system_prover_config, authorize_acl_feature_with_rate_limit,
    build_canonical_batch_submission, build_external_aggregate_submission, build_submission_plan,
    clear_prover_tasks, collect_prover_status, handle_created_batch_task,
    handle_created_external_aggregate_task, handle_existing_batch_task,
    handle_existing_external_aggregate_task, load_proof_artifact_material, load_task_data,
    parse_task_metadata, prover_type_for_proof_type, register_batch_task,
    register_external_aggregate_task, resolve_engine, resolved_pair, route_for_proof_type,
    validate_aggregate_route_specific_request, validate_public_prover_args,
};

// Bound client-supplied inclusive ranges before materializing them into Vecs.
pub(super) const MAX_RANGE_LEN: u64 = 100_000;

pub(crate) async fn request_proposal_proof(
    State(state): State<AppState>,
    headers: HeaderMap,
    req: Request<Body>,
) -> Result<Json<wire::TaskResponse<wire::ProofTaskData>>, Error> {
    authorize_acl_feature_with_rate_limit(&state, &headers, ServerAclFeature::ProverSubmit)
        .map_err(Error::from_api_error)?;
    let Json(req) = Json::<wire::ProposalRequest>::from_request(req, &state)
        .await
        .map_err(|err| Error::from_json_rejection(&err))?;
    let proof_type = req.proof_type;
    let proposal_id_start = req.proposal_id_start;
    let proposal_id_end = req.proposal_id_end;
    let submission = proposal_submission(&state, &req)?;
    let request_fingerprint = proposal_request_fingerprint(&submission)?;
    submit_submission(&state, &submission, &request_fingerprint).await?;
    let data = load_task_data(&state, &submission.public_task_id)
        .await
        .map_err(Error::from_api_error)?;
    Ok(Json(wire::TaskResponse {
        status: "ok",
        proof_type: proof_type.as_str().to_string(),
        proposal_id_start,
        proposal_id_end,
        data: proposal_task_data(&state, data).await?,
    }))
}

pub(crate) async fn request_aggregation_proof(
    State(state): State<AppState>,
    headers: HeaderMap,
    req: Request<Body>,
) -> Result<Json<wire::TaskResponse<wire::AggregationTaskData>>, Error> {
    authorize_acl_feature_with_rate_limit(&state, &headers, ServerAclFeature::ProverSubmit)
        .map_err(Error::from_api_error)?;
    let Json(req) = Json::<wire::AggregationRequest>::from_request(req, &state)
        .await
        .map_err(|err| Error::from_json_rejection(&err))?;
    let proof_type = req.proof_type;
    let proposal_id_start = req.proposal_id_start;
    let proposal_id_end = req.proposal_id_end;
    let task_id = aggregation_task_id(proof_type, proposal_id_start, proposal_id_end);
    collect_inclusive_range(
        proposal_id_start,
        proposal_id_end,
        "proposal_id_start",
        "proposal_id_end",
    )?;
    if state
        .runtime
        .get_task(&task_id)
        .await
        .map_err(|err| Error::from_api_error(ApiError::internal(err.to_string())))?
        .is_some()
    {
        let data = load_task_data(&state, &task_id)
            .await
            .map_err(Error::from_api_error)?;
        return Ok(Json(wire::TaskResponse {
            status: "ok",
            proof_type: proof_type.as_str().to_string(),
            proposal_id_start,
            proposal_id_end,
            data: aggregation_task_data(&state, data).await?,
        }));
    }
    let submission = aggregation_submission(&state, req).await?;
    submit_external_aggregation(&state, &submission).await?;
    let data = load_task_data(&state, &submission.public_task_id)
        .await
        .map_err(Error::from_api_error)?;
    Ok(Json(wire::TaskResponse {
        status: "ok",
        proof_type: proof_type.as_str().to_string(),
        proposal_id_start,
        proposal_id_end,
        data: aggregation_task_data(&state, data).await?,
    }))
}

pub(crate) async fn get_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<ApiData<TaskData>>, Error> {
    authorize_acl_feature_with_rate_limit(&state, &headers, ServerAclFeature::ProverSubmit)
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

fn collect_inclusive_range(
    start: u64,
    end: u64,
    start_field: &'static str,
    end_field: &'static str,
) -> Result<Vec<u64>, Error> {
    // V4 accepts compact inclusive ranges; internal batch paths consume explicit IDs.
    if end < start {
        return Err(Error::invalid_request(format!(
            "{end_field} must be greater than or equal to {start_field}"
        )));
    }

    let len = end
        .checked_sub(start)
        .and_then(|delta| delta.checked_add(1))
        .ok_or_else(|| {
            Error::invalid_request(format!(
                "{start_field}..={end_field} range length overflows u64"
            ))
        })?;
    if len > MAX_RANGE_LEN {
        return Err(Error::invalid_request(format!(
            "{start_field}..={end_field} range length {len} exceeds maximum {MAX_RANGE_LEN}"
        )));
    }

    Ok((start..=end).collect())
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

    fn dependency_not_ready(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "dependency_not_ready", message)
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
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(wire::ApiErrorBody {
                status: "error",
                error: self.code,
                message: self.message,
            }),
        )
            .into_response()
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

fn proposal_task_id(
    proof_type: wire::ProofType,
    proposal_id_start: u64,
    proposal_id_end: u64,
) -> String {
    format!(
        "v4:proposal:{}:{proposal_id_start}:{proposal_id_end}",
        proof_type.as_str()
    )
}

fn aggregation_task_id(
    proof_type: wire::ProofType,
    proposal_id_start: u64,
    proposal_id_end: u64,
) -> String {
    format!(
        "v4:aggregation:{}:{proposal_id_start}:{proposal_id_end}",
        proof_type.as_str()
    )
}

fn proposal_submission(
    state: &AppState,
    req: &wire::ProposalRequest,
) -> Result<CanonicalBatchSubmission, Error> {
    // Translate v4 proposal requests into the canonical batch path so routing and
    // metadata stay single-sourced.
    let proof_type = req.proof_type;
    if req.proposal_id_end != req.proposal_id_start {
        return Err(Error::invalid_request(
            "proposal_id_end must equal proposal_id_start for proposal proofs",
        ));
    }
    let proposal_id = req.proposal_id_start;
    let l2_block_numbers = collect_inclusive_range(
        req.l2_block_number_start,
        req.l2_block_number_end,
        "l2_block_number_start",
        "l2_block_number_end",
    )?;
    let batch_req = BatchShastaRequest {
        proposals: vec![ShastaProposal {
            proposal_id,
            checkpoint: req.checkpoint,
            l1_inclusion_block_number: req.l1_inclusion_block_number,
            l2_block_numbers,
            last_anchor_block_number: req.last_anchor_block_number,
        }],
        proof_type: batch_proof_type(proof_type),
        aggregate: false,
        prover: req.prover.map(|addr| addr.to_string()),
        network: None,
        l1_network: None,
        blob_proof_type: None,
        graffiti: None,
        prover_args: PublicProverArgs::default(),
    };

    let mut submission = build_canonical_batch_submission(state, batch_req)
        .map_err(Error::from_api_error)?
        .ok_or_else(|| Error::unsupported_proof_type("proof type was not selected"))?;
    submission.public_task_id =
        proposal_task_id(proof_type, req.proposal_id_start, req.proposal_id_end);
    Ok(submission)
}

async fn aggregation_submission(
    state: &AppState,
    req: wire::AggregationRequest,
) -> Result<ExternalAggregateSubmission, Error> {
    // V4 aggregation is local-first: it aggregates proposal proof artifacts already
    // known to this runtime.
    let proof_type = req.proof_type;
    let aggregation_ids = collect_inclusive_range(
        req.proposal_id_start,
        req.proposal_id_end,
        "proposal_id_start",
        "proposal_id_end",
    )?;
    let target = resolve_aggregation_target(state, proof_type)?;
    let records = state
        .runtime
        .list_tasks()
        .await
        .map_err(|err| Error::from_api_error(ApiError::internal(err.to_string())))?;
    let proposal_proofs = local_proposal_proof_index(&records, proof_type, &target);
    let mut proofs = Vec::with_capacity(aggregation_ids.len());
    for proposal_id in &aggregation_ids {
        proofs.push(
            local_proposal_proof(state, &proposal_proofs, proof_type, *proposal_id)
                .await?
                .proof,
        );
    }

    let aggregate_req = AggregateProofRequest {
        aggregation_ids,
        proofs,
        proof_type: batch_proof_type(proof_type),
        network: None,
        l1_network: None,
        graffiti: None,
        prover: None,
        blob_proof_type: None,
        prover_args: PublicProverArgs::default(),
    };
    let mut submission = build_external_aggregate_submission(state, aggregate_req)
        .await
        .map_err(Error::from_api_error)?;
    submission.public_task_id =
        aggregation_task_id(proof_type, req.proposal_id_start, req.proposal_id_end);
    Ok(submission)
}

#[derive(Debug)]
struct LocalProposalProofRef {
    network_pair: String,
    proof_ref: String,
}

struct AggregationTarget {
    pair: ResolvedNetworkPair,
    route: CanonicalProofRoute,
}

fn resolve_aggregation_target(
    state: &AppState,
    proof_type: wire::ProofType,
) -> Result<AggregationTarget, Error> {
    let proof_type = batch_proof_type(proof_type);
    let pair = resolved_pair(state, None, None).map_err(Error::from_api_error)?;
    let prover_config = augment_system_prover_config(
        &pair,
        validate_public_prover_args(proof_type, &PublicProverArgs::default())
            .map_err(Error::from_api_error)?,
    );
    let sp1_context = Sp1RequestContext::Aggregation;
    let route = route_for_proof_type(state, proof_type, &prover_config, sp1_context)
        .map_err(Error::from_api_error)?;
    let _prover_type: Option<ProverType> =
        prover_type_for_proof_type(state, proof_type, route.route, &prover_config, sp1_context)
            .map_err(Error::from_api_error)?;
    validate_aggregate_route_specific_request(state, &pair, route.proof_type(), &prover_config)
        .map_err(Error::from_api_error)?;

    Ok(AggregationTarget { pair, route })
}

fn local_proposal_proof_index(
    records: &[RuntimeTaskRecord],
    proof_type: wire::ProofType,
    target: &AggregationTarget,
) -> HashMap<u64, Vec<LocalProposalProofRef>> {
    let mut index: HashMap<u64, Vec<LocalProposalProofRef>> = HashMap::new();
    for record in records {
        let Ok(metadata) = parse_task_metadata(record) else {
            continue;
        };
        if metadata.network_pair != target.pair.key
            || record.pipeline_key != target.route.pipeline_key()
            || record.route != target.route.route
        {
            continue;
        }
        if metadata.requested_proof_type.as_deref() != Some(proof_type.as_str()) {
            continue;
        }

        for proposal in &metadata.proposals {
            let Some(request) = proposal.request.as_ref() else {
                continue;
            };
            index
                .entry(request.proposal_id)
                .or_default()
                .push(LocalProposalProofRef {
                    network_pair: metadata.network_pair.clone(),
                    proof_ref: proposal.task_id.clone(),
                });
        }
    }
    index
}

async fn local_proposal_proof(
    state: &AppState,
    proposals: &HashMap<u64, Vec<LocalProposalProofRef>>,
    proof_type: wire::ProofType,
    proposal_id: u64,
) -> Result<ProofArtifactMaterial, Error> {
    if let Some(candidates) = proposals.get(&proposal_id) {
        for candidate in candidates {
            let material = load_proof_artifact_material(
                &state.runtime,
                &candidate.network_pair,
                &candidate.proof_ref,
            )
            .await
            .map_err(|err| {
                Error::from_api_error(ApiError::internal(format!(
                    "failed to load proposal proof artifact: {err}"
                )))
            })?;
            if let Some(material) = material {
                return Ok(material);
            }
        }
    }

    Err(Error::dependency_not_ready(format!(
        "proposal proof {proposal_id} for proof_type={} is not completed in local state",
        proof_type.as_str()
    )))
}

fn proposal_request_fingerprint(submission: &CanonicalBatchSubmission) -> Result<String, Error> {
    // Use normalized submission data as the idempotency key, not the caller's raw JSON shape.
    let payload = serde_json::json!({
        "api": "v4/proof/proposal",
        "network_pair": submission.pair.key,
        "route": submission.route.route.to_string(),
        "requested_proof_type": submission.requested_proof_type.as_str(),
        "prover_type": submission.prover_type.map(ProverType::as_str),
        "prover": submission.prover.as_deref(),
        "proposals": submission.proposals,
        "aggregate_requested": submission.aggregate_requested,
    });
    let encoded = serde_json::to_vec(&payload)
        .map_err(|err| Error::invalid_request(format!("failed to serialize request: {err}")))?;
    Ok(hex::encode_prefixed(keccak256(encoded).as_slice()))
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
            return Err(Error::request_conflict(
                "same proof task key was submitted with different proof input",
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
        submission.requested_proof_type,
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
                    "same proof task key was submitted with different proof input",
                ));
            }
            handle_existing_batch_task(state, submission, existing, Some(request_fingerprint))
                .await
                .map_err(Error::from_api_error)?;
        }
    }
    Ok(())
}

async fn submit_external_aggregation(
    state: &AppState,
    submission: &ExternalAggregateSubmission,
) -> Result<(), Error> {
    // Aggregation has the same idempotency rule as proposal proving: same key, same inputs.
    if let Some(existing) = state
        .runtime
        .get_task(&submission.public_task_id)
        .await
        .map_err(|err| Error::from_api_error(ApiError::internal(err.to_string())))?
    {
        if existing.request_fingerprint.as_deref() != Some(submission.request_fingerprint.as_str())
        {
            return Err(Error::request_conflict(
                "same aggregation task key was submitted with different proof input",
            ));
        }
        let engine = resolve_engine(state, &submission.pair.key, submission.route.pipeline_key())
            .map_err(Error::from_api_error)?;
        handle_existing_external_aggregate_task(state, &engine, submission, existing)
            .await
            .map_err(Error::from_api_error)?;
        return Ok(());
    }

    let aggregate = PlannedAggregateTask {
        task_ref: aggregate_task_ref(submission.route.pipeline_key(), &submission.request),
        task_id: submission.task_id.clone(),
        request: submission.request.clone(),
    };
    let engine = resolve_engine(state, &submission.pair.key, submission.route.pipeline_key())
        .map_err(Error::from_api_error)?;
    register_external_aggregate_task(state, submission, &aggregate)
        .await
        .map_err(Error::from_api_error)?;
    handle_created_external_aggregate_task(state, &engine, submission, &aggregate)
        .await
        .map_err(Error::from_api_error)?;
    Ok(())
}

fn ensure_engine_available(
    state: &AppState,
    pair_key: &str,
    pipeline_key: raiko2_pipeline::PipelineKey,
    proof_type: BatchProofType,
) -> Result<(), Error> {
    resolve_engine(state, pair_key, pipeline_key)
        .map(|_| ())
        .map_err(|err| match err.status {
            StatusCode::NOT_FOUND => Error::unsupported_proof_type(format!(
                "proof_type={} is not supported by this server route",
                proof_type.as_str()
            )),
            _ => Error::from_api_error(err),
        })
}

async fn proposal_task_data(
    state: &AppState,
    data: TaskData,
) -> Result<wire::ProofTaskData, Error> {
    let proposal = data
        .proposals
        .into_iter()
        .next()
        .ok_or_else(|| Error::invalid_request("proposal task did not contain a proposal"))?;
    let proof = proof_from_status(
        state,
        &data.task_id,
        &proposal.status,
        proposal.proof_ref.as_deref(),
        "proposal",
    )
    .await?;
    Ok(wire::ProofTaskData {
        task_id: data.task_id,
        status: proof_status_string(&proposal.status),
        proof,
    })
}

async fn aggregation_task_data(
    state: &AppState,
    data: TaskData,
) -> Result<wire::AggregationTaskData, Error> {
    let aggregate = data
        .aggregate
        .ok_or_else(|| Error::invalid_request("aggregation task did not contain aggregate data"))?;
    let proof = proof_from_status(
        state,
        &data.task_id,
        &aggregate.status,
        aggregate.proof_ref.as_deref(),
        "aggregation",
    )
    .await?;
    Ok(wire::AggregationTaskData {
        task_id: data.task_id,
        status: proof_status_string(&aggregate.status),
        proof,
    })
}

pub(super) async fn proof_from_status(
    state: &AppState,
    task_id: &str,
    status: &ProofStatus,
    proof_ref: Option<&str>,
    task_kind: &'static str,
) -> Result<Option<String>, Error> {
    match (matches!(status, ProofStatus::Completed), proof_ref) {
        (false, _) => Ok(None),
        (true, None) => Err(Error::from_api_error(ApiError::internal(format!(
            "completed {task_kind} task is missing proof artifact reference"
        )))),
        (true, Some(proof_ref)) => {
            let record = state
                .runtime
                .get_task(task_id)
                .await
                .map_err(|err| {
                    Error::from_api_error(ApiError::internal(format!(
                        "failed to load task metadata: {err}"
                    )))
                })?
                .ok_or_else(|| {
                    Error::from_api_error(ApiError::internal(format!(
                        "completed {task_kind} task was not found: {task_id}"
                    )))
                })?;
            let metadata = parse_task_metadata(&record).map_err(Error::from_api_error)?;
            // TaskData.proof is only a legacy status string. V4 exposes the
            // chain-submittable proof hex and leaves artifact details to task inspection.
            let material =
                load_proof_artifact_material(&state.runtime, &metadata.network_pair, proof_ref)
                    .await
                    .map_err(|err| {
                        Error::from_api_error(ApiError::internal(format!(
                            "failed to load completed {task_kind} proof artifact: {err}"
                        )))
                    })?
                    .ok_or_else(|| {
                        Error::from_api_error(ApiError::internal(format!(
                            "completed {task_kind} proof artifact not found: {proof_ref}"
                        )))
                    })?;
            material.proof.proof.map(Some).ok_or_else(|| {
                Error::from_api_error(ApiError::internal(format!(
                    "completed {task_kind} proof artifact is missing proof hex"
                )))
            })
        }
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
pub(super) fn collect_inclusive_range_for_test(
    start: u64,
    end: u64,
    start_field: &'static str,
    end_field: &'static str,
) -> Result<Vec<u64>, Error> {
    collect_inclusive_range(start, end, start_field, end_field)
}
