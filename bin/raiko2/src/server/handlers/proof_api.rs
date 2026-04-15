use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
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
use raiko2_queue::{decode_task_id, encode_task_id};
use raiko2_runtime::{RunnerStatus as RuntimeRunnerStatus, TaskRegistration};
use std::collections::HashSet;
use std::sync::Arc;
use tracing::info;

use super::super::errors::ApiError;
use super::proof_route::{
    BatchProofDecision, CanonicalProofRoute, decide_batch_proof_type, generate_public_task_id,
    parse_pipeline_key, route_for_proof_type,
};
use super::proof_types::{
    AggregateProofRequest, BatchShastaRequest, CanonicalProposal, HoodiAggregateStatus,
    HoodiProofType, HoodiProposalStatus, HoodiRootRuntimeView, HoodiSuccess, HoodiTaskData,
    HoodiTaskRuntimeView, PruneStatus, PublicProverArgs, RegistrationData, RootTaskState,
    ShastaProposal, TaskMetadataParams,
};
use crate::config::ResolvedNetworkPair;
use crate::server::state::{AppState, EngineHandle, EngineStatusView, ProofStatus};
use crate::server::task_metadata::{
    HoodiProposalTask, HoodiRuntimeMetadata, HoodiTaskMetadata, HoodiTaskRuntimeMetadata,
};

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
    encoded_task_id: String,
    proposal: CanonicalProposal,
}

#[derive(Clone)]
struct PlannedAggregateTask {
    request: AggregationTaskRequest,
    task_id: EngineTaskId,
    encoded_task_id: String,
}

#[derive(Clone)]
struct SubmissionPlan {
    proposals: Vec<PlannedProposalTask>,
    aggregate: Option<PlannedAggregateTask>,
}

type ExternalAggregateSubmission = (
    ResolvedNetworkPair,
    CanonicalProofRoute,
    Vec<raiko2_primitives::Proof>,
    String,
    EngineTaskId,
    AggregationTaskRequest,
);

struct TaskLookup {
    record: raiko2_runtime::RuntimeTaskRecord,
    metadata: HoodiTaskMetadata,
    engine: Arc<dyn EngineHandle>,
}

pub async fn request_batch_shasta_proof(
    State(state): State<AppState>,
    req: Result<Json<BatchShastaRequest>, JsonRejection>,
) -> Result<Json<HoodiSuccess<RegistrationData>>, ApiError> {
    let Json(req) = req.map_err(|err| ApiError::bad_request(err.to_string()))?;
    let Some(submission) = build_canonical_batch_submission(&state, req)? else {
        return Ok(zk_any_not_drawn_response());
    };
    let plan = build_submission_plan(&submission)?;
    let engine = resolve_engine(&state, &submission.pair.key, submission.route.pipeline_key)?;

    info!(
        "Received hoodi shasta batch request: task_id={}, proposals={}, aggregate={}, route={}, pair={}",
        submission.public_task_id,
        submission.proposals.len(),
        submission.aggregate_requested,
        submission.route.route,
        submission.pair.key
    );

    register_batch_task(&state, &submission, &plan).await?;

    if let Err(err) = enqueue_submission_plan(&engine, &plan).await {
        let _ = cleanup_submission_plan(&state, &engine, &submission.public_task_id, &plan).await;
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

    Ok(registered_response(
        hoodi_response_proof_type(&submission),
        submission.public_task_id,
    ))
}

pub async fn request_aggregation_proof(
    State(state): State<AppState>,
    req: Result<Json<AggregateProofRequest>, JsonRejection>,
) -> Result<Json<HoodiSuccess<RegistrationData>>, ApiError> {
    let Json(req) = req.map_err(|err| ApiError::bad_request(err.to_string()))?;
    let (pair, route, proofs, public_task_id, task_id, request) =
        build_external_aggregate_submission(&state, req)?;
    let engine = resolve_engine(&state, &pair.key, route.pipeline_key)?;
    let encoded_task_id = encode_task_id(&task_id)
        .map_err(|err| ApiError::internal(format!("failed to encode task id: {err}")))?;
    let metadata = build_task_metadata(
        &pair,
        TaskMetadataParams {
            network: &pair.network,
            l1_network: &pair.l1_network,
            proof_type: route.proof_type,
            execution_mode: None,
            aggregate_requested: true,
        },
        &[],
        Some(encoded_task_id.clone()),
    );

    state
        .runtime
        .register_task(TaskRegistration {
            task_id: public_task_id.clone(),
            pipeline_key: route.pipeline_key.as_str().to_string(),
            route: route.route.to_string(),
            guest_system: route.route.guest_system.to_string(),
            runner: route.route.runner.to_string(),
            task_kind: "hoodi_aggregate".to_string(),
            proposal_id: None,
            proof_ids: Vec::new(),
            metadata: serde_json::to_value(metadata.clone()).map_err(|err| {
                ApiError::internal(format!("failed to serialize metadata: {err}"))
            })?,
        })
        .await
        .map_err(|err| ApiError::internal(format!("failed to register runtime task: {err}")))?;

    let enqueue_result = engine
        .submit_aggregation_proof_from_proofs(request, proofs)
        .await
        .map_err(|err| ApiError::internal(format!("failed to enqueue aggregation proof: {err}")));
    let actual_task_id = match enqueue_result {
        Ok(task_id) => task_id,
        Err(err) => {
            let _ = state.runtime.remove_task(&public_task_id).await;
            let _ = remove_task_children(&engine, &metadata, &mut HashSet::new()).await;
            return Err(err);
        }
    };
    if actual_task_id != task_id {
        let _ = state.runtime.remove_task(&public_task_id).await;
        let _ = remove_task_children(&engine, &metadata, &mut HashSet::new()).await;
        return Err(ApiError::internal(
            "engine returned unexpected aggregation task id",
        ));
    }

    Ok(registered_response(
        HoodiProofType::from_canonical(route.proof_type),
        public_task_id,
    ))
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

    cancel_registered_tasks(&state.runtime, &engine, &id, &metadata).await?;

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
        let pipeline_key = parse_pipeline_key(&record.pipeline_key)?;
        let engine = resolve_engine(&state, &metadata.network_pair, pipeline_key)?;

        remove_task_children(&engine, &metadata, &mut removed_engine_task_ids).await?;

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
    let pair = resolved_pair(state, &req.network, &req.l1_network)?;
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
        route.proof_type,
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
    if req.proofs.len() < 2 {
        return Err(ApiError::bad_request(
            "proofs must contain at least 2 entries",
        ));
    }
    Ok(())
}

fn validate_external_proofs(
    pipeline_key: PipelineKey,
    proofs: &[raiko2_primitives::Proof],
) -> Result<(), ApiError> {
    for (index, proof) in proofs.iter().enumerate() {
        match pipeline_key {
            PipelineKey::ShastaNative => {
                if proof.input.is_none() || proof.extra_data.is_none() {
                    return Err(ApiError::bad_request(format!(
                        "proof {index} is missing native aggregation metadata"
                    )));
                }
            }
            PipelineKey::ShastaSp1 => {
                if proof.input.is_none() || proof.extra_data.is_none() || proof.uuid.is_none() {
                    return Err(ApiError::bad_request(format!(
                        "proof {index} is missing SP1 aggregation metadata"
                    )));
                }
            }
            PipelineKey::ShastaRisc0 => {
                if proof.input.is_none()
                    || proof.extra_data.is_none()
                    || proof.uuid.is_none()
                    || proof.quote.is_none()
                {
                    return Err(ApiError::bad_request(format!(
                        "proof {index} is missing RISC0 aggregation metadata"
                    )));
                }
            }
            PipelineKey::ShastaRisc0Boundless => {
                if proof.quote.is_none() {
                    return Err(ApiError::bad_request(format!(
                        "proof {index} is missing receipt metadata"
                    )));
                }
            }
        }
    }

    Ok(())
}

fn build_external_aggregate_submission(
    state: &AppState,
    req: AggregateProofRequest,
) -> Result<ExternalAggregateSubmission, ApiError> {
    validate_aggregate_request_shape(&req)?;
    let pair = resolved_pair(state, &req.network, &req.l1_network)?;
    let prover_config = augment_system_prover_config(
        &pair,
        validate_public_prover_args(req.proof_type, &req.prover_args)?,
    );
    let route = route_for_proof_type(state, req.proof_type)?;
    validate_aggregate_route_specific_request(state, &pair, route.proof_type, &prover_config)?;
    validate_external_proofs(route.pipeline_key, &req.proofs)?;
    let public_task_id = generate_public_task_id();
    let request = AggregationTaskRequest {
        request_id: public_task_id.clone(),
        proposal_ids: Vec::new(),
        prover_config,
    };
    let task_id = EngineTaskId::new(EngineTaskKey::Aggregate {
        pipeline: route.pipeline_key,
        request: request.clone(),
    });
    let _ = (&req.graffiti, &req.prover, &req.blob_proof_type);

    Ok((pair, route, req.proofs, public_task_id, task_id, request))
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
                submission.route.pipeline_key,
                request.clone(),
                ProposalStage::Prove,
            );
            let encoded_task_id = encode_task_id(&task_id)
                .map_err(|err| ApiError::internal(format!("failed to encode task id: {err}")))?;
            Ok(PlannedProposalTask {
                request,
                task_id,
                encoded_task_id,
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
            pipeline: submission.route.pipeline_key,
            request: request.clone(),
        });
        let encoded_task_id = encode_task_id(&task_id)
            .map_err(|err| ApiError::internal(format!("failed to encode task id: {err}")))?;
        Some(PlannedAggregateTask {
            request,
            task_id,
            encoded_task_id,
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
) -> Result<(), ApiError> {
    let metadata = build_task_metadata(
        &submission.pair,
        TaskMetadataParams {
            network: &submission.pair.network,
            l1_network: &submission.pair.l1_network,
            proof_type: submission.route.proof_type,
            execution_mode: submission.execution_mode,
            aggregate_requested: submission.aggregate_requested,
        },
        &plan.proposals,
        plan.aggregate
            .as_ref()
            .map(|aggregate| aggregate.encoded_task_id.clone()),
    );

    state
        .runtime
        .register_task(TaskRegistration {
            task_id: submission.public_task_id.clone(),
            pipeline_key: submission.route.pipeline_key.as_str().to_string(),
            route: submission.route.route.to_string(),
            guest_system: submission.route.route.guest_system.to_string(),
            runner: submission.route.route.runner.to_string(),
            task_kind: "hoodi_batch".to_string(),
            proposal_id: submission
                .proposals
                .first()
                .map(|proposal| proposal.proposal_id),
            proof_ids: plan
                .proposals
                .iter()
                .map(|proposal| proposal.encoded_task_id.clone())
                .collect(),
            metadata: serde_json::to_value(metadata).map_err(|err| {
                ApiError::internal(format!("failed to serialize metadata: {err}"))
            })?,
        })
        .await
        .map(|_| ())
        .map_err(|err| ApiError::internal(format!("failed to register runtime task: {err}")))
}

fn build_task_metadata(
    pair: &ResolvedNetworkPair,
    params: TaskMetadataParams<'_>,
    proposals: &[PlannedProposalTask],
    aggregate_task_id: Option<String>,
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
                task_id: proposal.encoded_task_id.clone(),
            })
            .collect(),
        aggregate_task_id,
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
                task_id: proposal.encoded_task_id.clone(),
            })
            .collect(),
        aggregate_task_id: plan
            .aggregate
            .as_ref()
            .map(|aggregate| aggregate.encoded_task_id.clone()),
        runtime: HoodiRuntimeMetadata::default(),
    };

    let _ = cancel_registered_tasks(&state.runtime, engine, public_task_id, &metadata).await;
    remove_task_children(engine, &metadata, &mut HashSet::new()).await
}

const fn proposal_stage_task_id(
    pipeline_key: PipelineKey,
    request: ProposalTaskRequest,
    stage: ProposalStage,
) -> EngineTaskId {
    EngineTaskId::new(EngineTaskKey::Proposal {
        pipeline: pipeline_key,
        request,
        stage,
    })
}

fn proposal_task_chain_ids(task_id: &EngineTaskId) -> Vec<EngineTaskId> {
    let EngineTaskKey::Proposal {
        pipeline,
        request,
        stage: _,
    } = &task_id.0
    else {
        return Vec::new();
    };

    [
        ProposalStage::Preflight,
        ProposalStage::Validation,
        ProposalStage::Encode,
        ProposalStage::Prove,
    ]
    .into_iter()
    .map(|stage| proposal_stage_task_id(*pipeline, request.clone(), stage))
    .collect()
}

async fn load_task_data(state: &AppState, id: &str) -> Result<HoodiTaskData, ApiError> {
    let TaskLookup {
        record,
        metadata,
        engine,
    } = load_task_lookup(state, id).await?;
    let (proposals, proposal_engine_state_present): (Vec<HoodiProposalStatus>, bool) =
        load_proposal_statuses(&engine, &metadata, &record).await?;
    let (aggregate, aggregate_engine_state_present): (Option<HoodiAggregateStatus>, bool) =
        load_aggregate_status(&engine, &metadata, &record).await?;
    let root_engine_state_present = proposal_engine_state_present || aggregate_engine_state_present;
    let root_state = resolve_root_task_state(
        record.runner_status,
        &proposals,
        aggregate.as_ref(),
        metadata.has_runtime_progress(),
        record.error.as_deref(),
    );

    Ok(HoodiTaskData {
        task_id: id.to_string(),
        route: record.route.clone(),
        execution_mode: metadata.execution_mode_str(),
        status: root_state.status.clone(),
        network: metadata.network.clone(),
        l1_network: metadata.l1_network.clone(),
        runtime: root_runtime_view(
            &record,
            &metadata,
            &root_state.status,
            root_engine_state_present,
        ),
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
    let pipeline_key = parse_pipeline_key(&record.pipeline_key)?;
    let engine = resolve_engine(state, &metadata.network_pair, pipeline_key)?;

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
        let task_id = decode_task_id::<EngineTaskKey>(&proposal.task_id)
            .map_err(|err| ApiError::internal(format!("invalid stored proposal task id: {err}")))?;
        let mut stage_statuses = Vec::with_capacity(4);
        for stage_id in proposal_task_chain_ids(&task_id) {
            stage_statuses.push(
                engine.get_status(stage_id).await.map_err(|err| {
                    ApiError::internal(format!("failed to read task status: {err}"))
                })?,
            );
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
    let Some(task_id) = metadata.aggregate_task_id.as_ref() else {
        return Ok((None, false));
    };

    let task_id = decode_task_id::<EngineTaskKey>(task_id)
        .map_err(|err| ApiError::internal(format!("invalid stored aggregate task id: {err}")))?;
    let status = engine
        .get_status(task_id)
        .await
        .map_err(|err| ApiError::internal(format!("failed to read aggregation status: {err}")))?;
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

const fn runtime_status_from_proof_status(status: &ProofStatus) -> RuntimeRunnerStatus {
    match status {
        ProofStatus::Pending => RuntimeRunnerStatus::Allocated,
        ProofStatus::Proving => RuntimeRunnerStatus::Running,
        ProofStatus::Completed => RuntimeRunnerStatus::Completed,
        ProofStatus::Failed => RuntimeRunnerStatus::Failed,
        ProofStatus::Cancelled => RuntimeRunnerStatus::Cancelled,
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
    root_status: &ProofStatus,
    engine_state_present: bool,
) -> HoodiRootRuntimeView {
    HoodiRootRuntimeView {
        runner_status: runtime_status_from_proof_status(root_status),
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

async fn cancel_registered_tasks(
    runtime: &raiko2_runtime::RuntimeManager,
    engine: &Arc<dyn EngineHandle>,
    public_task_id: &str,
    metadata: &HoodiTaskMetadata,
) -> Result<(), ApiError> {
    let mut errors = Vec::new();

    for proposal in &metadata.proposals {
        if has_other_live_task_reference(runtime, public_task_id, &proposal.task_id).await? {
            continue;
        }
        let task_id = decode_task_id::<EngineTaskKey>(&proposal.task_id)
            .map_err(|err| ApiError::internal(format!("invalid stored proposal task id: {err}")))?;
        for stage_task_id in proposal_task_chain_ids(&task_id) {
            if let Err(err) = engine.cancel(stage_task_id.clone()).await {
                let encoded = encode_task_id(&stage_task_id)
                    .unwrap_or_else(|_| "<invalid-task-id>".to_string());
                errors.push(format!("{encoded}: {err}"));
            }
        }
    }

    if let Some(task_id) = &metadata.aggregate_task_id
        && !has_other_live_task_reference(runtime, public_task_id, task_id).await?
    {
        let task_id = decode_task_id::<EngineTaskKey>(task_id).map_err(|err| {
            ApiError::internal(format!("invalid stored aggregate task id: {err}"))
        })?;
        if let Err(err) = engine.cancel(task_id.clone()).await {
            let encoded =
                encode_task_id(&task_id).unwrap_or_else(|_| "<invalid-task-id>".to_string());
            errors.push(format!("{encoded}: {err}"));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ApiError::internal(format!(
            "failed to cancel one or more child tasks: {}",
            errors.join("; ")
        )))
    }
}

async fn has_other_live_task_reference(
    runtime: &raiko2_runtime::RuntimeManager,
    public_task_id: &str,
    engine_task_id: &str,
) -> Result<bool, ApiError> {
    let records = runtime
        .find_tasks_by_engine_task_id(engine_task_id)
        .await
        .map_err(|err| {
            ApiError::internal(format!(
                "failed to inspect runtime task references for cancellation: {err}"
            ))
        })?;
    Ok(records.into_iter().any(|record| {
        record.task_id != public_task_id
            && matches!(
                record.runner_status,
                RuntimeRunnerStatus::Allocated | RuntimeRunnerStatus::Running
            )
    }))
}

async fn remove_task_children(
    engine: &Arc<dyn EngineHandle>,
    metadata: &HoodiTaskMetadata,
    removed_engine_task_ids: &mut HashSet<String>,
) -> Result<(), ApiError> {
    for proposal in &metadata.proposals {
        let task_id = decode_task_id::<EngineTaskKey>(&proposal.task_id)
            .map_err(|err| ApiError::internal(format!("invalid stored proposal task id: {err}")))?;
        for stage_task_id in proposal_task_chain_ids(&task_id) {
            let encoded = encode_task_id(&stage_task_id)
                .map_err(|err| ApiError::internal(format!("failed to encode task id: {err}")))?;
            if removed_engine_task_ids.insert(encoded) {
                engine
                    .remove(stage_task_id)
                    .await
                    .map_err(|err| ApiError::internal(format!("failed to remove task: {err}")))?;
            }
        }
    }

    if let Some(task_id) = &metadata.aggregate_task_id {
        let task_id = decode_task_id::<EngineTaskKey>(task_id).map_err(|err| {
            ApiError::internal(format!("invalid stored aggregate task id: {err}"))
        })?;
        let encoded = encode_task_id(&task_id)
            .map_err(|err| ApiError::internal(format!("failed to encode task id: {err}")))?;
        if removed_engine_task_ids.insert(encoded) {
            engine
                .remove(task_id)
                .await
                .map_err(|err| ApiError::internal(format!("failed to remove task: {err}")))?;
        }
    }

    Ok(())
}

fn resolved_pair(
    state: &AppState,
    network: &str,
    l1_network: &str,
) -> Result<ResolvedNetworkPair, ApiError> {
    state
        .config
        .rpc
        .resolve_pair(network, l1_network)
        .map_err(|err| ApiError::bad_request(err.to_string()))
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
    match submission.route.proof_type {
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
