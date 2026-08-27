use axum::{
    Json,
    body::Body,
    extract::{
        FromRequest, Path, Query, State, rejection::JsonRejection, rejection::QueryRejection,
    },
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
};
#[cfg(test)]
use raiko2_pipeline::{PipelineKey, PipelineRoute};
use std::sync::Arc;

use super::super::proof_types::v4 as wire;
use super::{
    ApiData, ApiError, ApiOk, AppState, BatchProofType, BatchShastaRequest,
    CanonicalBatchSubmission, ClearProverStatus, EngineHandle, ProofStatus, ProverStatus,
    ProverTaskScope, PublicProverArgs, ServerAclFeature, ShastaProposal, TaskData,
    authorize_acl_feature_with_rate_limit, authorize_optional_acl_feature_with_rate_limit,
    build_canonical_batch_submission, build_submission_plan, clear_prover_tasks,
    collect_prover_status, handle_created_batch_task, handle_existing_batch_task, load_task_data,
    register_batch_task, replace_existing_batch_task, resolve_engine,
};
#[cfg(test)]
use super::{TaskMetadata, parse_task_metadata};
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
        wire::ProofType::Native => BatchProofType::Native,
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
    use crate::server::state::{EngineQueueTaskView, EngineStatusView, StaticPipelineFactory};
    use crate::server::task_metadata::RuntimeMetadata;
    use raiko2_engine::EngineTaskId;
    use raiko2_primitives::Proof;
    use raiko2_queue::TaskStoreError;
    use raiko2_runtime::{
        ProofArtifactRegistration, RunnerStatus as RuntimeTaskRunnerStatus, RuntimeManager,
        TaskRegistration,
    };
    use std::{future::Future, path::PathBuf, pin::Pin, process, sync::Arc, time::SystemTime};

    type TestBoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

    struct ActiveEngine;

    impl EngineHandle for ActiveEngine {
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
            Box::pin(async { Ok(true) })
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
            Box::pin(async move { Ok(raiko2_queue::DetachOutcome::not_attached(mode)) })
        }
    }

    #[test]
    fn native_request_uses_local_native_pipeline() {
        let runtime = Arc::new(
            RuntimeManager::new(test_runtime_root("native-v4-submission"))
                .expect("runtime manager"),
        );
        let state = AppState::from_parts(
            Arc::new(Config::default()),
            Arc::new(StaticPipelineFactory::default()),
            runtime,
        );
        let req = wire::ProofRequest {
            proof_type: wire::ProofType::Native,
            aggregate: true,
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

        let submission = proposal_submission(&state, &req).expect("canonical submission");

        assert!(matches!(
            submission.requested_proof_type,
            BatchProofType::Native
        ));
        assert_eq!(submission.route.pipeline_key(), PipelineKey::NativeLocal);
        assert_eq!(submission.route.route.to_string(), "native/local");
        assert!(submission.aggregate_requested);
    }

    #[tokio::test]
    async fn existing_submission_does_not_depend_on_a_global_lifecycle_operation() {
        let runtime = Arc::new(
            RuntimeManager::new(test_runtime_root("existing-submit-lifecycle"))
                .expect("runtime manager"),
        );
        let mut config = Config::default();
        config
            .prover
            .apply_routes_override(&"risc0/local".parse().expect("valid RISC0 route"));
        let state = AppState::from_parts(
            Arc::new(config),
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
    async fn repeated_post_and_get_keep_compressed_sp1_aggregate_pending_pollable() {
        let (state, task_id, request) =
            existing_sp1_aggregate_fixture("aggregate-pending-poll", false).await;

        for _ in 0..2 {
            let posted = post_proof_request(&state, &request).await;
            assert_eq!(posted.task_id, task_id);
            assert_eq!(posted.status, "work_in_progress");
            assert!(posted.proof.is_none());

            let task = get_proof_task(&state, &task_id).await;
            assert!(matches!(task.status, ProofStatus::Proving));
            assert_eq!(task.proposals.len(), 1);
            assert!(matches!(task.proposals[0].status, ProofStatus::Completed));
            assert!(task.proposals[0].proof.is_none());
            assert!(task.proposals[0].proof_ref.is_some());
            assert!(task.proposals[0].proof_uri.is_some());
            let aggregate = task.aggregate.expect("aggregate status");
            assert!(matches!(aggregate.status, ProofStatus::Proving));
            assert!(aggregate.proof.is_none());
            assert!(task.error.is_none());
        }
    }

    #[tokio::test]
    async fn repeated_post_and_get_return_completed_sp1_aggregate_proof() {
        let (state, task_id, request) =
            existing_sp1_aggregate_fixture("aggregate-completed-poll", true).await;

        for _ in 0..2 {
            let posted = post_proof_request(&state, &request).await;
            assert_eq!(posted.task_id, task_id);
            assert_eq!(posted.status, "completed");
            assert_eq!(posted.proof.as_deref(), Some("0xaggregate"));

            let task = get_proof_task(&state, &task_id).await;
            assert!(matches!(task.status, ProofStatus::Completed));
            assert_eq!(task.proof.as_deref(), Some("0xaggregate"));
            assert!(matches!(task.proposals[0].status, ProofStatus::Completed));
            assert!(task.proposals[0].proof.is_none());
            assert!(task.proposals[0].proof_ref.is_some());
            let aggregate = task.aggregate.expect("aggregate status");
            assert!(matches!(aggregate.status, ProofStatus::Completed));
            assert_eq!(aggregate.proof.as_deref(), Some("0xaggregate"));
            assert!(aggregate.proof_ref.is_some());
            assert!(aggregate.proof_uri.is_some());
            assert!(task.error.is_none());
        }
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

    async fn existing_sp1_aggregate_fixture(
        label: &str,
        completed: bool,
    ) -> (AppState, String, wire::ProofRequest) {
        let runtime =
            Arc::new(RuntimeManager::new(test_runtime_root(label)).expect("runtime manager"));
        let mut config = Config::default();
        config
            .prover
            .apply_routes_override(&"sp1/local".parse().expect("valid SP1 route"));
        config.prover.sp1.prover = raiko2_prover::sp1_config::ProverMode::Local;
        let mut factory = StaticPipelineFactory::default();
        factory.insert(
            "taiko_hoodi/hoodi",
            PipelineKey::Sp1Local,
            Arc::new(ActiveEngine),
        );
        let state = AppState::from_parts(Arc::new(config), Arc::new(factory), Arc::clone(&runtime));
        let request = wire::ProofRequest {
            proof_type: wire::ProofType::Sp1,
            aggregate: true,
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
        let mut submission = proposal_submission(&state, &request).expect("canonical submission");
        let fingerprint =
            proposal_request_fingerprint(runtime.environment(), runtime.namespace(), &submission)
                .expect("request fingerprint");
        submission.public_task_id = fingerprint.public_task_id();
        let plan =
            build_submission_plan(&submission, fingerprint.as_str()).expect("submission plan");
        let mut record = match register_batch_task(&state, &submission, &plan, fingerprint.as_str())
            .await
            .expect("register aggregate root")
        {
            raiko2_runtime::TaskRegistrationOutcome::Created(record) => record,
            raiko2_runtime::TaskRegistrationOutcome::Existing(_) => {
                panic!("fixture task id must be unique")
            }
        };
        let mut metadata = parse_task_metadata(&record).expect("task metadata");
        metadata.runtime.active_stage = Some("aggregate".to_string());
        metadata.runtime.last_event = Some("started:test".to_string());
        record.metadata = serde_json::to_value(&metadata).expect("serialize task metadata");
        record.runner_status = if completed {
            RuntimeTaskRunnerStatus::Completed
        } else {
            RuntimeTaskRunnerStatus::Running
        };
        runtime.upsert_task(&record).await.expect("update task");

        let compressed_proposal = Proof {
            input: Some(alloy_primitives::B256::ZERO),
            quote: Some(r#"{"Compressed":{}}"#.to_string()),
            uuid: Some("sp1-verifying-key".to_string()),
            extra_data: Some(serde_json::json!({ "shasta": {} })),
            ..Proof::default()
        };
        register_test_proof_artifact(
            runtime.as_ref(),
            &record.network_pair,
            record.pipeline_key,
            record.route,
            &plan.proposals[0].task_ref,
            &compressed_proposal,
        )
        .await;
        if completed {
            register_test_proof_artifact(
                runtime.as_ref(),
                &record.network_pair,
                record.pipeline_key,
                record.route,
                &plan.aggregate.as_ref().expect("aggregate task").task_ref,
                &Proof {
                    proof: Some("0xaggregate".to_string()),
                    ..Proof::default()
                },
            )
            .await;
        }

        (state, submission.public_task_id, request)
    }

    async fn post_proof_request(
        state: &AppState,
        request: &wire::ProofRequest,
    ) -> wire::ProofTaskData {
        let http_request = Request::builder()
            .method("POST")
            .uri("/v4/proof/proposal")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(request).expect("serialize proof request"),
            ))
            .expect("build proof request");
        match request_proposal_proof(State(state.clone()), HeaderMap::new(), http_request).await {
            Ok(Json(response)) => response.data,
            Err(error) => panic!("proof POST failed: {}", error.message),
        }
    }

    async fn get_proof_task(state: &AppState, task_id: &str) -> TaskData {
        match get_task(
            State(state.clone()),
            HeaderMap::new(),
            Path(task_id.to_string()),
        )
        .await
        {
            Ok(Json(response)) => response.data,
            Err(error) => panic!("proof GET failed: {}", error.message),
        }
    }

    async fn register_test_proof_artifact(
        runtime: &RuntimeManager,
        network_pair: &str,
        pipeline: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        proof: &Proof,
    ) {
        let bytes = serde_json::to_vec(proof).expect("serialize proof");
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
