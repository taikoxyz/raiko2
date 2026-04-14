use alloy_primitives::{hex, keccak256};
use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use raiko2_engine::{
    AggregateProofInput, AggregationTaskRequest, EngineTaskId, EngineTaskKey, ProofArtifactRef,
    ProposalTaskRequest, ProverTaskConfig,
};
use raiko2_pipeline::{PipelineKey, PipelineRoute, RunnerKind};
use raiko2_primitives::{L2BlockRange, Proof, ProofType};
use raiko2_primitives_shasta::instance::SHASTA_PROPOSAL_ID_MAX;
use raiko2_prover::sp1::{
    ExecutionMode as Sp1ExecutionMode, ProverMode as Sp1ProverMode, Sp1RemoteVerifyConfig,
    Sp1RequestContext, Sp1SystemConfig,
};
use raiko2_prover::validate_external_aggregate_proofs;
use raiko2_runtime::{
    RunnerStatus as RuntimeRunnerStatus, RuntimeManager, TaskRegistration, TaskRegistrationOutcome,
};
use std::sync::Arc;
use std::{collections::HashSet, future::Future};
use tokio::fs;
use tracing::{debug, info, warn};

use super::super::errors::ApiError;
use super::proof_route::{
    BatchProofDecision, CanonicalProofRoute, decide_batch_proof_type,
    public_task_id_from_fingerprint, route_for_proof_type,
};
use super::proof_types::{
    AggregateProofRequest, AggregateStatus, ApiOk, BatchProofType, BatchShastaRequest,
    CanonicalProposal, LegacyProofData, LegacyProofEnvelope, LegacyProofError, LegacyTaskStatus,
    ProposalStatus, PruneStatus, PublicProverArgs, RootRuntime, RootTaskState, ShastaProposal,
    TaskData, TaskRuntime,
};
use crate::config::ResolvedNetworkPair;
use crate::server::proof_artifact::{ProofArtifactMaterial, load_proof_artifact_material};
use crate::server::state::{AppState, EngineHandle, EngineStatusView, ProofStatus};
use crate::server::task_cleanup::{
    cancel_registered_tasks, proposal_task_chain_ids, proposal_task_id, remove_task_children,
};
use crate::server::task_metadata::{
    AggregateInputProofArtifact, BuildTaskMetadataParams, ProposalTask, ProverType,
    RuntimeMetadata, TaskMetadata, TaskRuntimeMetadata, aggregate_input_proof_ref,
    aggregate_task_ref, proposal_proof_artifact_refs, proposal_task_ref, root_proof_artifact_refs,
    stage_task_ref,
};
use crate::server::telemetry::{self, MetricContext};

#[derive(Clone)]
struct CanonicalBatchSubmission {
    public_task_id: String,
    pair: ResolvedNetworkPair,
    route: CanonicalProofRoute,
    proposals: Vec<CanonicalProposal>,
    aggregate_requested: bool,
    prover_config: ProverTaskConfig,
    prover_type: Option<ProverType>,
    blob_proof_type: Option<String>,
    prover: Option<String>,
    graffiti: Option<String>,
    execution_mode: Option<Sp1ExecutionMode>,
}

#[derive(Clone)]
struct PlannedProposalTask {
    request: ProposalTaskRequest,
    task_id: EngineTaskId,
    task_ref: String,
    proposal: CanonicalProposal,
}

#[derive(Clone)]
struct PlannedAggregateTask {
    request: AggregationTaskRequest,
    task_id: EngineTaskId,
    task_ref: String,
}

#[derive(Clone)]
struct SubmissionPlan {
    proposals: Vec<PlannedProposalTask>,
    proposal_sources: Vec<ProposalPlanSource>,
    aggregate: Option<PlannedAggregateTask>,
    aggregate_inputs: Vec<AggregateProofInput>,
}

#[derive(Clone)]
enum ProposalPlanSource {
    Pending,
    Cached,
}

struct ExternalAggregateSubmission {
    pair: ResolvedNetworkPair,
    route: CanonicalProofRoute,
    prover_type: Option<ProverType>,
    public_task_id: String,
    task_id: EngineTaskId,
    request: AggregationTaskRequest,
    inputs: Vec<AggregateProofInput>,
    input_artifacts: Vec<AggregateInputProofArtifact>,
    request_fingerprint: String,
}

struct TaskLookup {
    record: raiko2_runtime::RuntimeTaskRecord,
    metadata: TaskMetadata,
    engine: Arc<dyn EngineHandle>,
}

#[derive(Clone)]
struct ProofLocation {
    proof_ref: Option<String>,
    proof_path: Option<String>,
}

pub async fn request_batch_shasta_proof(
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
    let request_fingerprint = batch_request_fingerprint(&submission)?;
    submission.public_task_id = public_task_id_from_fingerprint(&request_fingerprint);
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

    match register_batch_task(&state, &submission, &plan, &request_fingerprint).await? {
        TaskRegistrationOutcome::Existing(existing) => {
            handle_existing_batch_task(&state, &submission, existing).await
        }
        TaskRegistrationOutcome::Created(_) => {
            handle_created_batch_task(&state, &submission, &plan).await
        }
    }
}

pub async fn request_aggregation_proof(
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
    let aggregate = planned_external_aggregate_task(&submission);

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

pub async fn get_task(
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

pub async fn cancel_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ApiOk<TaskData>>, ApiError> {
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

pub async fn report_proofs(State(state): State<AppState>) -> Result<Json<Vec<TaskData>>, ApiError> {
    let tasks = load_all_task_data(&state).await?;
    Ok(Json(tasks))
}

pub async fn list_proofs(State(state): State<AppState>) -> Result<Json<Vec<TaskData>>, ApiError> {
    let tasks = load_all_task_data(&state)
        .await?
        .into_iter()
        .filter(|task| matches!(task.status, ProofStatus::Completed) && task.proof.is_some())
        .collect::<Vec<_>>();
    Ok(Json(tasks))
}

pub async fn prune_proofs(State(state): State<AppState>) -> Result<Json<PruneStatus>, ApiError> {
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

        state
            .runtime
            .remove_task(&record.task_id)
            .await
            .map_err(|err| {
                ApiError::internal(format!("failed to prune task {}: {err}", record.task_id))
            })?;
    }

    Ok(Json(PruneStatus { status: "ok" }))
}

fn build_canonical_batch_submission(
    state: &AppState,
    req: BatchShastaRequest,
) -> Result<Option<CanonicalBatchSubmission>, ApiError> {
    validate_request_shape(&req)?;
    let pair = resolved_pair(state, req.network.as_deref(), req.l1_network.as_deref())?;
    let requested_prover_config = augment_system_prover_config(
        &pair,
        validate_public_prover_args(req.proof_type, &req.prover_args)?,
    );
    let proposals = req
        .proposals
        .iter()
        .map(canonicalize_proposal)
        .collect::<Result<Vec<_>, _>>()?;
    let selected_proof_type = match decide_batch_proof_type(state, &req)? {
        BatchProofDecision::Selected(proof_type) => proof_type,
        BatchProofDecision::NotDrawn => return Ok(None),
    };
    let sp1_context = Sp1RequestContext::ProposalBatch {
        aggregate: req.aggregate,
    };
    let route = route_for_proof_type(
        state,
        selected_proof_type,
        &requested_prover_config,
        sp1_context,
    )?;
    let prover_type = prover_type_for_proof_type(
        state,
        selected_proof_type,
        route.route,
        &requested_prover_config,
        sp1_context,
    )?;
    validate_route_specific_request(
        state,
        &pair,
        route.proof_type(),
        req.aggregate,
        &requested_prover_config,
    )?;

    let execution_mode = requested_prover_config
        .sp1
        .as_ref()
        .and_then(|config| config.mode);

    Ok(Some(CanonicalBatchSubmission {
        public_task_id: String::new(),
        pair,
        route,
        proposals,
        aggregate_requested: req.aggregate,
        prover_config: requested_prover_config,
        prover_type,
        blob_proof_type: req.blob_proof_type,
        prover: req.prover,
        graffiti: req.graffiti,
        execution_mode,
    }))
}

fn validate_request_shape(req: &BatchShastaRequest) -> Result<(), ApiError> {
    if req.proposals.is_empty() {
        return Err(ApiError::bad_request("proposals must not be empty"));
    }
    if !req.proof_type.is_public_batch_request_type() {
        return Err(unsupported_proof_type(req.proof_type));
    }
    if req.aggregate && matches!(req.proof_type, BatchProofType::ZkAny) {
        return Err(ApiError::bad_request(
            "proof_type=zk_any is not supported for aggregate requests",
        ));
    }
    Ok(())
}

fn validate_public_prover_args(
    proof_type: BatchProofType,
    args: &PublicProverArgs,
) -> Result<ProverTaskConfig, ApiError> {
    if matches!(proof_type, BatchProofType::ZkAny) && !args.is_empty() {
        return Err(ApiError::bad_request(
            "proof_type=zk_any does not support prover args",
        ));
    }
    if args.native.is_some() {
        return Err(ApiError::bad_request(
            "native prover args are not supported in this API",
        ));
    }
    if args.risc0.is_some() {
        return Err(ApiError::bad_request(
            "risc0 prover args are not supported in this API",
        ));
    }
    if args.sgx.is_some() {
        return Err(ApiError::bad_request(
            "sgx prover args are not supported in this API",
        ));
    }
    if args.sgxgeth.is_some() {
        return Err(ApiError::bad_request(
            "sgxgeth prover args are not supported in this API",
        ));
    }
    if args.sp1.is_some() && !matches!(proof_type, BatchProofType::Sp1 | BatchProofType::ZkAny) {
        return Err(ApiError::bad_request(
            "sp1 prover args require proof_type=sp1",
        ));
    }

    Ok(ProverTaskConfig {
        sp1: args.sp1.clone(),
        sp1_system: None,
    })
}

fn augment_system_prover_config(
    pair: &ResolvedNetworkPair,
    mut prover_config: ProverTaskConfig,
) -> ProverTaskConfig {
    prover_config.sp1_system = pair_sp1_system_config(pair);
    prover_config
}

fn pair_sp1_system_config(pair: &ResolvedNetworkPair) -> Option<Sp1SystemConfig> {
    match (
        pair.sp1_verifier_rpc_url.as_ref(),
        pair.sp1_verifier_address.as_ref(),
    ) {
        (Some(rpc_url), Some(verifier_address)) => Some(Sp1SystemConfig {
            remote_verify: Some(Sp1RemoteVerifyConfig {
                rpc_url: rpc_url.clone(),
                verifier_address: verifier_address.clone(),
            }),
        }),
        _ => None,
    }
}

fn prover_type_for_proof_type(
    state: &AppState,
    proof_type: BatchProofType,
    route: PipelineRoute,
    prover_config: &ProverTaskConfig,
    sp1_context: Sp1RequestContext,
) -> Result<Option<ProverType>, ApiError> {
    match proof_type {
        BatchProofType::Sp1 => {
            let effective_config = state
                .config
                .prover
                .sp1
                .resolve_request_config(prover_config.sp1.as_ref(), sp1_context)
                .map_err(|err| ApiError::bad_request(err.to_string()))?;
            Ok(Some(sp1_prover_type(effective_config.prover)))
        }
        BatchProofType::Risc0 => Ok(Some(risc0_prover_type(state, route))),
        BatchProofType::Boundless => Err(unsupported_proof_type(proof_type)),
        BatchProofType::Native | BatchProofType::Sgx | BatchProofType::SgxGeth => Ok(None),
        BatchProofType::ZkAny => Err(ApiError::bad_request(
            "proof_type=zk_any must be resolved before prover type selection",
        )),
    }
}

const fn sp1_prover_type(prover: Sp1ProverMode) -> ProverType {
    match prover {
        Sp1ProverMode::Mock => ProverType::Mock,
        Sp1ProverMode::Local => ProverType::Local,
        Sp1ProverMode::Network => ProverType::Network,
    }
}

fn risc0_prover_type(state: &AppState, route: PipelineRoute) -> ProverType {
    if matches!(route.runner, RunnerKind::Network) {
        ProverType::Network
    } else if state.config.prover.risc0.mock {
        ProverType::Mock
    } else {
        ProverType::Local
    }
}

const fn prover_type_label(prover_type: Option<ProverType>) -> &'static str {
    match prover_type {
        Some(kind) => kind.as_str(),
        None => "none",
    }
}

fn validate_route_specific_request(
    state: &AppState,
    pair: &ResolvedNetworkPair,
    proof_type: ProofType,
    aggregate: bool,
    prover_config: &ProverTaskConfig,
) -> Result<(), ApiError> {
    if matches!(proof_type, ProofType::Sp1) {
        let effective_config = state
            .config
            .prover
            .sp1
            .resolve_request_config(
                prover_config.sp1.as_ref(),
                Sp1RequestContext::ProposalBatch { aggregate },
            )
            .map_err(|err| ApiError::bad_request(err.to_string()))?;
        let effective_config = prover_config
            .sp1_system
            .as_ref()
            .map_or(effective_config.clone(), |system| {
                system.applied_to(&effective_config)
            });
        validate_hosted_sp1_posture(pair, &effective_config)?;
    } else if prover_config.sp1.is_some() {
        return Err(ApiError::bad_request(
            "sp1 prover args require proof_type=sp1",
        ));
    }

    Ok(())
}

fn validate_aggregate_route_specific_request(
    state: &AppState,
    pair: &ResolvedNetworkPair,
    proof_type: ProofType,
    prover_config: &ProverTaskConfig,
) -> Result<(), ApiError> {
    if matches!(proof_type, ProofType::Sp1) {
        let effective_config = state
            .config
            .prover
            .sp1
            .resolve_request_config(prover_config.sp1.as_ref(), Sp1RequestContext::Aggregation)
            .map_err(|err| ApiError::bad_request(err.to_string()))?;
        let effective_config = prover_config
            .sp1_system
            .as_ref()
            .map_or(effective_config.clone(), |system| {
                system.applied_to(&effective_config)
            });
        validate_hosted_sp1_posture(pair, &effective_config)?;
    } else if prover_config.sp1.is_some() {
        return Err(ApiError::bad_request(
            "sp1 prover args require proof_type=sp1",
        ));
    }

    Ok(())
}

fn validate_hosted_sp1_posture(
    pair: &ResolvedNetworkPair,
    config: &raiko2_prover::sp1::Sp1Config,
) -> Result<(), ApiError> {
    if matches!(config.mode, Sp1ExecutionMode::Prove) && !config.verify {
        return Err(ApiError::bad_request(
            "sp1.mode=prove requires sp1.verify=true on the hosted API",
        ));
    }
    if matches!(config.mode, Sp1ExecutionMode::Prove)
        && matches!(config.prover, raiko2_prover::sp1::ProverMode::Network)
        && config.remote_verify.is_none()
    {
        return Err(ApiError::bad_request(format!(
            "sp1 network verification is not enabled for network pair {}",
            pair.key
        )));
    }
    Ok(())
}

fn canonicalize_proposal(proposal: &ShastaProposal) -> Result<CanonicalProposal, ApiError> {
    validate_shasta_proposal_id("proposal.proposal_id", proposal.proposal_id)?;
    Ok(CanonicalProposal {
        proposal_id: proposal.proposal_id,
        checkpoint: proposal.checkpoint,
        l1_inclusion_block_number: proposal.l1_inclusion_block_number,
        l2_block_numbers: proposal.l2_block_numbers.clone(),
        l2_block_range: validate_l2_block_numbers(&proposal.l2_block_numbers)?,
        last_anchor_block_number: proposal.last_anchor_block_number,
    })
}

fn validate_aggregate_request_shape(req: &AggregateProofRequest) -> Result<(), ApiError> {
    if matches!(req.proof_type, BatchProofType::ZkAny) {
        return Err(ApiError::bad_request(
            "proof_type=zk_any is not supported for aggregate requests",
        ));
    }
    if !req.proof_type.is_concrete_public_proof_type() {
        return Err(unsupported_proof_type(req.proof_type));
    }
    if req.proofs.is_empty() {
        return Err(ApiError::bad_request("proofs must not be empty"));
    }
    for proposal_id in &req.aggregation_ids {
        validate_shasta_proposal_id("aggregation_ids[]", *proposal_id)?;
    }
    Ok(())
}

fn unsupported_proof_type(proof_type: BatchProofType) -> ApiError {
    ApiError::bad_request(format!(
        "proof_type={} is not supported",
        proof_type.as_str()
    ))
}

fn validate_shasta_proposal_id(field: &str, proposal_id: u64) -> Result<(), ApiError> {
    if proposal_id > SHASTA_PROPOSAL_ID_MAX {
        return Err(ApiError::bad_request(format!(
            "{field} does not fit in uint48: {proposal_id}"
        )));
    }
    Ok(())
}

async fn build_external_aggregate_submission(
    state: &AppState,
    req: AggregateProofRequest,
) -> Result<ExternalAggregateSubmission, ApiError> {
    validate_aggregate_request_shape(&req)?;
    let pair = resolved_pair(state, req.network.as_deref(), req.l1_network.as_deref())?;
    let prover_config = augment_system_prover_config(
        &pair,
        validate_public_prover_args(req.proof_type, &req.prover_args)?,
    );
    let sp1_context = Sp1RequestContext::Aggregation;
    let route = route_for_proof_type(state, req.proof_type, &prover_config, sp1_context)?;
    let prover_type = prover_type_for_proof_type(
        state,
        req.proof_type,
        route.route,
        &prover_config,
        sp1_context,
    )?;
    validate_aggregate_route_specific_request(state, &pair, route.proof_type(), &prover_config)?;
    validate_external_aggregate_proofs(route.route, &req.proofs)
        .map_err(|err| ApiError::bad_request(err.to_string()))?;
    let request_fingerprint =
        external_aggregate_request_fingerprint(&pair, route, prover_type, &req, &prover_config)?;
    let public_task_id = public_task_id_from_fingerprint(&request_fingerprint);
    let request = AggregationTaskRequest {
        request_id: aggregate_request_id(&request_fingerprint),
        proposal_ids: req.aggregation_ids.clone(),
        prover_config,
    };
    let task_id = EngineTaskId::new(EngineTaskKey::Aggregate {
        pipeline: route.pipeline_key(),
        request: request.clone(),
    });
    let (inputs, input_artifacts) = persist_external_aggregate_input_artifacts(
        state.runtime.as_ref(),
        &pair.key,
        route,
        &request_fingerprint,
        &req.proofs,
    )
    .await?;
    let _ = (&req.graffiti, &req.prover, &req.blob_proof_type);

    Ok(ExternalAggregateSubmission {
        pair,
        route,
        prover_type,
        public_task_id,
        task_id,
        request,
        inputs,
        input_artifacts,
        request_fingerprint,
    })
}

async fn persist_external_aggregate_input_artifacts(
    runtime: &RuntimeManager,
    network_pair: &str,
    route: CanonicalProofRoute,
    request_fingerprint: &str,
    proofs: &[Proof],
) -> Result<(Vec<AggregateProofInput>, Vec<AggregateInputProofArtifact>), ApiError> {
    let mut inputs = Vec::with_capacity(proofs.len());
    let mut input_artifacts = Vec::with_capacity(proofs.len());
    for (index, proof) in proofs.iter().enumerate() {
        let proof_ref = aggregate_input_proof_ref(request_fingerprint, index);
        let proof_path = runtime
            .write_proof_artifact_bytes(
                network_pair,
                &proof_ref,
                &serde_json::to_vec_pretty(proof).map_err(|err| {
                    ApiError::internal(format!("failed to serialize aggregate input proof: {err}"))
                })?,
            )
            .await
            .map_err(|err| {
                ApiError::internal(format!("failed to write aggregate input proof: {err}"))
            })?;
        let proof_path = proof_path.display().to_string();
        runtime
            .upsert_proof_artifact(raiko2_runtime::ProofArtifactRegistration {
                network_pair: network_pair.to_string(),
                proof_ref: proof_ref.clone(),
                pipeline_key: route.pipeline_key(),
                route: route.route,
                proof_path: proof_path.clone(),
            })
            .await
            .map_err(|err| {
                ApiError::internal(format!("failed to register aggregate input proof: {err}"))
            })?;
        inputs.push(AggregateProofInput::ProofArtifact(ProofArtifactRef {
            network_pair: network_pair.to_string(),
            proof_ref: proof_ref.clone(),
            proof_path: proof_path.clone(),
        }));
        input_artifacts.push(AggregateInputProofArtifact {
            proof_ref,
            proof_path,
        });
    }
    Ok((inputs, input_artifacts))
}

fn planned_external_aggregate_task(
    submission: &ExternalAggregateSubmission,
) -> PlannedAggregateTask {
    PlannedAggregateTask {
        task_ref: aggregate_task_ref(submission.route.pipeline_key(), &submission.request),
        request: submission.request.clone(),
        task_id: submission.task_id.clone(),
    }
}

fn validate_l2_block_numbers(numbers: &[u64]) -> Result<L2BlockRange, ApiError> {
    let Some((&start, rest)) = numbers.split_first() else {
        return Err(ApiError::bad_request(
            "proposal.l2_block_numbers must not be empty",
        ));
    };

    let mut previous = start;
    for number in rest {
        if *number <= previous {
            return Err(ApiError::bad_request(
                "proposal.l2_block_numbers must be strictly increasing",
            ));
        }
        if *number != previous + 1 {
            return Err(ApiError::bad_request(
                "proposal.l2_block_numbers must be contiguous",
            ));
        }
        previous = *number;
    }

    Ok(L2BlockRange {
        start,
        end: previous,
    })
}

async fn build_submission_plan(
    runtime: &RuntimeManager,
    submission: &CanonicalBatchSubmission,
    request_fingerprint: &str,
) -> Result<SubmissionPlan, ApiError> {
    let proposals = submission
        .proposals
        .iter()
        .cloned()
        .map(|proposal| -> Result<PlannedProposalTask, ApiError> {
            let request = proposal_task_request(
                &proposal,
                submission.blob_proof_type.clone(),
                submission.prover.clone(),
                submission.graffiti.clone(),
                submission.prover_config.clone(),
            );
            let task_id = proposal_task_id(submission.route.pipeline_key(), request.clone());
            Ok(PlannedProposalTask {
                task_ref: proposal_task_ref(submission.route.pipeline_key(), &request),
                request,
                task_id,
                proposal,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;

    let mut proposal_sources = Vec::with_capacity(proposals.len());
    let mut aggregate_inputs = Vec::new();
    if submission.aggregate_requested {
        aggregate_inputs.reserve(proposals.len());
        for proposal in &proposals {
            if let Some(material) =
                load_cached_proposal_artifact(runtime, submission, &proposal.task_ref).await?
            {
                proposal_sources.push(ProposalPlanSource::Cached);
                aggregate_inputs.push(AggregateProofInput::ProofArtifact(ProofArtifactRef {
                    network_pair: material.record.network_pair,
                    proof_ref: material.record.proof_ref,
                    proof_path: material.record.proof_path,
                }));
            } else {
                proposal_sources.push(ProposalPlanSource::Pending);
                aggregate_inputs.push(AggregateProofInput::PendingProofArtifact {
                    artifact: proof_artifact_ref(runtime, &submission.pair.key, &proposal.task_ref),
                    dependency: Box::new(proposal.task_id.clone()),
                });
            }
        }
    } else {
        proposal_sources.resize(proposals.len(), ProposalPlanSource::Pending);
    }

    let aggregate = if submission.aggregate_requested {
        let request = AggregationTaskRequest {
            request_id: aggregate_request_id(request_fingerprint),
            proposal_ids: submission
                .proposals
                .iter()
                .map(|proposal| proposal.proposal_id)
                .collect(),
            prover_config: submission.prover_config.clone(),
        };
        let task_id = EngineTaskId::new(EngineTaskKey::Aggregate {
            pipeline: submission.route.pipeline_key(),
            request: request.clone(),
        });
        Some(PlannedAggregateTask {
            task_ref: aggregate_task_ref(submission.route.pipeline_key(), &request),
            request,
            task_id,
        })
    } else {
        None
    };

    Ok(SubmissionPlan {
        proposals,
        proposal_sources,
        aggregate,
        aggregate_inputs,
    })
}

const fn proposal_task_request(
    proposal: &CanonicalProposal,
    blob_proof_type: Option<String>,
    prover: Option<String>,
    graffiti: Option<String>,
    prover_config: ProverTaskConfig,
) -> ProposalTaskRequest {
    ProposalTaskRequest {
        proposal_id: proposal.proposal_id,
        l2_block_range: Some(proposal.l2_block_range),
        l1_inclusion_block_number: proposal.l1_inclusion_block_number,
        last_anchor_block_number: proposal.last_anchor_block_number,
        checkpoint: proposal.checkpoint,
        blob_proof_type,
        prover,
        graffiti,
        prover_config,
    }
}

async fn register_batch_task(
    state: &AppState,
    submission: &CanonicalBatchSubmission,
    plan: &SubmissionPlan,
    request_fingerprint: &str,
) -> Result<TaskRegistrationOutcome, ApiError> {
    let metadata = build_task_metadata(
        &submission.pair,
        BuildTaskMetadataParams {
            network: &submission.pair.network,
            l1_network: &submission.pair.l1_network,
            proof_type: submission.route.proof_type(),
            prover_type: submission.prover_type,
            execution_mode: submission.execution_mode,
            aggregate_requested: submission.aggregate_requested,
        },
        &plan.proposals,
        plan.aggregate.as_ref(),
    );

    state
        .runtime
        .register_task_if_absent(TaskRegistration {
            task_id: submission.public_task_id.clone(),
            route: submission.route.route,
            task_kind: "hoodi_batch".to_string(),
            proposal_id: submission
                .proposals
                .first()
                .map(|proposal| proposal.proposal_id),
            proof_ids: plan
                .proposals
                .iter()
                .map(|proposal| proposal.task_ref.clone())
                .chain(
                    plan.aggregate
                        .iter()
                        .map(|aggregate| aggregate.task_ref.clone()),
                )
                .collect(),
            metadata: serde_json::to_value(metadata).map_err(|err| {
                ApiError::internal(format!("failed to serialize metadata: {err}"))
            })?,
            request_fingerprint: Some(request_fingerprint.to_string()),
        })
        .await
        .map_err(|err| ApiError::internal(format!("failed to register runtime task: {err}")))
}

async fn register_external_aggregate_task(
    state: &AppState,
    submission: &ExternalAggregateSubmission,
    aggregate: &PlannedAggregateTask,
) -> Result<TaskRegistrationOutcome, ApiError> {
    let mut metadata = build_task_metadata(
        &submission.pair,
        BuildTaskMetadataParams {
            network: &submission.pair.network,
            l1_network: &submission.pair.l1_network,
            proof_type: submission.route.proof_type(),
            prover_type: submission.prover_type,
            execution_mode: None,
            aggregate_requested: true,
        },
        &[],
        Some(aggregate),
    );
    metadata.aggregate_input_artifacts = submission.input_artifacts.clone();

    state
        .runtime
        .register_task_if_absent(TaskRegistration {
            task_id: submission.public_task_id.clone(),
            route: submission.route.route,
            task_kind: "hoodi_aggregate".to_string(),
            proposal_id: None,
            proof_ids: vec![aggregate.task_ref.clone()],
            metadata: serde_json::to_value(metadata).map_err(|err| {
                ApiError::internal(format!("failed to serialize metadata: {err}"))
            })?,
            request_fingerprint: Some(submission.request_fingerprint.clone()),
        })
        .await
        .map_err(|err| ApiError::internal(format!("failed to register runtime task: {err}")))
}

fn build_task_metadata(
    pair: &ResolvedNetworkPair,
    params: BuildTaskMetadataParams<'_>,
    proposals: &[PlannedProposalTask],
    aggregate: Option<&PlannedAggregateTask>,
) -> TaskMetadata {
    TaskMetadata {
        network_pair: pair.key.clone(),
        network: params.network.to_string(),
        l1_network: params.l1_network.to_string(),
        proof_type: params.proof_type,
        prover_type: params.prover_type,
        execution_mode: params.execution_mode,
        aggregate_requested: params.aggregate_requested,
        proposals: proposals
            .iter()
            .map(|proposal| ProposalTask {
                proposal_id: proposal.proposal.proposal_id,
                checkpoint: proposal.proposal.checkpoint,
                l1_inclusion_block_number: proposal.proposal.l1_inclusion_block_number,
                l2_block_numbers: proposal.proposal.l2_block_numbers.clone(),
                last_anchor_block_number: proposal.proposal.last_anchor_block_number,
                task_id: proposal.task_ref.clone(),
                request: Some(proposal.request.clone()),
            })
            .collect(),
        aggregate_task_id: aggregate.map(|task| task.task_ref.clone()),
        aggregate_request: aggregate.map(|task| task.request.clone()),
        aggregate_input_artifacts: Vec::new(),
        runtime: RuntimeMetadata::default(),
    }
}

async fn enqueue_submission_plan(
    engine: &Arc<dyn EngineHandle>,
    plan: &SubmissionPlan,
) -> Result<(), ApiError> {
    let mut previous = None;

    for (index, proposal) in plan.proposals.iter().enumerate() {
        if matches!(plan.proposal_sources[index], ProposalPlanSource::Cached) {
            continue;
        }
        let task_id = engine
            .submit_proposal_proof_with_dependencies(
                proposal.request.clone(),
                previous.iter().cloned().collect(),
            )
            .await
            .map_err(|err| {
                ApiError::internal(format!("failed to enqueue proposal proof: {err}"))
            })?;
        if task_id != proposal.task_id {
            return Err(ApiError::internal(
                "engine returned unexpected proposal task id",
            ));
        }
        previous = Some(task_id);
    }

    if let Some(aggregate) = &plan.aggregate {
        let task_id = engine
            .submit_aggregation_proof_from_inputs(
                aggregate.request.clone(),
                plan.aggregate_inputs.clone(),
            )
            .await
            .map_err(|err| {
                ApiError::internal(format!("failed to enqueue aggregation proof: {err}"))
            })?;
        if task_id != aggregate.task_id {
            return Err(ApiError::internal(
                "engine returned unexpected aggregation task id",
            ));
        }
    }

    Ok(())
}

async fn cleanup_submission_plan(
    state: &AppState,
    engine: &Arc<dyn EngineHandle>,
    public_task_id: &str,
    plan: &SubmissionPlan,
) -> Result<(), ApiError> {
    let metadata = TaskMetadata {
        network_pair: String::new(),
        network: String::new(),
        l1_network: String::new(),
        proof_type: ProofType::Native,
        prover_type: None,
        execution_mode: None,
        aggregate_requested: plan.aggregate.is_some(),
        proposals: plan
            .proposals
            .iter()
            .zip(plan.proposal_sources.iter())
            .filter_map(|(proposal, source)| {
                matches!(source, ProposalPlanSource::Pending).then_some(ProposalTask {
                    proposal_id: proposal.proposal.proposal_id,
                    checkpoint: proposal.proposal.checkpoint,
                    l1_inclusion_block_number: proposal.proposal.l1_inclusion_block_number,
                    l2_block_numbers: proposal.proposal.l2_block_numbers.clone(),
                    last_anchor_block_number: proposal.proposal.last_anchor_block_number,
                    task_id: proposal.task_ref.clone(),
                    request: Some(proposal.request.clone()),
                })
            })
            .collect(),
        aggregate_task_id: plan
            .aggregate
            .as_ref()
            .map(|aggregate| aggregate.task_ref.clone()),
        aggregate_request: plan
            .aggregate
            .as_ref()
            .map(|aggregate| aggregate.request.clone()),
        aggregate_input_artifacts: Vec::new(),
        runtime: RuntimeMetadata::default(),
    };

    let pipeline_key = plan
        .aggregate
        .as_ref()
        .map(|aggregate| aggregate.task_id.0.pipeline_key())
        .or_else(|| {
            plan.proposals
                .first()
                .map(|proposal| proposal.task_id.0.pipeline_key())
        })
        .ok_or_else(|| ApiError::internal("submission plan must contain at least one task"))?;

    let _ = cancel_registered_tasks(
        &state.runtime,
        engine,
        public_task_id,
        pipeline_key,
        &metadata,
    )
    .await;
    remove_task_children(engine, pipeline_key, &metadata, &mut HashSet::new())
        .await
        .map_err(|err| ApiError::internal(err.to_string()))
}

async fn handle_existing_batch_task(
    state: &AppState,
    submission: &CanonicalBatchSubmission,
    existing: raiko2_runtime::RuntimeTaskRecord,
) -> Result<Response, ApiError> {
    info!(
        "Detected concurrent duplicate hoodi shasta batch request: task_id={}, aggregate={}, route={}, proof_type={}, prover_type={}, pair={}",
        existing.task_id,
        submission.aggregate_requested,
        submission.route.route,
        submission.route.proof_type(),
        prover_type_label(submission.prover_type),
        submission.pair.key
    );
    let existing_metadata: TaskMetadata = serde_json::from_value(existing.metadata.clone())
        .map_err(|err| {
            ApiError::internal(format!("failed to parse existing task metadata: {err}"))
        })?;
    if should_reenqueue_existing_submission(state, &existing, &existing_metadata).await? {
        let response = compatibility_response_for_task(state, &existing.task_id).await?;
        if response_is_completed(&response) {
            return Ok(response);
        }
        recover_existing_task(state, &existing, || {
            reenqueue_existing_batch_task(state, submission, &existing, &existing_metadata)
        })
        .await?;
    }
    compatibility_response_for_task(state, &existing.task_id).await
}

async fn reenqueue_existing_batch_task(
    state: &AppState,
    submission: &CanonicalBatchSubmission,
    existing: &raiko2_runtime::RuntimeTaskRecord,
    existing_metadata: &TaskMetadata,
) -> Result<(), ApiError> {
    let engine = resolve_engine(
        state,
        &existing_metadata.network_pair,
        existing.pipeline_key,
    )?;
    if matches!(
        FailedStage::from_active_stage(existing_metadata.runtime.active_stage.as_deref()),
        Some(FailedStage::Aggregate)
    ) {
        return reenqueue_existing_batch_aggregate_task(
            state.runtime.as_ref(),
            &engine,
            existing,
            existing_metadata,
        )
        .await;
    }

    let request_fingerprint = match existing.request_fingerprint.as_deref() {
        Some(value) => value.to_string(),
        None => batch_request_fingerprint(submission)?,
    };
    let recovery_plan = build_submission_plan(
        state.runtime.as_ref(),
        &CanonicalBatchSubmission {
            public_task_id: existing.task_id.clone(),
            ..submission.clone()
        },
        &request_fingerprint,
    )
    .await?;
    let recovery_plan =
        with_existing_aggregate_request(recovery_plan, existing.pipeline_key, existing_metadata);
    enqueue_submission_plan(&engine, &recovery_plan)
        .await
        .map_err(|err| {
            ApiError::internal(format!(
                "failed to recover dormant task {}: {err}",
                existing.task_id,
                err = err.message
            ))
        })
}

async fn reenqueue_existing_batch_aggregate_task(
    runtime: &RuntimeManager,
    engine: &Arc<dyn EngineHandle>,
    existing: &raiko2_runtime::RuntimeTaskRecord,
    existing_metadata: &TaskMetadata,
) -> Result<(), ApiError> {
    let request = existing_metadata
        .aggregate_request
        .clone()
        .ok_or_else(|| ApiError::internal("existing batch task missing aggregate_request"))?;
    let expected_task_id = existing_metadata
        .aggregate_engine_task_id(existing.pipeline_key)
        .ok_or_else(|| ApiError::internal("existing batch task missing aggregate task id"))?;
    let inputs = existing_batch_aggregate_inputs(runtime, existing, existing_metadata).await?;
    let actual_task_id = engine
        .submit_aggregation_proof_from_inputs(request, inputs)
        .await
        .map_err(|err| {
            ApiError::internal(format!(
                "failed to recover dormant aggregate task {}: {err}",
                existing.task_id
            ))
        })?;
    if actual_task_id != expected_task_id {
        return Err(ApiError::internal(
            "engine returned unexpected aggregation task id",
        ));
    }
    Ok(())
}

async fn existing_batch_aggregate_inputs(
    runtime: &RuntimeManager,
    existing: &raiko2_runtime::RuntimeTaskRecord,
    existing_metadata: &TaskMetadata,
) -> Result<Vec<AggregateProofInput>, ApiError> {
    let route = CanonicalProofRoute {
        route: existing.route,
    };
    let mut inputs = Vec::with_capacity(existing_metadata.proposals.len());

    for proposal in &existing_metadata.proposals {
        if let Some(material) = load_first_cached_proposal_artifact(
            runtime,
            &existing_metadata.network_pair,
            route,
            &proposal_proof_artifact_refs(existing.pipeline_key, proposal),
        )
        .await?
        {
            inputs.push(AggregateProofInput::ProofArtifact(ProofArtifactRef {
                network_pair: material.record.network_pair,
                proof_ref: material.record.proof_ref,
                proof_path: material.record.proof_path,
            }));
            continue;
        }

        return Err(ApiError::internal(format!(
            "existing aggregate task {} missing completed proposal proof artifact {}",
            existing.task_id, proposal.task_id
        )));
    }

    if inputs.is_empty() {
        return Err(ApiError::internal(
            "existing aggregate task has no proposal proof inputs",
        ));
    }
    Ok(inputs)
}

async fn load_first_cached_proposal_artifact(
    runtime: &RuntimeManager,
    network_pair: &str,
    route: CanonicalProofRoute,
    proof_refs: &[String],
) -> Result<Option<ProofArtifactMaterial>, ApiError> {
    for proof_ref in proof_refs {
        if let Some(material) =
            load_cached_proposal_artifact_for_route(runtime, network_pair, route, proof_ref).await?
        {
            return Ok(Some(material));
        }
    }
    Ok(None)
}

async fn handle_created_batch_task(
    state: &AppState,
    submission: &CanonicalBatchSubmission,
    plan: &SubmissionPlan,
) -> Result<Response, ApiError> {
    let engine = resolve_engine(state, &submission.pair.key, submission.route.pipeline_key())?;

    if let Err(err) = enqueue_submission_plan(&engine, plan).await {
        let _ = cleanup_submission_plan(state, &engine, &submission.public_task_id, plan).await;
        let _ = state
            .runtime
            .sync_status(
                &submission.public_task_id,
                RuntimeRunnerStatus::Failed,
                Some(err.message.clone()),
                None,
            )
            .await;
        return Err(err);
    }

    telemetry::record_request_registered(
        &MetricContext::new(
            submission.route.route.to_string(),
            submission.route.proof_type(),
            submission.pair.key.clone(),
            submission.aggregate_requested,
        ),
        submission.aggregate_requested,
    );

    Ok(registered_response(
        hoodi_response_proof_type(submission),
        submission.public_task_id.clone(),
    )
    .into_response())
}

async fn handle_existing_external_aggregate_task(
    state: &AppState,
    engine: &Arc<dyn EngineHandle>,
    submission: &ExternalAggregateSubmission,
    existing: raiko2_runtime::RuntimeTaskRecord,
) -> Result<Response, ApiError> {
    info!(
        "Detected concurrent duplicate hoodi aggregate request: task_id={}, route={}, proof_type={}, prover_type={}, pair={}",
        existing.task_id,
        submission.route.route,
        submission.route.proof_type(),
        prover_type_label(submission.prover_type),
        submission.pair.key
    );
    let existing_metadata: TaskMetadata = serde_json::from_value(existing.metadata.clone())
        .map_err(|err| {
            ApiError::internal(format!("failed to parse existing task metadata: {err}"))
        })?;
    if should_reenqueue_existing_submission(state, &existing, &existing_metadata).await? {
        let response = compatibility_response_for_task(state, &existing.task_id).await?;
        if response_is_completed(&response) {
            return Ok(response);
        }
        recover_existing_task(state, &existing, || {
            reenqueue_existing_external_aggregate_task(
                engine,
                submission,
                &existing,
                &existing_metadata,
            )
        })
        .await?;
    }
    compatibility_response_for_task(state, &existing.task_id).await
}

async fn recover_existing_task<'a, F, Fut>(
    state: &AppState,
    existing: &'a raiko2_runtime::RuntimeTaskRecord,
    reenqueue: F,
) -> Result<(), ApiError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), ApiError>> + 'a,
{
    reset_runtime_task_to_allocated(state, &existing.task_id).await?;
    if let Err(err) = reenqueue().await {
        restore_runtime_task_status(state, existing, &err).await;
        return Err(err);
    }
    Ok(())
}

async fn reset_runtime_task_to_allocated(state: &AppState, task_id: &str) -> Result<(), ApiError> {
    state
        .runtime
        .sync_status(task_id, RuntimeRunnerStatus::Allocated, None, None)
        .await
        .map_err(|err| {
            ApiError::internal(format!("failed to reset recovered task {task_id}: {err}"))
        })
}

async fn restore_runtime_task_status(
    state: &AppState,
    existing: &raiko2_runtime::RuntimeTaskRecord,
    enqueue_error: &ApiError,
) {
    if let Err(err) = state
        .runtime
        .sync_status(
            &existing.task_id,
            existing.runner_status,
            existing.error.clone(),
            None,
        )
        .await
    {
        warn!(
            task_id = existing.task_id,
            original_status = existing.runner_status.as_str(),
            enqueue_error = %enqueue_error.message,
            restore_error = %err,
            "failed to restore runtime status after recovery enqueue failure"
        );
    }
}

async fn reenqueue_existing_external_aggregate_task(
    engine: &Arc<dyn EngineHandle>,
    submission: &ExternalAggregateSubmission,
    existing: &raiko2_runtime::RuntimeTaskRecord,
    existing_metadata: &TaskMetadata,
) -> Result<(), ApiError> {
    let request = existing_metadata
        .aggregate_request
        .clone()
        .ok_or_else(|| ApiError::internal("existing aggregate task missing aggregate_request"))?;
    let expected_task_id = existing_metadata
        .aggregate_engine_task_id(existing.pipeline_key)
        .ok_or_else(|| ApiError::internal("existing aggregate task missing aggregate task id"))?;
    let inputs = if existing_metadata.aggregate_input_artifacts.is_empty() {
        submission.inputs.clone()
    } else {
        aggregate_inputs_from_artifacts(
            &existing_metadata.network_pair,
            &existing_metadata.aggregate_input_artifacts,
        )
    };
    let actual_task_id = engine
        .submit_aggregation_proof_from_inputs(request, inputs)
        .await
        .map_err(|err| {
            ApiError::internal(format!(
                "failed to recover dormant aggregate task {}: {err}",
                existing.task_id
            ))
        })?;
    if actual_task_id != expected_task_id {
        return Err(ApiError::internal(
            "engine returned unexpected aggregation task id",
        ));
    }
    Ok(())
}

fn aggregate_inputs_from_artifacts(
    network_pair: &str,
    artifacts: &[AggregateInputProofArtifact],
) -> Vec<AggregateProofInput> {
    artifacts
        .iter()
        .map(|artifact| {
            AggregateProofInput::ProofArtifact(ProofArtifactRef {
                network_pair: network_pair.to_string(),
                proof_ref: artifact.proof_ref.clone(),
                proof_path: artifact.proof_path.clone(),
            })
        })
        .collect()
}

async fn handle_created_external_aggregate_task(
    state: &AppState,
    engine: &Arc<dyn EngineHandle>,
    submission: &ExternalAggregateSubmission,
    aggregate: &PlannedAggregateTask,
) -> Result<Response, ApiError> {
    let mut metadata = build_task_metadata(
        &submission.pair,
        BuildTaskMetadataParams {
            network: &submission.pair.network,
            l1_network: &submission.pair.l1_network,
            proof_type: submission.route.proof_type(),
            prover_type: submission.prover_type,
            execution_mode: None,
            aggregate_requested: true,
        },
        &[],
        Some(aggregate),
    );
    metadata.aggregate_input_artifacts = submission.input_artifacts.clone();
    let actual_task_id = engine
        .submit_aggregation_proof_from_inputs(submission.request.clone(), submission.inputs.clone())
        .await
        .map_err(|err| ApiError::internal(format!("failed to enqueue aggregation proof: {err}")));
    let actual_task_id = match actual_task_id {
        Ok(task_id) => task_id,
        Err(err) => {
            cleanup_external_aggregate_submission(state, engine, submission, &metadata).await;
            return Err(err);
        }
    };
    if actual_task_id != aggregate.task_id {
        cleanup_external_aggregate_submission(state, engine, submission, &metadata).await;
        return Err(ApiError::internal(
            "engine returned unexpected aggregation task id",
        ));
    }

    telemetry::record_request_registered(
        &MetricContext::new(
            submission.route.route.to_string(),
            submission.route.proof_type(),
            submission.pair.key.clone(),
            true,
        ),
        true,
    );

    Ok(registered_response(
        BatchProofType::from_canonical(submission.route.proof_type()),
        submission.public_task_id.clone(),
    )
    .into_response())
}

async fn cleanup_external_aggregate_submission(
    state: &AppState,
    engine: &Arc<dyn EngineHandle>,
    submission: &ExternalAggregateSubmission,
    metadata: &TaskMetadata,
) {
    let _ = state.runtime.remove_task(&submission.public_task_id).await;
    let _ = remove_task_children(
        engine,
        submission.route.pipeline_key(),
        metadata,
        &mut HashSet::new(),
    )
    .await;
}

async fn load_task_data(state: &AppState, id: &str) -> Result<TaskData, ApiError> {
    let lookup = load_task_lookup(state, id).await?;
    load_task_data_from_lookup(state, id, &lookup).await
}

async fn load_task_data_from_lookup(
    state: &AppState,
    id: &str,
    lookup: &TaskLookup,
) -> Result<TaskData, ApiError> {
    let (proposals, proposal_engine_state_present): (Vec<ProposalStatus>, bool) =
        load_proposal_statuses(
            state.runtime.as_ref(),
            &lookup.engine,
            &lookup.metadata,
            &lookup.record,
        )
        .await?;
    let (aggregate, aggregate_engine_state_present): (Option<AggregateStatus>, bool) =
        load_aggregate_status(
            state.runtime.as_ref(),
            &lookup.engine,
            &lookup.metadata,
            &lookup.record,
        )
        .await?;
    let root_engine_state_present = proposal_engine_state_present || aggregate_engine_state_present;
    let root_state = resolve_root_task_state(
        lookup.record.runner_status,
        &proposals,
        aggregate.as_ref(),
        lookup.metadata.has_runtime_progress(),
        lookup.record.error.as_deref(),
    );
    let root_proof_location = root_proof_location(
        &lookup.record,
        &lookup.metadata,
        &proposals,
        aggregate.as_ref(),
    );
    let root_proof = root_state.proof;
    let root_proof = if root_proof.is_none()
        && matches!(root_state.status, ProofStatus::Completed)
        && root_proof_artifact_refs(&lookup.metadata, lookup.record.pipeline_key).is_some()
    {
        load_persisted_root_proof(&lookup.record).await?
    } else {
        root_proof
    };

    Ok(TaskData {
        task_id: id.to_string(),
        route: lookup.record.route.to_string(),
        prover_type: lookup.metadata.prover_type_str(),
        execution_mode: lookup.metadata.execution_mode_str(),
        status: root_state.status.clone(),
        network: lookup.metadata.network.clone(),
        l1_network: lookup.metadata.l1_network.clone(),
        runtime: root_runtime_view(&lookup.record, &lookup.metadata, root_engine_state_present),
        current_index: root_state.current_index,
        proposals,
        aggregate,
        proof: root_proof,
        proof_ref: root_proof_location
            .as_ref()
            .and_then(|location| location.proof_ref.clone()),
        proof_path: root_proof_location.and_then(|location| location.proof_path),
        error: root_state.error,
    })
}

async fn load_persisted_root_proof(
    record: &raiko2_runtime::RuntimeTaskRecord,
) -> Result<Option<String>, ApiError> {
    Ok(load_persisted_root_proof_material(record)
        .await?
        .and_then(|proof| proof.proof))
}

async fn load_cached_proposal_artifact(
    runtime: &RuntimeManager,
    submission: &CanonicalBatchSubmission,
    proof_ref: &str,
) -> Result<Option<ProofArtifactMaterial>, ApiError> {
    load_cached_proposal_artifact_for_route(
        runtime,
        &submission.pair.key,
        submission.route,
        proof_ref,
    )
    .await
}

async fn load_cached_proposal_artifact_for_route(
    runtime: &RuntimeManager,
    network_pair: &str,
    route: CanonicalProofRoute,
    proof_ref: &str,
) -> Result<Option<ProofArtifactMaterial>, ApiError> {
    let Some(material) = load_proof_artifact_material(runtime, network_pair, proof_ref)
        .await
        .map_err(|err| ApiError::internal(format!("failed to load proof artifact: {err}")))?
    else {
        return Ok(None);
    };

    if material.record.pipeline_key != route.pipeline_key() || material.record.route != route.route
    {
        warn!(
            network_pair = material.record.network_pair,
            proof_ref = material.record.proof_ref,
            artifact_pipeline = material.record.pipeline_key.as_str(),
            artifact_route = %material.record.route,
            request_pipeline = route.pipeline_key().as_str(),
            request_route = %route.route,
            "proof artifact route does not match request; treating it as a cache miss"
        );
        return Ok(None);
    }

    if let Err(err) =
        validate_external_aggregate_proofs(route.route, std::slice::from_ref(&material.proof))
    {
        warn!(
            network_pair = material.record.network_pair,
            proof_ref = material.record.proof_ref,
            error = %err,
            "proof artifact is not valid aggregate input; treating it as a cache miss"
        );
        return Ok(None);
    }

    Ok(Some(material))
}

async fn load_persisted_root_proof_material(
    record: &raiko2_runtime::RuntimeTaskRecord,
) -> Result<Option<Proof>, ApiError> {
    let Some(path) = record.proof_path.as_deref() else {
        return Ok(None);
    };
    let bytes = fs::read(path)
        .await
        .map_err(|err| ApiError::internal(format!("failed to read proof file {path}: {err}")))?;
    let proof: Proof = serde_json::from_slice(&bytes)
        .map_err(|err| ApiError::internal(format!("failed to parse proof file {path}: {err}")))?;
    Ok(Some(proof))
}

fn artifact_proof_location(record: &raiko2_runtime::ProofArtifactRecord) -> ProofLocation {
    ProofLocation {
        proof_ref: Some(record.proof_ref.clone()),
        proof_path: Some(record.proof_path.clone()),
    }
}

fn proof_artifact_ref(
    runtime: &RuntimeManager,
    network_pair: &str,
    proof_ref: &str,
) -> ProofArtifactRef {
    ProofArtifactRef {
        network_pair: network_pair.to_string(),
        proof_ref: proof_ref.to_string(),
        proof_path: runtime
            .proof_artifact_path(network_pair, proof_ref)
            .display()
            .to_string(),
    }
}

fn status_proof_location(
    proof_ref: Option<&String>,
    proof_path: Option<&String>,
) -> Option<ProofLocation> {
    if proof_ref.is_none() && proof_path.is_none() {
        return None;
    }

    Some(ProofLocation {
        proof_ref: proof_ref.cloned(),
        proof_path: proof_path.cloned(),
    })
}

fn root_proof_location(
    record: &raiko2_runtime::RuntimeTaskRecord,
    metadata: &TaskMetadata,
    proposals: &[ProposalStatus],
    aggregate: Option<&AggregateStatus>,
) -> Option<ProofLocation> {
    let record_location = || {
        root_proof_artifact_refs(metadata, record.pipeline_key)?;
        record.proof_path.as_ref().map(|proof_path| ProofLocation {
            proof_ref: None,
            proof_path: Some(proof_path.clone()),
        })
    };

    if let Some(aggregate) = aggregate
        && let Some(location) =
            status_proof_location(aggregate.proof_ref.as_ref(), aggregate.proof_path.as_ref())
    {
        return Some(location);
    }
    if aggregate.is_some() {
        return record_location();
    }

    if let [proposal] = proposals
        && let Some(location) =
            status_proof_location(proposal.proof_ref.as_ref(), proposal.proof_path.as_ref())
    {
        return Some(location);
    }

    record_location()
}

async fn load_all_task_data(state: &AppState) -> Result<Vec<TaskData>, ApiError> {
    let records = state
        .runtime
        .list_tasks()
        .await
        .map_err(|err| ApiError::internal(format!("failed to list runtime tasks: {err}")))?;
    let mut tasks = Vec::with_capacity(records.len());
    for record in records {
        tasks.push(load_task_data(state, &record.task_id).await?);
    }
    Ok(tasks)
}

async fn load_task_lookup(state: &AppState, id: &str) -> Result<TaskLookup, ApiError> {
    let record = state
        .runtime
        .get_task(id)
        .await
        .map_err(|err| ApiError::internal(format!("failed to load task: {err}")))?
        .ok_or_else(|| ApiError::not_found(format!("task not found: {id}")))?;
    let metadata: TaskMetadata = serde_json::from_value(record.metadata.clone())
        .map_err(|err| ApiError::internal(format!("failed to parse task metadata: {err}")))?;
    let engine = resolve_engine(state, &metadata.network_pair, record.pipeline_key)?;

    Ok(TaskLookup {
        record,
        metadata,
        engine,
    })
}

async fn load_proposal_statuses(
    runtime_manager: &RuntimeManager,
    engine: &Arc<dyn EngineHandle>,
    metadata: &TaskMetadata,
    record: &raiko2_runtime::RuntimeTaskRecord,
) -> Result<(Vec<ProposalStatus>, bool), ApiError> {
    let mut proposals = Vec::with_capacity(metadata.proposals.len());
    let mut any_engine_state_present = false;

    for (index, proposal) in metadata.proposals.iter().enumerate() {
        let mut stage_statuses = Vec::with_capacity(4);
        if let Some(task_id) = proposal.engine_task_id(record.pipeline_key) {
            for stage_id in proposal_task_chain_ids(&task_id) {
                stage_statuses.push(engine.get_status(stage_id).await.map_err(|err| {
                    ApiError::internal(format!("failed to read task status: {err}"))
                })?);
            }
        }

        let engine_state_present = stage_statuses.iter().any(Option::is_some);
        any_engine_state_present |= engine_state_present;
        let runtime = metadata.proposal_runtime(&proposal.task_id);
        let status = fallback_status_without_engine(
            summarize_proposal_task_state(&stage_statuses),
            record.runner_status,
            runtime.is_some() || metadata.has_runtime_progress(),
            record.error.as_deref(),
        );
        let mut proof_location = None;
        let status = if let Some(material) =
            load_proof_artifact_material(runtime_manager, &metadata.network_pair, &proposal.task_id)
                .await
                .map_err(|err| {
                    ApiError::internal(format!("failed to load proof artifact: {err}"))
                })? {
            proof_location = Some(artifact_proof_location(&material.record));
            let proof = material.proof;
            EngineStatusView {
                status: ProofStatus::Completed,
                proof: proof.proof,
                error: None,
                extra_data: proof.extra_data,
            }
        } else {
            status
        };
        proposals.push(ProposalStatus {
            index,
            proposal_id: proposal.proposal_id,
            checkpoint: proposal.checkpoint,
            task_id: proposal.task_id.clone(),
            status: status.status.clone(),
            l1_inclusion_block_number: proposal.l1_inclusion_block_number,
            l2_block_numbers: proposal.l2_block_numbers.clone(),
            last_anchor_block_number: proposal.last_anchor_block_number,
            proof: status.proof,
            proof_ref: proof_location
                .as_ref()
                .and_then(|location| location.proof_ref.clone()),
            proof_path: proof_location.and_then(|location| location.proof_path),
            error: status.error,
            runtime: task_runtime_view(runtime, engine_state_present, record.updated_at),
            extra_data: status.extra_data,
        });
    }

    Ok((proposals, any_engine_state_present))
}

async fn load_aggregate_status(
    runtime_manager: &RuntimeManager,
    engine: &Arc<dyn EngineHandle>,
    metadata: &TaskMetadata,
    record: &raiko2_runtime::RuntimeTaskRecord,
) -> Result<(Option<AggregateStatus>, bool), ApiError> {
    let Some(_task_id) = metadata.aggregate_task_id.as_ref() else {
        return Ok((None, false));
    };

    let status = if let Some(task_id) = metadata.aggregate_engine_task_id(record.pipeline_key) {
        engine.get_status(task_id).await.map_err(|err| {
            ApiError::internal(format!("failed to read aggregation status: {err}"))
        })?
    } else {
        None
    };
    let engine_state_present = status.is_some();
    let status = fallback_status_without_engine(
        status.unwrap_or(EngineStatusView {
            status: ProofStatus::Pending,
            proof: None,
            error: None,
            extra_data: None,
        }),
        record.runner_status,
        metadata.aggregate_runtime().is_some() || metadata.has_runtime_progress(),
        record.error.as_deref(),
    );
    let mut proof_location = None;
    let status = if let Some(task_id) = metadata.aggregate_task_id.as_deref() {
        if let Some(material) =
            load_proof_artifact_material(runtime_manager, &metadata.network_pair, task_id)
                .await
                .map_err(|err| {
                    ApiError::internal(format!("failed to load proof artifact: {err}"))
                })?
        {
            proof_location = Some(artifact_proof_location(&material.record));
            let proof = material.proof;
            EngineStatusView {
                status: ProofStatus::Completed,
                proof: proof.proof,
                error: None,
                extra_data: proof.extra_data,
            }
        } else {
            status
        }
    } else {
        status
    };

    Ok((
        Some(AggregateStatus {
            task_id: metadata.aggregate_task_id.clone().expect("checked"),
            status: status.status.clone(),
            proof: status.proof,
            proof_ref: proof_location
                .as_ref()
                .and_then(|location| location.proof_ref.clone()),
            proof_path: proof_location.and_then(|location| location.proof_path),
            error: status.error,
            runtime: task_runtime_view(
                metadata.aggregate_runtime(),
                engine_state_present,
                record.updated_at,
            ),
            extra_data: status.extra_data,
        }),
        engine_state_present,
    ))
}

fn summarize_proposal_task_state(stages: &[Option<EngineStatusView>]) -> EngineStatusView {
    let mut pending_error = None;

    for stage in stages {
        let Some(stage) = stage else {
            continue;
        };

        match stage.status {
            ProofStatus::Failed | ProofStatus::Cancelled | ProofStatus::Completed => {
                return stage.clone();
            }
            ProofStatus::Proving => {
                return EngineStatusView {
                    status: ProofStatus::Proving,
                    proof: None,
                    error: stage.error.clone(),
                    extra_data: None,
                };
            }
            ProofStatus::Pending => {
                if pending_error.is_none() {
                    pending_error.clone_from(&stage.error);
                }
            }
        }
    }

    EngineStatusView {
        status: ProofStatus::Pending,
        proof: None,
        error: pending_error,
        extra_data: None,
    }
}

fn fallback_status_without_engine(
    status: EngineStatusView,
    runner_status: RuntimeRunnerStatus,
    has_runtime_progress: bool,
    runtime_error: Option<&str>,
) -> EngineStatusView {
    if !matches!(status.status, ProofStatus::Pending) {
        return status;
    }

    let next_status = runner_status_to_proof_status(runner_status, has_runtime_progress);
    let error = failed_runtime_error(&next_status, runtime_error).or(status.error);
    EngineStatusView {
        status: next_status,
        proof: None,
        error,
        extra_data: None,
    }
}

fn resolve_root_task_state(
    runner_status: RuntimeRunnerStatus,
    proposals: &[ProposalStatus],
    aggregate: Option<&AggregateStatus>,
    runtime_has_progress: bool,
    runtime_error: Option<&str>,
) -> RootTaskState {
    if matches!(runner_status, RuntimeRunnerStatus::Cancelled) {
        return RootTaskState {
            status: ProofStatus::Cancelled,
            proof: None,
            error: None,
            current_index: proposals
                .iter()
                .position(|proposal| !matches!(proposal.status, ProofStatus::Completed)),
        };
    }

    let computed_root_status = summarize_root_status(proposals, aggregate);
    let status = match computed_root_status {
        ProofStatus::Pending => runner_status_to_proof_status(runner_status, runtime_has_progress),
        status => status,
    };
    let proof = match aggregate {
        Some(aggregate) => aggregate.proof.clone(),
        None if proposals.len() == 1 => proposals
            .first()
            .and_then(|proposal| proposal.proof.clone()),
        None => None,
    };
    let error = aggregate
        .and_then(|aggregate| aggregate.error.clone())
        .or_else(|| proposals.iter().find_map(|proposal| proposal.error.clone()))
        .or_else(|| failed_runtime_error(&status, runtime_error));
    let current_index = proposals
        .iter()
        .position(|proposal| !matches!(proposal.status, ProofStatus::Completed))
        .or_else(|| {
            aggregate
                .filter(|aggregate| !matches!(aggregate.status, ProofStatus::Completed))
                .map(|_| proposals.len())
        });

    RootTaskState {
        status,
        proof,
        error,
        current_index,
    }
}

fn summarize_root_status(
    proposals: &[ProposalStatus],
    aggregate: Option<&AggregateStatus>,
) -> ProofStatus {
    if proposals.is_empty() {
        return aggregate.map_or(ProofStatus::Pending, |aggregate| aggregate.status.clone());
    }
    if proposals
        .iter()
        .any(|proposal| matches!(proposal.status, ProofStatus::Failed))
        || aggregate.is_some_and(|aggregate| matches!(aggregate.status, ProofStatus::Failed))
    {
        return ProofStatus::Failed;
    }
    if proposals
        .iter()
        .any(|proposal| matches!(proposal.status, ProofStatus::Proving))
        || aggregate.is_some_and(|aggregate| matches!(aggregate.status, ProofStatus::Proving))
    {
        return ProofStatus::Proving;
    }
    if proposals
        .iter()
        .all(|proposal| matches!(proposal.status, ProofStatus::Completed))
    {
        return aggregate.map_or(ProofStatus::Completed, |aggregate| aggregate.status.clone());
    }
    if proposals
        .iter()
        .any(|proposal| matches!(proposal.status, ProofStatus::Cancelled))
    {
        return ProofStatus::Cancelled;
    }
    ProofStatus::Pending
}

const fn runner_status_to_proof_status(
    runner_status: RuntimeRunnerStatus,
    has_runtime_progress: bool,
) -> ProofStatus {
    match runner_status {
        RuntimeRunnerStatus::Cancelled => ProofStatus::Cancelled,
        RuntimeRunnerStatus::Failed => ProofStatus::Failed,
        RuntimeRunnerStatus::Completed => ProofStatus::Completed,
        RuntimeRunnerStatus::Allocated | RuntimeRunnerStatus::Running if has_runtime_progress => {
            ProofStatus::Proving
        }
        RuntimeRunnerStatus::Allocated | RuntimeRunnerStatus::Running => ProofStatus::Pending,
    }
}

fn failed_runtime_error(status: &ProofStatus, runtime_error: Option<&str>) -> Option<String> {
    matches!(status, ProofStatus::Failed)
        .then_some(runtime_error.map(str::to_string))
        .flatten()
}

fn root_runtime_view(
    record: &raiko2_runtime::RuntimeTaskRecord,
    metadata: &TaskMetadata,
    engine_state_present: bool,
) -> RootRuntime {
    RootRuntime {
        runner_status: record.runner_status,
        active_stage: metadata.runtime.active_stage.clone(),
        last_event: metadata.runtime.last_event.clone(),
        updated_at: record.updated_at,
        engine_state_present,
    }
}

fn task_runtime_view(
    runtime: Option<&TaskRuntimeMetadata>,
    engine_state_present: bool,
    fallback_updated_at: i64,
) -> Option<TaskRuntime> {
    if engine_state_present && runtime.is_none() {
        return None;
    }
    let runtime = runtime.cloned().unwrap_or_default();
    Some(TaskRuntime {
        updated_at: if runtime.updated_at == 0 {
            fallback_updated_at
        } else {
            runtime.updated_at
        },
        engine_state_present,
        provider_request_id: runtime.provider_request_id,
        remote_tx_hash: runtime.remote_tx_hash,
        image_ref: runtime.image_ref,
        deployment: runtime.deployment,
        offchain: runtime.offchain,
        expires_at: runtime.expires_at,
        quoted_mcycles_count: runtime.quoted_mcycles_count,
        evaluated_mcycles_count: runtime.evaluated_mcycles_count,
        sp1_network_mode: runtime
            .sp1_network_mode
            .map(|mode| mode.as_str().to_string()),
        sp1_fulfillment_strategy: runtime
            .sp1_fulfillment_strategy
            .map(|strategy| strategy.as_str().to_string()),
        sp1_skip_simulation: runtime.sp1_skip_simulation,
        sp1_cycle_limit: runtime.sp1_cycle_limit,
        sp1_timeout_secs: runtime.sp1_timeout_secs,
    })
}

fn resolved_pair(
    state: &AppState,
    network: Option<&str>,
    l1_network: Option<&str>,
) -> Result<ResolvedNetworkPair, ApiError> {
    match (network, l1_network) {
        (Some(network), Some(l1_network)) => {
            return state
                .config
                .rpc
                .resolve_pair(network, l1_network)
                .map_err(|err| ApiError::bad_request(err.to_string()));
        }
        (None, None) => {}
        _ => {
            return Err(ApiError::bad_request(
                "network and l1_network must be provided together",
            ));
        }
    }

    let resolved_pairs = state
        .config
        .rpc
        .resolved_pairs()
        .map_err(|err| ApiError::bad_request(err.to_string()))?;
    resolved_pairs
        .first()
        .cloned()
        .ok_or_else(|| ApiError::internal("rpc.pairs must contain at least one network pair"))
}

fn batch_request_fingerprint(submission: &CanonicalBatchSubmission) -> Result<String, ApiError> {
    let payload = serde_json::json!({
        "pair_key": submission.pair.key.as_str(),
        "route": submission.route.route.to_string(),
        "prover_type": submission.prover_type.map(ProverType::as_str),
        "aggregate_requested": submission.aggregate_requested,
        "execution_mode": submission
            .execution_mode
            .map(|mode| mode.as_str()),
        "blob_proof_type": submission.blob_proof_type.as_deref(),
        "prover": submission.prover.as_deref(),
        "graffiti": submission.graffiti.as_deref(),
        "prover_config": &submission.prover_config,
        "proposals": &submission.proposals,
    });
    let encoded = serde_json::to_vec(&payload).map_err(|err| {
        ApiError::internal(format!("failed to serialize request fingerprint: {err}"))
    })?;
    Ok(hex::encode_prefixed(keccak256(encoded).as_slice()))
}

fn external_aggregate_request_fingerprint(
    pair: &ResolvedNetworkPair,
    route: CanonicalProofRoute,
    prover_type: Option<ProverType>,
    req: &AggregateProofRequest,
    prover_config: &ProverTaskConfig,
) -> Result<String, ApiError> {
    let payload = serde_json::json!({
        "pair_key": pair.key.as_str(),
        "route": route.route.to_string(),
        "prover_type": prover_type.map(ProverType::as_str),
        "proof_type": route.proof_type().to_string(),
        "aggregation_ids": &req.aggregation_ids,
        "prover_config": prover_config,
        "proofs": &req.proofs,
        "graffiti": req.graffiti.as_deref(),
        "prover": req.prover.as_deref(),
        "blob_proof_type": req.blob_proof_type.as_deref(),
    });
    let encoded = serde_json::to_vec(&payload).map_err(|err| {
        ApiError::internal(format!(
            "failed to serialize aggregate request fingerprint: {err}"
        ))
    })?;
    Ok(hex::encode_prefixed(keccak256(encoded).as_slice()))
}

fn aggregate_request_id(request_fingerprint: &str) -> String {
    format!("request:{request_fingerprint}")
}

fn with_existing_aggregate_request(
    mut plan: SubmissionPlan,
    pipeline_key: PipelineKey,
    metadata: &TaskMetadata,
) -> SubmissionPlan {
    if let (Some(aggregate), Some(request)) =
        (plan.aggregate.as_mut(), metadata.aggregate_request.clone())
    {
        aggregate.task_ref = aggregate_task_ref(pipeline_key, &request);
        aggregate.task_id = EngineTaskId::new(EngineTaskKey::Aggregate {
            pipeline: pipeline_key,
            request: request.clone(),
        });
        aggregate.request = request;
    }
    plan
}

fn resolve_engine(
    state: &AppState,
    network_pair: &str,
    pipeline_key: PipelineKey,
) -> Result<Arc<dyn EngineHandle>, ApiError> {
    state
        .pipelines
        .get(network_pair, pipeline_key)
        .ok_or_else(|| {
            ApiError::not_found(format!("pipeline not available: {}", pipeline_key.as_str()))
        })
}

const fn hoodi_response_proof_type(submission: &CanonicalBatchSubmission) -> BatchProofType {
    match submission.route.proof_type() {
        ProofType::Native => BatchProofType::Native,
        ProofType::Sp1 => BatchProofType::Sp1,
        ProofType::Sgx => BatchProofType::Sgx,
        ProofType::SgxGeth => BatchProofType::SgxGeth,
        ProofType::Risc0 => BatchProofType::Risc0,
    }
}

fn registration_response(
    proof_type: &str,
    status: LegacyTaskStatus,
    batch_id: Option<u64>,
) -> Response {
    Json(LegacyProofEnvelope {
        status: "ok",
        proof_type: proof_type.to_string(),
        batch_id,
        data: LegacyProofData::Status { status },
    })
    .into_response()
}

fn registered_response(proof_type: BatchProofType, _public_task_id: String) -> Response {
    registration_response(proof_type.as_str(), LegacyTaskStatus::Registered, None)
}

fn zk_any_not_drawn_response(batch_id: Option<u64>) -> Response {
    registration_response(
        BatchProofType::ZkAny.as_str(),
        LegacyTaskStatus::ZkAnyNotDrawn,
        batch_id,
    )
}

async fn should_reenqueue_existing_submission(
    state: &AppState,
    record: &raiko2_runtime::RuntimeTaskRecord,
    metadata: &TaskMetadata,
) -> Result<bool, ApiError> {
    if should_reenqueue_existing_submission_without_engine(record, metadata) {
        return Ok(true);
    }

    if !matches!(
        record.runner_status,
        RuntimeRunnerStatus::Allocated | RuntimeRunnerStatus::Running
    ) || !metadata.has_runtime_progress()
        || metadata.has_remote_submission_progress()
    {
        return Ok(false);
    }

    let engine = resolve_engine(state, &metadata.network_pair, record.pipeline_key)?;
    let engine_state_present =
        registered_engine_state_present(&engine, record.pipeline_key, metadata).await?;
    Ok(stale_nonterminal_runtime_is_reenqueueable(
        record,
        metadata,
        engine_state_present,
    ))
}

fn should_reenqueue_existing_submission_without_engine(
    record: &raiko2_runtime::RuntimeTaskRecord,
    metadata: &TaskMetadata,
) -> bool {
    match record.runner_status {
        RuntimeRunnerStatus::Failed => failed_stage_is_reenqueueable(record, metadata),
        RuntimeRunnerStatus::Completed | RuntimeRunnerStatus::Cancelled => false,
        RuntimeRunnerStatus::Allocated | RuntimeRunnerStatus::Running => {
            !metadata.has_runtime_progress()
        }
    }
}

fn stale_nonterminal_runtime_is_reenqueueable(
    record: &raiko2_runtime::RuntimeTaskRecord,
    metadata: &TaskMetadata,
    engine_state_present: bool,
) -> bool {
    matches!(
        record.runner_status,
        RuntimeRunnerStatus::Allocated | RuntimeRunnerStatus::Running
    ) && metadata.has_runtime_progress()
        && !metadata.has_remote_submission_progress()
        && !engine_state_present
}

async fn registered_engine_state_present(
    engine: &Arc<dyn EngineHandle>,
    pipeline_key: PipelineKey,
    metadata: &TaskMetadata,
) -> Result<bool, ApiError> {
    for proposal in &metadata.proposals {
        let Some(task_id) = proposal.engine_task_id(pipeline_key) else {
            continue;
        };

        for stage_id in proposal_task_chain_ids(&task_id) {
            if engine
                .get_status(stage_id)
                .await
                .map_err(|err| ApiError::internal(format!("failed to read task status: {err}")))?
                .is_some()
            {
                return Ok(true);
            }
        }
    }

    if let Some(task_id) = metadata.aggregate_engine_task_id(pipeline_key)
        && engine
            .get_status(task_id)
            .await
            .map_err(|err| ApiError::internal(format!("failed to read aggregation status: {err}")))?
            .is_some()
    {
        return Ok(true);
    }

    Ok(false)
}

fn failed_stage_is_reenqueueable(
    record: &raiko2_runtime::RuntimeTaskRecord,
    metadata: &TaskMetadata,
) -> bool {
    if record.proof_path.is_some() {
        return false;
    }

    match FailedStage::from_active_stage(metadata.runtime.active_stage.as_deref()) {
        Some(FailedStage::Preflight | FailedStage::Validation | FailedStage::Encode) => true,
        Some(FailedStage::Prove) => {
            proposal_failed_stage_is_reenqueueable(record.pipeline_key, metadata)
        }
        Some(FailedStage::Aggregate) => {
            stage_runtime_is_reenqueueable(metadata.aggregate_runtime())
        }
        None => {
            record.provider_request_id.is_none()
                && record.remote_tx_hash.is_none()
                && !metadata.has_remote_submission_progress()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FailedStage {
    Preflight,
    Validation,
    Encode,
    Prove,
    Aggregate,
}

impl FailedStage {
    fn from_active_stage(stage: Option<&str>) -> Option<Self> {
        match stage {
            Some("preflight") => Some(Self::Preflight),
            Some("validation") => Some(Self::Validation),
            Some("encode") => Some(Self::Encode),
            Some("prove") => Some(Self::Prove),
            Some("aggregate") => Some(Self::Aggregate),
            _ => None,
        }
    }
}

fn stage_runtime_is_reenqueueable(runtime: Option<&TaskRuntimeMetadata>) -> bool {
    runtime.is_none_or(|runtime| {
        !runtime.has_remote_submission_progress() || runtime.has_resumable_remote_submission()
    })
}

fn proposal_failed_stage_is_reenqueueable(
    pipeline_key: PipelineKey,
    metadata: &TaskMetadata,
) -> bool {
    if let Some(proposal) = metadata
        .proposals
        .iter()
        .find(|proposal| proposal_stage_failed(pipeline_key, metadata, proposal))
    {
        return stage_runtime_is_reenqueueable(metadata.proposal_runtime(&proposal.task_id));
    }

    match metadata.proposals.as_slice() {
        [proposal] => stage_runtime_is_reenqueueable(metadata.proposal_runtime(&proposal.task_id)),
        _ => proposal_failed_before_remote_submission(metadata),
    }
}

fn proposal_stage_failed(
    pipeline_key: PipelineKey,
    metadata: &TaskMetadata,
    proposal: &ProposalTask,
) -> bool {
    let Some(task_id) = proposal.engine_task_id(pipeline_key) else {
        return false;
    };
    let timing_key = stage_task_ref(&task_id);
    metadata
        .runtime
        .stage_timings
        .get(&timing_key)
        .is_some_and(|timing| {
            timing.stage == "prove" && timing.terminal_status.as_deref() == Some("failed")
        })
}

fn proposal_failed_before_remote_submission(metadata: &TaskMetadata) -> bool {
    !metadata.proposals.is_empty()
        && metadata
            .runtime
            .proposals
            .values()
            .all(|runtime| !runtime.has_remote_submission_progress())
}

fn response_is_completed(response: &Response) -> bool {
    response
        .extensions()
        .get::<CompatibilityResponseStatus>()
        .is_some_and(|status| status.completed)
}

async fn compatibility_response_for_task(
    state: &AppState,
    task_id: &str,
) -> Result<Response, ApiError> {
    let lookup = load_task_lookup(state, task_id).await?;
    let proof_type = BatchProofType::from_canonical(lookup.metadata.proof_type);
    let task = load_task_data_from_lookup(state, task_id, &lookup).await?;

    let status = task.status.clone();
    let response = match task.status {
        ProofStatus::Pending => legacy_status_response(proof_type, LegacyTaskStatus::Registered),
        ProofStatus::Proving => {
            legacy_status_response(proof_type, LegacyTaskStatus::WorkInProgress)
        }
        ProofStatus::Completed => legacy_proof_response(
            proof_type,
            legacy_root_proof_material(&lookup.record, &lookup.metadata, task.proof).await?,
        ),
        ProofStatus::Failed => legacy_status_response(
            proof_type,
            LegacyTaskStatus::AnyhowError(
                task.error
                    .unwrap_or_else(|| format!("task {task_id} failed")),
            ),
        ),
        ProofStatus::Cancelled => legacy_status_response(proof_type, LegacyTaskStatus::Cancelled),
    };
    Ok(with_compatibility_status(response, &status))
}

#[derive(Clone)]
struct CompatibilityResponseStatus {
    completed: bool,
}

fn with_compatibility_status(mut response: Response, status: &ProofStatus) -> Response {
    response
        .extensions_mut()
        .insert(CompatibilityResponseStatus {
            completed: matches!(status, ProofStatus::Completed),
        });
    response
}

async fn legacy_root_proof_material(
    record: &raiko2_runtime::RuntimeTaskRecord,
    metadata: &TaskMetadata,
    fallback_proof: Option<String>,
) -> Result<Proof, ApiError> {
    if root_proof_artifact_refs(metadata, record.pipeline_key).is_some()
        && let Some(proof) = load_persisted_root_proof_material(record).await?
    {
        return Ok(proof);
    }

    Ok(Proof {
        proof: fallback_proof,
        ..Proof::default()
    })
}

fn legacy_status_response(proof_type: BatchProofType, status: LegacyTaskStatus) -> Response {
    Json(LegacyProofEnvelope {
        status: "ok",
        proof_type: proof_type.as_str().to_string(),
        batch_id: None,
        data: LegacyProofData::Status { status },
    })
    .into_response()
}

fn legacy_proof_response(proof_type: BatchProofType, proof: Proof) -> Response {
    Json(LegacyProofEnvelope {
        status: "ok",
        proof_type: proof_type.as_str().to_string(),
        batch_id: None,
        data: LegacyProofData::Proof { proof },
    })
    .into_response()
}

fn legacy_error_response(error: &'static str, message: String) -> Response {
    (
        StatusCode::OK,
        Json(LegacyProofError {
            status: "error",
            error,
            message,
        }),
    )
        .into_response()
}

fn legacy_api_error_response(err: ApiError) -> Response {
    let error = match err.status {
        StatusCode::BAD_REQUEST => "invalid_request_config",
        StatusCode::NOT_FOUND => "not_found",
        StatusCode::SERVICE_UNAVAILABLE => "service_unavailable",
        _ => "anyhow_error",
    };
    legacy_error_response(error, err.message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BoundlessPairConfig, Config};
    use crate::server::sampling::ZkAnySampler;
    use crate::server::state::{EngineHandle, StaticPipelineFactory};
    use anyhow::{Result, anyhow};
    use raiko2_engine::EngineTaskId;
    use raiko2_pipeline::{PipelineRoute, RunnerKind};
    use raiko2_primitives::SupportedChainSpecs;
    use raiko2_queue::TaskStoreError;
    use raiko2_runtime::{
        ProofArtifactRegistration, RunnerStatus as RuntimeRunnerStatus, RuntimeManager,
        RuntimeTaskRecord,
    };
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};

    type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

    fn batch_request_fingerprint_for_test(submission: &CanonicalBatchSubmission) -> Result<String> {
        batch_request_fingerprint(submission).map_err(|err| anyhow!(err.message))
    }

    struct NoopEngine;

    impl EngineHandle for NoopEngine {
        fn submit_proposal_proof_with_dependencies(
            &self,
            _request: ProposalTaskRequest,
            _dependencies: Vec<EngineTaskId>,
        ) -> BoxFuture<'_, Result<EngineTaskId, TaskStoreError>> {
            Box::pin(async { panic!("unexpected proposal submission") })
        }

        fn submit_aggregation_proof_from_inputs(
            &self,
            _request: AggregationTaskRequest,
            _inputs: Vec<AggregateProofInput>,
        ) -> BoxFuture<'_, Result<EngineTaskId, TaskStoreError>> {
            Box::pin(async { panic!("unexpected aggregation input submission") })
        }

        fn get_status(
            &self,
            _id: EngineTaskId,
        ) -> BoxFuture<'_, Result<Option<EngineStatusView>, TaskStoreError>> {
            Box::pin(async { Ok(None) })
        }

        fn cancel(&self, _id: EngineTaskId) -> BoxFuture<'_, Result<(), TaskStoreError>> {
            Box::pin(async { Ok(()) })
        }

        fn remove(&self, _id: EngineTaskId) -> BoxFuture<'_, Result<(), TaskStoreError>> {
            Box::pin(async { Ok(()) })
        }
    }

    struct RecordingEngine {
        pipeline_key: PipelineKey,
        proposals: Mutex<Vec<(ProposalTaskRequest, Vec<EngineTaskId>)>>,
        aggregate_inputs: Mutex<Vec<AggregateProofInput>>,
    }

    impl RecordingEngine {
        const fn new(pipeline_key: PipelineKey) -> Self {
            Self {
                pipeline_key,
                proposals: Mutex::new(Vec::new()),
                aggregate_inputs: Mutex::new(Vec::new()),
            }
        }
    }

    impl EngineHandle for RecordingEngine {
        fn submit_proposal_proof_with_dependencies(
            &self,
            request: ProposalTaskRequest,
            dependencies: Vec<EngineTaskId>,
        ) -> BoxFuture<'_, Result<EngineTaskId, TaskStoreError>> {
            Box::pin(async move {
                self.proposals
                    .lock()
                    .expect("proposal submissions mutex")
                    .push((request.clone(), dependencies));
                Ok(proposal_task_id(self.pipeline_key, request))
            })
        }

        fn submit_aggregation_proof_from_inputs(
            &self,
            request: AggregationTaskRequest,
            inputs: Vec<AggregateProofInput>,
        ) -> BoxFuture<'_, Result<EngineTaskId, TaskStoreError>> {
            Box::pin(async move {
                *self
                    .aggregate_inputs
                    .lock()
                    .expect("aggregate inputs mutex") = inputs;
                Ok(EngineTaskId::new(EngineTaskKey::Aggregate {
                    pipeline: self.pipeline_key,
                    request,
                }))
            })
        }

        fn get_status(
            &self,
            _id: EngineTaskId,
        ) -> BoxFuture<'_, Result<Option<EngineStatusView>, TaskStoreError>> {
            Box::pin(async { Ok(None) })
        }

        fn cancel(&self, _id: EngineTaskId) -> BoxFuture<'_, Result<(), TaskStoreError>> {
            Box::pin(async { Ok(()) })
        }

        fn remove(&self, _id: EngineTaskId) -> BoxFuture<'_, Result<(), TaskStoreError>> {
            Box::pin(async { Ok(()) })
        }
    }

    fn resolved_pair() -> ResolvedNetworkPair {
        let specs = SupportedChainSpecs::default();
        ResolvedNetworkPair {
            key: "taiko_dev/ethereum".to_string(),
            network: "taiko_dev".to_string(),
            l1_network: "ethereum".to_string(),
            l1_rpc: "http://l1.example".to_string(),
            l2_rpc: "http://l2.example".to_string(),
            l2_provider: crate::config::L2ProviderKind::Reth,
            l2_witness_rpc: "http://l2w.example".to_string(),
            sp1_verifier_rpc_url: None,
            sp1_verifier_address: None,
            boundless: BoundlessPairConfig::default(),
            l1_spec: specs
                .get_chain_spec("ethereum")
                .expect("ethereum chain spec"),
            l2_spec: specs
                .get_chain_spec("taiko_dev")
                .expect("taiko_dev chain spec"),
        }
    }

    fn canonical_submission(
        route: CanonicalProofRoute,
        aggregate_requested: bool,
    ) -> CanonicalBatchSubmission {
        CanonicalBatchSubmission {
            public_task_id: format!("task-{aggregate_requested}"),
            pair: resolved_pair(),
            route,
            proposals: vec![CanonicalProposal {
                proposal_id: 7,
                checkpoint: None,
                l1_inclusion_block_number: 11,
                l2_block_numbers: vec![7],
                l2_block_range: L2BlockRange { start: 7, end: 7 },
                last_anchor_block_number: 6,
            }],
            aggregate_requested,
            prover_config: ProverTaskConfig::default(),
            prover_type: None,
            blob_proof_type: None,
            prover: None,
            graffiti: None,
            execution_mode: None,
        }
    }

    fn canonical_multi_submission(route: CanonicalProofRoute) -> CanonicalBatchSubmission {
        let mut submission = canonical_submission(route, true);
        submission.public_task_id = "task-mixed-cache".to_string();
        submission.proposals = vec![
            CanonicalProposal {
                proposal_id: 7,
                checkpoint: None,
                l1_inclusion_block_number: 11,
                l2_block_numbers: vec![7],
                l2_block_range: L2BlockRange { start: 7, end: 7 },
                last_anchor_block_number: 6,
            },
            CanonicalProposal {
                proposal_id: 8,
                checkpoint: None,
                l1_inclusion_block_number: 12,
                l2_block_numbers: vec![8],
                l2_block_range: L2BlockRange { start: 8, end: 8 },
                last_anchor_block_number: 7,
            },
        ];
        submission
    }

    fn valid_native_proof() -> Proof {
        Proof {
            proof: Some("0xproof".to_string()),
            input: Some(alloy_primitives::B256::ZERO),
            extra_data: Some(serde_json::json!({ "native": true })),
            ..Proof::default()
        }
    }

    async fn write_test_proof_artifact(
        runtime: &RuntimeManager,
        network_pair: &str,
        proof_ref: &str,
        proof: &Proof,
    ) -> Result<()> {
        let proof_path = runtime
            .write_proof_artifact_bytes(network_pair, proof_ref, &serde_json::to_vec_pretty(proof)?)
            .await?;
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: network_pair.to_string(),
                proof_ref: proof_ref.to_string(),
                pipeline_key: PipelineKey::ShastaNative,
                route: "native/local".parse().expect("route"),
                proof_path: proof_path.display().to_string(),
            })
            .await?;
        Ok(())
    }

    fn test_state(runtime: Arc<RuntimeManager>, engine: Arc<dyn EngineHandle>) -> AppState {
        let config = Config::default();
        let mut factory = StaticPipelineFactory::default();
        factory.insert("taiko_dev/ethereum", PipelineKey::ShastaNative, engine);
        AppState {
            zk_any_sampler: Arc::new(Mutex::new(ZkAnySampler::from_config(&config.prover.zk_any))),
            config: Arc::new(config),
            pipelines: Arc::new(factory),
            runtime,
        }
    }

    fn runtime_record(
        runner_status: RuntimeRunnerStatus,
        metadata: &TaskMetadata,
    ) -> RuntimeTaskRecord {
        RuntimeTaskRecord {
            task_id: "task_public".to_string(),
            pipeline_key: PipelineKey::ShastaRisc0Network,
            route: "risc0/network".parse().expect("parse route"),
            task_kind: "hoodi_batch".to_string(),
            proposal_id: Some(42),
            proof_ids: vec![],
            runner_status,
            task_dir: "/tmp/task_public".to_string(),
            image_ref: None,
            provider_request_id: None,
            remote_tx_hash: None,
            proof_path: None,
            error: None,
            metadata: serde_json::to_value(metadata).expect("serialize metadata"),
            request_fingerprint: Some("0xfingerprint".to_string()),
            updated_at: 1,
        }
    }

    fn task_metadata_with_stage(stage: Option<&str>) -> TaskMetadata {
        TaskMetadata {
            network_pair: "taiko_dev/ethereum".to_string(),
            network: "taiko_dev".to_string(),
            l1_network: "ethereum".to_string(),
            proof_type: ProofType::Risc0,
            prover_type: None,
            execution_mode: None,
            aggregate_requested: true,
            proposals: vec![ProposalTask {
                proposal_id: 42,
                checkpoint: None,
                l1_inclusion_block_number: 1,
                l2_block_numbers: vec![42],
                last_anchor_block_number: 41,
                task_id: "proposal-task".to_string(),
                request: None,
            }],
            aggregate_task_id: None,
            aggregate_request: None,
            aggregate_input_artifacts: Vec::new(),
            runtime: RuntimeMetadata {
                active_stage: stage.map(str::to_string),
                ..RuntimeMetadata::default()
            },
        }
    }

    #[test]
    fn failed_submission_before_remote_progress_is_reenqueueable() {
        let metadata = task_metadata_with_stage(Some("preflight"));

        for pipeline_key in [
            PipelineKey::ShastaNative,
            PipelineKey::ShastaRisc0,
            PipelineKey::ShastaSp1,
            PipelineKey::ShastaRisc0Network,
        ] {
            let mut record = runtime_record(RuntimeRunnerStatus::Failed, &metadata);
            record.pipeline_key = pipeline_key;

            assert!(
                should_reenqueue_existing_submission_without_engine(&record, &metadata),
                "{pipeline_key}"
            );
        }
    }

    #[test]
    fn failed_submission_with_remote_progress_is_not_reenqueueable() {
        let mut metadata = task_metadata_with_stage(Some("prove"));
        metadata.runtime.proposals.insert(
            "proposal-task".to_string(),
            TaskRuntimeMetadata {
                provider_request_id: Some("0x1234".to_string()),
                ..TaskRuntimeMetadata::default()
            },
        );
        let record = runtime_record(RuntimeRunnerStatus::Failed, &metadata);

        assert!(!should_reenqueue_existing_submission_without_engine(
            &record, &metadata
        ));
    }

    #[test]
    fn failed_submission_with_boundless_resume_metadata_is_reenqueueable_for_failed_prove_stage() {
        let mut metadata = task_metadata_with_stage(Some("prove"));
        metadata.runtime.proposals.insert(
            "proposal-task".to_string(),
            TaskRuntimeMetadata {
                provider_request_id: Some("0x1234".to_string()),
                expires_at: Some(123_456),
                ..TaskRuntimeMetadata::default()
            },
        );

        for pipeline_key in [
            PipelineKey::ShastaNative,
            PipelineKey::ShastaRisc0,
            PipelineKey::ShastaSp1,
            PipelineKey::ShastaRisc0Network,
        ] {
            let mut record = runtime_record(RuntimeRunnerStatus::Failed, &metadata);
            record.pipeline_key = pipeline_key;
            record.provider_request_id = Some("0x1234".to_string());

            assert!(
                should_reenqueue_existing_submission_without_engine(&record, &metadata),
                "{pipeline_key}"
            );
        }
    }

    #[test]
    fn failed_prove_submission_before_remote_submission_is_reenqueueable() {
        let metadata = task_metadata_with_stage(Some("prove"));
        let record = runtime_record(RuntimeRunnerStatus::Failed, &metadata);

        assert!(should_reenqueue_existing_submission_without_engine(
            &record, &metadata
        ));
    }

    #[test]
    fn failed_submission_with_sp1_resume_metadata_is_reenqueueable_for_failed_prove_stage() {
        let mut metadata = task_metadata_with_stage(Some("prove"));
        metadata.proof_type = ProofType::Sp1;
        metadata.runtime.proposals.insert(
            "proposal-task".to_string(),
            TaskRuntimeMetadata {
                provider_request_id: Some("0xsp1".to_string()),
                sp1_network_mode: Some(raiko2_prover::Sp1NetworkMode::Reserved),
                sp1_fulfillment_strategy: Some(raiko2_prover::Sp1FulfillmentStrategy::Reserved),
                sp1_timeout_secs: Some(7_200),
                ..TaskRuntimeMetadata::default()
            },
        );

        for pipeline_key in [
            PipelineKey::ShastaNative,
            PipelineKey::ShastaRisc0,
            PipelineKey::ShastaSp1,
            PipelineKey::ShastaRisc0Network,
        ] {
            let mut record = runtime_record(RuntimeRunnerStatus::Failed, &metadata);
            record.pipeline_key = pipeline_key;
            record.provider_request_id = Some("0xsp1".to_string());

            assert!(
                should_reenqueue_existing_submission_without_engine(&record, &metadata),
                "{pipeline_key}"
            );
        }
    }

    #[test]
    fn failed_sp1_prove_before_network_submission_is_reenqueueable() {
        let mut metadata = task_metadata_with_stage(Some("prove"));
        metadata.proof_type = ProofType::Sp1;
        let mut record = runtime_record(RuntimeRunnerStatus::Failed, &metadata);
        record.pipeline_key = PipelineKey::ShastaSp1;

        assert!(should_reenqueue_existing_submission_without_engine(
            &record, &metadata
        ));
    }

    #[test]
    fn running_submission_with_stale_runtime_progress_is_reenqueueable() {
        let metadata = task_metadata_with_stage(Some("prove"));
        let mut record = runtime_record(RuntimeRunnerStatus::Running, &metadata);
        record.pipeline_key = PipelineKey::ShastaSp1;

        assert!(stale_nonterminal_runtime_is_reenqueueable(
            &record, &metadata, false
        ));
        assert!(!stale_nonterminal_runtime_is_reenqueueable(
            &record, &metadata, true
        ));
    }

    #[test]
    fn running_submission_with_remote_progress_is_not_reenqueueable() {
        let mut metadata = task_metadata_with_stage(Some("prove"));
        metadata.runtime.proposals.insert(
            "proposal-task".to_string(),
            TaskRuntimeMetadata {
                provider_request_id: Some("0xsp1".to_string()),
                sp1_network_mode: Some(raiko2_prover::Sp1NetworkMode::Reserved),
                ..TaskRuntimeMetadata::default()
            },
        );
        let mut record = runtime_record(RuntimeRunnerStatus::Running, &metadata);
        record.pipeline_key = PipelineKey::ShastaSp1;

        assert!(!stale_nonterminal_runtime_is_reenqueueable(
            &record, &metadata, false
        ));
    }

    #[tokio::test]
    async fn stale_recovery_returns_cached_artifact_before_reenqueue() -> Result<()> {
        let route = CanonicalProofRoute {
            route: PipelineRoute::new(crate::config::GuestSystem::Native, RunnerKind::Local),
        };
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "stale-cache-before-reenqueue",
        ))?);
        let submission = canonical_submission(route, false);
        let mut metadata = task_metadata_with_stage(Some("prove"));
        metadata.proof_type = ProofType::Native;
        metadata.aggregate_requested = false;
        metadata.proposals[0].task_id = "proposal-task".to_string();
        let mut record = runtime_record(RuntimeRunnerStatus::Running, &metadata);
        record.pipeline_key = PipelineKey::ShastaNative;
        record.route = "native/local".parse().expect("parse route");
        record.error = Some("stale running".to_string());
        runtime.upsert_task(&record).await?;
        write_test_proof_artifact(
            &runtime,
            &metadata.network_pair,
            &metadata.proposals[0].task_id,
            &valid_native_proof(),
        )
        .await?;
        let state = test_state(Arc::clone(&runtime), Arc::new(NoopEngine));

        let response = handle_existing_batch_task(&state, &submission, record.clone())
            .await
            .map_err(|err| anyhow::anyhow!(err.message))?;

        assert!(response_is_completed(&response));
        let stored = runtime
            .get_task(&record.task_id)
            .await?
            .expect("runtime task");
        assert_eq!(stored.runner_status, RuntimeRunnerStatus::Running);
        assert_eq!(stored.error.as_deref(), Some("stale running"));
        Ok(())
    }

    #[tokio::test]
    async fn recovery_enqueue_failure_restores_previous_runtime_status() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "recover-rollback",
        ))?);
        let metadata = task_metadata_with_stage(Some("preflight"));
        let mut record = runtime_record(RuntimeRunnerStatus::Failed, &metadata);
        record.pipeline_key = PipelineKey::ShastaNative;
        record.route = "native/local".parse().expect("parse route");
        record.error = Some("old failure".to_string());
        runtime.upsert_task(&record).await?;
        let state = test_state(Arc::clone(&runtime), Arc::new(NoopEngine));

        let err = recover_existing_task(&state, &record, || async {
            Err(ApiError::internal("enqueue failed"))
        })
        .await
        .expect_err("enqueue failure");

        assert_eq!(err.message, "enqueue failed");
        let stored = runtime
            .get_task(&record.task_id)
            .await?
            .expect("runtime task");
        assert_eq!(stored.runner_status, RuntimeRunnerStatus::Failed);
        assert_eq!(stored.error.as_deref(), Some("old failure"));
        Ok(())
    }

    #[test]
    fn failed_submission_after_remote_stage_without_resume_metadata_is_not_reenqueueable() {
        let mut metadata = task_metadata_with_stage(Some("prove"));
        metadata.runtime.proposals.insert(
            "proposal-task".to_string(),
            TaskRuntimeMetadata {
                remote_tx_hash: Some("0xremote".to_string()),
                ..TaskRuntimeMetadata::default()
            },
        );
        let mut record = runtime_record(RuntimeRunnerStatus::Failed, &metadata);
        record.pipeline_key = PipelineKey::ShastaNative;

        assert!(!should_reenqueue_existing_submission_without_engine(
            &record, &metadata
        ));
    }

    #[test]
    fn failed_aggregate_before_remote_submission_is_reenqueueable() {
        let metadata = task_metadata_with_stage(Some("aggregate"));
        let record = runtime_record(RuntimeRunnerStatus::Failed, &metadata);

        assert!(should_reenqueue_existing_submission_without_engine(
            &record, &metadata
        ));
    }

    #[test]
    fn failed_aggregate_ignores_proposal_resume_metadata() {
        let mut metadata = task_metadata_with_stage(Some("aggregate"));
        metadata.runtime.proposals.insert(
            "proposal-task".to_string(),
            TaskRuntimeMetadata {
                provider_request_id: Some("0xproposal".to_string()),
                expires_at: Some(123_456),
                ..TaskRuntimeMetadata::default()
            },
        );
        metadata.runtime.aggregate = Some(TaskRuntimeMetadata {
            provider_request_id: Some("0xaggregate".to_string()),
            ..TaskRuntimeMetadata::default()
        });
        let mut record = runtime_record(RuntimeRunnerStatus::Failed, &metadata);
        record.provider_request_id = Some("0xproposal".to_string());

        assert!(!should_reenqueue_existing_submission_without_engine(
            &record, &metadata
        ));
    }

    #[test]
    fn failed_aggregate_with_aggregate_resume_metadata_is_reenqueueable() {
        let mut metadata = task_metadata_with_stage(Some("aggregate"));
        metadata.runtime.proposals.insert(
            "proposal-task".to_string(),
            TaskRuntimeMetadata {
                provider_request_id: Some("0xproposal".to_string()),
                expires_at: Some(123_456),
                ..TaskRuntimeMetadata::default()
            },
        );
        metadata.runtime.aggregate = Some(TaskRuntimeMetadata {
            provider_request_id: Some("0xaggregate".to_string()),
            expires_at: Some(456_789),
            ..TaskRuntimeMetadata::default()
        });
        let mut record = runtime_record(RuntimeRunnerStatus::Failed, &metadata);
        record.provider_request_id = Some("0xproposal".to_string());

        assert!(should_reenqueue_existing_submission_without_engine(
            &record, &metadata
        ));
    }

    #[tokio::test]
    async fn native_proposal_task_id_is_reused_across_aggregate_flags() {
        let route = CanonicalProofRoute {
            route: PipelineRoute::new(crate::config::GuestSystem::Native, RunnerKind::Local),
        };
        let runtime =
            RuntimeManager::new(unique_test_runtime_root("plan-task-id")).expect("runtime manager");
        let single_submission = canonical_submission(route, false);
        let single_fingerprint =
            batch_request_fingerprint(&single_submission).expect("single fingerprint");
        let single = build_submission_plan(&runtime, &single_submission, &single_fingerprint)
            .await
            .expect("single submission plan");
        let aggregate_submission = canonical_submission(route, true);
        let aggregate_fingerprint =
            batch_request_fingerprint(&aggregate_submission).expect("aggregate fingerprint");
        let aggregate =
            build_submission_plan(&runtime, &aggregate_submission, &aggregate_fingerprint)
                .await
                .expect("aggregate submission plan");

        assert_eq!(single.proposals.len(), 1);
        assert_eq!(aggregate.proposals.len(), 1);
        assert_eq!(single.proposals[0].task_id, aggregate.proposals[0].task_id);
    }

    #[tokio::test]
    async fn aggregate_plan_uses_request_fingerprint_as_idempotent_key() {
        let route = CanonicalProofRoute {
            route: PipelineRoute::new(crate::config::GuestSystem::Native, RunnerKind::Local),
        };
        let runtime = RuntimeManager::new(unique_test_runtime_root("aggregate-idempotent-key"))
            .expect("runtime manager");
        let mut first_submission = canonical_submission(route, true);
        first_submission.public_task_id = "task-public-a".to_string();
        let mut second_submission = first_submission.clone();
        second_submission.public_task_id = "task-public-b".to_string();

        let first_fingerprint =
            batch_request_fingerprint(&first_submission).expect("first fingerprint");
        let second_fingerprint =
            batch_request_fingerprint(&second_submission).expect("second fingerprint");
        assert_eq!(first_fingerprint, second_fingerprint);

        let first = build_submission_plan(&runtime, &first_submission, &first_fingerprint)
            .await
            .expect("first submission plan");
        let second = build_submission_plan(&runtime, &second_submission, &second_fingerprint)
            .await
            .expect("second submission plan");

        let first_aggregate = first.aggregate.expect("first aggregate");
        let second_aggregate = second.aggregate.expect("second aggregate");
        assert_eq!(first_aggregate.task_ref, second_aggregate.task_ref);
        assert_eq!(first_aggregate.task_id, second_aggregate.task_id);
        assert_eq!(
            first_aggregate.request.request_id,
            aggregate_request_id(&first_fingerprint)
        );
        assert_ne!(
            first_aggregate.request.request_id,
            first_submission.public_task_id
        );
    }

    #[tokio::test]
    async fn aggregate_plan_uses_cached_artifact_refs_and_enqueues_only_missing_proposals()
    -> Result<()> {
        let route = CanonicalProofRoute {
            route: PipelineRoute::new(crate::config::GuestSystem::Native, RunnerKind::Local),
        };
        let runtime = RuntimeManager::new(unique_test_runtime_root("mixed-cache-plan"))
            .expect("runtime manager");
        let submission = canonical_multi_submission(route);
        let first_request = ProposalTaskRequest {
            proposal_id: 7,
            l2_block_range: Some(L2BlockRange { start: 7, end: 7 }),
            l1_inclusion_block_number: 11,
            last_anchor_block_number: 6,
            checkpoint: None,
            blob_proof_type: None,
            prover: None,
            graffiti: None,
            prover_config: ProverTaskConfig::default(),
        };
        let cached_ref = proposal_task_ref(PipelineKey::ShastaNative, &first_request);
        write_test_proof_artifact(
            &runtime,
            &submission.pair.key,
            &cached_ref,
            &valid_native_proof(),
        )
        .await
        .expect("write cached proof");

        let request_fingerprint = batch_request_fingerprint_for_test(&submission)?;
        let plan = build_submission_plan(&runtime, &submission, &request_fingerprint)
            .await
            .expect("submission plan");
        assert!(matches!(
            plan.proposal_sources[0],
            ProposalPlanSource::Cached
        ));
        assert!(matches!(
            plan.proposal_sources[1],
            ProposalPlanSource::Pending
        ));

        let recorder = Arc::new(RecordingEngine::new(PipelineKey::ShastaNative));
        let engine: Arc<dyn EngineHandle> = recorder.clone();
        enqueue_submission_plan(&engine, &plan)
            .await
            .expect("enqueue plan");

        let proposals = recorder.proposals.lock().expect("proposal submissions");
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].0.proposal_id, 8);
        assert!(proposals[0].1.is_empty());
        drop(proposals);

        let aggregate_inputs = recorder.aggregate_inputs.lock().expect("aggregate inputs");
        assert_eq!(aggregate_inputs.len(), 2);
        assert_eq!(
            aggregate_inputs[0],
            AggregateProofInput::ProofArtifact(proof_artifact_ref(
                &runtime,
                &submission.pair.key,
                &cached_ref
            ))
        );
        assert_eq!(
            aggregate_inputs[1],
            AggregateProofInput::PendingProofArtifact {
                artifact: proof_artifact_ref(
                    &runtime,
                    &submission.pair.key,
                    &plan.proposals[1].task_ref
                ),
                dependency: Box::new(plan.proposals[1].task_id.clone()),
            }
        );
        Ok(())
    }

    #[tokio::test]
    async fn aggregate_recovery_only_reenqueues_aggregate_from_artifacts() -> Result<()> {
        let route = CanonicalProofRoute {
            route: PipelineRoute::new(crate::config::GuestSystem::Native, RunnerKind::Local),
        };
        let runtime = RuntimeManager::new(unique_test_runtime_root("aggregate-recovery-artifacts"))
            .expect("runtime manager");
        let submission = canonical_multi_submission(route);
        let request_fingerprint = batch_request_fingerprint_for_test(&submission)?;
        let plan = build_submission_plan(&runtime, &submission, &request_fingerprint)
            .await
            .expect("submission plan");
        for proposal in &plan.proposals {
            write_test_proof_artifact(
                &runtime,
                &submission.pair.key,
                &proposal.task_ref,
                &valid_native_proof(),
            )
            .await?;
        }

        let mut metadata = build_task_metadata(
            &submission.pair,
            BuildTaskMetadataParams {
                network: &submission.pair.network,
                l1_network: &submission.pair.l1_network,
                proof_type: submission.route.proof_type(),
                prover_type: submission.prover_type,
                execution_mode: submission.execution_mode,
                aggregate_requested: true,
            },
            &plan.proposals,
            plan.aggregate.as_ref(),
        );
        metadata.runtime.active_stage = Some("aggregate".to_string());
        let mut record = runtime_record(RuntimeRunnerStatus::Failed, &metadata);
        record.task_id.clone_from(&submission.public_task_id);
        record.pipeline_key = route.pipeline_key();
        record.route = route.route;

        let recorder = Arc::new(RecordingEngine::new(route.pipeline_key()));
        let engine: Arc<dyn EngineHandle> = recorder.clone();
        reenqueue_existing_batch_aggregate_task(&runtime, &engine, &record, &metadata)
            .await
            .expect("aggregate recovery should use persisted proof artifacts");

        let proposals = recorder.proposals.lock().expect("proposal submissions");
        assert!(proposals.is_empty());
        drop(proposals);

        let aggregate_inputs = recorder.aggregate_inputs.lock().expect("aggregate inputs");
        assert_eq!(aggregate_inputs.len(), 2);
        assert!(aggregate_inputs.iter().all(|input| {
            matches!(
                input,
                AggregateProofInput::ProofArtifact(artifact)
                    if artifact.proof_ref.starts_with("task_")
            )
        }));
        Ok(())
    }

    #[tokio::test]
    async fn aggregate_recovery_does_not_resubmit_missing_proposal_artifacts() -> Result<()> {
        let route = CanonicalProofRoute {
            route: PipelineRoute::new(crate::config::GuestSystem::Native, RunnerKind::Local),
        };
        let runtime = RuntimeManager::new(unique_test_runtime_root("aggregate-recovery-missing"))
            .expect("runtime manager");
        let submission = canonical_multi_submission(route);
        let request_fingerprint = batch_request_fingerprint_for_test(&submission)?;
        let plan = build_submission_plan(&runtime, &submission, &request_fingerprint)
            .await
            .expect("submission plan");
        let mut metadata = build_task_metadata(
            &submission.pair,
            BuildTaskMetadataParams {
                network: &submission.pair.network,
                l1_network: &submission.pair.l1_network,
                proof_type: submission.route.proof_type(),
                prover_type: submission.prover_type,
                execution_mode: submission.execution_mode,
                aggregate_requested: true,
            },
            &plan.proposals,
            plan.aggregate.as_ref(),
        );
        metadata.runtime.active_stage = Some("aggregate".to_string());
        let mut record = runtime_record(RuntimeRunnerStatus::Failed, &metadata);
        record.task_id.clone_from(&submission.public_task_id);
        record.pipeline_key = route.pipeline_key();
        record.route = route.route;

        let recorder = Arc::new(RecordingEngine::new(route.pipeline_key()));
        let engine: Arc<dyn EngineHandle> = recorder.clone();
        let err = reenqueue_existing_batch_aggregate_task(&runtime, &engine, &record, &metadata)
            .await
            .expect_err("missing proposal artifact should fail aggregate recovery");

        assert!(
            err.message.contains("missing completed proposal proof"),
            "unexpected error: {}",
            err.message
        );
        let proposals = recorder.proposals.lock().expect("proposal submissions");
        assert!(proposals.is_empty());
        let aggregate_inputs = recorder.aggregate_inputs.lock().expect("aggregate inputs");
        assert!(aggregate_inputs.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn corrupt_proof_artifact_is_treated_as_cache_miss() -> Result<()> {
        let route = CanonicalProofRoute {
            route: PipelineRoute::new(crate::config::GuestSystem::Native, RunnerKind::Local),
        };
        let runtime = RuntimeManager::new(unique_test_runtime_root("corrupt-cache-plan"))
            .expect("runtime manager");
        let submission = canonical_submission(route, true);
        let request = ProposalTaskRequest {
            proposal_id: 7,
            l2_block_range: Some(L2BlockRange { start: 7, end: 7 }),
            l1_inclusion_block_number: 11,
            last_anchor_block_number: 6,
            checkpoint: None,
            blob_proof_type: None,
            prover: None,
            graffiti: None,
            prover_config: ProverTaskConfig::default(),
        };
        let proof_ref = proposal_task_ref(PipelineKey::ShastaNative, &request);
        let proof_path = runtime
            .write_proof_artifact_bytes(&submission.pair.key, &proof_ref, b"{bad-json")
            .await
            .expect("write corrupt artifact");
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: submission.pair.key.clone(),
                proof_ref,
                pipeline_key: PipelineKey::ShastaNative,
                route: "native/local".parse().expect("route"),
                proof_path: proof_path.display().to_string(),
            })
            .await
            .expect("register corrupt artifact");

        let request_fingerprint = batch_request_fingerprint_for_test(&submission)?;
        let plan = build_submission_plan(&runtime, &submission, &request_fingerprint)
            .await
            .expect("submission plan");
        assert!(matches!(
            plan.proposal_sources[0],
            ProposalPlanSource::Pending
        ));
        assert_eq!(
            plan.aggregate_inputs[0],
            AggregateProofInput::PendingProofArtifact {
                artifact: proof_artifact_ref(
                    &runtime,
                    &submission.pair.key,
                    &proposal_task_ref(PipelineKey::ShastaNative, &request)
                ),
                dependency: Box::new(plan.proposals[0].task_id.clone()),
            }
        );
        Ok(())
    }

    #[tokio::test]
    async fn external_aggregate_inputs_are_persisted_as_artifacts() -> Result<()> {
        let route = CanonicalProofRoute {
            route: PipelineRoute::new(crate::config::GuestSystem::Native, RunnerKind::Local),
        };
        let runtime = RuntimeManager::new(unique_test_runtime_root("external-agg-inputs"))
            .expect("runtime manager");
        let proof = valid_native_proof();
        let (inputs, artifacts) = persist_external_aggregate_input_artifacts(
            &runtime,
            "taiko_dev/ethereum",
            route,
            "0xfingerprint",
            std::slice::from_ref(&proof),
        )
        .await
        .expect("persist aggregate input artifacts");

        assert_eq!(artifacts.len(), 1);
        assert_eq!(
            artifacts[0].proof_ref,
            aggregate_input_proof_ref("0xfingerprint", 0)
        );
        assert_eq!(
            inputs,
            aggregate_inputs_from_artifacts("taiko_dev/ethereum", &artifacts)
        );

        let stored =
            load_proof_artifact_material(&runtime, "taiko_dev/ethereum", &artifacts[0].proof_ref)
                .await?
                .expect("stored aggregate input proof");
        assert_eq!(stored.proof, proof);
        Ok(())
    }

    fn unique_test_runtime_root(prefix: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("raiko2-{prefix}-{unique}"))
    }

    fn proposal_status(status: ProofStatus, proof: Option<&str>) -> ProposalStatus {
        ProposalStatus {
            index: 0,
            proposal_id: 42,
            checkpoint: None,
            task_id: "proposal-task".to_string(),
            status,
            l1_inclusion_block_number: 1,
            l2_block_numbers: vec![42],
            last_anchor_block_number: 41,
            proof: proof.map(str::to_string),
            proof_ref: None,
            proof_path: None,
            error: None,
            runtime: None,
            extra_data: None,
        }
    }

    fn aggregate_status(status: ProofStatus, proof: Option<&str>) -> AggregateStatus {
        AggregateStatus {
            task_id: "aggregate-task".to_string(),
            status,
            proof: proof.map(str::to_string),
            proof_ref: None,
            proof_path: None,
            error: None,
            runtime: None,
            extra_data: None,
        }
    }

    #[test]
    fn resolve_engine_reports_network_pipeline_as_unavailable_when_not_registered() {
        let pair = resolved_pair();
        let mut factory = StaticPipelineFactory::default();
        let risc0_engine: Arc<dyn EngineHandle> = Arc::new(NoopEngine);
        factory.insert(pair.key.clone(), PipelineKey::ShastaRisc0, risc0_engine);

        let mut config = Config::default();
        config.runtime.root = unique_test_runtime_root("resolve-engine-network-config");
        let zk_any_sampler = ZkAnySampler::from_config(&config.prover.zk_any);
        let state = AppState {
            config: Arc::new(config),
            pipelines: Arc::new(factory),
            runtime: Arc::new(
                RuntimeManager::new(unique_test_runtime_root("resolve-engine-network-runtime"))
                    .expect("runtime manager"),
            ),
            zk_any_sampler: Arc::new(Mutex::new(zk_any_sampler)),
        };

        let Err(err) = resolve_engine(&state, &pair.key, PipelineKey::ShastaRisc0Network) else {
            panic!("network pipeline should be unavailable");
        };

        assert_eq!(err.status, StatusCode::NOT_FOUND);
        assert!(
            err.message
                .contains("pipeline not available: shasta-risc0-network"),
            "unexpected error: {}",
            err.message
        );
    }

    #[test]
    fn aggregate_root_waits_for_aggregate_proof_even_after_single_proposal_completes() {
        let proposals = vec![proposal_status(ProofStatus::Completed, Some("0xproposal"))];
        let aggregate = aggregate_status(ProofStatus::Pending, None);

        let root = resolve_root_task_state(
            RuntimeRunnerStatus::Allocated,
            &proposals,
            Some(&aggregate),
            true,
            None,
        );

        assert!(matches!(root.status, ProofStatus::Proving));
        assert_eq!(root.proof, None);
        assert_eq!(root.current_index, Some(1));
    }

    #[test]
    fn aggregate_root_returns_aggregate_proof_not_single_proposal_proof() {
        let proposals = vec![proposal_status(ProofStatus::Completed, Some("0xproposal"))];
        let aggregate = aggregate_status(ProofStatus::Completed, Some("0xaggregate"));

        let root = resolve_root_task_state(
            RuntimeRunnerStatus::Allocated,
            &proposals,
            Some(&aggregate),
            true,
            None,
        );

        assert!(matches!(root.status, ProofStatus::Completed));
        assert_eq!(root.proof.as_deref(), Some("0xaggregate"));
        assert_eq!(root.current_index, None);
    }

    #[test]
    fn non_aggregate_single_proposal_root_returns_proposal_proof() {
        let proposals = vec![proposal_status(ProofStatus::Completed, Some("0xproposal"))];

        let root =
            resolve_root_task_state(RuntimeRunnerStatus::Allocated, &proposals, None, true, None);

        assert!(matches!(root.status, ProofStatus::Completed));
        assert_eq!(root.proof.as_deref(), Some("0xproposal"));
        assert_eq!(root.current_index, None);
    }

    #[test]
    fn canonicalize_proposal_rejects_uint48_overflow() {
        let err = canonicalize_proposal(&ShastaProposal {
            proposal_id: SHASTA_PROPOSAL_ID_MAX + 1,
            checkpoint: None,
            l1_inclusion_block_number: 1,
            l2_block_numbers: vec![1],
            last_anchor_block_number: 0,
        })
        .expect_err("proposal id overflow must fail");

        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(
            err.message.contains("uint48"),
            "unexpected error: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn load_proposal_statuses_tolerates_invalid_legacy_child_id() -> Result<()> {
        let engine: Arc<dyn EngineHandle> = Arc::new(NoopEngine);
        let metadata = TaskMetadata {
            network_pair: "taiko_dev/ethereum".to_string(),
            network: "taiko_dev".to_string(),
            l1_network: "ethereum".to_string(),
            proof_type: ProofType::Sp1,
            prover_type: None,
            execution_mode: None,
            aggregate_requested: false,
            proposals: vec![ProposalTask {
                proposal_id: 42,
                checkpoint: None,
                l1_inclusion_block_number: 1,
                l2_block_numbers: vec![42],
                last_anchor_block_number: 0,
                task_id: "legacy-corrupt-id".to_string(),
                request: None,
            }],
            aggregate_task_id: None,
            aggregate_request: None,
            aggregate_input_artifacts: Vec::new(),
            runtime: RuntimeMetadata {
                active_stage: Some("prove".to_string()),
                ..RuntimeMetadata::default()
            },
        };
        let record = RuntimeTaskRecord {
            task_id: "task_public".to_string(),
            pipeline_key: PipelineKey::ShastaSp1,
            route: "sp1/local".parse().expect("parse route"),
            task_kind: "hoodi_batch".to_string(),
            proposal_id: Some(42),
            proof_ids: vec!["legacy-corrupt-id".to_string()],
            runner_status: RuntimeRunnerStatus::Allocated,
            task_dir: "/tmp/task_public".to_string(),
            image_ref: None,
            provider_request_id: None,
            remote_tx_hash: None,
            proof_path: None,
            error: None,
            metadata: serde_json::to_value(&metadata)?,
            request_fingerprint: None,
            updated_at: 1,
        };

        let runtime = RuntimeManager::new(unique_test_runtime_root("legacy-child-id"))
            .expect("runtime manager");
        let (proposals, engine_state_present) =
            load_proposal_statuses(&runtime, &engine, &metadata, &record)
                .await
                .expect("legacy child id should not fail task loading");

        assert!(!engine_state_present);
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].task_id, "legacy-corrupt-id");
        assert!(matches!(proposals[0].status, ProofStatus::Proving));
        Ok(())
    }
}
