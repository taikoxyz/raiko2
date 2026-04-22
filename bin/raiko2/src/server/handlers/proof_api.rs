use alloy_primitives::{hex, keccak256};
use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use raiko2_engine::{
    AggregationTaskRequest, EngineTaskId, EngineTaskKey, ProposalStage, ProposalTaskRequest,
    ProverTaskConfig,
};
use raiko2_pipeline::PipelineKey;
use raiko2_primitives::{L2BlockRange, ProofType};
use raiko2_prover::sp1::{
    ExecutionMode as Sp1ExecutionMode, Sp1RemoteVerifyConfig, Sp1RequestContext, Sp1SystemConfig,
};
use raiko2_prover::validate_external_aggregate_proofs;
use raiko2_runtime::{
    RunnerStatus as RuntimeRunnerStatus, TaskRegistration, TaskRegistrationOutcome,
};
use std::collections::HashSet;
use std::sync::Arc;
use tracing::info;

use super::super::errors::ApiError;
use super::proof_route::{
    BatchProofDecision, CanonicalProofRoute, decide_batch_proof_type, generate_public_task_id,
    route_for_proof_type,
};
use super::proof_types::{
    AggregateProofRequest, BatchShastaRequest, CanonicalProposal, HoodiAggregateStatus,
    HoodiProofType, HoodiProposalStatus, HoodiRootRuntimeView, HoodiSuccess, HoodiTaskData,
    HoodiTaskRuntimeView, LegacyProofData, LegacyProofEnvelope, LegacyProofError,
    LegacyProofMaterial, PruneStatus, PublicProverArgs, RegistrationData, RootTaskState,
    ShastaProposal, TaskMetadataParams,
};
use crate::config::ResolvedNetworkPair;
use crate::server::state::{AppState, EngineHandle, EngineStatusView, ProofStatus};
use crate::server::task_cleanup::{
    cancel_registered_tasks, proposal_stage_task_id, proposal_task_chain_ids, remove_task_children,
};
use crate::server::task_metadata::{
    HoodiProposalTask, HoodiRuntimeMetadata, HoodiTaskMetadata, HoodiTaskRuntimeMetadata,
    aggregate_task_ref, proposal_task_ref,
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
    aggregate: Option<PlannedAggregateTask>,
}

struct ExternalAggregateSubmission {
    pair: ResolvedNetworkPair,
    route: CanonicalProofRoute,
    proofs: Vec<raiko2_primitives::Proof>,
    public_task_id: String,
    task_id: EngineTaskId,
    request: AggregationTaskRequest,
    request_fingerprint: String,
}

struct TaskLookup {
    record: raiko2_runtime::RuntimeTaskRecord,
    metadata: HoodiTaskMetadata,
    engine: Arc<dyn EngineHandle>,
}

pub async fn request_batch_shasta_proof(
    State(state): State<AppState>,
    req: Result<Json<BatchShastaRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let Json(req) = req.map_err(|err| ApiError::bad_request(err.to_string()))?;
    let Some(submission) = build_canonical_batch_submission(&state, req)? else {
        return Ok(zk_any_not_drawn_response().into_response());
    };
    let request_fingerprint = batch_request_fingerprint(&submission)?;
    let plan = build_submission_plan(&submission)?;

    info!(
        "Received hoodi shasta batch request: task_id={}, proposals={}, aggregate={}, route={}, pair={}",
        submission.public_task_id,
        submission.proposals.len(),
        submission.aggregate_requested,
        submission.route.route,
        submission.pair.key
    );

    match register_batch_task(&state, &submission, &plan, &request_fingerprint).await? {
        TaskRegistrationOutcome::Existing(existing) => {
            info!(
                "Detected concurrent duplicate hoodi shasta batch request: task_id={}, aggregate={}, route={}, pair={}",
                existing.task_id,
                submission.aggregate_requested,
                submission.route.route,
                submission.pair.key
            );
            let existing_metadata: HoodiTaskMetadata =
                serde_json::from_value(existing.metadata.clone()).map_err(|err| {
                    ApiError::internal(format!("failed to parse existing task metadata: {err}"))
                })?;
            if should_reenqueue_existing_submission(&existing, &existing_metadata) {
                let recovery_plan = build_submission_plan(&CanonicalBatchSubmission {
                    public_task_id: existing.task_id.clone(),
                    ..submission.clone()
                })?;
                let engine = resolve_engine(
                    &state,
                    &existing_metadata.network_pair,
                    existing.pipeline_key,
                )?;
                enqueue_submission_plan(&engine, &recovery_plan)
                    .await
                    .map_err(|err| {
                        ApiError::internal(format!(
                            "failed to recover dormant task {}: {err}",
                            existing.task_id,
                            err = err.message
                        ))
                    })?;
            }
            compatibility_response_for_task(&state, &existing.task_id).await
        }
        TaskRegistrationOutcome::Created(_) => {
            let engine = resolve_engine(
                &state,
                &submission.pair.key,
                submission.route.pipeline_key(),
            )?;

            if let Err(err) = enqueue_submission_plan(&engine, &plan).await {
                let _ = cleanup_submission_plan(&state, &engine, &submission.public_task_id, &plan)
                    .await;
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
                hoodi_response_proof_type(&submission),
                submission.public_task_id,
            )
            .into_response())
        }
    }
}

pub async fn request_aggregation_proof(
    State(state): State<AppState>,
    req: Result<Json<AggregateProofRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let Json(req) = req.map_err(|err| ApiError::bad_request(err.to_string()))?;
    let submission = build_external_aggregate_submission(&state, req)?;
    let engine = resolve_engine(
        &state,
        &submission.pair.key,
        submission.route.pipeline_key(),
    )?;
    let aggregate = planned_external_aggregate_task(&submission);

    info!(
        "Received hoodi aggregate request: task_id={}, proofs={}, aggregate_ids={}, route={}, pair={}",
        submission.public_task_id,
        submission.proofs.len(),
        submission.request.proposal_ids.len(),
        submission.route.route,
        submission.pair.key
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
) -> Result<Json<HoodiSuccess<HoodiTaskData>>, ApiError> {
    let data = load_task_data(&state, &id).await?;
    let lookup = load_task_lookup(&state, &id).await?;

    Ok(Json(HoodiSuccess {
        status: "ok",
        proof_type: lookup.metadata.proof_type.to_string(),
        data,
    }))
}

pub async fn cancel_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<HoodiSuccess<HoodiTaskData>>, ApiError> {
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

pub async fn report_proofs(
    State(state): State<AppState>,
) -> Result<Json<Vec<HoodiTaskData>>, ApiError> {
    let tasks = load_all_task_data(&state).await?;
    Ok(Json(tasks))
}

pub async fn list_proofs(
    State(state): State<AppState>,
) -> Result<Json<Vec<HoodiTaskData>>, ApiError> {
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
        let metadata: HoodiTaskMetadata = serde_json::from_value(record.metadata.clone())
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
    let selected_proof_type = match decide_batch_proof_type(state, &req)? {
        BatchProofDecision::Selected(proof_type) => proof_type,
        BatchProofDecision::NotDrawn => return Ok(None),
    };
    let route = route_for_proof_type(state, selected_proof_type)?;
    validate_route_specific_request(
        state,
        &pair,
        route.proof_type(),
        req.aggregate,
        &requested_prover_config,
    )?;

    let proposals = req
        .proposals
        .iter()
        .map(canonicalize_proposal)
        .collect::<Result<Vec<_>, _>>()?;
    let execution_mode = requested_prover_config
        .sp1
        .as_ref()
        .and_then(|config| config.mode);

    Ok(Some(CanonicalBatchSubmission {
        public_task_id: generate_public_task_id(),
        pair,
        route,
        proposals,
        aggregate_requested: req.aggregate,
        prover_config: requested_prover_config,
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
    Ok(())
}

fn validate_public_prover_args(
    proof_type: HoodiProofType,
    args: &PublicProverArgs,
) -> Result<ProverTaskConfig, ApiError> {
    if matches!(proof_type, HoodiProofType::ZkAny) && !args.is_empty() {
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
    if args.sp1.is_some() && !matches!(proof_type, HoodiProofType::Sp1 | HoodiProofType::ZkAny) {
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
    if matches!(req.proof_type, HoodiProofType::ZkAny) {
        return Err(ApiError::bad_request(
            "proof_type=zk_any is not supported for aggregate requests",
        ));
    }
    if req.proofs.is_empty() {
        return Err(ApiError::bad_request("proofs must not be empty"));
    }
    Ok(())
}

fn build_external_aggregate_submission(
    state: &AppState,
    req: AggregateProofRequest,
) -> Result<ExternalAggregateSubmission, ApiError> {
    validate_aggregate_request_shape(&req)?;
    let pair = resolved_pair(state, req.network.as_deref(), req.l1_network.as_deref())?;
    let prover_config = augment_system_prover_config(
        &pair,
        validate_public_prover_args(req.proof_type, &req.prover_args)?,
    );
    let route = route_for_proof_type(state, req.proof_type)?;
    validate_aggregate_route_specific_request(state, &pair, route.proof_type(), &prover_config)?;
    validate_external_aggregate_proofs(route.route, &req.proofs)
        .map_err(|err| ApiError::bad_request(err.to_string()))?;
    let public_task_id = generate_public_task_id();
    let request = AggregationTaskRequest {
        request_id: public_task_id.clone(),
        proposal_ids: req.aggregation_ids.clone(),
        prover_config,
    };
    let task_id = EngineTaskId::new(EngineTaskKey::Aggregate {
        pipeline: route.pipeline_key(),
        request: request.clone(),
    });
    let request_fingerprint = external_aggregate_request_fingerprint(&pair, route, &req, &request)?;
    let _ = (&req.graffiti, &req.prover, &req.blob_proof_type);

    Ok(ExternalAggregateSubmission {
        pair,
        route,
        proofs: req.proofs,
        public_task_id,
        task_id,
        request,
        request_fingerprint,
    })
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

fn build_submission_plan(
    submission: &CanonicalBatchSubmission,
) -> Result<SubmissionPlan, ApiError> {
    let proposals = submission
        .proposals
        .iter()
        .cloned()
        .map(|proposal| -> Result<PlannedProposalTask, ApiError> {
            let request = ProposalTaskRequest {
                proposal_id: proposal.proposal_id,
                l2_block_range: Some(proposal.l2_block_range),
                l1_inclusion_block_number: proposal.l1_inclusion_block_number,
                last_anchor_block_number: proposal.last_anchor_block_number,
                checkpoint: proposal.checkpoint,
                blob_proof_type: submission.blob_proof_type.clone(),
                prover: submission.prover.clone(),
                graffiti: submission.graffiti.clone(),
                prover_config: submission.prover_config.clone(),
            };
            let task_id = proposal_stage_task_id(
                submission.route.pipeline_key(),
                request.clone(),
                ProposalStage::Prove,
            );
            Ok(PlannedProposalTask {
                task_ref: proposal_task_ref(submission.route.pipeline_key(), &request),
                request,
                task_id,
                proposal,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;

    let aggregate = if submission.aggregate_requested {
        let request = AggregationTaskRequest {
            request_id: submission.public_task_id.clone(),
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
        aggregate,
    })
}

async fn register_batch_task(
    state: &AppState,
    submission: &CanonicalBatchSubmission,
    plan: &SubmissionPlan,
    request_fingerprint: &str,
) -> Result<TaskRegistrationOutcome, ApiError> {
    let metadata = build_task_metadata(
        &submission.pair,
        TaskMetadataParams {
            network: &submission.pair.network,
            l1_network: &submission.pair.l1_network,
            proof_type: submission.route.proof_type(),
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
    let metadata = build_task_metadata(
        &submission.pair,
        TaskMetadataParams {
            network: &submission.pair.network,
            l1_network: &submission.pair.l1_network,
            proof_type: submission.route.proof_type(),
            execution_mode: None,
            aggregate_requested: true,
        },
        &[],
        Some(aggregate),
    );

    state
        .runtime
        .register_task_if_absent(TaskRegistration {
            task_id: submission.public_task_id.clone(),
            route: submission.route.route,
            task_kind: "hoodi_aggregate".to_string(),
            proposal_id: None,
            proof_ids: Vec::new(),
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
    params: TaskMetadataParams<'_>,
    proposals: &[PlannedProposalTask],
    aggregate: Option<&PlannedAggregateTask>,
) -> HoodiTaskMetadata {
    HoodiTaskMetadata {
        network_pair: pair.key.clone(),
        network: params.network.to_string(),
        l1_network: params.l1_network.to_string(),
        proof_type: params.proof_type,
        execution_mode: params.execution_mode,
        aggregate_requested: params.aggregate_requested,
        proposals: proposals
            .iter()
            .map(|proposal| HoodiProposalTask {
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
        runtime: HoodiRuntimeMetadata::default(),
    }
}

async fn enqueue_submission_plan(
    engine: &Arc<dyn EngineHandle>,
    plan: &SubmissionPlan,
) -> Result<(), ApiError> {
    let mut previous = None;

    for proposal in &plan.proposals {
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
            .submit_aggregation_proof_from_tasks(
                aggregate.request.clone(),
                plan.proposals
                    .iter()
                    .map(|proposal| proposal.task_id.clone())
                    .collect(),
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
    let metadata = HoodiTaskMetadata {
        network_pair: String::new(),
        network: String::new(),
        l1_network: String::new(),
        proof_type: ProofType::Native,
        execution_mode: None,
        aggregate_requested: plan.aggregate.is_some(),
        proposals: plan
            .proposals
            .iter()
            .map(|proposal| HoodiProposalTask {
                proposal_id: proposal.proposal.proposal_id,
                checkpoint: proposal.proposal.checkpoint,
                l1_inclusion_block_number: proposal.proposal.l1_inclusion_block_number,
                l2_block_numbers: proposal.proposal.l2_block_numbers.clone(),
                last_anchor_block_number: proposal.proposal.last_anchor_block_number,
                task_id: proposal.task_ref.clone(),
                request: Some(proposal.request.clone()),
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
        runtime: HoodiRuntimeMetadata::default(),
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

async fn handle_existing_external_aggregate_task(
    state: &AppState,
    engine: &Arc<dyn EngineHandle>,
    submission: &ExternalAggregateSubmission,
    existing: raiko2_runtime::RuntimeTaskRecord,
) -> Result<Response, ApiError> {
    info!(
        "Detected concurrent duplicate hoodi aggregate request: task_id={}, route={}, pair={}",
        existing.task_id, submission.route.route, submission.pair.key
    );
    let existing_metadata: HoodiTaskMetadata = serde_json::from_value(existing.metadata.clone())
        .map_err(|err| {
            ApiError::internal(format!("failed to parse existing task metadata: {err}"))
        })?;
    if should_reenqueue_existing_submission(&existing, &existing_metadata) {
        reenqueue_existing_external_aggregate_task(
            engine,
            submission,
            &existing,
            &existing_metadata,
        )
        .await?;
    }
    compatibility_response_for_task(state, &existing.task_id).await
}

async fn reenqueue_existing_external_aggregate_task(
    engine: &Arc<dyn EngineHandle>,
    submission: &ExternalAggregateSubmission,
    existing: &raiko2_runtime::RuntimeTaskRecord,
    existing_metadata: &HoodiTaskMetadata,
) -> Result<(), ApiError> {
    let request = existing_metadata
        .aggregate_request
        .clone()
        .ok_or_else(|| ApiError::internal("existing aggregate task missing aggregate_request"))?;
    let expected_task_id = existing_metadata
        .aggregate_engine_task_id(existing.pipeline_key)
        .ok_or_else(|| ApiError::internal("existing aggregate task missing aggregate task id"))?;
    let actual_task_id = engine
        .submit_aggregation_proof_from_proofs(request, submission.proofs.clone())
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

async fn handle_created_external_aggregate_task(
    state: &AppState,
    engine: &Arc<dyn EngineHandle>,
    submission: &ExternalAggregateSubmission,
    aggregate: &PlannedAggregateTask,
) -> Result<Response, ApiError> {
    let metadata = build_task_metadata(
        &submission.pair,
        TaskMetadataParams {
            network: &submission.pair.network,
            l1_network: &submission.pair.l1_network,
            proof_type: submission.route.proof_type(),
            execution_mode: None,
            aggregate_requested: true,
        },
        &[],
        Some(aggregate),
    );
    let actual_task_id = engine
        .submit_aggregation_proof_from_proofs(submission.request.clone(), submission.proofs.clone())
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
        HoodiProofType::from_canonical(submission.route.proof_type()),
        submission.public_task_id.clone(),
    )
    .into_response())
}

async fn cleanup_external_aggregate_submission(
    state: &AppState,
    engine: &Arc<dyn EngineHandle>,
    submission: &ExternalAggregateSubmission,
    metadata: &HoodiTaskMetadata,
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

async fn load_task_data(state: &AppState, id: &str) -> Result<HoodiTaskData, ApiError> {
    let lookup = load_task_lookup(state, id).await?;
    load_task_data_from_lookup(id, &lookup).await
}

async fn load_task_data_from_lookup(
    id: &str,
    lookup: &TaskLookup,
) -> Result<HoodiTaskData, ApiError> {
    let (proposals, proposal_engine_state_present): (Vec<HoodiProposalStatus>, bool) =
        load_proposal_statuses(&lookup.engine, &lookup.metadata, &lookup.record).await?;
    let (aggregate, aggregate_engine_state_present): (Option<HoodiAggregateStatus>, bool) =
        load_aggregate_status(&lookup.engine, &lookup.metadata, &lookup.record).await?;
    let root_engine_state_present = proposal_engine_state_present || aggregate_engine_state_present;
    let root_state = resolve_root_task_state(
        lookup.record.runner_status,
        &proposals,
        aggregate.as_ref(),
        lookup.metadata.has_runtime_progress(),
        lookup.record.error.as_deref(),
    );

    Ok(HoodiTaskData {
        task_id: id.to_string(),
        route: lookup.record.route.to_string(),
        execution_mode: lookup.metadata.execution_mode_str(),
        status: root_state.status.clone(),
        network: lookup.metadata.network.clone(),
        l1_network: lookup.metadata.l1_network.clone(),
        runtime: root_runtime_view(&lookup.record, &lookup.metadata, root_engine_state_present),
        current_index: root_state.current_index,
        proposals,
        aggregate,
        proof: root_state.proof,
        error: root_state.error,
    })
}

async fn load_all_task_data(state: &AppState) -> Result<Vec<HoodiTaskData>, ApiError> {
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
    let metadata: HoodiTaskMetadata = serde_json::from_value(record.metadata.clone())
        .map_err(|err| ApiError::internal(format!("failed to parse task metadata: {err}")))?;
    let engine = resolve_engine(state, &metadata.network_pair, record.pipeline_key)?;

    Ok(TaskLookup {
        record,
        metadata,
        engine,
    })
}

async fn load_proposal_statuses(
    engine: &Arc<dyn EngineHandle>,
    metadata: &HoodiTaskMetadata,
    record: &raiko2_runtime::RuntimeTaskRecord,
) -> Result<(Vec<HoodiProposalStatus>, bool), ApiError> {
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
        proposals.push(HoodiProposalStatus {
            index,
            proposal_id: proposal.proposal_id,
            checkpoint: proposal.checkpoint,
            task_id: proposal.task_id.clone(),
            status: status.status.clone(),
            l1_inclusion_block_number: proposal.l1_inclusion_block_number,
            l2_block_numbers: proposal.l2_block_numbers.clone(),
            last_anchor_block_number: proposal.last_anchor_block_number,
            proof: status.proof,
            error: status.error,
            runtime: task_runtime_view(runtime, engine_state_present, record.updated_at),
            extra_data: status.extra_data,
        });
    }

    Ok((proposals, any_engine_state_present))
}

async fn load_aggregate_status(
    engine: &Arc<dyn EngineHandle>,
    metadata: &HoodiTaskMetadata,
    record: &raiko2_runtime::RuntimeTaskRecord,
) -> Result<(Option<HoodiAggregateStatus>, bool), ApiError> {
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

    Ok((
        Some(HoodiAggregateStatus {
            task_id: metadata.aggregate_task_id.clone().expect("checked"),
            status: status.status.clone(),
            proof: status.proof,
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
    let mut saw_progress = false;
    let mut pending_error = None;

    for stage in stages {
        let Some(stage) = stage else {
            continue;
        };

        match stage.status {
            ProofStatus::Failed | ProofStatus::Cancelled => return stage.clone(),
            ProofStatus::Completed => {
                if stage.proof.is_some() || stage.extra_data.is_some() {
                    return stage.clone();
                }
                saw_progress = true;
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
                if saw_progress {
                    return EngineStatusView {
                        status: ProofStatus::Proving,
                        proof: None,
                        error: pending_error,
                        extra_data: None,
                    };
                }
            }
        }
    }

    if saw_progress {
        EngineStatusView {
            status: ProofStatus::Proving,
            proof: None,
            error: pending_error,
            extra_data: None,
        }
    } else {
        EngineStatusView {
            status: ProofStatus::Pending,
            proof: None,
            error: pending_error,
            extra_data: None,
        }
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
    proposals: &[HoodiProposalStatus],
    aggregate: Option<&HoodiAggregateStatus>,
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
    let proof = aggregate
        .and_then(|aggregate| aggregate.proof.clone())
        .or_else(|| {
            (proposals.len() == 1)
                .then(|| {
                    proposals
                        .first()
                        .and_then(|proposal| proposal.proof.clone())
                })
                .flatten()
        });
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
    proposals: &[HoodiProposalStatus],
    aggregate: Option<&HoodiAggregateStatus>,
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
    metadata: &HoodiTaskMetadata,
    engine_state_present: bool,
) -> HoodiRootRuntimeView {
    HoodiRootRuntimeView {
        runner_status: record.runner_status,
        active_stage: metadata.runtime.active_stage.clone(),
        last_event: metadata.runtime.last_event.clone(),
        updated_at: record.updated_at,
        engine_state_present,
    }
}

fn task_runtime_view(
    runtime: Option<&HoodiTaskRuntimeMetadata>,
    engine_state_present: bool,
    fallback_updated_at: i64,
) -> Option<HoodiTaskRuntimeView> {
    if engine_state_present && runtime.is_none() {
        return None;
    }
    let runtime = runtime.cloned().unwrap_or_default();
    Some(HoodiTaskRuntimeView {
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
    if let (Some(network), Some(l1_network)) = (network, l1_network) {
        return state
            .config
            .rpc
            .resolve_pair(network, l1_network)
            .map_err(|err| ApiError::bad_request(err.to_string()));
    }

    let resolved_pairs = state
        .config
        .rpc
        .resolved_pairs()
        .map_err(|err| ApiError::bad_request(err.to_string()))?;
    let default_pair = resolved_pairs
        .first()
        .ok_or_else(|| ApiError::internal("rpc.pairs must contain at least one network pair"))?;
    let default_network = default_pair.network.clone();
    let default_l1_network = default_pair.l1_network.clone();
    let network = network.unwrap_or(default_network.as_str());
    let l1_network = l1_network.unwrap_or(default_l1_network.as_str());

    resolved_pairs
        .into_iter()
        .find(|pair| pair.network == network && pair.l1_network == l1_network)
        .ok_or_else(|| {
            ApiError::bad_request(format!(
                "unsupported network pair: network={network}, l1_network={l1_network}"
            ))
        })
}

fn batch_request_fingerprint(submission: &CanonicalBatchSubmission) -> Result<String, ApiError> {
    let payload = serde_json::json!({
        "pair_key": submission.pair.key.as_str(),
        "route": submission.route.route.to_string(),
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
    req: &AggregateProofRequest,
    request: &AggregationTaskRequest,
) -> Result<String, ApiError> {
    let payload = serde_json::json!({
        "pair_key": pair.key.as_str(),
        "route": route.route.to_string(),
        "proof_type": req.proof_type.as_str(),
        "aggregation_ids": &req.aggregation_ids,
        "prover_config": &request.prover_config,
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

const fn hoodi_response_proof_type(submission: &CanonicalBatchSubmission) -> HoodiProofType {
    match submission.route.proof_type() {
        ProofType::Native => HoodiProofType::Native,
        ProofType::Sp1 => HoodiProofType::Sp1,
        ProofType::Sgx => HoodiProofType::Sgx,
        ProofType::Risc0 => HoodiProofType::Risc0,
    }
}

fn registration_response(
    proof_type: &str,
    status: &'static str,
    public_task_id: Option<String>,
) -> Json<HoodiSuccess<RegistrationData>> {
    Json(HoodiSuccess {
        status: "ok",
        proof_type: proof_type.to_string(),
        data: RegistrationData {
            status,
            task_id: public_task_id,
        },
    })
}

fn registered_response(
    proof_type: HoodiProofType,
    public_task_id: String,
) -> Json<HoodiSuccess<RegistrationData>> {
    registration_response(proof_type.as_str(), "registered", Some(public_task_id))
}

fn zk_any_not_drawn_response() -> Json<HoodiSuccess<RegistrationData>> {
    registration_response(HoodiProofType::Native.as_str(), "zk_any_not_drawn", None)
}

fn should_reenqueue_existing_submission(
    record: &raiko2_runtime::RuntimeTaskRecord,
    metadata: &HoodiTaskMetadata,
) -> bool {
    !matches!(
        record.runner_status,
        RuntimeRunnerStatus::Completed
            | RuntimeRunnerStatus::Failed
            | RuntimeRunnerStatus::Cancelled
    ) && !metadata.has_runtime_progress()
}

async fn compatibility_response_for_task(
    state: &AppState,
    task_id: &str,
) -> Result<Response, ApiError> {
    let lookup = load_task_lookup(state, task_id).await?;
    let proof_type = HoodiProofType::from_canonical(lookup.metadata.proof_type);
    let task = load_task_data_from_lookup(task_id, &lookup).await?;
    let public_task_id = Some(task.task_id.clone());

    Ok(match task.status {
        ProofStatus::Pending => {
            legacy_success_response(proof_type, "registered", public_task_id, None)
        }
        ProofStatus::Proving => {
            legacy_success_response(proof_type, "work_in_progress", public_task_id, None)
        }
        ProofStatus::Completed => match task.proof {
            Some(proof) => legacy_success_response(
                proof_type,
                "completed",
                public_task_id,
                Some(LegacyProofMaterial {
                    proof,
                    kzg_proof: String::new(),
                    quote: String::new(),
                }),
            ),
            None => legacy_success_response(proof_type, "completed", public_task_id, None),
        },
        ProofStatus::Failed => legacy_error_response(
            proof_type,
            "task_failed",
            task.error
                .unwrap_or_else(|| format!("task {task_id} failed")),
        ),
        ProofStatus::Cancelled => legacy_error_response(
            proof_type,
            "task_cancelled",
            task.error
                .unwrap_or_else(|| format!("task {task_id} was cancelled")),
        ),
    })
}

fn legacy_success_response(
    proof_type: HoodiProofType,
    status: &'static str,
    task_id: Option<String>,
    proof: Option<LegacyProofMaterial>,
) -> Response {
    Json(LegacyProofEnvelope {
        status: "ok",
        proof_type: proof_type.as_str().to_string(),
        data: LegacyProofData {
            status,
            task_id,
            proof,
        },
    })
    .into_response()
}

fn legacy_error_response(
    proof_type: HoodiProofType,
    error: &'static str,
    message: String,
) -> Response {
    (
        StatusCode::OK,
        Json(LegacyProofError {
            status: "error",
            proof_type: proof_type.as_str().to_string(),
            error,
            message,
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BoundlessPairConfig;
    use crate::server::state::EngineHandle;
    use anyhow::Result;
    use raiko2_engine::EngineTaskId;
    use raiko2_pipeline::{PipelineRoute, RunnerKind};
    use raiko2_primitives::SupportedChainSpecs;
    use raiko2_queue::TaskStoreError;
    use raiko2_runtime::{RunnerStatus as RuntimeRunnerStatus, RuntimeTaskRecord};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;

    type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

    struct NoopEngine;

    impl EngineHandle for NoopEngine {
        fn submit_proposal_proof_with_dependencies(
            &self,
            _request: ProposalTaskRequest,
            _dependencies: Vec<EngineTaskId>,
        ) -> BoxFuture<'_, Result<EngineTaskId, TaskStoreError>> {
            Box::pin(async { panic!("unexpected proposal submission") })
        }

        fn submit_aggregation_proof_from_tasks(
            &self,
            _request: AggregationTaskRequest,
            _proof_tasks: Vec<EngineTaskId>,
        ) -> BoxFuture<'_, Result<EngineTaskId, TaskStoreError>> {
            Box::pin(async { panic!("unexpected aggregation task submission") })
        }

        fn submit_aggregation_proof_from_proofs(
            &self,
            _request: AggregationTaskRequest,
            _proofs: Vec<raiko2_primitives::Proof>,
        ) -> BoxFuture<'_, Result<EngineTaskId, TaskStoreError>> {
            Box::pin(async { panic!("unexpected external aggregation submission") })
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
            blob_proof_type: None,
            prover: None,
            graffiti: None,
            execution_mode: None,
        }
    }

    #[test]
    fn native_proposal_task_id_is_reused_across_aggregate_flags() {
        let route = CanonicalProofRoute {
            route: PipelineRoute::new(crate::config::GuestSystem::Native, RunnerKind::Local),
        };
        let single = build_submission_plan(&canonical_submission(route, false))
            .expect("single submission plan");
        let aggregate = build_submission_plan(&canonical_submission(route, true))
            .expect("aggregate submission plan");

        assert_eq!(single.proposals.len(), 1);
        assert_eq!(aggregate.proposals.len(), 1);
        assert_eq!(single.proposals[0].task_id, aggregate.proposals[0].task_id);
    }

    #[tokio::test]
    async fn load_proposal_statuses_tolerates_invalid_legacy_child_id() -> Result<()> {
        let engine: Arc<dyn EngineHandle> = Arc::new(NoopEngine);
        let metadata = HoodiTaskMetadata {
            network_pair: "taiko_dev/ethereum".to_string(),
            network: "taiko_dev".to_string(),
            l1_network: "ethereum".to_string(),
            proof_type: ProofType::Sp1,
            execution_mode: None,
            aggregate_requested: false,
            proposals: vec![HoodiProposalTask {
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
            runtime: HoodiRuntimeMetadata {
                active_stage: Some("prove".to_string()),
                ..HoodiRuntimeMetadata::default()
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

        let (proposals, engine_state_present) = load_proposal_statuses(&engine, &metadata, &record)
            .await
            .expect("legacy child id should not fail task loading");

        assert!(!engine_state_present);
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].task_id, "legacy-corrupt-id");
        assert!(matches!(proposals[0].status, ProofStatus::Proving));
        Ok(())
    }
}
