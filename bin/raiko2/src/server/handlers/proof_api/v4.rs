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
use std::sync::Arc;

use super::super::proof_types::v4 as wire;
use super::{
    ApiData, ApiError, ApiOk, AppState, BatchProofType, BatchShastaRequest,
    CanonicalBatchSubmission, ClearProverStatus, EngineHandle, ProofStatus, ProverStatus,
    ProverTaskScope, ProverType, PublicProverArgs, ServerAclFeature, ShastaProposal, TaskData,
    authorize_acl_feature_with_rate_limit, authorize_optional_acl_feature_with_rate_limit,
    build_canonical_batch_submission, build_submission_plan, clear_prover_tasks,
    collect_prover_status, handle_created_batch_task, handle_existing_batch_task, load_task_data,
    parse_task_metadata, register_batch_task, replace_existing_batch_task, resolve_engine,
};

// Bound client-supplied inclusive ranges before materializing them into Vecs.
const MAX_RANGE_LEN: u64 = 100_000;

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
    let (proposal_id_start, proposal_id_end) = proposal_id_range(&req.proposals)?;
    let submission = proposal_submission(&state, &req, proposal_id_start, proposal_id_end)?;
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

fn proof_task_id(
    proof_type: wire::ProofType,
    aggregate: bool,
    proposal_id_start: u64,
    proposal_id_end: u64,
) -> String {
    let kind = if aggregate {
        "proposal_aggregation"
    } else {
        "proposal"
    };
    format!(
        "v4:{kind}:{}:{proposal_id_start}:{proposal_id_end}",
        proof_type.as_str()
    )
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

fn proposal_submission(
    state: &AppState,
    req: &wire::ProofRequest,
    proposal_id_start: u64,
    proposal_id_end: u64,
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

    let mut submission = build_canonical_batch_submission(state, batch_req)
        .map_err(Error::from_api_error)?
        .ok_or_else(|| Error::unsupported_proof_type("proof type was not selected"))?;
    submission.public_task_id = proof_task_id(
        proof_type,
        req.aggregate,
        proposal_id_start,
        proposal_id_end,
    );
    Ok(submission)
}

fn proposal_request_fingerprint(submission: &CanonicalBatchSubmission) -> Result<String, Error> {
    // Only client-supplied request data belongs in the idempotency key, not the caller's raw JSON
    // shape. `route` and `prover_type` are derived from server prover config; including them made a
    // benign config change (mock->real, local->network) mint a different fingerprint for a
    // byte-identical client request, permanently 409-ing the documented poll-by-re-POST.
    let payload = serde_json::json!({
        "api": "v4/proof/proposal",
        "network_pair": submission.pair.key,
        "requested_proof_type": submission.requested_proof_type.as_str(),
        "prover": submission.prover.as_deref(),
        "proposals": submission.proposals,
        "aggregate_requested": submission.aggregate_requested,
    });
    let encoded = serde_json::to_vec(&payload)
        .map_err(|err| Error::invalid_request(format!("failed to serialize request: {err}")))?;
    Ok(hex::encode_prefixed(keccak256(encoded).as_slice()))
}

fn legacy_proposal_request_fingerprint(
    submission: &CanonicalBatchSubmission,
) -> Result<String, Error> {
    // Pre-F3 fingerprint shape: it also included the server-derived `route` and `prover_type`. Kept so
    // a v4 proposal row registered before F3 is still recognized as the same request on re-POST across
    // a rolling upgrade (when the prover config is unchanged), instead of 409-ing on the new shape.
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
        let legacy_fingerprint = legacy_proposal_request_fingerprint(submission)?;
        if existing.request_fingerprint.as_deref() != Some(request_fingerprint)
            && existing.request_fingerprint.as_deref() != Some(legacy_fingerprint.as_str())
        {
            // A stored legacy (pre-F3) fingerprint for the same request is treated as a match above,
            // so an in-flight/completed row registered before this deploy still polls by re-POST.
            // The v4 task id pins (proof_type, proposal range). If the previous attempt for that
            // slot terminally failed or was cancelled, let a corrected resubmission replace it
            // (e.g. an L1 reorg changed l1_inclusion_block_number) instead of wedging the slot with
            // a permanent 409. Completed and in-flight tasks still conflict.
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
    let proposals = data
        .proposals
        .into_iter()
        .map(|proposal| {
            let l2_block_number_start = proposal
                .l2_block_numbers
                .first()
                .copied()
                .unwrap_or_default();
            let l2_block_number_end = proposal
                .l2_block_numbers
                .last()
                .copied()
                .unwrap_or(l2_block_number_start);
            wire::ProofProposalData {
                index: proposal.index,
                proposal_id: proposal.proposal_id,
                task_id: proposal.task_id,
                status: proof_status_string(&proposal.status),
                l1_inclusion_block_number: proposal.l1_inclusion_block_number,
                l2_block_number_start,
                l2_block_number_end,
                last_anchor_block_number: proposal.last_anchor_block_number,
                proof: proposal.proof,
                error: proposal.error,
            }
        })
        .collect();
    let aggregate = data.aggregate.map(|aggregate| wire::ProofAggregateData {
        task_id: aggregate.task_id,
        status: proof_status_string(&aggregate.status),
        proof: aggregate.proof,
        error: aggregate.error,
    });

    wire::ProofTaskData {
        status: proof_status_string(&data.status),
        proof: data.proof,
        error: data.error,
        current_index: data.current_index,
        proposals,
        aggregate,
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
    fn v4_proof_task_data_projects_stage_statuses_and_proofs() {
        let data = TaskData {
            task_id: "v4:proposal_aggregation:sp1:10:10".to_string(),
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
        assert_eq!(response.proof.as_deref(), Some("0xroot"));
        assert_eq!(response.current_index, Some(1));
        assert_eq!(response.proposals.len(), 1);
        assert_eq!(response.proposals[0].status, "completed");
        assert_eq!(response.proposals[0].l2_block_number_start, 20);
        assert_eq!(response.proposals[0].l2_block_number_end, 21);
        assert_eq!(response.proposals[0].proof.as_deref(), Some("0xproposal"));
        let aggregate = response.aggregate.expect("aggregate status");
        assert_eq!(aggregate.status, "completed");
        assert_eq!(aggregate.proof.as_deref(), Some("0xaggregate"));
    }
}

#[cfg(test)]
pub(super) fn proposal_request_fingerprint_for_test(
    submission: &CanonicalBatchSubmission,
) -> Result<String, Error> {
    proposal_request_fingerprint(submission)
}
