use alloy_primitives::{hex, keccak256};
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use raiko2_engine::{
    AggregateProofInput, AggregationTaskRequest, EngineAggregationPlan, EngineExecutionPlan,
    EngineTaskId, ProofArtifactRef, ProposalTaskRequest, ProverTaskConfig,
};
use raiko2_pipeline::{PipelineKey, PipelineRoute, RunnerKind};
use raiko2_primitives::{L2BlockRange, Proof, ProofType};
use raiko2_primitives_shasta::instance::SHASTA_PROPOSAL_ID_MAX;
use raiko2_prover::sp1_config::{
    ExecutionMode as Sp1ExecutionMode, ProverMode as Sp1ProverMode, Sp1RemoteVerifyConfig,
    Sp1RequestContext, Sp1SystemConfig,
};
use raiko2_prover::validate_external_aggregate_proofs;
use raiko2_queue::RootOwner;
use raiko2_runtime::{
    RunnerStatus as RuntimeRunnerStatus, RuntimeManager, RuntimeMutationOutcome, TaskRegistration,
    TaskRegistrationOutcome,
};
use std::sync::Arc;
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::{info, warn};

use super::super::auth::{
    authorize_acl_feature_with_rate_limit, authorize_optional_acl_feature_with_rate_limit,
};
use super::super::errors::ApiError;
use super::proof_route::{
    BatchProofDecision, CanonicalProofRoute, decide_batch_proof_type,
    public_task_id_from_fingerprint, route_for_proof_type, unsupported_proof_type,
};
#[path = "proof_api/v3.rs"]
pub(crate) mod v3;
#[path = "proof_api/v4.rs"]
pub(crate) mod v4;

use super::proof_types::{
    AggregateProofRequest, AggregateStatus, ApiData, ApiOk, BatchProofType, BatchShastaRequest,
    CanonicalProposal, ClearProverStatus, LegacyProofData, LegacyProofEnvelope, LegacyProofError,
    LegacyTaskStatus, ProposalStatus, ProverNetworkStatus, ProverSkippedStatusCounts, ProverStatus,
    ProverTaskStatusCounts, PruneStatus, PublicProverArgs, RootRuntime, RootTaskState,
    ShastaProposal, TaskData, TaskRuntime,
};
use crate::config::{ResolvedNetworkPair, ServerAclFeature};
use crate::server::proof_artifact::{
    ProofArtifactMaterial, ProofArtifactPayload, load_proof_artifact_material,
};
use crate::server::state::{
    AppState, EngineHandle, EngineQueueTaskState, EngineQueueTaskView, EngineStatusView,
    ProofStatus,
};
use crate::server::task_cleanup::{
    proposal_task_chain_ids, proposal_task_id, reconcile_runtime_task_from_artifacts,
};
use crate::server::task_metadata::{
    AggregateInputProofArtifact, BuildTaskMetadataParams, ProofArtifactKind, ProposalTask,
    ProverType, RuntimeMetadata, TaskMetadata, TaskRuntimeMetadata, aggregate_input_proof_ref,
    aggregate_task_ref, proposal_proof_artifact_refs, proposal_task_ref,
    publication_proof_artifact_refs, root_proof_artifact_refs, stage_task_ref,
};
use crate::server::telemetry::{self, MetricContext};

#[derive(Clone)]
struct CanonicalBatchSubmission {
    public_task_id: String,
    pair: ResolvedNetworkPair,
    route: CanonicalProofRoute,
    requested_proof_type: BatchProofType,
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
    task_ref: String,
}

#[derive(Clone)]
struct SubmissionPlan {
    proposals: Vec<PlannedProposalTask>,
    aggregate: Option<PlannedAggregateTask>,
    aggregate_inputs: Vec<AggregateProofInput>,
}

struct ExternalAggregateSubmission {
    pair: ResolvedNetworkPair,
    route: CanonicalProofRoute,
    prover_type: Option<ProverType>,
    public_task_id: String,
    request: AggregationTaskRequest,
    inputs: Vec<AggregateProofInput>,
    input_artifacts: Vec<AggregateInputProofArtifact>,
    input_bytes: Vec<Vec<u8>>,
    request_fingerprint: String,
}

struct PreparedExternalAggregateInputs {
    inputs: Vec<AggregateProofInput>,
    artifacts: Vec<AggregateInputProofArtifact>,
    bytes: Vec<Vec<u8>>,
}

struct TaskLookup {
    record: raiko2_runtime::RuntimeTaskRecord,
    metadata: TaskMetadata,
    engine: Arc<dyn EngineHandle>,
}

#[derive(Clone, Copy)]
enum ProverTaskScope {
    ZkAny,
    ProofType(BatchProofType),
}

impl ProverTaskScope {
    fn matches(self, metadata: &TaskMetadata) -> bool {
        match self {
            Self::ZkAny => is_zk_any_metadata(metadata),
            Self::ProofType(proof_type) => {
                // V4 filters by the requested concrete backend, not by any fallback that may run.
                metadata.requested_proof_type.as_deref() == Some(proof_type.as_str())
            }
        }
    }
}

struct DuplicateTaskLogContext {
    proposal_ids: String,
    proposal_count: usize,
    active_stage: String,
    last_event: String,
    error: String,
}

fn duplicate_task_log_context(
    existing: &raiko2_runtime::RuntimeTaskRecord,
    metadata: &TaskMetadata,
) -> DuplicateTaskLogContext {
    let proposal_ids = duplicate_task_proposal_ids(metadata);
    DuplicateTaskLogContext {
        proposal_count: proposal_ids.len(),
        proposal_ids: format_proposal_ids(&proposal_ids),
        active_stage: metadata
            .runtime
            .active_stage
            .clone()
            .unwrap_or_else(|| "none".to_string()),
        last_event: metadata
            .runtime
            .last_event
            .clone()
            .unwrap_or_else(|| "none".to_string()),
        error: existing.error.clone().unwrap_or_else(|| "none".to_string()),
    }
}

fn duplicate_task_proposal_ids(metadata: &TaskMetadata) -> Vec<u64> {
    metadata.aggregate_request.as_ref().map_or_else(
        || {
            metadata
                .proposals
                .iter()
                .map(|proposal| proposal.proposal_id)
                .collect()
        },
        |request| request.proposal_ids.clone(),
    )
}

fn format_proposal_ids(proposal_ids: &[u64]) -> String {
    match proposal_ids {
        [] => "none".to_string(),
        [single] => single.to_string(),
        ids if ids
            .windows(2)
            .all(|window| window[0].checked_add(1) == Some(window[1])) =>
        {
            format!("{}..{}", ids[0], ids[ids.len() - 1])
        }
        ids => ids.iter().map(u64::to_string).collect::<Vec<_>>().join(","),
    }
}

#[derive(Clone)]
struct ProofLocation {
    proof_ref: Option<String>,
    proof_uri: Option<String>,
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
        requested_proof_type: req.proof_type,
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
    config: &raiko2_prover::sp1_config::Sp1Config,
) -> Result<(), ApiError> {
    if matches!(config.mode, Sp1ExecutionMode::Execute) {
        return Err(ApiError::bad_request(
            "sp1.mode=execute is not supported by the proof API",
        ));
    }
    if matches!(config.mode, Sp1ExecutionMode::Prove) && !config.verify {
        return Err(ApiError::bad_request(
            "sp1.mode=prove requires sp1.verify=true on the hosted API",
        ));
    }
    if matches!(config.mode, Sp1ExecutionMode::Prove)
        && matches!(
            config.prover,
            raiko2_prover::sp1_config::ProverMode::Network
        )
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
    if !req.aggregation_ids.is_empty() && req.aggregation_ids.len() != req.proofs.len() {
        return Err(ApiError::bad_request(
            "aggregation_ids must be empty or match proofs length",
        ));
    }
    for proposal_id in &req.aggregation_ids {
        validate_shasta_proposal_id("aggregation_ids[]", *proposal_id)?;
    }
    Ok(())
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
    validate_external_aggregate_proofs(route.pipeline_key(), &req.proofs)
        .map_err(|err| ApiError::bad_request(err.to_string()))?;
    let request_fingerprint = external_aggregate_request_fingerprint(
        state.runtime.environment(),
        state.runtime.namespace(),
        &pair,
        route,
        prover_type,
        &req,
        &prover_config,
    )?;
    let public_task_id = public_task_id_from_fingerprint(&request_fingerprint);
    let request = AggregationTaskRequest {
        request_id: aggregate_request_id(&request_fingerprint),
        proposal_ids: req.aggregation_ids.clone(),
        prover_config,
    };
    let prepared =
        prepare_external_aggregate_inputs(&pair.key, route, &request_fingerprint, &req.proofs)?;
    let _ = (&req.graffiti, &req.prover, &req.blob_proof_type);

    Ok(ExternalAggregateSubmission {
        pair,
        route,
        prover_type,
        public_task_id,
        request,
        inputs: prepared.inputs,
        input_artifacts: prepared.artifacts,
        input_bytes: prepared.bytes,
        request_fingerprint,
    })
}

fn prepare_external_aggregate_inputs(
    network_pair: &str,
    route: CanonicalProofRoute,
    request_fingerprint: &str,
    proofs: &[Proof],
) -> Result<PreparedExternalAggregateInputs, ApiError> {
    let mut inputs = Vec::with_capacity(proofs.len());
    let mut input_artifacts = Vec::with_capacity(proofs.len());
    let mut input_bytes = Vec::with_capacity(proofs.len());
    for (index, proof) in proofs.iter().enumerate() {
        let proof_ref = aggregate_input_proof_ref(request_fingerprint, index);
        input_bytes.push(serde_json::to_vec(proof).map_err(|err| {
            ApiError::internal(format!("failed to serialize aggregate input proof: {err}"))
        })?);
        inputs.push(AggregateProofInput::ProofArtifact(ProofArtifactRef {
            network_pair: network_pair.to_string(),
            pipeline_key: route.pipeline_key(),
            route: route.route,
            proof_ref: proof_ref.clone(),
        }));
        input_artifacts.push(AggregateInputProofArtifact { proof_ref });
    }
    Ok(PreparedExternalAggregateInputs {
        inputs,
        artifacts: input_artifacts,
        bytes: input_bytes,
    })
}

async fn persist_external_aggregate_input_artifacts(
    runtime: &RuntimeManager,
    network_pair: &str,
    pipeline_key: PipelineKey,
    route: PipelineRoute,
    input_artifacts: &[AggregateInputProofArtifact],
    input_bytes: &[Vec<u8>],
    owner_incarnation: uuid::Uuid,
) -> Result<(), ApiError> {
    if input_artifacts.len() != input_bytes.len() {
        return Err(ApiError::internal(
            "aggregate input artifact and payload counts differ",
        ));
    }
    for (artifact, bytes) in input_artifacts.iter().zip(input_bytes) {
        let publication = runtime
            .publish_active_proof_artifact_bytes(
                network_pair,
                pipeline_key,
                route,
                &artifact.proof_ref,
                owner_incarnation,
                bytes,
            )
            .await
            .map_err(|err| {
                ApiError::internal(format!("failed to write aggregate input proof: {err}"))
            })?;
        publication.try_object().ok_or_else(|| {
            ApiError::internal("aggregate input proof conflict references missing content")
        })?;
    }
    Ok(())
}

async fn planned_external_aggregate_task(
    _runtime: &RuntimeManager,
    submission: &ExternalAggregateSubmission,
) -> Result<PlannedAggregateTask, ApiError> {
    let task_ref = aggregate_task_ref(submission.route.pipeline_key(), &submission.request);
    Ok(PlannedAggregateTask {
        task_ref,
        request: submission.request.clone(),
    })
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

    let mut aggregate_inputs = Vec::new();
    if submission.aggregate_requested {
        aggregate_inputs.reserve(proposals.len());
        for proposal in &proposals {
            aggregate_inputs.push(AggregateProofInput::PendingProofArtifact {
                artifact: proof_artifact_ref(
                    &submission.pair.key,
                    submission.route.pipeline_key(),
                    submission.route.route,
                    &proposal.task_ref,
                ),
                dependency: Box::new(proposal.task_id.clone()),
            });
        }
    }

    let aggregate = planned_batch_aggregate(submission, request_fingerprint);

    Ok(SubmissionPlan {
        proposals,
        aggregate,
        aggregate_inputs,
    })
}

fn planned_batch_aggregate(
    submission: &CanonicalBatchSubmission,
    request_fingerprint: &str,
) -> Option<PlannedAggregateTask> {
    if !submission.aggregate_requested {
        return None;
    }
    let request = AggregationTaskRequest {
        request_id: aggregate_request_id(request_fingerprint),
        proposal_ids: submission
            .proposals
            .iter()
            .map(|proposal| proposal.proposal_id)
            .collect(),
        prover_config: submission.prover_config.clone(),
    };
    let task_ref = aggregate_task_ref(submission.route.pipeline_key(), &request);
    Some(PlannedAggregateTask { request, task_ref })
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
    let registration = build_batch_task_registration(submission, plan, request_fingerprint)?;

    state
        .runtime
        .register_task_if_absent(registration)
        .await
        .map_err(|err| ApiError::internal(format!("failed to register runtime task: {err}")))
}

fn build_batch_task_registration(
    submission: &CanonicalBatchSubmission,
    plan: &SubmissionPlan,
    request_fingerprint: &str,
) -> Result<TaskRegistration, ApiError> {
    let metadata = build_task_metadata(
        &submission.pair,
        BuildTaskMetadataParams {
            network: &submission.pair.network,
            l1_network: &submission.pair.l1_network,
            proof_type: submission.route.proof_type(),
            requested_proof_type: Some(submission.requested_proof_type.as_str()),
            prover_type: submission.prover_type,
            execution_mode: submission.execution_mode,
            aggregate_requested: submission.aggregate_requested,
        },
        &plan.proposals,
        plan.aggregate.as_ref(),
    );
    let artifact_refs = publication_proof_artifact_refs(&metadata, submission.route.pipeline_key());

    Ok(TaskRegistration {
        task_id: submission.public_task_id.clone(),
        pipeline_key: submission.route.pipeline_key(),
        route: submission.route.route,
        task_kind: "hoodi_batch".to_string(),
        network_pair: submission.pair.key.clone(),
        artifact_refs,
        metadata: serde_json::to_value(metadata)
            .map_err(|err| ApiError::internal(format!("failed to serialize metadata: {err}")))?,
        request_fingerprint: request_fingerprint.to_string(),
    })
}

async fn register_external_aggregate_task(
    state: &AppState,
    submission: &ExternalAggregateSubmission,
    aggregate: &PlannedAggregateTask,
) -> Result<TaskRegistrationOutcome, ApiError> {
    let requested_proof_type = submission.route.proof_type();
    let requested_proof_type = requested_proof_type.to_string();
    let mut metadata = build_task_metadata(
        &submission.pair,
        BuildTaskMetadataParams {
            network: &submission.pair.network,
            l1_network: &submission.pair.l1_network,
            proof_type: submission.route.proof_type(),
            // Runtime metadata is persisted as strings, but the request path keeps
            // proof type typed.
            requested_proof_type: Some(&requested_proof_type),
            prover_type: submission.prover_type,
            execution_mode: None,
            aggregate_requested: true,
        },
        &[],
        Some(aggregate),
    );
    metadata.aggregate_input_artifacts = submission.input_artifacts.clone();
    let artifact_refs = publication_proof_artifact_refs(&metadata, submission.route.pipeline_key());

    state
        .runtime
        .register_task_if_absent(TaskRegistration {
            task_id: submission.public_task_id.clone(),
            pipeline_key: submission.route.pipeline_key(),
            route: submission.route.route,
            task_kind: "hoodi_aggregate".to_string(),
            network_pair: submission.pair.key.clone(),
            artifact_refs,
            metadata: serde_json::to_value(metadata).map_err(|err| {
                ApiError::internal(format!("failed to serialize metadata: {err}"))
            })?,
            request_fingerprint: submission.request_fingerprint.clone(),
        })
        .await
        .map_err(|err| ApiError::internal(format!("failed to register runtime task: {err}")))
}

const fn external_aggregate_inputs_need_persistence(outcome: &TaskRegistrationOutcome) -> bool {
    match outcome {
        TaskRegistrationOutcome::Created(_) => true,
        TaskRegistrationOutcome::Existing(record) => {
            existing_external_aggregate_inputs_need_persistence(record.runner_status)
        }
    }
}

const fn existing_external_aggregate_inputs_need_persistence(status: RuntimeRunnerStatus) -> bool {
    matches!(
        status,
        RuntimeRunnerStatus::Allocated | RuntimeRunnerStatus::Failed
    )
}

async fn persist_registered_external_aggregate_inputs(
    state: &AppState,
    submission: &ExternalAggregateSubmission,
    registration: &mut TaskRegistrationOutcome,
) -> Result<(), ApiError> {
    let mut reopened_failed = None;
    if let TaskRegistrationOutcome::Existing(record) = registration
        && record.runner_status == RuntimeRunnerStatus::Failed
    {
        let original = record.clone();
        let prepared = reset_runtime_task_to_allocated(state, &original).await?;
        *record = prepared.clone();
        reopened_failed = Some((original, prepared));
    }
    let owner_incarnation = match &*registration {
        TaskRegistrationOutcome::Created(record) | TaskRegistrationOutcome::Existing(record) => {
            record.incarnation_id
        }
    };
    let persist_inputs = external_aggregate_inputs_need_persistence(registration);
    let persistence = if persist_inputs {
        persist_external_aggregate_input_artifacts(
            &state.runtime,
            &submission.pair.key,
            submission.route.pipeline_key(),
            submission.route.route,
            &submission.input_artifacts,
            &submission.input_bytes,
            owner_incarnation,
        )
        .await
    } else {
        Ok(())
    };
    if let Some((original, prepared)) = &reopened_failed {
        let outcome = state
            .runtime
            .restore_task_after_recovery_if_unchanged(prepared, original)
            .await
            .map_err(|error| {
                ApiError::internal(format!(
                    "failed to restore aggregate root after input repair: {error}"
                ))
            })?;
        if outcome != RuntimeMutationOutcome::Applied {
            return Err(ApiError::internal(format!(
                "aggregate root changed during input repair: {outcome:?}"
            )));
        }
        if let TaskRegistrationOutcome::Existing(record) = registration {
            *record = original.clone();
        }
    }
    if let Err(error) = persistence {
        if let TaskRegistrationOutcome::Created(record) = registration {
            match state
                .runtime
                .fail_task_if_unchanged(record, error.message.clone())
                .await
            {
                Ok(RuntimeMutationOutcome::Applied) => {}
                Ok(outcome) => warn!(
                    task_id = record.task_id,
                    outcome = ?outcome,
                    "aggregate root changed before input persistence failure was recorded"
                ),
                Err(failure_error) => warn!(
                    task_id = record.task_id,
                    error = %failure_error,
                    "failed to record aggregate input persistence failure"
                ),
            }
        }
        return Err(error);
    }
    Ok(())
}

fn build_task_metadata(
    pair: &ResolvedNetworkPair,
    params: BuildTaskMetadataParams<'_>,
    proposals: &[PlannedProposalTask],
    aggregate: Option<&PlannedAggregateTask>,
) -> TaskMetadata {
    let runtime = RuntimeMetadata::current();
    TaskMetadata {
        network_pair: pair.key.clone(),
        network: params.network.to_string(),
        l1_network: params.l1_network.to_string(),
        proof_type: params.proof_type,
        requested_proof_type: params.requested_proof_type.map(str::to_string),
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
                request: proposal.request.clone(),
            })
            .collect(),
        aggregate_task_id: aggregate.map(|task| task.task_ref.clone()),
        aggregate_request: aggregate.map(|task| task.request.clone()),
        aggregate_input_artifacts: Vec::new(),
        runtime,
    }
}

fn execution_plan(plan: &SubmissionPlan) -> EngineExecutionPlan {
    let proposals = plan
        .proposals
        .iter()
        .map(|proposal| proposal.request.clone())
        .collect();
    let aggregate = plan
        .aggregate
        .as_ref()
        .map(|aggregate| EngineAggregationPlan {
            request: aggregate.request.clone(),
            inputs: plan.aggregate_inputs.clone(),
        });
    EngineExecutionPlan {
        proposals,
        aggregate,
    }
}

async fn attach_submission_plan(
    state: &AppState,
    engine: &Arc<dyn EngineHandle>,
    root: &raiko2_runtime::RuntimeTaskRecord,
    plan: &SubmissionPlan,
) -> Result<(), ApiError> {
    state
        .lifecycle
        .attach(root, engine, execution_plan(plan))
        .await
        .map_err(|err| ApiError::internal(format!("failed to attach submission plan: {err}")))?;
    Ok(())
}

async fn handle_existing_batch_task(
    state: &AppState,
    submission: &CanonicalBatchSubmission,
    existing: raiko2_runtime::RuntimeTaskRecord,
    replacement_request_fingerprint: Option<&str>,
) -> Result<Response, ApiError> {
    let existing_metadata = parse_task_metadata(&existing)?;
    let log_context = duplicate_task_log_context(&existing, &existing_metadata);
    info!(
        task_id = %existing.task_id,
        aggregate = submission.aggregate_requested,
        proposal_ids = %log_context.proposal_ids,
        proposal_count = log_context.proposal_count,
        runner_status = %existing.runner_status.as_str(),
        active_stage = %log_context.active_stage,
        last_event = %log_context.last_event,
        error = %log_context.error,
        updated_at = existing.updated_at,
        route = %submission.route.route,
        proof_type = %submission.route.proof_type(),
        prover_type = %prover_type_label(submission.prover_type),
        network_pair = %submission.pair.key,
        "detected duplicate shasta batch request"
    );
    let missing_completed_artifact =
        completed_root_artifact_missing(state.runtime.as_ref(), &existing, &existing_metadata)
            .await?;
    let canonical_existing_route = canonical_persisted_route(&existing)?;
    let uses_legacy_route_alias = canonical_existing_route != existing.route;
    telemetry::record_duplicate_request(
        &MetricContext::new(
            submission.route.route.to_string(),
            submission.route.proof_type(),
            submission.pair.key.clone(),
            submission.aggregate_requested,
        ),
        duplicate_runner_status_label(existing.runner_status, missing_completed_artifact),
    );
    if existing.pipeline_key != submission.route.pipeline_key()
        || canonical_existing_route != submission.route.route
        || (uses_legacy_route_alias
            && (existing.runner_status != RuntimeRunnerStatus::Completed
                || missing_completed_artifact))
    {
        return replace_existing_batch_task(
            state,
            submission,
            &existing,
            replacement_request_fingerprint,
        )
        .await;
    }
    if let Some(response) =
        readable_completed_response(state, &existing, missing_completed_artifact).await?
    {
        return Ok(response);
    }
    if missing_completed_artifact
        || should_reenqueue_existing_submission(state, &existing, &existing_metadata).await?
    {
        let response = compatibility_response_for_task(state, &existing.task_id).await?;
        if response_is_completed(&response) && !missing_completed_artifact {
            return Ok(response);
        }
        if let Err(err) = recover_existing_task(state, &existing, || {
            reenqueue_existing_batch_task(state, &existing, &existing_metadata)
        })
        .await
        {
            warn!(
                task_id = existing.task_id,
                existing_pipeline = %existing.pipeline_key,
                requested_pipeline = %submission.route.pipeline_key(),
                error = %err.message,
                "failed to recover reenqueueable task; replacing from scratch"
            );
            return replace_existing_batch_task(
                state,
                submission,
                &existing,
                replacement_request_fingerprint,
            )
            .await;
        }
    }
    compatibility_response_for_task(state, &existing.task_id).await
}

async fn readable_completed_response(
    state: &AppState,
    existing: &raiko2_runtime::RuntimeTaskRecord,
    missing_completed_artifact: bool,
) -> Result<Option<Response>, ApiError> {
    if existing.runner_status != RuntimeRunnerStatus::Completed || missing_completed_artifact {
        return Ok(None);
    }
    let response = compatibility_response_for_task(state, &existing.task_id).await?;
    Ok(response_is_completed(&response).then_some(response))
}

async fn replace_existing_batch_task(
    state: &AppState,
    submission: &CanonicalBatchSubmission,
    existing: &raiko2_runtime::RuntimeTaskRecord,
    replacement_request_fingerprint: Option<&str>,
) -> Result<Response, ApiError> {
    let request_fingerprint = replacement_request_fingerprint
        .map(ToOwned::to_owned)
        .map_or_else(
            || {
                batch_request_fingerprint(
                    state.runtime.environment(),
                    state.runtime.namespace(),
                    submission,
                )
            },
            Ok,
        )?;
    let plan = build_submission_plan(submission, &request_fingerprint)?;
    let engine = resolve_engine(state, &submission.pair.key, submission.route.pipeline_key())?;
    let registration = build_batch_task_registration(submission, &plan, &request_fingerprint)?;
    let Some(_replacement) = state
        .lifecycle
        .replace(existing, registration, &[], &engine, execution_plan(&plan))
        .await
        .map_err(|err| ApiError::internal(format!("failed to replace runtime task: {err}")))?
    else {
        let matching = state
            .runtime
            .find_task_by_request_fingerprint(&request_fingerprint)
            .await
            .map_err(|err| {
                ApiError::internal(format!("failed to find concurrent replacement: {err}"))
            })?;
        if let Some(current) = matching {
            return compatibility_response_for_task(state, &current.task_id).await;
        }

        let occupying = state
            .runtime
            .get_task(&submission.public_task_id)
            .await
            .map_err(|err| {
                ApiError::internal(format!("failed to load concurrent replacement: {err}"))
            })?;
        return Err(if let Some(occupying) = occupying {
            ApiError::conflict(format!(
                "task {} changed to a different request while replacement was in progress",
                occupying.task_id
            ))
        } else {
            ApiError::internal("runtime task changed during replacement and then disappeared")
        });
    };

    Ok(registered_batch_response(submission))
}

async fn reenqueue_existing_batch_task(
    state: &AppState,
    existing: &raiko2_runtime::RuntimeTaskRecord,
    existing_metadata: &TaskMetadata,
) -> Result<(), ApiError> {
    let engine = resolve_engine(state, &existing.network_pair, existing.pipeline_key)?;
    let recovery_plan = build_recovery_plan_from_metadata(existing, existing_metadata)?;
    attach_submission_plan(state, &engine, existing, &recovery_plan)
        .await
        .map_err(|err| {
            ApiError::internal(format!(
                "failed to recover dormant task {}: {}",
                existing.task_id, err.message
            ))
        })
}

fn build_recovery_plan_from_metadata(
    existing: &raiko2_runtime::RuntimeTaskRecord,
    metadata: &TaskMetadata,
) -> Result<SubmissionPlan, ApiError> {
    let proposals = planned_recovery_proposals(existing, metadata)?;
    let mut aggregate_inputs = Vec::new();
    if metadata.aggregate_requested {
        aggregate_inputs.reserve(proposals.len());
        for proposal in &proposals {
            aggregate_inputs.push(AggregateProofInput::PendingProofArtifact {
                artifact: proof_artifact_ref(
                    &existing.network_pair,
                    existing.pipeline_key,
                    existing.route,
                    &proposal.task_ref,
                ),
                dependency: Box::new(proposal.task_id.clone()),
            });
        }
    }

    let aggregate = planned_recovery_aggregate(existing, metadata);
    if metadata.aggregate_requested && aggregate.is_none() {
        return Err(ApiError::internal(format!(
            "task {} is missing persisted aggregate request",
            existing.task_id
        )));
    }

    Ok(SubmissionPlan {
        proposals,
        aggregate,
        aggregate_inputs,
    })
}

fn planned_recovery_proposals(
    existing: &raiko2_runtime::RuntimeTaskRecord,
    metadata: &TaskMetadata,
) -> Result<Vec<PlannedProposalTask>, ApiError> {
    let mut proposals = Vec::with_capacity(metadata.proposals.len());
    for proposal in &metadata.proposals {
        let request = proposal.request.clone();
        let task_ref = proposal_task_ref(existing.pipeline_key, &request);
        proposals.push(PlannedProposalTask {
            task_id: proposal_task_id(existing.pipeline_key, request.clone()),
            request,
            task_ref: task_ref.clone(),
            proposal: CanonicalProposal {
                proposal_id: proposal.proposal_id,
                checkpoint: proposal.checkpoint,
                l1_inclusion_block_number: proposal.l1_inclusion_block_number,
                l2_block_range: validate_l2_block_numbers(&proposal.l2_block_numbers)?,
                l2_block_numbers: proposal.l2_block_numbers.clone(),
                last_anchor_block_number: proposal.last_anchor_block_number,
            },
        });
    }
    Ok(proposals)
}

fn planned_recovery_aggregate(
    existing: &raiko2_runtime::RuntimeTaskRecord,
    metadata: &TaskMetadata,
) -> Option<PlannedAggregateTask> {
    metadata.aggregate_request.clone().map(|request| {
        let task_ref = aggregate_task_ref(existing.pipeline_key, &request);
        PlannedAggregateTask { request, task_ref }
    })
}

async fn handle_created_batch_task(
    state: &AppState,
    submission: &CanonicalBatchSubmission,
    plan: &SubmissionPlan,
    root: &raiko2_runtime::RuntimeTaskRecord,
) -> Result<Response, ApiError> {
    let engine = resolve_engine(state, &submission.pair.key, submission.route.pipeline_key())?;
    attach_submission_plan(state, &engine, root, plan).await?;

    Ok(registered_batch_response(submission))
}

fn registered_batch_response(submission: &CanonicalBatchSubmission) -> Response {
    telemetry::record_request_registered(
        &MetricContext::new(
            submission.route.route.to_string(),
            submission.route.proof_type(),
            submission.pair.key.clone(),
            submission.aggregate_requested,
        ),
        submission.aggregate_requested,
    );

    registered_response(
        hoodi_response_proof_type(submission),
        submission.public_task_id.clone(),
    )
    .into_response()
}

async fn handle_existing_external_aggregate_task(
    state: &AppState,
    engine: &Arc<dyn EngineHandle>,
    submission: &ExternalAggregateSubmission,
    existing: raiko2_runtime::RuntimeTaskRecord,
) -> Result<Response, ApiError> {
    let existing_metadata = parse_task_metadata(&existing)?;
    let log_context = duplicate_task_log_context(&existing, &existing_metadata);
    info!(
        task_id = %existing.task_id,
        aggregate = true,
        proposal_ids = %log_context.proposal_ids,
        proposal_count = log_context.proposal_count,
        runner_status = %existing.runner_status.as_str(),
        active_stage = %log_context.active_stage,
        last_event = %log_context.last_event,
        error = %log_context.error,
        updated_at = existing.updated_at,
        route = %submission.route.route,
        proof_type = %submission.route.proof_type(),
        prover_type = %prover_type_label(submission.prover_type),
        network_pair = %submission.pair.key,
        "detected duplicate shasta aggregate request"
    );
    let missing_completed_artifact =
        completed_root_artifact_missing(state.runtime.as_ref(), &existing, &existing_metadata)
            .await?;
    telemetry::record_duplicate_request(
        &MetricContext::new(
            submission.route.route.to_string(),
            submission.route.proof_type(),
            submission.pair.key.clone(),
            true,
        ),
        duplicate_runner_status_label(existing.runner_status, missing_completed_artifact),
    );
    if missing_completed_artifact
        || should_reenqueue_existing_submission(state, &existing, &existing_metadata).await?
    {
        let response = compatibility_response_for_task(state, &existing.task_id).await?;
        if response_is_completed(&response) && !missing_completed_artifact {
            return Ok(response);
        }
        recover_existing_task(state, &existing, || {
            reenqueue_existing_external_aggregate_task(
                state,
                engine,
                Some(&submission.inputs),
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
    let reopened = reset_runtime_task_to_allocated(state, existing).await?;
    if let Err(err) = reenqueue().await {
        restore_runtime_task_status(state, &reopened, existing, &err).await;
        return Err(err);
    }
    Ok(())
}

async fn reset_runtime_task_to_allocated(
    state: &AppState,
    record: &raiko2_runtime::RuntimeTaskRecord,
) -> Result<raiko2_runtime::RuntimeTaskRecord, ApiError> {
    let reopened = state
        .runtime
        .prepare_task_for_recovery_if_unchanged(record)
        .await
        .map_err(|err| {
            ApiError::internal(format!(
                "failed to reset recovered task {}: {err}",
                record.task_id
            ))
        })?;
    reopened.ok_or_else(|| {
        ApiError::internal(format!(
            "task {} is no longer eligible for failed-task recovery",
            record.task_id
        ))
    })
}

async fn restore_runtime_task_status(
    state: &AppState,
    reopened: &raiko2_runtime::RuntimeTaskRecord,
    existing: &raiko2_runtime::RuntimeTaskRecord,
    enqueue_error: &ApiError,
) {
    match state
        .runtime
        .restore_task_after_recovery_if_unchanged(reopened, existing)
        .await
    {
        Ok(RuntimeMutationOutcome::Applied) => {}
        Ok(outcome) => warn!(
            task_id = existing.task_id,
            original_status = existing.runner_status.as_str(),
            enqueue_error = %enqueue_error.message,
            outcome = ?outcome,
            "runtime task changed before recovery rollback"
        ),
        Err(err) => warn!(
            task_id = existing.task_id,
            original_status = existing.runner_status.as_str(),
            enqueue_error = %enqueue_error.message,
            restore_error = %err,
            "failed to restore runtime status after recovery enqueue failure"
        ),
    }
}

async fn reenqueue_existing_external_aggregate_task(
    state: &AppState,
    engine: &Arc<dyn EngineHandle>,
    fallback_inputs: Option<&[AggregateProofInput]>,
    existing: &raiko2_runtime::RuntimeTaskRecord,
    existing_metadata: &TaskMetadata,
) -> Result<(), ApiError> {
    let request = existing_metadata
        .aggregate_request
        .clone()
        .ok_or_else(|| ApiError::internal("existing aggregate task missing aggregate_request"))?;
    let inputs = if existing_metadata.aggregate_input_artifacts.is_empty() {
        fallback_inputs
            .map(<[AggregateProofInput]>::to_vec)
            .ok_or_else(|| {
                ApiError::internal(format!(
                    "existing aggregate task {} has no persisted input artifacts",
                    existing.task_id
                ))
            })?
    } else {
        aggregate_inputs_from_artifacts(
            &existing.network_pair,
            existing.pipeline_key,
            existing.route,
            &existing_metadata.aggregate_input_artifacts,
        )
    };
    state
        .lifecycle
        .attach(
            existing,
            engine,
            EngineExecutionPlan {
                proposals: Vec::new(),
                aggregate: Some(EngineAggregationPlan { request, inputs }),
            },
        )
        .await
        .map_err(|err| {
            ApiError::internal(format!(
                "failed to recover dormant aggregate task {}: {err}",
                existing.task_id
            ))
        })?;
    Ok(())
}

pub(crate) async fn validate_persisted_runtime_task_metadata(
    state: &AppState,
) -> anyhow::Result<()> {
    for record in state.runtime.list_tasks().await? {
        TaskMetadata::decode_for_record(&record).map_err(|error| {
            anyhow::anyhow!(
                "runtime task {} has invalid canonical metadata: {error}",
                record.task_id
            )
        })?;
    }
    Ok(())
}

pub(crate) async fn recover_pending_runtime_tasks(state: &AppState) -> anyhow::Result<usize> {
    let records = state.runtime.list_tasks().await?;
    let mut recovered = 0;
    for record in records {
        let metadata = TaskMetadata::decode_for_record(&record).map_err(|error| {
            anyhow::anyhow!(
                "runtime task {} has invalid canonical metadata during recovery: {error}",
                record.task_id
            )
        })?;
        if matches!(
            record.runner_status,
            RuntimeRunnerStatus::Completed | RuntimeRunnerStatus::Cancelled
        ) {
            continue;
        }
        if canonical_persisted_route(&record).map_err(|error| anyhow::anyhow!(error.message))?
            != record.route
        {
            warn!(
                task_id = record.task_id,
                pipeline = %record.pipeline_key,
                stored_route = %record.route,
                canonical_route = %record.pipeline_key.route(),
                "skipping startup recovery for a legacy route alias; a repeated request will replace the task canonically"
            );
            continue;
        }
        if record.runner_status == RuntimeRunnerStatus::Failed
            && !failed_stage_is_reenqueueable(&record, &metadata)
        {
            continue;
        }

        let Some(recovery_record) = state
            .runtime
            .prepare_task_for_recovery_if_unchanged(&record)
            .await?
        else {
            warn!(
                task_id = record.task_id,
                "runtime root changed before startup recovery preparation"
            );
            continue;
        };

        let external_aggregate =
            metadata.proposals.is_empty() && metadata.aggregate_request.is_some();
        let result = if external_aggregate {
            recover_external_aggregate_runtime_task(state, &recovery_record, &metadata).await
        } else {
            reenqueue_existing_batch_task(state, &recovery_record, &metadata).await
        };
        match result {
            Ok(()) => {
                recovered += 1;
            }
            Err(error) => {
                record_recovery_failure_if_unchanged(
                    state,
                    &recovery_record,
                    format!("runtime recovery failed: {}", error.message),
                )
                .await?;
            }
        }
    }
    Ok(recovered)
}

async fn record_recovery_failure_if_unchanged(
    state: &AppState,
    expected: &raiko2_runtime::RuntimeTaskRecord,
    error: String,
) -> anyhow::Result<()> {
    match state
        .runtime
        .fail_task_if_unchanged(expected, error)
        .await?
    {
        RuntimeMutationOutcome::Applied | RuntimeMutationOutcome::AlreadyApplied => {}
        outcome => warn!(
            task_id = expected.task_id,
            outcome = ?outcome,
            "runtime root changed before startup recovery failure was recorded"
        ),
    }
    Ok(())
}

async fn recover_external_aggregate_runtime_task(
    state: &AppState,
    record: &raiko2_runtime::RuntimeTaskRecord,
    metadata: &TaskMetadata,
) -> Result<(), ApiError> {
    recover_external_aggregate_input_artifacts(state.runtime.as_ref(), record, metadata).await?;
    let engine = resolve_engine(state, &record.network_pair, record.pipeline_key)?;
    reenqueue_existing_external_aggregate_task(state, &engine, None, record, metadata).await
}

fn aggregate_inputs_from_artifacts(
    network_pair: &str,
    pipeline_key: PipelineKey,
    route: PipelineRoute,
    artifacts: &[AggregateInputProofArtifact],
) -> Vec<AggregateProofInput> {
    artifacts
        .iter()
        .map(|artifact| {
            AggregateProofInput::ProofArtifact(ProofArtifactRef {
                network_pair: network_pair.to_string(),
                pipeline_key,
                route,
                proof_ref: artifact.proof_ref.clone(),
            })
        })
        .collect()
}

async fn recover_external_aggregate_input_artifacts(
    runtime: &RuntimeManager,
    record: &raiko2_runtime::RuntimeTaskRecord,
    metadata: &TaskMetadata,
) -> Result<(), ApiError> {
    let mut bytes = Vec::with_capacity(metadata.aggregate_input_artifacts.len());
    for artifact in &metadata.aggregate_input_artifacts {
        let pending = runtime
            .get_recoverable_pending_proof_publication(
                &record.network_pair,
                record.pipeline_key,
                record.route,
                &artifact.proof_ref,
            )
            .await
            .map_err(|err| {
                ApiError::internal(format!("failed to read pending aggregate input: {err}"))
            })?;
        let payload = if let Some(pending) = pending {
            pending.bytes
        } else {
            let registration = runtime
                .get_proof_artifact(
                    &record.network_pair,
                    record.pipeline_key,
                    record.route,
                    &artifact.proof_ref,
                )
                .await
                .map_err(|err| {
                    ApiError::internal(format!(
                        "failed to read aggregate input registration: {err}"
                    ))
                })?
                .ok_or_else(|| {
                    ApiError::internal(format!(
                        "aggregate input {} has no owned pending or active registration",
                        artifact.proof_ref
                    ))
                })?;
            let object = runtime
                .read_proof_artifact_bytes(
                    &record.network_pair,
                    record.pipeline_key,
                    record.route,
                    &artifact.proof_ref,
                )
                .await
                .map_err(|err| {
                    ApiError::internal(format!("failed to read aggregate input: {err}"))
                })?
                .ok_or_else(|| {
                    ApiError::internal(format!(
                        "active aggregate input {} is missing content",
                        artifact.proof_ref
                    ))
                })?;
            if object.descriptor() != registration.descriptor() {
                return Err(ApiError::internal(format!(
                    "active aggregate input {} descriptor changed",
                    artifact.proof_ref
                )));
            }
            object.bytes
        };
        bytes.push(payload);
    }
    persist_external_aggregate_input_artifacts(
        runtime,
        &record.network_pair,
        record.pipeline_key,
        record.route,
        &metadata.aggregate_input_artifacts,
        &bytes,
        record.incarnation_id,
    )
    .await
}

async fn handle_created_external_aggregate_task(
    state: &AppState,
    engine: &Arc<dyn EngineHandle>,
    submission: &ExternalAggregateSubmission,
) -> Result<Response, ApiError> {
    let root = state
        .runtime
        .get_task(&submission.public_task_id)
        .await
        .map_err(|err| ApiError::internal(format!("failed to load registered task: {err}")))?
        .ok_or_else(|| ApiError::internal("registered runtime task disappeared"))?;
    state
        .lifecycle
        .attach(
            &root,
            engine,
            EngineExecutionPlan {
                proposals: Vec::new(),
                aggregate: Some(EngineAggregationPlan {
                    request: submission.request.clone(),
                    inputs: submission.inputs.clone(),
                }),
            },
        )
        .await
        .map_err(|err| ApiError::internal(format!("failed to attach aggregation plan: {err}")))?;

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

async fn load_task_data(state: &AppState, id: &str) -> Result<TaskData, ApiError> {
    let lookup = load_task_lookup(state, id).await?;
    load_task_data_from_lookup(state, id, &lookup).await
}

async fn load_task_data_from_lookup(
    state: &AppState,
    id: &str,
    lookup: &TaskLookup,
) -> Result<TaskData, ApiError> {
    let reconciled_proof_uri = reconcile_runtime_task_from_artifacts(
        state.runtime.as_ref(),
        &lookup.record,
        &lookup.metadata,
    )
    .await
    .map_err(|err| ApiError::internal(format!("failed to reconcile task completion: {err}")))?;
    let mut record = lookup.record.clone();
    if let Some(proof_uri) = reconciled_proof_uri {
        record.runner_status = RuntimeRunnerStatus::Completed;
        record.error = None;
        record.proof_uri = Some(proof_uri);
    }
    let (proposals, proposal_engine_state_present): (Vec<ProposalStatus>, bool) =
        load_proposal_statuses(
            state.runtime.as_ref(),
            &lookup.engine,
            &lookup.metadata,
            &record,
        )
        .await?;
    let (aggregate, aggregate_engine_state_present): (Option<AggregateStatus>, bool) =
        load_aggregate_status(
            state.runtime.as_ref(),
            &lookup.engine,
            &lookup.metadata,
            &record,
        )
        .await?;
    let root_engine_state_present = proposal_engine_state_present || aggregate_engine_state_present;
    let root_state = resolve_root_task_state(
        record.runner_status,
        &proposals,
        aggregate.as_ref(),
        lookup.metadata.has_runtime_progress(),
        record.error.as_deref(),
    );
    let root_proof_location =
        root_proof_location(&record, &lookup.metadata, &proposals, aggregate.as_ref());
    let root_proof = root_state.proof;
    let root_proof = if root_proof.is_none()
        && matches!(root_state.status, ProofStatus::Completed)
        && root_proof_artifact_refs(&lookup.metadata, record.pipeline_key).is_some()
    {
        load_persisted_root_proof(state.runtime.as_ref(), &record, &lookup.metadata).await?
    } else {
        root_proof
    };
    Ok(TaskData {
        task_id: id.to_string(),
        route: canonical_persisted_route(&lookup.record)?.to_string(),
        prover_type: lookup.metadata.prover_type_str(),
        execution_mode: lookup.metadata.execution_mode_str(),
        status: root_state.status.clone(),
        network: lookup.metadata.network.clone(),
        l1_network: lookup.metadata.l1_network.clone(),
        runtime: root_runtime_view(&record, &lookup.metadata, root_engine_state_present),
        current_index: root_state.current_index,
        proposals,
        aggregate,
        proof: root_proof,
        proof_ref: root_proof_location
            .as_ref()
            .and_then(|location| location.proof_ref.clone()),
        proof_uri: root_proof_location.and_then(|location| location.proof_uri),
        error: root_state.error,
    })
}

async fn load_persisted_root_proof(
    runtime: &RuntimeManager,
    record: &raiko2_runtime::RuntimeTaskRecord,
    metadata: &TaskMetadata,
) -> Result<Option<String>, ApiError> {
    Ok(
        load_persisted_root_proof_material(runtime, record, metadata)
            .await?
            .and_then(|proof| proof.proof),
    )
}

async fn load_persisted_root_proof_material(
    runtime: &RuntimeManager,
    record: &raiko2_runtime::RuntimeTaskRecord,
    metadata: &TaskMetadata,
) -> Result<Option<Proof>, ApiError> {
    let Some(refs) = root_proof_artifact_refs(metadata, record.pipeline_key) else {
        return Ok(None);
    };
    let expected_payload = match refs.kind {
        ProofArtifactKind::Proposal => ProofArtifactPayload::Proposal,
        ProofArtifactKind::Aggregate => ProofArtifactPayload::Final,
    };
    for proof_ref in refs.refs {
        if let Some(material) = load_proof_artifact_material(
            runtime,
            &record.network_pair,
            record.pipeline_key,
            record.route,
            &proof_ref,
            expected_payload,
        )
        .await
        .map_err(|err| ApiError::internal(format!("failed to load proof artifact: {err}")))?
        {
            return Ok(Some(material.proof));
        }
    }
    Ok(None)
}

fn artifact_proof_location(record: &raiko2_runtime::ProofArtifactRecord) -> ProofLocation {
    ProofLocation {
        proof_ref: Some(record.proof_ref.clone()),
        proof_uri: Some(record.proof_uri.clone()),
    }
}

fn proof_artifact_ref(
    network_pair: &str,
    pipeline_key: PipelineKey,
    route: PipelineRoute,
    proof_ref: &str,
) -> ProofArtifactRef {
    ProofArtifactRef {
        network_pair: network_pair.to_string(),
        pipeline_key,
        route,
        proof_ref: proof_ref.to_string(),
    }
}

fn status_proof_location(
    proof_ref: Option<&String>,
    proof_uri: Option<&String>,
) -> Option<ProofLocation> {
    if proof_ref.is_none() && proof_uri.is_none() {
        return None;
    }

    Some(ProofLocation {
        proof_ref: proof_ref.cloned(),
        proof_uri: proof_uri.cloned(),
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
        record.proof_uri.as_ref().map(|proof_uri| ProofLocation {
            proof_ref: None,
            proof_uri: Some(proof_uri.clone()),
        })
    };

    if let Some(aggregate) = aggregate
        && let Some(location) =
            status_proof_location(aggregate.proof_ref.as_ref(), aggregate.proof_uri.as_ref())
    {
        return Some(location);
    }
    if aggregate.is_some() {
        return record_location();
    }

    if let [proposal] = proposals
        && let Some(location) =
            status_proof_location(proposal.proof_ref.as_ref(), proposal.proof_uri.as_ref())
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

async fn collect_prover_status(
    state: &AppState,
    scope: ProverTaskScope,
) -> Result<
    (
        ProverTaskStatusCounts,
        ProverNetworkStatus,
        ProverSkippedStatusCounts,
    ),
    ApiError,
> {
    let records = state
        .runtime
        .list_tasks()
        .await
        .map_err(|err| ApiError::internal(format!("failed to list runtime tasks: {err}")))?;
    let mut tasks = ProverTaskStatusCounts::default();
    let mut network = ProverNetworkStatus::default();
    let mut skipped = ProverSkippedStatusCounts::default();
    let mut queue_groups: HashMap<ProverQueueKey, (Arc<dyn EngineHandle>, HashSet<EngineTaskId>)> =
        HashMap::new();
    let now_secs = unix_now_secs();

    for record in records {
        let metadata = match parse_task_metadata(&record) {
            Ok(metadata) => metadata,
            Err(err) => {
                skipped.invalid_metadata = skipped.invalid_metadata.saturating_add(1);
                warn!(
                    task_id = %record.task_id,
                    error = %err.message,
                    "skipping prover status record with invalid metadata"
                );
                continue;
            }
        };
        if !scope.matches(&metadata) {
            continue;
        }

        if is_terminal_runtime_status(record.runner_status) {
            continue;
        }
        count_network_inflight(&metadata, &mut network, now_secs);
        let engine = match resolve_engine(state, &record.network_pair, record.pipeline_key) {
            Ok(engine) => engine,
            Err(err) if err.status == StatusCode::NOT_FOUND => {
                skipped.unavailable_pipeline = skipped.unavailable_pipeline.saturating_add(1);
                warn!(
                    task_id = %record.task_id,
                    network_pair = %record.network_pair,
                    pipeline = %record.pipeline_key,
                    error = %err.message,
                    "skipping prover status record with unavailable pipeline"
                );
                continue;
            }
            Err(err) => return Err(err),
        };
        let has_active_execution = engine
            .has_active_execution(RootOwner::new(
                record.task_id.clone(),
                record.incarnation_id,
            ))
            .await
            .map_err(|err| {
                ApiError::internal(format!(
                    "failed to inspect execution projection for {}: {err}",
                    record.task_id
                ))
            })?;
        if !has_active_execution && !metadata.has_remote_submission_progress() {
            tasks.orphaned = tasks.orphaned.saturating_add(1);
        }
        let engine_key = (record.network_pair.clone(), record.pipeline_key);
        let task_ids = metadata_queue_task_ids(&metadata, record.pipeline_key);
        queue_groups
            .entry(engine_key)
            .or_insert_with(|| (engine, HashSet::new()))
            .1
            .extend(task_ids.iter().cloned());
    }

    for (engine, task_ids) in queue_groups.into_values() {
        count_matching_queue_tasks(engine.as_ref(), &task_ids, &mut tasks).await?;
    }

    Ok((tasks, network, skipped))
}

#[allow(clippy::too_many_lines)]
async fn clear_prover_tasks(
    state: &AppState,
    scope: ProverTaskScope,
) -> Result<ClearProverStatus, ApiError> {
    let records = state
        .runtime
        .list_tasks()
        .await
        .map_err(|err| ApiError::internal(format!("failed to list runtime tasks: {err}")))?;
    let mut status = ClearProverStatus {
        status: "ok",
        cancelled: 0,
        failed: 0,
        skipped: ProverSkippedStatusCounts::default(),
    };

    for record in records {
        if is_terminal_runtime_status(record.runner_status) {
            continue;
        }

        let metadata = match parse_task_metadata(&record) {
            Ok(metadata) => metadata,
            Err(err) => {
                status.skipped.invalid_metadata = status.skipped.invalid_metadata.saturating_add(1);
                warn!(
                    task_id = %record.task_id,
                    error = %err.message,
                    "skipping prover clear record with invalid metadata"
                );
                continue;
            }
        };

        if !scope.matches(&metadata) {
            continue;
        }
        if metadata.has_remote_submission_progress() {
            status.skipped.remote_progress = status.skipped.remote_progress.saturating_add(1);
            warn!(
                task_id = %record.task_id,
                "skipping prover clear record with remote submission progress"
            );
            continue;
        }

        let cancelled = match state.lifecycle.cancel(&record, None).await {
            Ok(RuntimeMutationOutcome::Applied) => true,
            Ok(
                RuntimeMutationOutcome::AlreadyApplied
                | RuntimeMutationOutcome::Blocked
                | RuntimeMutationOutcome::Missing
                | RuntimeMutationOutcome::Stale
                | RuntimeMutationOutcome::Conflict,
            ) => false,
            Err(err) => {
                match state.runtime.get_task(&record.task_id).await {
                    Ok(Some(current))
                        if current.incarnation_id == record.incarnation_id
                            && current.runner_status == RuntimeRunnerStatus::Cancelled =>
                    {
                        status.cancelled = status.cancelled.saturating_add(1);
                    }
                    Ok(_) => {}
                    Err(read_error) => warn!(
                        task_id = %record.task_id,
                        error = %read_error,
                        "failed to confirm runtime root after prover clear error"
                    ),
                }
                status.failed = status.failed.saturating_add(1);
                warn!(
                    task_id = %record.task_id,
                    error = %err,
                    "failed to sync prover clear cancellation"
                );
                continue;
            }
        };
        if state
            .pipelines
            .get(&record.network_pair, record.pipeline_key)
            .is_none()
        {
            status.skipped.unavailable_pipeline =
                status.skipped.unavailable_pipeline.saturating_add(1);
        }
        if !cancelled {
            continue;
        }
        status.cancelled = status.cancelled.saturating_add(1);
    }

    status.status = operation_status(status.failed);
    Ok(status)
}

const fn operation_status(failed: usize) -> &'static str {
    if failed == 0 { "ok" } else { "partial_failure" }
}

type ProverQueueKey = (String, PipelineKey);

fn parse_task_metadata(
    record: &raiko2_runtime::RuntimeTaskRecord,
) -> Result<TaskMetadata, ApiError> {
    TaskMetadata::decode_for_record(record).map_err(|err| {
        ApiError::internal(format!(
            "failed parse runtime task metadata {}: {err}",
            record.task_id
        ))
    })
}

fn canonical_persisted_route(
    record: &raiko2_runtime::RuntimeTaskRecord,
) -> Result<PipelineRoute, ApiError> {
    record
        .pipeline_key
        .canonicalize_persisted_route(record.route)
        .ok_or_else(|| {
            ApiError::internal(format!(
                "runtime task route {} does not match pipeline {}",
                record.route, record.pipeline_key
            ))
        })
}

const fn is_terminal_runtime_status(status: RuntimeRunnerStatus) -> bool {
    matches!(
        status,
        RuntimeRunnerStatus::Completed
            | RuntimeRunnerStatus::Failed
            | RuntimeRunnerStatus::Cancelled
    )
}

fn is_zk_any_metadata(metadata: &TaskMetadata) -> bool {
    metadata.requested_proof_type.as_deref() == Some("zk_any")
}

fn metadata_queue_task_ids(
    metadata: &TaskMetadata,
    pipeline_key: PipelineKey,
) -> HashSet<EngineTaskId> {
    let mut task_ids = HashSet::new();
    for proposal in &metadata.proposals {
        let task_id = proposal.engine_task_id(pipeline_key);
        task_ids.extend(proposal_task_chain_ids(&task_id));
    }
    if let Some(task_id) = metadata.aggregate_engine_task_id(pipeline_key) {
        task_ids.insert(task_id);
    }
    task_ids
}

async fn count_matching_queue_tasks(
    engine: &dyn EngineHandle,
    task_ids: &HashSet<EngineTaskId>,
    counts: &mut ProverTaskStatusCounts,
) -> Result<(), ApiError> {
    if task_ids.is_empty() {
        return Ok(());
    }
    for task_id in task_ids {
        let Some(view) = engine
            .get_task_state(task_id.clone())
            .await
            .map_err(|err| ApiError::internal(format!("failed to load queue task: {err}")))?
        else {
            continue;
        };
        count_queue_task_state(&view, counts);
    }
    Ok(())
}

const fn count_queue_task_state(view: &EngineQueueTaskView, counts: &mut ProverTaskStatusCounts) {
    match view.state {
        EngineQueueTaskState::Pending => {
            counts.pending = counts.pending.saturating_add(1);
        }
        EngineQueueTaskState::Ready => {
            counts.ready = counts.ready.saturating_add(1);
        }
        EngineQueueTaskState::Retrying => {
            counts.retrying = counts.retrying.saturating_add(1);
        }
        EngineQueueTaskState::Running => {
            counts.running = counts.running.saturating_add(1);
        }
        EngineQueueTaskState::Succeeded
        | EngineQueueTaskState::Failed
        | EngineQueueTaskState::Cancelled => {}
    }
}

fn count_network_inflight(
    metadata: &TaskMetadata,
    network: &mut ProverNetworkStatus,
    now_secs: u64,
) {
    for runtime in metadata.runtime.proposals.values() {
        count_runtime_network_inflight(runtime, network, now_secs);
    }
    if let Some(runtime) = metadata.runtime.aggregate.as_ref() {
        count_runtime_network_inflight(runtime, network, now_secs);
    }
}

#[allow(clippy::missing_const_for_fn)]
fn count_runtime_network_inflight(
    runtime: &TaskRuntimeMetadata,
    network: &mut ProverNetworkStatus,
    now_secs: u64,
) {
    if runtime.has_sp1_network_submission_progress() {
        network.sp1.inflight_orders = network.sp1.inflight_orders.saturating_add(1);
    } else if runtime.has_boundless_submission_resume()
        && matches!(runtime.expires_at, Some(expires_at) if expires_at > now_secs)
    {
        network.risc0.inflight_orders = network.risc0.inflight_orders.saturating_add(1);
    }
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

async fn load_task_lookup(state: &AppState, id: &str) -> Result<TaskLookup, ApiError> {
    let record = state
        .runtime
        .get_task(id)
        .await
        .map_err(|err| ApiError::internal(format!("failed to load task: {err}")))?
        .ok_or_else(|| ApiError::not_found(format!("task not found: {id}")))?;
    let metadata = parse_task_metadata(&record)?;
    let engine = resolve_engine(state, &record.network_pair, record.pipeline_key)?;

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
        let task_id = proposal.engine_task_id(record.pipeline_key);
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
        let mut proof_location = None;
        let proof_refs = proposal_proof_artifact_refs(record.pipeline_key, proposal);
        let status = if should_probe_proof_artifact(&status, engine_state_present) {
            if let Some(material) = load_first_proof_artifact_material(
                runtime_manager,
                &record.network_pair,
                record.pipeline_key,
                record.route,
                &proof_refs,
                ProofArtifactPayload::Proposal,
            )
            .await?
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
                require_published_proof(status, &proposal.task_id)
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
            proof_uri: proof_location.and_then(|location| location.proof_uri),
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
    let status = if should_probe_proof_artifact(&status, engine_state_present) {
        if let Some(refs) = root_proof_artifact_refs(metadata, record.pipeline_key) {
            if let Some(material) = load_first_proof_artifact_material(
                runtime_manager,
                &record.network_pair,
                record.pipeline_key,
                record.route,
                &refs.refs,
                ProofArtifactPayload::Final,
            )
            .await?
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
                require_published_proof(status, &refs.refs[0])
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
            proof_uri: proof_location.and_then(|location| location.proof_uri),
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

const fn should_probe_proof_artifact(
    status: &EngineStatusView,
    engine_state_present: bool,
) -> bool {
    !engine_state_present || !matches!(status.status, ProofStatus::Pending | ProofStatus::Proving)
}

async fn load_first_proof_artifact_material(
    runtime: &RuntimeManager,
    network_pair: &str,
    pipeline_key: PipelineKey,
    route: PipelineRoute,
    proof_refs: &[String],
    expected_payload: ProofArtifactPayload,
) -> Result<Option<ProofArtifactMaterial>, ApiError> {
    for proof_ref in proof_refs {
        if let Some(material) = load_proof_artifact_material(
            runtime,
            network_pair,
            pipeline_key,
            route,
            proof_ref,
            expected_payload,
        )
        .await
        .map_err(|err| ApiError::internal(format!("failed to load proof artifact: {err}")))?
        {
            return Ok(Some(material));
        }
    }
    Ok(None)
}

fn require_published_proof(status: EngineStatusView, proof_ref: &str) -> EngineStatusView {
    if !matches!(status.status, ProofStatus::Completed) {
        return status;
    }

    EngineStatusView {
        status: ProofStatus::Failed,
        proof: None,
        error: Some(format!(
            "proof publication incomplete: artifact {proof_ref} is not readable"
        )),
        extra_data: None,
    }
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
    let mut status = match computed_root_status {
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
    let mut error = aggregate
        .and_then(|aggregate| aggregate.error.clone())
        .or_else(|| proposals.iter().find_map(|proposal| proposal.error.clone()))
        .or_else(|| failed_runtime_error(&status, runtime_error));
    let root_requires_proof = aggregate.is_some() || proposals.len() == 1;
    let root_has_readable_artifact = match aggregate {
        Some(aggregate) => aggregate.proof_ref.is_some() && aggregate.proof_uri.is_some(),
        None => proposals.first().is_some_and(|proposal| {
            proposals.len() == 1 && proposal.proof_ref.is_some() && proposal.proof_uri.is_some()
        }),
    };
    if root_requires_proof
        && matches!(status, ProofStatus::Completed)
        && proof.is_none()
        && !root_has_readable_artifact
    {
        status = ProofStatus::Failed;
        error = Some("completed task has no readable root proof artifact".to_string());
    }
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
        submitted_at: runtime.submitted_at,
        quoted_mcycles_count: runtime.quoted_mcycles_count,
        evaluated_mcycles_count: runtime.evaluated_mcycles_count,
        max_price_multiplier: runtime.max_price_multiplier,
        max_price_wei: runtime.max_price_wei,
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

fn batch_request_fingerprint(
    environment: &str,
    namespace: &str,
    submission: &CanonicalBatchSubmission,
) -> Result<String, ApiError> {
    let payload = serde_json::json!({
        "environment": environment,
        "namespace": namespace,
        "pair_key": submission.pair.key.as_str(),
        "route": submission.route.route.to_string(),
        "requested_proof_type": submission.requested_proof_type.as_str(),
        "proof_type": submission.route.proof_type().to_string(),
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
    environment: &str,
    namespace: &str,
    pair: &ResolvedNetworkPair,
    route: CanonicalProofRoute,
    prover_type: Option<ProverType>,
    req: &AggregateProofRequest,
    prover_config: &ProverTaskConfig,
) -> Result<String, ApiError> {
    let payload = serde_json::json!({
        "environment": environment,
        "namespace": namespace,
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

    let engine = resolve_engine(state, &record.network_pair, record.pipeline_key)?;
    let engine_state_present = engine
        .has_active_execution(RootOwner::new(
            record.task_id.clone(),
            record.incarnation_id,
        ))
        .await
        .map_err(|err| ApiError::internal(format!("failed to inspect execution: {err}")))?;
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

fn failed_stage_is_reenqueueable(
    record: &raiko2_runtime::RuntimeTaskRecord,
    metadata: &TaskMetadata,
) -> bool {
    if record.proof_uri.is_some() {
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
        None => !metadata.has_remote_submission_progress(),
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
    let task_id = proposal.engine_task_id(pipeline_key);
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

async fn completed_root_artifact_missing(
    runtime: &RuntimeManager,
    record: &raiko2_runtime::RuntimeTaskRecord,
    metadata: &TaskMetadata,
) -> Result<bool, ApiError> {
    if record.runner_status != RuntimeRunnerStatus::Completed {
        return Ok(false);
    }
    let Some(root_refs) = root_proof_artifact_refs(metadata, record.pipeline_key) else {
        return Ok(false);
    };
    if load_persisted_root_proof_material(runtime, record, metadata)
        .await?
        .is_some()
    {
        return Ok(false);
    }
    warn!(
        task_id = record.task_id,
        artifact_kind = ?root_refs.kind,
        proof_refs = ?root_refs.refs,
        "completed runtime task is missing proof artifact; treating it as stale"
    );
    Ok(true)
}

const fn duplicate_runner_status_label(
    runner_status: RuntimeRunnerStatus,
    missing_completed_artifact: bool,
) -> &'static str {
    if missing_completed_artifact {
        "completed_artifact_missing"
    } else {
        runner_status.as_str()
    }
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
            legacy_root_proof_material(
                state.runtime.as_ref(),
                &lookup.record,
                &lookup.metadata,
                task.proof,
            )
            .await?,
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
    runtime: &RuntimeManager,
    record: &raiko2_runtime::RuntimeTaskRecord,
    metadata: &TaskMetadata,
    fallback_proof: Option<String>,
) -> Result<Proof, ApiError> {
    if root_proof_artifact_refs(metadata, record.pipeline_key).is_some()
        && let Some(proof) = load_persisted_root_proof_material(runtime, record, metadata).await?
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

    #[test]
    fn operation_status_reports_partial_failures() {
        assert_eq!(operation_status(0), "ok");
        assert_eq!(operation_status(1), "partial_failure");
    }

    #[test]
    fn aggregate_ids_must_match_proof_count_when_present() {
        let request = AggregateProofRequest {
            aggregation_ids: vec![1, 2],
            proofs: vec![valid_native_proof()],
            proof_type: BatchProofType::Risc0,
            network: None,
            l1_network: None,
            graffiti: None,
            prover: None,
            blob_proof_type: None,
            prover_args: PublicProverArgs::default(),
        };

        let error = validate_aggregate_request_shape(&request)
            .expect_err("mismatched aggregate ids must be rejected");
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert!(error.message.contains("match proofs length"));
    }
    use crate::config::{BoundlessPairConfig, Config, ServerAclKey};
    use crate::server::state::{EngineHandle, StaticPipelineFactory};
    use anyhow::{Context, Result, anyhow};
    use axum::{
        body::Body,
        extract::{Path, State},
        http::{HeaderMap, Request},
    };
    use raiko2_engine::{EngineTaskId, EngineTaskKey};
    use raiko2_pipeline::{PipelineRoute, RunnerKind};
    use raiko2_primitives::SupportedChainSpecs;
    use raiko2_queue::TaskStoreError;
    use raiko2_runtime::{
        ProofArtifactRegistration, RunnerStatus as RuntimeRunnerStatus, RuntimeManager,
        RuntimeTaskRecord,
    };
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

    fn batch_request_fingerprint_for_test(submission: &CanonicalBatchSubmission) -> Result<String> {
        batch_request_fingerprint("test", "raiko2-test", submission)
            .map_err(|err| anyhow!(err.message))
    }

    #[test]
    fn format_proposal_ids_compacts_contiguous_ranges() {
        assert_eq!(format_proposal_ids(&[]), "none");
        assert_eq!(format_proposal_ids(&[18504]), "18504");
        assert_eq!(format_proposal_ids(&[18498, 18499, 18500]), "18498..18500");
    }

    #[test]
    fn format_proposal_ids_keeps_non_contiguous_lists_explicit() {
        assert_eq!(
            format_proposal_ids(&[18498, 18500, 18503]),
            "18498,18500,18503"
        );
    }

    #[test]
    fn duplicate_runner_status_label_marks_missing_completed_artifacts() {
        assert_eq!(
            duplicate_runner_status_label(RuntimeRunnerStatus::Completed, true),
            "completed_artifact_missing"
        );
        assert_eq!(
            duplicate_runner_status_label(RuntimeRunnerStatus::Completed, false),
            "completed"
        );
        assert_eq!(
            duplicate_runner_status_label(RuntimeRunnerStatus::Failed, false),
            "failed"
        );
    }

    struct NoopEngine;

    impl EngineHandle for NoopEngine {
        fn get_status(
            &self,
            _id: EngineTaskId,
        ) -> BoxFuture<'_, Result<Option<EngineStatusView>, TaskStoreError>> {
            Box::pin(async { Ok(None) })
        }

        fn get_task_state(
            &self,
            _id: EngineTaskId,
        ) -> BoxFuture<'_, Result<Option<EngineQueueTaskView>, TaskStoreError>> {
            Box::pin(async { Ok(None) })
        }

        fn has_active_execution(
            &self,
            _owner: raiko2_queue::RootOwner,
        ) -> BoxFuture<'_, Result<bool, TaskStoreError>> {
            Box::pin(async { Ok(false) })
        }

        fn attach_execution_plan(
            &self,
            _owner: raiko2_queue::RootOwner,
            _plan: EngineExecutionPlan,
        ) -> BoxFuture<'_, Result<raiko2_queue::AttachOutcome, TaskStoreError>> {
            Box::pin(async { panic!("unexpected execution attachment") })
        }

        fn detach_execution(
            &self,
            _owner: raiko2_queue::RootOwner,
            mode: raiko2_queue::DetachMode,
        ) -> BoxFuture<'_, Result<raiko2_queue::DetachOutcome<EngineTaskKey>, TaskStoreError>>
        {
            Box::pin(async move { Ok(raiko2_queue::DetachOutcome::not_attached(mode)) })
        }
    }

    struct CancelFailEngine;

    impl EngineHandle for CancelFailEngine {
        fn get_status(
            &self,
            _id: EngineTaskId,
        ) -> BoxFuture<'_, Result<Option<EngineStatusView>, TaskStoreError>> {
            Box::pin(async { Ok(None) })
        }

        fn get_task_state(
            &self,
            _id: EngineTaskId,
        ) -> BoxFuture<'_, Result<Option<EngineQueueTaskView>, TaskStoreError>> {
            Box::pin(async { Ok(None) })
        }

        fn has_active_execution(
            &self,
            _owner: raiko2_queue::RootOwner,
        ) -> BoxFuture<'_, Result<bool, TaskStoreError>> {
            Box::pin(async { Ok(false) })
        }

        fn attach_execution_plan(
            &self,
            _owner: raiko2_queue::RootOwner,
            _plan: EngineExecutionPlan,
        ) -> BoxFuture<'_, Result<raiko2_queue::AttachOutcome, TaskStoreError>> {
            Box::pin(async { panic!("unexpected execution attachment") })
        }

        fn detach_execution(
            &self,
            _owner: raiko2_queue::RootOwner,
            _mode: raiko2_queue::DetachMode,
        ) -> BoxFuture<'_, Result<raiko2_queue::DetachOutcome<EngineTaskKey>, TaskStoreError>>
        {
            Box::pin(async {
                Err(TaskStoreError::backend(std::io::Error::other(
                    "detach failed",
                )))
            })
        }
    }

    struct ListingEngine {
        views: Vec<(EngineTaskId, EngineQueueTaskState)>,
        active_owners: HashSet<raiko2_queue::RootOwner>,
        list_calls: AtomicUsize,
    }

    impl ListingEngine {
        fn new(views: Vec<(EngineTaskId, EngineQueueTaskState)>) -> Self {
            Self {
                views,
                active_owners: HashSet::new(),
                list_calls: AtomicUsize::new(0),
            }
        }

        fn with_active_owners(
            views: Vec<(EngineTaskId, EngineQueueTaskState)>,
            active_owners: HashSet<raiko2_queue::RootOwner>,
        ) -> Self {
            Self {
                views,
                active_owners,
                list_calls: AtomicUsize::new(0),
            }
        }
    }

    impl EngineHandle for ListingEngine {
        fn get_status(
            &self,
            _id: EngineTaskId,
        ) -> BoxFuture<'_, Result<Option<EngineStatusView>, TaskStoreError>> {
            Box::pin(async { Ok(None) })
        }

        fn get_task_state(
            &self,
            id: EngineTaskId,
        ) -> BoxFuture<'_, Result<Option<EngineQueueTaskView>, TaskStoreError>> {
            self.list_calls.fetch_add(1, Ordering::Relaxed);
            let view = self
                .views
                .iter()
                .find(|(view_id, _)| *view_id == id)
                .map(|(_, state)| EngineQueueTaskView { state: *state });
            Box::pin(async move { Ok(view) })
        }

        fn has_active_execution(
            &self,
            owner: raiko2_queue::RootOwner,
        ) -> BoxFuture<'_, Result<bool, TaskStoreError>> {
            let active = self.active_owners.contains(&owner);
            Box::pin(async move { Ok(active) })
        }

        fn attach_execution_plan(
            &self,
            _owner: raiko2_queue::RootOwner,
            _plan: EngineExecutionPlan,
        ) -> BoxFuture<'_, Result<raiko2_queue::AttachOutcome, TaskStoreError>> {
            Box::pin(async { panic!("unexpected execution attachment") })
        }

        fn detach_execution(
            &self,
            _owner: raiko2_queue::RootOwner,
            mode: raiko2_queue::DetachMode,
        ) -> BoxFuture<'_, Result<raiko2_queue::DetachOutcome<EngineTaskKey>, TaskStoreError>>
        {
            Box::pin(async move { Ok(raiko2_queue::DetachOutcome::not_attached(mode)) })
        }
    }

    struct RecordingEngine {
        runtime_on_attach: Option<Arc<RuntimeManager>>,
        proposals: Mutex<Vec<(ProposalTaskRequest, Vec<EngineTaskId>)>>,
        aggregate_inputs: Mutex<Vec<AggregateProofInput>>,
        attached: Mutex<Vec<raiko2_queue::RootOwner>>,
        detached: Mutex<Vec<raiko2_queue::RootOwner>>,
    }

    impl RecordingEngine {
        const fn new() -> Self {
            Self {
                runtime_on_attach: None,
                proposals: Mutex::new(Vec::new()),
                aggregate_inputs: Mutex::new(Vec::new()),
                attached: Mutex::new(Vec::new()),
                detached: Mutex::new(Vec::new()),
            }
        }

        fn progressing(runtime: Arc<RuntimeManager>) -> Self {
            Self {
                runtime_on_attach: Some(runtime),
                ..Self::new()
            }
        }
    }

    impl EngineHandle for RecordingEngine {
        fn attach_execution_plan(
            &self,
            owner: raiko2_queue::RootOwner,
            plan: EngineExecutionPlan,
        ) -> BoxFuture<'_, Result<raiko2_queue::AttachOutcome, TaskStoreError>> {
            Box::pin(async move {
                if let Some(runtime) = &self.runtime_on_attach {
                    let task_id = owner.task_id.clone();
                    let incarnation_id = owner.incarnation_id;
                    runtime
                        .update_tasks_by_ref(&task_id, |records| {
                            let record = records
                                .iter_mut()
                                .find(|record| record.incarnation_id == incarnation_id)
                                .ok_or_else(|| {
                                    anyhow!("attached root incarnation is no longer current")
                                })?;
                            record.runner_status = RuntimeRunnerStatus::Running;
                            record.error = None;
                            Ok(())
                        })
                        .await
                        .map_err(|error| {
                            TaskStoreError::backend(std::io::Error::other(error.to_string()))
                        })?;
                }
                self.attached
                    .lock()
                    .expect("attached owners mutex")
                    .push(owner);
                let mut proposals = self.proposals.lock().expect("proposal submissions mutex");
                for request in plan.proposals {
                    proposals.push((request, Vec::new()));
                }
                drop(proposals);
                if let Some(aggregate) = plan.aggregate {
                    *self
                        .aggregate_inputs
                        .lock()
                        .expect("aggregate inputs mutex") = aggregate.inputs;
                }
                Ok(raiko2_queue::AttachOutcome::Attached)
            })
        }

        fn get_status(
            &self,
            _id: EngineTaskId,
        ) -> BoxFuture<'_, Result<Option<EngineStatusView>, TaskStoreError>> {
            Box::pin(async { Ok(None) })
        }

        fn get_task_state(
            &self,
            _id: EngineTaskId,
        ) -> BoxFuture<'_, Result<Option<EngineQueueTaskView>, TaskStoreError>> {
            Box::pin(async { Ok(None) })
        }

        fn has_active_execution(
            &self,
            owner: raiko2_queue::RootOwner,
        ) -> BoxFuture<'_, Result<bool, TaskStoreError>> {
            let active = self
                .attached
                .lock()
                .expect("attached owners mutex")
                .contains(&owner)
                && !self
                    .detached
                    .lock()
                    .expect("detached owners mutex")
                    .contains(&owner);
            Box::pin(async move { Ok(active) })
        }

        fn detach_execution(
            &self,
            owner: raiko2_queue::RootOwner,
            mode: raiko2_queue::DetachMode,
        ) -> BoxFuture<'_, Result<raiko2_queue::DetachOutcome<EngineTaskKey>, TaskStoreError>>
        {
            Box::pin(async move {
                self.detached
                    .lock()
                    .expect("detached owners mutex")
                    .push(owner);
                Ok(raiko2_queue::DetachOutcome::not_attached(mode))
            })
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
            l2_provider: raiko2_provider::L2ProviderKind::Reth,
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
            requested_proof_type: BatchProofType::Risc0,
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

    fn native_local_route() -> CanonicalProofRoute {
        CanonicalProofRoute::new(
            PipelineRoute::new(crate::config::GuestSystem::Native, RunnerKind::Local),
            PipelineKey::ShastaNative,
        )
    }

    fn sgx_remote_route() -> CanonicalProofRoute {
        CanonicalProofRoute::new(
            PipelineRoute::new(crate::config::GuestSystem::Sgx, RunnerKind::Remote),
            PipelineKey::ShastaSgx,
        )
    }

    fn sgxgeth_remote_route() -> CanonicalProofRoute {
        CanonicalProofRoute::new(
            PipelineRoute::new(crate::config::GuestSystem::SgxGeth, RunnerKind::Remote),
            PipelineKey::ShastaSgxGeth,
        )
    }

    fn legacy_sgxgeth_record(
        submission: &CanonicalBatchSubmission,
        runner_status: RuntimeRunnerStatus,
    ) -> Result<(TaskMetadata, RuntimeTaskRecord)> {
        let request_fingerprint = batch_request_fingerprint_for_test(submission)?;
        let plan = build_submission_plan(submission, &request_fingerprint)
            .map_err(|err| anyhow!(err.message))?;
        let mut metadata = build_task_metadata(
            &submission.pair,
            BuildTaskMetadataParams {
                network: &submission.pair.network,
                l1_network: &submission.pair.l1_network,
                proof_type: submission.route.proof_type(),
                requested_proof_type: Some(submission.requested_proof_type.as_str()),
                prover_type: submission.prover_type,
                execution_mode: submission.execution_mode,
                aggregate_requested: false,
            },
            &plan.proposals,
            plan.aggregate.as_ref(),
        );
        let mut record = runtime_record(runner_status, &metadata);
        record.task_id.clone_from(&submission.public_task_id);
        align_runtime_record_identity(
            &mut record,
            &mut metadata,
            PipelineKey::ShastaSgxGeth,
            PipelineKey::ShastaSgxGeth.route(),
        );
        record.route = "sgx/remote".parse().expect("legacy SGXGETH route");
        record.request_fingerprint = request_fingerprint;
        Ok((metadata, record))
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
        write_test_proof_artifact_for_route(
            runtime,
            network_pair,
            PipelineKey::ShastaNative,
            PipelineKey::ShastaNative.route(),
            proof_ref,
            proof,
        )
        .await
    }

    async fn write_test_proof_artifact_for_route(
        runtime: &RuntimeManager,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        proof: &Proof,
    ) -> Result<()> {
        let publication = runtime
            .publish_proof_artifact_bytes(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
                &serde_json::to_vec(proof)?,
            )
            .await?;
        let artifact = publication
            .try_object()
            .expect("proof publication should materialize content");
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: network_pair.to_string(),
                proof_ref: proof_ref.to_string(),
                pipeline_key,
                route,
                proof_uri: artifact.proof_uri.clone(),
                content_hash: artifact.content_hash.clone(),
                generation: artifact.generation,
            })
            .await?;
        Ok(())
    }

    fn test_state(runtime: Arc<RuntimeManager>, engine: Arc<dyn EngineHandle>) -> AppState {
        let config = Config::default();
        let mut factory = StaticPipelineFactory::default();
        factory.insert("taiko_dev/ethereum", PipelineKey::ShastaNative, engine);
        AppState::from_parts(Arc::new(config), Arc::new(factory), runtime)
    }

    fn test_state_with_engines(
        runtime: Arc<RuntimeManager>,
        engines: impl IntoIterator<Item = (PipelineKey, Arc<dyn EngineHandle>)>,
    ) -> AppState {
        let config = Config::default();
        let mut factory = StaticPipelineFactory::default();
        for (pipeline_key, engine) in engines {
            factory.insert("taiko_dev/ethereum", pipeline_key, engine);
        }
        AppState::from_parts(Arc::new(config), Arc::new(factory), runtime)
    }

    fn runtime_record(
        runner_status: RuntimeRunnerStatus,
        metadata: &TaskMetadata,
    ) -> RuntimeTaskRecord {
        let pipeline_key = PipelineKey::ShastaRisc0Network;
        let mut metadata = metadata.clone();
        canonicalize_test_metadata(&mut metadata, pipeline_key);
        let artifact_refs = publication_proof_artifact_refs(&metadata, pipeline_key);
        RuntimeTaskRecord {
            task_id: "task_public".to_string(),
            incarnation_id: uuid::Uuid::new_v4(),
            pipeline_key,
            route: pipeline_key.route(),
            task_kind: "hoodi_batch".to_string(),
            network_pair: metadata.network_pair.clone(),
            artifact_refs,
            runner_status,
            image_ref: None,
            proof_uri: None,
            error: None,
            metadata: serde_json::to_value(&metadata).expect("serialize metadata"),
            request_fingerprint: "0xfingerprint".to_string(),
            updated_at: 1,
        }
    }

    fn align_runtime_record_identity(
        record: &mut RuntimeTaskRecord,
        metadata: &mut TaskMetadata,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
    ) {
        canonicalize_test_metadata(metadata, pipeline_key);
        assert!(pipeline_key.supports_route(route));
        assert_eq!(metadata.proof_type, pipeline_key.proof_type());
        record.pipeline_key = pipeline_key;
        record.route = route;
        record.network_pair.clone_from(&metadata.network_pair);
        record.artifact_refs = publication_proof_artifact_refs(metadata, pipeline_key);
        record.metadata = serde_json::to_value(metadata).expect("serialize task metadata");
    }

    fn canonicalize_test_metadata(metadata: &mut TaskMetadata, pipeline_key: PipelineKey) {
        for proposal in &mut metadata.proposals {
            let request = &proposal.request;
            let previous_task_id = proposal.task_id.clone();
            proposal.proposal_id = request.proposal_id;
            proposal.checkpoint = request.checkpoint;
            proposal.l1_inclusion_block_number = request.l1_inclusion_block_number;
            proposal.last_anchor_block_number = request.last_anchor_block_number;
            if let Some(range) = request.l2_block_range {
                proposal.l2_block_numbers = (range.start..=range.end).collect();
            }
            proposal.task_id = proposal_task_ref(pipeline_key, request);
            if previous_task_id != proposal.task_id
                && let Some(runtime) = metadata.runtime.proposals.remove(&previous_task_id)
            {
                metadata
                    .runtime
                    .proposals
                    .insert(proposal.task_id.clone(), runtime);
            }
        }
        metadata.aggregate_requested = metadata.aggregate_request.is_some();
        metadata.aggregate_task_id = metadata
            .aggregate_request
            .as_ref()
            .map(|request| aggregate_task_ref(pipeline_key, request));
    }

    fn task_metadata_with_stage(stage: Option<&str>) -> TaskMetadata {
        let request = test_proposal_request(42);
        TaskMetadata {
            network_pair: "taiko_dev/ethereum".to_string(),
            network: "taiko_dev".to_string(),
            l1_network: "ethereum".to_string(),
            proof_type: ProofType::Risc0,
            requested_proof_type: None,
            prover_type: None,
            execution_mode: None,
            aggregate_requested: false,
            proposals: vec![ProposalTask {
                proposal_id: 42,
                checkpoint: None,
                l1_inclusion_block_number: 1,
                l2_block_numbers: vec![42],
                last_anchor_block_number: 41,
                task_id: proposal_task_ref(PipelineKey::ShastaRisc0, &request),
                request,
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

    fn zk_any_metadata(stage: Option<&str>) -> TaskMetadata {
        let mut metadata = task_metadata_with_stage(stage);
        metadata.requested_proof_type = Some("zk_any".to_string());
        metadata
    }

    fn test_proposal_request(proposal_id: u64) -> ProposalTaskRequest {
        ProposalTaskRequest {
            proposal_id,
            l2_block_range: Some(L2BlockRange {
                start: proposal_id,
                end: proposal_id,
            }),
            l1_inclusion_block_number: 1,
            last_anchor_block_number: proposal_id.saturating_sub(1),
            checkpoint: None,
            blob_proof_type: None,
            prover: None,
            graffiti: None,
            prover_config: ProverTaskConfig::default(),
        }
    }

    fn set_test_proposal_runtime(metadata: &mut TaskMetadata, runtime: TaskRuntimeMetadata) {
        let task_id = metadata.proposals[0].task_id.clone();
        metadata.runtime.proposals.insert(task_id, runtime);
    }

    fn test_boundless_runtime(
        provider_request_id: &str,
        submitted_at: u64,
        lock_expires_at: u64,
        expires_at: u64,
    ) -> TaskRuntimeMetadata {
        TaskRuntimeMetadata {
            provider_request_id: Some(provider_request_id.to_string()),
            image_ref: Some("0ximage".to_string()),
            deployment: Some("base".to_string()),
            offchain: Some(false),
            expires_at: Some(expires_at),
            lock_expires_at: Some(lock_expires_at),
            submitted_at: Some(submitted_at),
            quoted_mcycles_count: Some(1),
            evaluated_mcycles_count: Some(1),
            max_price_multiplier: Some(1),
            max_price_wei: Some("1000".to_string()),
            rebid_attempt: Some(1),
            ..TaskRuntimeMetadata::default()
        }
    }

    fn test_sp1_runtime(
        provider_request_id: &str,
        submitted_at: u64,
        timeout_secs: u64,
    ) -> TaskRuntimeMetadata {
        TaskRuntimeMetadata {
            provider_request_id: Some(provider_request_id.to_string()),
            expires_at: Some(submitted_at.saturating_add(timeout_secs)),
            submitted_at: Some(submitted_at),
            rebid_attempt: Some(1),
            sp1_network_mode: Some(raiko2_prover::Sp1NetworkMode::Reserved),
            sp1_fulfillment_strategy: Some(raiko2_prover::Sp1FulfillmentStrategy::Reserved),
            sp1_skip_simulation: Some(false),
            sp1_cycle_limit: Some(1_000_000),
            sp1_timeout_secs: Some(timeout_secs),
            ..TaskRuntimeMetadata::default()
        }
    }

    fn configure_test_aggregate(metadata: &mut TaskMetadata) {
        let request = AggregationTaskRequest {
            request_id: "aggregate-request".to_string(),
            proposal_ids: metadata
                .proposals
                .iter()
                .map(|proposal| proposal.proposal_id)
                .collect(),
            prover_config: ProverTaskConfig::default(),
        };
        metadata.aggregate_requested = true;
        metadata.aggregate_task_id = Some(aggregate_task_ref(
            PipelineKey::ShastaRisc0Network,
            &request,
        ));
        metadata.aggregate_request = Some(request);
    }

    #[test]
    fn network_inflight_counts_each_remote_checkpoint_for_exactly_one_backend() {
        let mut network = ProverNetworkStatus::default();
        count_runtime_network_inflight(&test_sp1_runtime("sp1-request", 1, 100), &mut network, 10);

        assert_eq!(network.sp1.inflight_orders, 1);
        assert_eq!(network.risc0.inflight_orders, 0);

        count_runtime_network_inflight(
            &test_boundless_runtime("0x1", 1, 50, 100),
            &mut network,
            10,
        );

        assert_eq!(network.sp1.inflight_orders, 1);
        assert_eq!(network.risc0.inflight_orders, 1);
    }

    async fn upsert_test_record(
        runtime: &RuntimeManager,
        task_id: &str,
        runner_status: RuntimeRunnerStatus,
        metadata: &TaskMetadata,
        pipeline_key: PipelineKey,
    ) -> Result<()> {
        let mut record = runtime_record(runner_status, metadata);
        record.task_id = task_id.to_string();
        let mut canonical_metadata = metadata.clone();
        align_runtime_record_identity(
            &mut record,
            &mut canonical_metadata,
            pipeline_key,
            pipeline_key.route(),
        );
        record.request_fingerprint = format!("fingerprint-{task_id}");
        runtime.upsert_task(&record).await?;
        Ok(())
    }

    async fn upsert_invalid_metadata_record(runtime: &RuntimeManager, task_id: &str) -> Result<()> {
        let mut record = runtime_record(
            RuntimeRunnerStatus::Running,
            &zk_any_metadata(Some("prove")),
        );
        record.task_id = task_id.to_string();
        record.request_fingerprint = format!("fingerprint-{task_id}");
        record.metadata = serde_json::json!({ "invalid": true });
        runtime.upsert_task(&record).await?;
        Ok(())
    }

    fn test_state_with_acl(
        runtime: Arc<RuntimeManager>,
        engines: impl IntoIterator<Item = (PipelineKey, Arc<dyn EngineHandle>)>,
    ) -> AppState {
        let mut config = Config::default();
        config.server.acl.keys.push(ServerAclKey {
            id: "ops".to_string(),
            key: "secret".to_string(),
            allow: vec![ServerAclFeature::ProverClear],
            rate_limit_per_minute: None,
        });
        let mut factory = StaticPipelineFactory::default();
        for (pipeline_key, engine) in engines {
            factory.insert("taiko_dev/ethereum", pipeline_key, engine);
        }
        AppState::from_parts(Arc::new(config), Arc::new(factory), runtime)
    }

    #[tokio::test]
    async fn prover_status_skips_invalid_metadata_and_missing_pipeline() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "prover-status-skips",
        ))?);
        let metadata = zk_any_metadata(Some("prove"));
        let mut missing_pipeline_metadata = metadata.clone();
        missing_pipeline_metadata.network_pair = "missing/ethereum".to_string();
        missing_pipeline_metadata.network = "missing".to_string();
        upsert_invalid_metadata_record(&runtime, "bad-metadata").await?;
        upsert_test_record(
            &runtime,
            "missing-pipeline",
            RuntimeRunnerStatus::Running,
            &missing_pipeline_metadata,
            PipelineKey::ShastaRisc0Network,
        )
        .await?;

        let state = test_state_with_engines(runtime, Vec::new());
        let (tasks, network, skipped) = collect_prover_status(&state, ProverTaskScope::ZkAny)
            .await
            .map_err(|err| anyhow!(err.message))?;

        assert!(tasks.is_clean());
        assert!(network.is_clean());
        assert_eq!(skipped.invalid_metadata, 1);
        assert_eq!(skipped.unavailable_pipeline, 1);
        assert!(!skipped.is_clean());
        Ok(())
    }

    #[tokio::test]
    async fn prover_status_ignores_expired_boundless_inflight_order() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "prover-status-expired-boundless",
        ))?);
        let mut metadata = zk_any_metadata(Some("prove"));
        let expires_at = unix_now_secs().saturating_sub(1);
        set_test_proposal_runtime(
            &mut metadata,
            test_boundless_runtime(
                "0xexpired",
                expires_at.saturating_sub(2),
                expires_at.saturating_sub(1),
                expires_at,
            ),
        );
        upsert_test_record(
            &runtime,
            "expired-boundless",
            RuntimeRunnerStatus::Running,
            &metadata,
            PipelineKey::ShastaRisc0Network,
        )
        .await?;

        let state = test_state_with_engines(
            runtime,
            [(
                PipelineKey::ShastaRisc0Network,
                Arc::new(NoopEngine) as Arc<dyn EngineHandle>,
            )],
        );
        let (_, network, skipped) = collect_prover_status(&state, ProverTaskScope::ZkAny)
            .await
            .map_err(|err| anyhow!(err.message))?;

        assert_eq!(network.risc0.inflight_orders, 0);
        assert!(skipped.is_clean());
        Ok(())
    }

    #[tokio::test]
    async fn prover_status_counts_missing_queue_task_as_orphaned() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "prover-status-orphaned",
        ))?);
        let request = test_proposal_request(42);
        let mut metadata = zk_any_metadata(Some("prove"));
        metadata.proposals[0].request = request;
        upsert_test_record(
            &runtime,
            "orphaned-root",
            RuntimeRunnerStatus::Running,
            &metadata,
            PipelineKey::ShastaRisc0Network,
        )
        .await?;

        let state = test_state_with_engines(
            runtime,
            [(
                PipelineKey::ShastaRisc0Network,
                Arc::new(NoopEngine) as Arc<dyn EngineHandle>,
            )],
        );
        let (tasks, network, skipped) = collect_prover_status(&state, ProverTaskScope::ZkAny)
            .await
            .map_err(|err| anyhow!(err.message))?;

        assert_eq!(tasks.pending, 0);
        assert_eq!(tasks.orphaned, 1);
        assert!(network.is_clean());
        assert!(skipped.is_clean());
        Ok(())
    }

    #[tokio::test]
    async fn prover_status_counts_terminal_queue_task_as_orphaned() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "prover-status-terminal-queue",
        ))?);
        let request = test_proposal_request(42);
        let mut metadata = zk_any_metadata(Some("prove"));
        metadata.proposals[0].request = request.clone();
        let task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaRisc0Network,
            request,
        });
        upsert_test_record(
            &runtime,
            "root",
            RuntimeRunnerStatus::Running,
            &metadata,
            PipelineKey::ShastaRisc0Network,
        )
        .await?;

        let engine = Arc::new(ListingEngine::new(vec![(
            task_id,
            EngineQueueTaskState::Succeeded,
        )]));
        let state = test_state_with_engines(
            runtime,
            [(
                PipelineKey::ShastaRisc0Network,
                Arc::clone(&engine) as Arc<dyn EngineHandle>,
            )],
        );
        let (tasks, network, skipped) = collect_prover_status(&state, ProverTaskScope::ZkAny)
            .await
            .map_err(|err| anyhow!(err.message))?;

        assert_eq!(tasks.orphaned, 1);
        assert_eq!(tasks.pending, 0);
        assert_eq!(tasks.ready, 0);
        assert_eq!(tasks.retrying, 0);
        assert_eq!(tasks.running, 0);
        assert!(network.is_clean());
        assert!(skipped.is_clean());
        Ok(())
    }

    #[tokio::test]
    async fn prover_status_lists_queue_once_per_engine_and_counts_unique_tasks() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "prover-status-list-once",
        ))?);
        let request = test_proposal_request(42);
        let mut metadata = zk_any_metadata(Some("prove"));
        metadata.proposals[0].request = request.clone();
        let task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaRisc0Network,
            request,
        });
        upsert_test_record(
            &runtime,
            "root-a",
            RuntimeRunnerStatus::Running,
            &metadata,
            PipelineKey::ShastaRisc0Network,
        )
        .await?;
        upsert_test_record(
            &runtime,
            "root-b",
            RuntimeRunnerStatus::Running,
            &metadata,
            PipelineKey::ShastaRisc0Network,
        )
        .await?;

        let active_owner = runtime
            .get_task("root-a")
            .await?
            .map(|record| RootOwner::new(record.task_id, record.incarnation_id))
            .expect("root-a");

        let engine = Arc::new(ListingEngine::with_active_owners(
            vec![(task_id, EngineQueueTaskState::Running)],
            HashSet::from([active_owner]),
        ));
        let state = test_state_with_engines(
            runtime,
            [(
                PipelineKey::ShastaRisc0Network,
                Arc::clone(&engine) as Arc<dyn EngineHandle>,
            )],
        );
        let (tasks, network, skipped) = collect_prover_status(&state, ProverTaskScope::ZkAny)
            .await
            .map_err(|err| anyhow!(err.message))?;

        assert_eq!(engine.list_calls.load(Ordering::Relaxed), 1);
        assert_eq!(tasks.running, 1);
        assert_eq!(tasks.orphaned, 1);
        assert!(network.is_clean());
        assert!(skipped.is_clean());
        Ok(())
    }

    #[tokio::test]
    async fn clear_prover_reports_skipped_records_and_keeps_clearing() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "clear-prover-skips",
        ))?);
        let metadata = zk_any_metadata(Some("prove"));
        let mut missing_pipeline_metadata = metadata.clone();
        missing_pipeline_metadata.network_pair = "missing/ethereum".to_string();
        missing_pipeline_metadata.network = "missing".to_string();
        upsert_invalid_metadata_record(&runtime, "bad-metadata").await?;
        upsert_test_record(
            &runtime,
            "missing-pipeline",
            RuntimeRunnerStatus::Running,
            &missing_pipeline_metadata,
            PipelineKey::ShastaRisc0Network,
        )
        .await?;
        upsert_test_record(
            &runtime,
            "clearable",
            RuntimeRunnerStatus::Running,
            &metadata,
            PipelineKey::ShastaRisc0Network,
        )
        .await?;
        let mut remote_metadata = metadata.clone();
        let now = unix_now_secs();
        set_test_proposal_runtime(
            &mut remote_metadata,
            test_boundless_runtime(
                "0xremote",
                now,
                now.saturating_add(300),
                now.saturating_add(600),
            ),
        );
        upsert_test_record(
            &runtime,
            "remote-progress",
            RuntimeRunnerStatus::Running,
            &remote_metadata,
            PipelineKey::ShastaRisc0Network,
        )
        .await?;

        let state = test_state_with_acl(
            Arc::clone(&runtime),
            [(
                PipelineKey::ShastaRisc0Network,
                Arc::new(NoopEngine) as Arc<dyn EngineHandle>,
            )],
        );
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", axum::http::HeaderValue::from_static("secret"));

        let Json(status) = v3::clear_prover(State(state), headers)
            .await
            .expect("clear prover should not fail on skipped records");

        assert_eq!(status.cancelled, 2);
        assert_eq!(status.failed, 0);
        assert_eq!(status.skipped.invalid_metadata, 1);
        assert_eq!(status.skipped.unavailable_pipeline, 1);
        assert_eq!(status.skipped.remote_progress, 1);
        let missing_pipeline = runtime
            .get_task("missing-pipeline")
            .await?
            .expect("missing pipeline record still present");
        assert!(matches!(
            missing_pipeline.runner_status,
            RuntimeRunnerStatus::Cancelled
        ));
        let cleared = runtime
            .get_task("clearable")
            .await?
            .expect("clearable record still present");
        assert!(matches!(
            cleared.runner_status,
            RuntimeRunnerStatus::Cancelled
        ));
        let remote = runtime
            .get_task("remote-progress")
            .await?
            .expect("remote progress record still present");
        assert!(matches!(remote.runner_status, RuntimeRunnerStatus::Running));
        Ok(())
    }

    #[tokio::test]
    async fn clear_prover_keeps_runtime_cancelled_when_queue_cancel_fails() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "clear-prover-queue-fail",
        ))?);
        let request = test_proposal_request(42);
        let mut metadata = zk_any_metadata(Some("prove"));
        metadata.proposals[0].request = request;
        upsert_test_record(
            &runtime,
            "clearable",
            RuntimeRunnerStatus::Running,
            &metadata,
            PipelineKey::ShastaRisc0Network,
        )
        .await?;

        let state = test_state_with_acl(
            Arc::clone(&runtime),
            [(
                PipelineKey::ShastaRisc0Network,
                Arc::new(CancelFailEngine) as Arc<dyn EngineHandle>,
            )],
        );
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", axum::http::HeaderValue::from_static("secret"));

        let Json(status) = v3::clear_prover(State(state), headers)
            .await
            .expect("clear prover should preserve runtime cancellation");

        assert_eq!(status.cancelled, 1);
        assert_eq!(status.failed, 1);
        let cleared = runtime
            .get_task("clearable")
            .await?
            .expect("clearable record still present");
        assert!(matches!(
            cleared.runner_status,
            RuntimeRunnerStatus::Cancelled
        ));
        Ok(())
    }

    #[tokio::test]
    async fn clear_prover_rejects_oversized_api_key() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "oversized-api-key",
        ))?);
        let state = test_state_with_acl(Arc::clone(&runtime), []);
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-api-key",
            axum::http::HeaderValue::from_static("secret-with-extra-bytes"),
        );

        let Err(err) = v3::clear_prover(State(state), headers).await else {
            panic!("oversized API key should fail");
        };
        assert_eq!(err.status, StatusCode::UNAUTHORIZED);
        assert_eq!(err.message, "invalid API key");
        Ok(())
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
        set_test_proposal_runtime(
            &mut metadata,
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
        set_test_proposal_runtime(
            &mut metadata,
            test_boundless_runtime("0x1234", 123_000, 123_400, 123_456),
        );
        let record = runtime_record(RuntimeRunnerStatus::Failed, &metadata);
        let metadata = TaskMetadata::decode_for_record(&record).expect("canonical Boundless root");

        assert!(should_reenqueue_existing_submission_without_engine(
            &record, &metadata
        ));
    }

    #[test]
    fn failed_submission_with_incomplete_boundless_metadata_is_not_reenqueueable() {
        let mut metadata = task_metadata_with_stage(Some("prove"));
        set_test_proposal_runtime(
            &mut metadata,
            TaskRuntimeMetadata {
                provider_request_id: Some("0x1234".to_string()),
                expires_at: Some(123_456),
                ..TaskRuntimeMetadata::default()
            },
        );
        let record = runtime_record(RuntimeRunnerStatus::Failed, &metadata);
        assert!(!should_reenqueue_existing_submission_without_engine(
            &record, &metadata
        ));
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
        set_test_proposal_runtime(&mut metadata, test_sp1_runtime("0xsp1", 1, 7_200));
        let mut record = runtime_record(RuntimeRunnerStatus::Failed, &metadata);
        align_runtime_record_identity(
            &mut record,
            &mut metadata,
            PipelineKey::ShastaSp1,
            "sp1/network".parse().expect("SP1 network route"),
        );
        let metadata = TaskMetadata::decode_for_record(&record).expect("canonical SP1 root");

        assert!(should_reenqueue_existing_submission_without_engine(
            &record, &metadata
        ));
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

    #[tokio::test]
    async fn another_owner_of_a_shared_task_does_not_block_reenqueue() -> Result<()> {
        let metadata = task_metadata_with_stage(Some("prove"));
        let record = runtime_record(RuntimeRunnerStatus::Running, &metadata);
        let task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: record.pipeline_key,
            request: metadata.proposals[0].request.clone(),
        });
        let queue_views = vec![(task_id, EngineQueueTaskState::Running)];
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "shared-task-reenqueue-owner",
        ))?);
        let other_owner = RootOwner::new("other-root", uuid::Uuid::new_v4());
        let state = test_state_with_engines(
            Arc::clone(&runtime),
            [(
                record.pipeline_key,
                Arc::new(ListingEngine::with_active_owners(
                    queue_views.clone(),
                    HashSet::from([other_owner]),
                )) as Arc<dyn EngineHandle>,
            )],
        );

        assert!(
            should_reenqueue_existing_submission(&state, &record, &metadata)
                .await
                .map_err(|error| anyhow!(error.message))?
        );

        let exact_owner = RootOwner::new(record.task_id.clone(), record.incarnation_id);
        let state = test_state_with_engines(
            runtime,
            [(
                record.pipeline_key,
                Arc::new(ListingEngine::with_active_owners(
                    queue_views,
                    HashSet::from([exact_owner]),
                )) as Arc<dyn EngineHandle>,
            )],
        );
        assert!(
            !should_reenqueue_existing_submission(&state, &record, &metadata)
                .await
                .map_err(|error| anyhow!(error.message))?
        );
        Ok(())
    }

    #[test]
    fn running_submission_with_remote_progress_is_not_reenqueueable() {
        let mut metadata = task_metadata_with_stage(Some("prove"));
        set_test_proposal_runtime(
            &mut metadata,
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
    async fn prover_status_skips_terminal_network_inflight_roots() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "terminal-network-status",
        ))?);
        let mut metadata = task_metadata_with_stage(Some("prove"));
        metadata.requested_proof_type = Some("zk_any".to_string());
        set_test_proposal_runtime(
            &mut metadata,
            test_boundless_runtime("0x1234", 123_000, 123_400, 123_456),
        );
        let record = runtime_record(RuntimeRunnerStatus::Cancelled, &metadata);
        runtime.upsert_task(&record).await?;
        let state = test_state(Arc::clone(&runtime), Arc::new(NoopEngine));

        let (tasks, network, skipped) = collect_prover_status(&state, ProverTaskScope::ZkAny)
            .await
            .map_err(|err| anyhow!(err.message))?;

        assert!(tasks.is_clean());
        assert!(network.is_clean());
        assert!(skipped.is_clean());
        assert_eq!(network.risc0.inflight_orders, 0);
        assert_eq!(network.sp1.inflight_orders, 0);

        Ok(())
    }

    #[tokio::test]
    async fn stale_recovery_returns_cached_artifact_before_reenqueue() -> Result<()> {
        let route = native_local_route();
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "stale-cache-before-reenqueue",
        ))?);
        let submission = canonical_submission(route, false);
        let mut metadata = task_metadata_with_stage(Some("prove"));
        metadata.proof_type = ProofType::Native;
        metadata.aggregate_requested = false;
        let request = proposal_task_request(
            &submission.proposals[0],
            submission.blob_proof_type.clone(),
            submission.prover.clone(),
            submission.graffiti.clone(),
            submission.prover_config.clone(),
        );
        let proof_ref = proposal_task_ref(PipelineKey::ShastaNative, &request);
        metadata.proposals[0].request = request;
        let mut record = runtime_record(RuntimeRunnerStatus::Running, &metadata);
        align_runtime_record_identity(
            &mut record,
            &mut metadata,
            PipelineKey::ShastaNative,
            PipelineKey::ShastaNative.route(),
        );
        record.error = Some("stale running".to_string());
        runtime.upsert_task(&record).await?;
        write_test_proof_artifact(
            &runtime,
            &metadata.network_pair,
            &proof_ref,
            &valid_native_proof(),
        )
        .await?;
        let state = test_state(Arc::clone(&runtime), Arc::new(NoopEngine));

        let response = handle_existing_batch_task(&state, &submission, record.clone(), None)
            .await
            .map_err(|err| anyhow::anyhow!(err.message))?;

        assert!(response_is_completed(&response));
        let stored = runtime
            .get_task(&record.task_id)
            .await?
            .expect("runtime task");
        assert_eq!(stored.runner_status, RuntimeRunnerStatus::Completed);
        assert!(stored.error.is_none());
        assert!(stored.proof_uri.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn completed_duplicate_with_missing_artifact_is_reenqueued() -> Result<()> {
        let route = native_local_route();
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "completed-missing-artifact-reenqueue",
        ))?);
        let mut submission = canonical_submission(route, false);
        let mut metadata = task_metadata_with_stage(None);
        metadata.proof_type = ProofType::Native;
        metadata.aggregate_requested = false;
        let mut record = runtime_record(RuntimeRunnerStatus::Completed, &metadata);
        record.task_id.clone_from(&submission.public_task_id);
        align_runtime_record_identity(
            &mut record,
            &mut metadata,
            PipelineKey::ShastaNative,
            PipelineKey::ShastaNative.route(),
        );
        record.request_fingerprint = batch_request_fingerprint_for_test(&submission)?;
        runtime.upsert_task(&record).await?;
        submission.public_task_id.clone_from(&record.task_id);
        let recorder = Arc::new(RecordingEngine::new());
        let state = test_state(Arc::clone(&runtime), recorder.clone());

        let response = handle_existing_batch_task(&state, &submission, record.clone(), None)
            .await
            .map_err(|err| anyhow::anyhow!(err.message))?;

        assert!(!response_is_completed(&response));
        assert_eq!(
            recorder
                .proposals
                .lock()
                .expect("proposal submissions")
                .len(),
            1
        );
        let stored = runtime
            .get_task(&record.task_id)
            .await?
            .expect("runtime task");
        assert_eq!(stored.runner_status, RuntimeRunnerStatus::Allocated);
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

    #[tokio::test]
    async fn inconsistent_sgx_record_is_rejected_before_pipeline_replacement() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "stale-sgx-to-sgxgeth-retry",
        ))?);
        let sgx_engine = Arc::new(RecordingEngine::new());
        let sgxgeth_engine = Arc::new(RecordingEngine::new());
        let state = test_state_with_engines(
            Arc::clone(&runtime),
            [
                (
                    PipelineKey::ShastaSgx,
                    Arc::clone(&sgx_engine) as Arc<dyn EngineHandle>,
                ),
                (
                    PipelineKey::ShastaSgxGeth,
                    Arc::clone(&sgxgeth_engine) as Arc<dyn EngineHandle>,
                ),
            ],
        );
        let route = CanonicalProofRoute::new(
            PipelineRoute::new(crate::config::GuestSystem::SgxGeth, RunnerKind::Remote),
            PipelineKey::ShastaSgxGeth,
        );
        let submission = canonical_submission(route, false);
        let request_fingerprint = batch_request_fingerprint_for_test(&submission)?;
        let plan = build_submission_plan(&submission, &request_fingerprint)
            .map_err(|err| anyhow!(err.message))?;
        let metadata = build_task_metadata(
            &submission.pair,
            BuildTaskMetadataParams {
                network: &submission.pair.network,
                l1_network: &submission.pair.l1_network,
                proof_type: submission.route.proof_type(),
                requested_proof_type: Some(submission.requested_proof_type.as_str()),
                prover_type: submission.prover_type,
                execution_mode: submission.execution_mode,
                aggregate_requested: false,
            },
            &plan.proposals,
            plan.aggregate.as_ref(),
        );
        let mut record = runtime_record(RuntimeRunnerStatus::Failed, &metadata);
        record.task_id.clone_from(&submission.public_task_id);
        record.pipeline_key = PipelineKey::ShastaSgx;
        record.route = PipelineRoute::new(crate::config::GuestSystem::Sgx, RunnerKind::Remote);
        record.artifact_refs = publication_proof_artifact_refs(&metadata, PipelineKey::ShastaSgx);
        record.request_fingerprint = request_fingerprint;
        runtime.upsert_task(&record).await?;

        let error = handle_existing_batch_task(&state, &submission, record.clone(), None)
            .await
            .expect_err("inconsistent durable identity must fail closed");
        assert!(error.message.contains("proof_type does not match"));

        let stored = runtime
            .get_task(&submission.public_task_id)
            .await?
            .expect("runtime task exists");
        assert_eq!(stored, record);

        let sgx_submissions = sgx_engine.proposals.lock().expect("sgx submissions");
        assert!(
            sgx_submissions.is_empty(),
            "stale sgx engine should not be used"
        );
        drop(sgx_submissions);

        assert!(
            sgxgeth_engine
                .proposals
                .lock()
                .expect("sgxgeth submissions")
                .is_empty()
        );
        Ok(())
    }

    #[tokio::test]
    async fn legacy_sgxgeth_record_projects_canonical_route() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "legacy-sgxgeth-request",
        ))?);
        let state = test_state_with_engines(
            Arc::clone(&runtime),
            [(
                PipelineKey::ShastaSgxGeth,
                Arc::new(NoopEngine) as Arc<dyn EngineHandle>,
            )],
        );
        let submission = canonical_submission(sgxgeth_remote_route(), false);
        let (_metadata, record) =
            legacy_sgxgeth_record(&submission, RuntimeRunnerStatus::Completed)?;
        runtime.upsert_task(&record).await?;

        let stored = runtime
            .get_task(&record.task_id)
            .await?
            .expect("legacy runtime task");
        assert_eq!(stored.incarnation_id, record.incarnation_id);
        assert_eq!(stored.route, record.route);
        let task = load_task_data(&state, &record.task_id)
            .await
            .map_err(|err| anyhow!(err.message))?;
        assert_eq!(task.route, "sgxgeth/remote");
        Ok(())
    }

    #[tokio::test]
    async fn startup_recovery_skips_legacy_nonterminal_sgxgeth_record() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "startup-legacy-sgxgeth-skip",
        ))?);
        let submission = canonical_submission(sgxgeth_remote_route(), false);
        let (_metadata, record) = legacy_sgxgeth_record(&submission, RuntimeRunnerStatus::Running)?;
        runtime.upsert_task(&record).await?;
        let recorder = Arc::new(RecordingEngine::new());
        let state = test_state_with_engines(
            Arc::clone(&runtime),
            [(
                PipelineKey::ShastaSgxGeth,
                recorder.clone() as Arc<dyn EngineHandle>,
            )],
        );

        assert_eq!(recover_pending_runtime_tasks(&state).await?, 0);
        assert!(
            recorder
                .proposals
                .lock()
                .expect("proposal submissions")
                .is_empty()
        );
        assert_eq!(
            runtime
                .get_task(&record.task_id)
                .await?
                .expect("legacy runtime task"),
            record
        );
        Ok(())
    }

    #[tokio::test]
    async fn duplicate_legacy_nonterminal_sgxgeth_is_replaced_canonically() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "duplicate-legacy-sgxgeth-replace",
        ))?);
        let submission = canonical_submission(sgxgeth_remote_route(), false);
        let (_metadata, record) = legacy_sgxgeth_record(&submission, RuntimeRunnerStatus::Running)?;
        runtime.upsert_task(&record).await?;
        let recorder = Arc::new(RecordingEngine::new());
        let state = test_state_with_engines(
            Arc::clone(&runtime),
            [(
                PipelineKey::ShastaSgxGeth,
                recorder.clone() as Arc<dyn EngineHandle>,
            )],
        );

        let response = handle_existing_batch_task(&state, &submission, record.clone(), None)
            .await
            .map_err(|err| anyhow!(err.message))?;

        assert!(!response_is_completed(&response));
        let stored = runtime
            .get_task(&record.task_id)
            .await?
            .expect("canonical replacement task");
        assert_ne!(stored.incarnation_id, record.incarnation_id);
        assert_eq!(stored.pipeline_key, PipelineKey::ShastaSgxGeth);
        assert_eq!(stored.route, PipelineKey::ShastaSgxGeth.route());
        assert_eq!(
            recorder
                .proposals
                .lock()
                .expect("proposal submissions")
                .len(),
            1
        );
        Ok(())
    }

    #[tokio::test]
    async fn running_sp1_network_record_is_replaced_after_local_route_change() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "running-sp1-network-to-local",
        ))?);
        let recorder = Arc::new(RecordingEngine::new());
        let state = test_state_with_engines(
            Arc::clone(&runtime),
            [(
                PipelineKey::ShastaSp1,
                recorder.clone() as Arc<dyn EngineHandle>,
            )],
        );
        let requested_route = CanonicalProofRoute::new(
            PipelineRoute::new(crate::config::GuestSystem::Sp1, RunnerKind::Local),
            PipelineKey::ShastaSp1,
        );
        let mut submission = canonical_submission(requested_route, false);
        submission.requested_proof_type = BatchProofType::Sp1;
        let request_fingerprint = batch_request_fingerprint_for_test(&submission)?;
        let plan = build_submission_plan(&submission, &request_fingerprint)
            .map_err(|err| anyhow!(err.message))?;
        let mut metadata = build_task_metadata(
            &submission.pair,
            BuildTaskMetadataParams {
                network: &submission.pair.network,
                l1_network: &submission.pair.l1_network,
                proof_type: submission.route.proof_type(),
                requested_proof_type: Some(submission.requested_proof_type.as_str()),
                prover_type: submission.prover_type,
                execution_mode: submission.execution_mode,
                aggregate_requested: false,
            },
            &plan.proposals,
            plan.aggregate.as_ref(),
        );
        metadata.runtime.proposals.insert(
            plan.proposals[0].task_ref.clone(),
            test_sp1_runtime("sp1-request", 1, 7_200),
        );
        let mut record = runtime_record(RuntimeRunnerStatus::Running, &metadata);
        record.task_id.clone_from(&submission.public_task_id);
        align_runtime_record_identity(
            &mut record,
            &mut metadata,
            PipelineKey::ShastaSp1,
            PipelineRoute::new(crate::config::GuestSystem::Sp1, RunnerKind::Network),
        );
        record.request_fingerprint = request_fingerprint;
        runtime.upsert_task(&record).await?;

        let response = handle_existing_batch_task(&state, &submission, record, None)
            .await
            .map_err(|err| anyhow!(err.message))?;

        assert!(!response_is_completed(&response));
        let stored = runtime
            .get_task(&submission.public_task_id)
            .await?
            .expect("replacement runtime task");
        assert_eq!(stored.pipeline_key, PipelineKey::ShastaSp1);
        assert_eq!(stored.route, submission.route.route);
        assert_eq!(recorder.proposals.lock().expect("submissions").len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn completed_sp1_artifact_is_not_reused_across_routes() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "completed-sp1-local-to-network",
        ))?);
        let requested_route = CanonicalProofRoute::new(
            PipelineRoute::new(crate::config::GuestSystem::Sp1, RunnerKind::Network),
            PipelineKey::ShastaSp1,
        );
        let mut submission = canonical_submission(requested_route, false);
        submission.requested_proof_type = BatchProofType::Sp1;
        let mut metadata = task_metadata_with_stage(None);
        metadata.proof_type = ProofType::Sp1;
        metadata.aggregate_requested = false;
        let mut record = runtime_record(RuntimeRunnerStatus::Completed, &metadata);
        record.task_id.clone_from(&submission.public_task_id);
        align_runtime_record_identity(
            &mut record,
            &mut metadata,
            PipelineKey::ShastaSp1,
            PipelineRoute::new(crate::config::GuestSystem::Sp1, RunnerKind::Local),
        );
        runtime.upsert_task(&record).await?;
        write_test_proof_artifact_for_route(
            &runtime,
            &metadata.network_pair,
            record.pipeline_key,
            record.route,
            &metadata.proposals[0].task_id,
            &valid_native_proof(),
        )
        .await?;
        let recorder = Arc::new(RecordingEngine::new());
        let state = test_state_with_engines(
            Arc::clone(&runtime),
            [(
                PipelineKey::ShastaSp1,
                recorder.clone() as Arc<dyn EngineHandle>,
            )],
        );

        let response = handle_existing_batch_task(&state, &submission, record.clone(), None)
            .await
            .map_err(|err| anyhow!(err.message))?;

        assert!(!response_is_completed(&response));
        let stored = runtime
            .get_task(&record.task_id)
            .await?
            .expect("replacement runtime task");
        assert_eq!(stored.route, submission.route.route);
        assert_eq!(recorder.proposals.lock().expect("submissions").len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn completed_sp1_root_accepts_compressed_proposal_artifact() -> Result<()> {
        let runtime = RuntimeManager::new(unique_test_runtime_root(
            "completed-sp1-compressed-proposal",
        ))?;
        let route = PipelineRoute::new(crate::config::GuestSystem::Sp1, RunnerKind::Network);
        let mut metadata = task_metadata_with_stage(None);
        metadata.proof_type = ProofType::Sp1;
        let mut record = runtime_record(RuntimeRunnerStatus::Completed, &metadata);
        align_runtime_record_identity(&mut record, &mut metadata, PipelineKey::ShastaSp1, route);
        runtime.upsert_task(&record).await?;
        let proof_ref = metadata.proposals[0].task_id.clone();
        let proof = Proof {
            input: Some(alloy_primitives::B256::ZERO),
            quote: Some(r#"{"Compressed":{}}"#.to_string()),
            uuid: Some("sp1-verifying-key".to_string()),
            extra_data: Some(serde_json::json!({ "shasta": {} })),
            ..Proof::default()
        };
        write_test_proof_artifact_for_route(
            &runtime,
            &metadata.network_pair,
            PipelineKey::ShastaSp1,
            route,
            &proof_ref,
            &proof,
        )
        .await?;

        assert!(
            !completed_root_artifact_missing(&runtime, &record, &metadata)
                .await
                .map_err(|err| anyhow!(err.message))?
        );
        assert_eq!(
            load_persisted_root_proof_material(&runtime, &record, &metadata)
                .await
                .map_err(|err| anyhow!(err.message))?,
            Some(proof)
        );
        assert!(
            load_proof_artifact_material(
                &runtime,
                &metadata.network_pair,
                PipelineKey::ShastaSp1,
                route,
                &proof_ref,
                ProofArtifactPayload::Final,
            )
            .await
            .is_err(),
            "compressed proposal material must not pass the final-proof loader"
        );

        let state = test_state_with_engines(
            Arc::new(runtime),
            [(
                PipelineKey::ShastaSp1,
                Arc::new(NoopEngine) as Arc<dyn EngineHandle>,
            )],
        );
        let task = load_task_data(&state, &record.task_id)
            .await
            .map_err(|err| anyhow!(err.message))?;
        assert!(matches!(task.status, ProofStatus::Completed));
        assert!(task.proof.is_none());
        assert_eq!(task.proof_ref.as_deref(), Some(proof_ref.as_str()));
        assert!(task.proof_uri.is_some());
        assert!(task.error.is_none());
        Ok(())
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn clearing_aggregate_outboxes_preserves_shared_proposal_until_last_root() -> Result<()> {
        let runtime = RuntimeManager::new(unique_test_runtime_root("shared-proposal-outbox"))?;
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let mut aggregate_metadata = task_metadata_with_stage(Some("aggregate"));
        aggregate_metadata.proof_type = ProofType::Native;
        aggregate_metadata.aggregate_requested = true;
        let shared_request = test_proposal_request(42);
        let shared_ref = proposal_task_ref(pipeline, &shared_request);
        aggregate_metadata.proposals[0].request = shared_request.clone();
        aggregate_metadata.proposals.push(ProposalTask {
            proposal_id: 43,
            checkpoint: None,
            l1_inclusion_block_number: 2,
            l2_block_numbers: vec![43],
            last_anchor_block_number: 42,
            task_id: "unique-proposal".to_string(),
            request: test_proposal_request(43),
        });
        aggregate_metadata.aggregate_request = Some(AggregationTaskRequest {
            request_id: "aggregate-request".to_string(),
            proposal_ids: vec![42, 43],
            prover_config: ProverTaskConfig::default(),
        });
        let mut aggregate_record =
            runtime_record(RuntimeRunnerStatus::Running, &aggregate_metadata);
        aggregate_record.task_id = "aggregate-root".to_string();
        aggregate_record.request_fingerprint = "aggregate-root-fingerprint".to_string();
        aggregate_record.pipeline_key = pipeline;
        aggregate_record.route = route;
        let aggregate_refs = publication_proof_artifact_refs(&aggregate_metadata, pipeline);
        aggregate_record.artifact_refs = aggregate_refs.clone();
        runtime.upsert_task(&aggregate_record).await?;

        let mut proposal_metadata = task_metadata_with_stage(Some("prove"));
        proposal_metadata.proof_type = ProofType::Native;
        proposal_metadata.aggregate_requested = false;
        proposal_metadata.proposals[0].request = shared_request;
        let mut proposal_record = runtime_record(RuntimeRunnerStatus::Running, &proposal_metadata);
        proposal_record.task_id = "proposal-root".to_string();
        proposal_record.request_fingerprint = "proposal-root-fingerprint".to_string();
        proposal_record.pipeline_key = pipeline;
        proposal_record.route = route;
        proposal_record.artifact_refs = vec![shared_ref.clone()];
        runtime.upsert_task(&proposal_record).await?;

        for proof_ref in &aggregate_refs {
            let mut owners = vec![aggregate_record.incarnation_id];
            if proof_ref == &shared_ref {
                owners.push(proposal_record.incarnation_id);
            }
            assert!(
                runtime
                    .checkpoint_pending_proof_publication(
                        &aggregate_metadata.network_pair,
                        pipeline,
                        route,
                        proof_ref,
                        &owners,
                        proof_ref.as_bytes(),
                    )
                    .await?
            );
        }

        runtime
            .release_task_pending_publications(&aggregate_record)
            .await?;

        assert!(
            runtime
                .get_pending_proof_publication(
                    &aggregate_metadata.network_pair,
                    pipeline,
                    route,
                    &shared_ref,
                )
                .await?
                .is_some()
        );
        for proof_ref in aggregate_refs
            .iter()
            .filter(|proof_ref| proof_ref.as_str() != shared_ref)
        {
            assert!(
                runtime
                    .get_pending_proof_publication(
                        &aggregate_metadata.network_pair,
                        pipeline,
                        route,
                        proof_ref,
                    )
                    .await?
                    .is_none(),
                "unshared outbox {proof_ref} was retained"
            );
        }

        assert!(matches!(
            runtime
                .retire_task_if_unchanged(&aggregate_record, None)
                .await?,
            RuntimeMutationOutcome::Applied
        ));
        assert!(matches!(
            runtime
                .remove_task_if_current(&aggregate_record.lifetime())
                .await?,
            RuntimeMutationOutcome::Applied
        ));
        runtime
            .release_task_pending_publications(&proposal_record)
            .await?;
        assert!(
            runtime
                .get_pending_proof_publication(
                    &proposal_metadata.network_pair,
                    pipeline,
                    route,
                    &shared_ref,
                )
                .await?
                .is_none()
        );
        assert!(
            runtime
                .get_recoverable_pending_proof_publication(
                    &proposal_metadata.network_pair,
                    pipeline,
                    route,
                    &shared_ref,
                )
                .await?
                .is_none()
        );
        Ok(())
    }

    #[tokio::test]
    async fn replacement_swaps_only_the_observed_root_projection() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "replacement-preserves-shared-proposal",
        ))?);
        let pipeline = PipelineKey::ShastaNative;
        let request = test_proposal_request(42);
        let shared_task_ref = proposal_task_ref(pipeline, &request);
        let mut first_metadata = task_metadata_with_stage(Some("prove"));
        first_metadata.proof_type = ProofType::Native;
        first_metadata.aggregate_requested = false;
        first_metadata.proposals[0]
            .task_id
            .clone_from(&shared_task_ref);
        first_metadata.proposals[0].request = request.clone();
        let mut first = runtime_record(RuntimeRunnerStatus::Failed, &first_metadata);
        first.task_id = "root-being-replaced".to_string();
        first.pipeline_key = pipeline;
        first.route = pipeline.route();
        first.artifact_refs = vec![shared_task_ref.clone()];
        first.request_fingerprint = "root-being-replaced-fingerprint".to_string();
        runtime.upsert_task(&first).await?;

        let mut second_metadata = first_metadata.clone();
        second_metadata.runtime.active_stage = Some("prove".to_string());
        let mut second = runtime_record(RuntimeRunnerStatus::Running, &second_metadata);
        second.task_id = "root-still-live".to_string();
        second.pipeline_key = pipeline;
        second.route = pipeline.route();
        second.artifact_refs = vec![shared_task_ref];
        second.request_fingerprint = "root-still-live-fingerprint".to_string();
        runtime.upsert_task(&second).await?;

        let recorder = Arc::new(RecordingEngine::new());
        let state = test_state_with_engines(
            Arc::clone(&runtime),
            [(pipeline, recorder.clone() as Arc<dyn EngineHandle>)],
        );
        let replacement_engine: Arc<dyn EngineHandle> = recorder.clone();
        let replacement = state
            .lifecycle
            .replace(
                &first,
                TaskRegistration {
                    task_id: first.task_id.clone(),
                    pipeline_key: pipeline,
                    route: pipeline.route(),
                    task_kind: first.task_kind.clone(),
                    network_pair: first.network_pair.clone(),
                    artifact_refs: first.artifact_refs.clone(),
                    metadata: first.metadata.clone(),
                    request_fingerprint: "replacement-fingerprint".to_string(),
                },
                &[],
                &replacement_engine,
                EngineExecutionPlan {
                    proposals: vec![request],
                    aggregate: None,
                },
            )
            .await
            .context("replace observed root")?
            .context("replacement must win")?;

        assert_eq!(
            recorder
                .detached
                .lock()
                .expect("detached owners")
                .iter()
                .map(|owner| owner.task_id.as_str())
                .collect::<Vec<_>>(),
            vec!["root-being-replaced"],
        );
        assert_eq!(
            recorder
                .attached
                .lock()
                .expect("attached owners")
                .as_slice(),
            &[raiko2_queue::RootOwner::new(
                replacement.task_id.clone(),
                replacement.incarnation_id,
            )],
        );
        assert_eq!(
            runtime
                .get_task(&first.task_id)
                .await?
                .context("replacement root")?
                .incarnation_id,
            replacement.incarnation_id,
        );
        assert_eq!(
            runtime
                .get_task(&second.task_id)
                .await?
                .context("shared root")?
                .incarnation_id,
            second.incarnation_id,
        );
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_replacement_commits_one_successor_projection() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "concurrent-root-replacement",
        ))?);
        let pipeline = PipelineKey::ShastaNative;
        let request = test_proposal_request(42);
        let proof_ref = proposal_task_ref(pipeline, &request);
        let mut metadata = task_metadata_with_stage(Some("prove"));
        metadata.proof_type = ProofType::Native;
        metadata.aggregate_requested = false;
        metadata.proposals[0].task_id.clone_from(&proof_ref);
        metadata.proposals[0].request = request.clone();
        let mut expected = runtime_record(RuntimeRunnerStatus::Running, &metadata);
        expected.task_id = "concurrent-root".to_string();
        expected.pipeline_key = pipeline;
        expected.route = pipeline.route();
        expected.artifact_refs = vec![proof_ref.clone()];
        expected.request_fingerprint = "previous-fingerprint".to_string();
        runtime.upsert_task(&expected).await?;
        let proof = br#"{"proof":"0x01"}"#;
        assert!(
            runtime
                .checkpoint_pending_proof_publication(
                    &expected.network_pair,
                    pipeline,
                    expected.route,
                    &proof_ref,
                    &[expected.incarnation_id],
                    proof,
                )
                .await?
        );
        runtime
            .commit_proof_artifact_publication(
                &expected.network_pair,
                pipeline,
                expected.route,
                &proof_ref,
                proof,
            )
            .await?;

        let recorder = Arc::new(RecordingEngine::new());
        let state = test_state_with_engines(
            Arc::clone(&runtime),
            [(pipeline, recorder.clone() as Arc<dyn EngineHandle>)],
        );
        let replacement_engine: Arc<dyn EngineHandle> = recorder.clone();
        let registration = |fingerprint: &str| TaskRegistration {
            task_id: expected.task_id.clone(),
            pipeline_key: pipeline,
            route: pipeline.route(),
            task_kind: expected.task_kind.clone(),
            network_pair: expected.network_pair.clone(),
            artifact_refs: vec![proof_ref.clone()],
            metadata: expected.metadata.clone(),
            request_fingerprint: fingerprint.to_string(),
        };
        let barrier = Arc::new(tokio::sync::Barrier::new(3));
        let registration_a = registration("replacement-a");
        let registration_b = registration("replacement-b");
        let request_a = request.clone();
        let state_ref = &state;
        let expected_ref = &expected;
        let first = {
            let barrier = Arc::clone(&barrier);
            let engine = Arc::clone(&replacement_engine);
            async move {
                barrier.wait().await;
                state_ref
                    .lifecycle
                    .replace(
                        expected_ref,
                        registration_a,
                        &[],
                        &engine,
                        EngineExecutionPlan {
                            proposals: vec![request_a],
                            aggregate: None,
                        },
                    )
                    .await
            }
        };
        let second = {
            let barrier = Arc::clone(&barrier);
            let engine = Arc::clone(&replacement_engine);
            async move {
                barrier.wait().await;
                state_ref
                    .lifecycle
                    .replace(
                        expected_ref,
                        registration_b,
                        &[],
                        &engine,
                        EngineExecutionPlan {
                            proposals: vec![request],
                            aggregate: None,
                        },
                    )
                    .await
            }
        };
        let release = async { barrier.wait().await };
        let (first, second, _) = tokio::join!(first, second, release);
        let first = first?;
        let second = second?;
        assert_eq!(
            usize::from(first.is_some()) + usize::from(second.is_some()),
            1
        );
        let winner = first.or(second).context("one replacement must win")?;

        let attached = recorder.attached.lock().expect("attached owners");
        assert_eq!(attached.len(), 1);
        assert_eq!(attached[0].incarnation_id, winner.incarnation_id);
        drop(attached);
        assert_eq!(recorder.detached.lock().expect("detached owners").len(), 1);
        assert_eq!(
            runtime
                .get_task(&expected.task_id)
                .await?
                .context("successor root")?
                .incarnation_id,
            winner.incarnation_id,
        );
        assert!(
            runtime
                .get_pending_proof_publication(
                    &expected.network_pair,
                    pipeline,
                    expected.route,
                    &proof_ref,
                )
                .await?
                .is_none(),
            "the winning replacement must clean the predecessor publication intent"
        );
        assert_eq!(
            runtime
                .reconcile_unowned_pending_proof_publications()
                .await?,
            0,
        );
        assert!(
            runtime
                .read_proof_artifact_bytes(
                    &expected.network_pair,
                    pipeline,
                    expected.route,
                    &proof_ref,
                )
                .await?
                .is_none(),
            "the winning replacement must invalidate the predecessor canonical manifest"
        );
        Ok(())
    }

    #[tokio::test]
    async fn stale_replacement_does_not_return_a_conflicting_root() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "stale-replacement-conflict",
        ))?);
        let route = native_local_route();
        let mut submission = canonical_submission(route, false);
        submission.public_task_id = "replacement-conflict-root".to_string();
        let request_fingerprint = batch_request_fingerprint_for_test(&submission)?;
        let plan = build_submission_plan(&submission, &request_fingerprint)
            .map_err(|error| anyhow!(error.message))?;
        let stale = runtime
            .register_task(
                build_batch_task_registration(&submission, &plan, &request_fingerprint)
                    .map_err(|error| anyhow!(error.message))?,
            )
            .await?;
        let mut occupying = stale.clone();
        occupying.request_fingerprint = "concurrent-request".to_string();
        occupying.updated_at = occupying.updated_at.saturating_add(1);
        runtime.upsert_task(&occupying).await?;

        let state = test_state(Arc::clone(&runtime), Arc::new(RecordingEngine::new()));
        let error =
            replace_existing_batch_task(&state, &submission, &stale, Some(&request_fingerprint))
                .await
                .expect_err("a stale replacement must not adopt another request");

        assert_eq!(error.status, StatusCode::CONFLICT);
        assert_eq!(
            runtime
                .get_task(&submission.public_task_id)
                .await?
                .context("occupying root")?
                .request_fingerprint,
            "concurrent-request"
        );
        Ok(())
    }

    #[tokio::test]
    async fn stale_retirement_cannot_remove_a_reopened_attached_root() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "stale-retirement-after-recovery",
        ))?);
        let pipeline = PipelineKey::ShastaNative;
        let metadata = task_metadata_with_stage(Some("prove"));
        let mut stale = runtime_record(RuntimeRunnerStatus::Failed, &metadata);
        stale.task_id = "recovered-root".to_string();
        stale.pipeline_key = pipeline;
        stale.route = pipeline.route();
        stale.request_fingerprint = "recovered-root-fingerprint".to_string();
        runtime.upsert_task(&stale).await?;

        let recovered = runtime
            .prepare_task_for_recovery_if_unchanged(&stale)
            .await?
            .context("reopened root")?;
        let recorder = Arc::new(RecordingEngine::new());
        let state = test_state_with_engines(
            Arc::clone(&runtime),
            [(pipeline, recorder.clone() as Arc<dyn EngineHandle>)],
        );
        let engine: Arc<dyn EngineHandle> = recorder.clone();
        state
            .lifecycle
            .attach(
                &recovered,
                &engine,
                EngineExecutionPlan {
                    proposals: Vec::new(),
                    aggregate: None,
                },
            )
            .await?;

        assert_eq!(
            state
                .lifecycle
                .remove(&stale, raiko2_queue::DetachMode::Remove)
                .await?
                .0,
            RuntimeMutationOutcome::Blocked,
        );
        assert!(
            recorder
                .detached
                .lock()
                .expect("detached owners")
                .is_empty()
        );
        assert_eq!(
            runtime
                .get_task(&stale.task_id)
                .await?
                .context("current root")?
                .runner_status,
            RuntimeRunnerStatus::Allocated,
        );
        Ok(())
    }

    #[test]
    fn failed_submission_after_remote_stage_without_resume_metadata_is_not_reenqueueable() {
        let mut metadata = task_metadata_with_stage(Some("prove"));
        set_test_proposal_runtime(
            &mut metadata,
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
    fn failed_aggregate_reenqueues_when_only_the_proposal_has_remote_progress() {
        let mut metadata = task_metadata_with_stage(Some("aggregate"));
        configure_test_aggregate(&mut metadata);
        set_test_proposal_runtime(
            &mut metadata,
            test_boundless_runtime("0xproposal", 123_000, 123_400, 123_456),
        );
        let record = runtime_record(RuntimeRunnerStatus::Failed, &metadata);
        let metadata = TaskMetadata::decode_for_record(&record).expect("canonical aggregate root");

        assert!(should_reenqueue_existing_submission_without_engine(
            &record, &metadata
        ));
    }

    #[test]
    fn failed_aggregate_with_aggregate_resume_metadata_is_reenqueueable() {
        let mut metadata = task_metadata_with_stage(Some("aggregate"));
        configure_test_aggregate(&mut metadata);
        set_test_proposal_runtime(
            &mut metadata,
            test_boundless_runtime("0xproposal", 123_000, 123_400, 123_456),
        );
        metadata.runtime.aggregate = Some(test_boundless_runtime(
            "0xaggregate",
            456_000,
            456_700,
            456_789,
        ));
        let record = runtime_record(RuntimeRunnerStatus::Failed, &metadata);
        let metadata = TaskMetadata::decode_for_record(&record).expect("canonical aggregate root");

        assert!(should_reenqueue_existing_submission_without_engine(
            &record, &metadata
        ));
    }

    #[tokio::test]
    async fn native_proposal_task_id_is_reused_across_aggregate_flags() {
        let route = native_local_route();
        let runtime =
            RuntimeManager::new(unique_test_runtime_root("plan-task-id")).expect("runtime manager");
        let single_submission = canonical_submission(route, false);
        let single_fingerprint =
            batch_request_fingerprint("test", "raiko2-test", &single_submission)
                .expect("single fingerprint");
        let single = build_submission_plan(&single_submission, &single_fingerprint)
            .expect("single submission plan");
        runtime
            .register_task(
                build_batch_task_registration(&single_submission, &single, &single_fingerprint)
                    .expect("single task registration"),
            )
            .await
            .expect("register single task");
        let aggregate_submission = canonical_submission(route, true);
        let aggregate_fingerprint =
            batch_request_fingerprint("test", "raiko2-test", &aggregate_submission)
                .expect("aggregate fingerprint");
        let aggregate = build_submission_plan(&aggregate_submission, &aggregate_fingerprint)
            .expect("aggregate submission plan");

        assert_eq!(single.proposals.len(), 1);
        assert_eq!(aggregate.proposals.len(), 1);
        assert_eq!(single.proposals[0].task_id, aggregate.proposals[0].task_id);
    }

    #[test]
    fn request_fingerprint_is_scoped_to_environment() {
        let submission = canonical_submission(native_local_route(), false);
        let dev =
            batch_request_fingerprint("dev", "raiko2-a", &submission).expect("dev fingerprint");
        let prod =
            batch_request_fingerprint("prod", "raiko2-a", &submission).expect("prod fingerprint");

        assert_ne!(dev, prod);
    }

    #[test]
    fn request_fingerprint_is_scoped_to_namespace() {
        let submission = canonical_submission(native_local_route(), false);
        let first = batch_request_fingerprint("dev", "raiko2-a", &submission)
            .expect("first namespace fingerprint");
        let second = batch_request_fingerprint("dev", "raiko2-b", &submission)
            .expect("second namespace fingerprint");

        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn startup_recovery_requeues_pending_runtime_task() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "startup-pending-recovery",
        ))?);
        let submission = canonical_submission(native_local_route(), false);
        let fingerprint = batch_request_fingerprint_for_test(&submission)?;
        let plan = build_submission_plan(&submission, &fingerprint)
            .map_err(|error| anyhow!(error.message))?;
        let registration = build_batch_task_registration(&submission, &plan, &fingerprint)
            .map_err(|error| anyhow!(error.message))?;
        let task_id = registration.task_id.clone();
        runtime.register_task(registration).await?;
        let mut registered = runtime.get_task(&task_id).await?.expect("registered task");
        registered.runner_status = RuntimeRunnerStatus::Running;
        runtime.upsert_task(&registered).await?;

        let recorder = Arc::new(RecordingEngine::new());
        let state = test_state(runtime.clone(), recorder.clone());
        assert_eq!(recover_pending_runtime_tasks(&state).await?, 1);
        assert_eq!(
            recorder
                .proposals
                .lock()
                .expect("proposal submissions")
                .len(),
            1
        );
        assert_eq!(
            runtime
                .get_task(&task_id)
                .await?
                .expect("runtime task")
                .runner_status,
            RuntimeRunnerStatus::Allocated
        );
        Ok(())
    }

    #[tokio::test]
    async fn startup_rejects_noncanonical_metadata_before_recovery() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "startup-metadata-hard-cut",
        ))?);
        let submission = canonical_submission(native_local_route(), false);
        let fingerprint = batch_request_fingerprint_for_test(&submission)?;
        let plan = build_submission_plan(&submission, &fingerprint)
            .map_err(|error| anyhow!(error.message))?;
        let registration = build_batch_task_registration(&submission, &plan, &fingerprint)
            .map_err(|error| anyhow!(error.message))?;
        let task_id = registration.task_id.clone();
        runtime.register_task(registration).await?;
        let mut record = runtime.get_task(&task_id).await?.expect("registered task");
        record.runner_status = RuntimeRunnerStatus::Completed;
        record.metadata["proof_ids"] = serde_json::json!(["removed-legacy-index"]);
        runtime.upsert_task(&record).await?;

        let state = test_state(Arc::clone(&runtime), Arc::new(NoopEngine));
        let validation_error = validate_persisted_runtime_task_metadata(&state)
            .await
            .expect_err("startup validation must reject removed metadata fields");
        assert!(validation_error.to_string().contains(&task_id));
        assert!(recover_pending_runtime_tasks(&state).await.is_err());
        assert_eq!(
            runtime
                .get_task(&task_id)
                .await?
                .expect("runtime task")
                .runner_status,
            RuntimeRunnerStatus::Completed
        );
        Ok(())
    }

    #[tokio::test]
    async fn startup_recovery_does_not_overwrite_progress_from_attachment() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "startup-attachment-progress",
        ))?);
        let submission = canonical_submission(native_local_route(), false);
        let fingerprint = batch_request_fingerprint_for_test(&submission)?;
        let plan = build_submission_plan(&submission, &fingerprint)
            .map_err(|error| anyhow!(error.message))?;
        let registration = build_batch_task_registration(&submission, &plan, &fingerprint)
            .map_err(|error| anyhow!(error.message))?;
        let task_id = registration.task_id.clone();
        runtime.register_task(registration).await?;

        let recorder = Arc::new(RecordingEngine::progressing(Arc::clone(&runtime)));
        let state = test_state(Arc::clone(&runtime), recorder);

        assert_eq!(recover_pending_runtime_tasks(&state).await?, 1);
        assert_eq!(
            runtime
                .get_task(&task_id)
                .await?
                .expect("runtime task")
                .runner_status,
            RuntimeRunnerStatus::Running
        );
        Ok(())
    }

    #[tokio::test]
    async fn startup_recovery_reopens_failed_runtime_task_before_attaching() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "startup-failed-recovery",
        ))?);
        let submission = canonical_submission(native_local_route(), false);
        let fingerprint = batch_request_fingerprint_for_test(&submission)?;
        let plan = build_submission_plan(&submission, &fingerprint)
            .map_err(|error| anyhow!(error.message))?;
        let registration = build_batch_task_registration(&submission, &plan, &fingerprint)
            .map_err(|error| anyhow!(error.message))?;
        let task_id = registration.task_id.clone();
        runtime.register_task(registration).await?;
        let mut registered = runtime.get_task(&task_id).await?.expect("registered task");
        registered.runner_status = RuntimeRunnerStatus::Failed;
        registered.error = Some("recoverable fixture failure".to_string());
        runtime.upsert_task(&registered).await?;

        let recorder = Arc::new(RecordingEngine::new());
        let state = test_state(runtime.clone(), recorder.clone());
        assert_eq!(recover_pending_runtime_tasks(&state).await?, 1);
        assert_eq!(
            recorder
                .proposals
                .lock()
                .expect("proposal submissions")
                .len(),
            1
        );
        assert_eq!(
            runtime
                .get_task(&task_id)
                .await?
                .expect("runtime task")
                .runner_status,
            RuntimeRunnerStatus::Allocated
        );
        Ok(())
    }

    #[tokio::test]
    async fn cancelled_root_cannot_attach_a_recovered_execution_plan() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "cancelled-root-attach-fence",
        ))?);
        let submission = canonical_submission(native_local_route(), false);
        let fingerprint = batch_request_fingerprint_for_test(&submission)?;
        let plan = build_submission_plan(&submission, &fingerprint)
            .map_err(|error| anyhow!(error.message))?;
        let root = runtime
            .register_task(
                build_batch_task_registration(&submission, &plan, &fingerprint)
                    .map_err(|error| anyhow!(error.message))?,
            )
            .await?;
        let recorder = Arc::new(RecordingEngine::new());
        let engine: Arc<dyn EngineHandle> = recorder.clone();
        let state = test_state(Arc::clone(&runtime), Arc::clone(&engine));
        let proof_ref = root
            .artifact_refs
            .first()
            .expect("root proof artifact")
            .clone();
        let proof = br#"{"proof":"0x01"}"#;
        assert!(
            runtime
                .checkpoint_pending_proof_publication(
                    &root.network_pair,
                    root.pipeline_key,
                    root.route,
                    &proof_ref,
                    &[root.incarnation_id],
                    proof,
                )
                .await?
        );
        runtime
            .commit_proof_artifact_publication(
                &root.network_pair,
                root.pipeline_key,
                root.route,
                &proof_ref,
                proof,
            )
            .await?;

        state.lifecycle.cancel(&root, None).await?;
        let err = attach_submission_plan(&state, &engine, &root, &plan)
            .await
            .expect_err("cancelled root must not reattach execution");

        assert!(err.message.contains("no longer active"));
        assert!(
            recorder
                .proposals
                .lock()
                .expect("proposal submissions")
                .is_empty()
        );
        assert_eq!(
            runtime
                .get_task(&root.task_id)
                .await?
                .expect("runtime task")
                .runner_status,
            RuntimeRunnerStatus::Cancelled
        );
        assert!(
            runtime
                .read_proof_artifact_bytes(
                    &root.network_pair,
                    root.pipeline_key,
                    root.route,
                    &proof_ref,
                )
                .await?
                .is_none(),
            "root cancellation must invalidate its unowned canonical publication"
        );
        Ok(())
    }

    #[tokio::test]
    async fn completed_root_skips_a_late_execution_attachment() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "completed-root-attach-fence",
        ))?);
        let submission = canonical_submission(native_local_route(), false);
        let fingerprint = batch_request_fingerprint_for_test(&submission)?;
        let plan = build_submission_plan(&submission, &fingerprint)
            .map_err(|error| anyhow!(error.message))?;
        let mut root = runtime
            .register_task(
                build_batch_task_registration(&submission, &plan, &fingerprint)
                    .map_err(|error| anyhow!(error.message))?,
            )
            .await?;
        root.runner_status = RuntimeRunnerStatus::Completed;
        root.proof_uri = Some("memory://canonical-proof".into());
        runtime.upsert_task(&root).await?;
        let recorder = Arc::new(RecordingEngine::new());
        let engine: Arc<dyn EngineHandle> = recorder.clone();
        let state = test_state(Arc::clone(&runtime), Arc::clone(&engine));

        attach_submission_plan(&state, &engine, &root, &plan)
            .await
            .map_err(|error| anyhow!(error.message))?;

        assert!(
            recorder
                .proposals
                .lock()
                .expect("proposal submissions")
                .is_empty()
        );
        assert_eq!(
            runtime
                .get_task(&root.task_id)
                .await?
                .expect("runtime task")
                .runner_status,
            RuntimeRunnerStatus::Completed
        );
        Ok(())
    }

    #[tokio::test]
    async fn draining_runtime_cannot_attach_a_recovered_execution_plan() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "draining-root-attach-fence",
        ))?);
        let submission = canonical_submission(native_local_route(), false);
        let fingerprint = batch_request_fingerprint_for_test(&submission)?;
        let plan = build_submission_plan(&submission, &fingerprint)
            .map_err(|error| anyhow!(error.message))?;
        let root = runtime
            .register_task(
                build_batch_task_registration(&submission, &plan, &fingerprint)
                    .map_err(|error| anyhow!(error.message))?,
            )
            .await?;
        let recorder = Arc::new(RecordingEngine::new());
        let engine: Arc<dyn EngineHandle> = recorder.clone();
        let state = test_state(Arc::clone(&runtime), Arc::clone(&engine));

        state.begin_shutdown().await;
        let err = attach_submission_plan(&state, &engine, &root, &plan)
            .await
            .expect_err("draining runtime must reject execution attachment");

        assert!(err.message.contains("no longer active"));
        assert!(
            recorder
                .proposals
                .lock()
                .expect("proposal submissions")
                .is_empty()
        );
        Ok(())
    }

    #[test]
    fn aggregate_plan_uses_request_fingerprint_as_idempotent_key() {
        let route = native_local_route();
        let mut first_submission = canonical_submission(route, true);
        first_submission.public_task_id = "task-public-a".to_string();
        let mut second_submission = first_submission.clone();
        second_submission.public_task_id = "task-public-b".to_string();

        let first_fingerprint = batch_request_fingerprint("test", "raiko2-test", &first_submission)
            .expect("first fingerprint");
        let second_fingerprint =
            batch_request_fingerprint("test", "raiko2-test", &second_submission)
                .expect("second fingerprint");
        assert_eq!(first_fingerprint, second_fingerprint);

        let first = build_submission_plan(&first_submission, &first_fingerprint)
            .expect("first submission plan");
        let second = build_submission_plan(&second_submission, &second_fingerprint)
            .expect("second submission plan");

        let first_aggregate = first.aggregate.expect("first aggregate");
        let second_aggregate = second.aggregate.expect("second aggregate");
        assert_eq!(first_aggregate.task_ref, second_aggregate.task_ref);
        assert_eq!(
            first_aggregate.request.request_id,
            aggregate_request_id(&first_fingerprint)
        );
        assert_ne!(
            first_aggregate.request.request_id,
            first_submission.public_task_id
        );
    }

    #[test]
    fn sgx_and_sgxgeth_batch_fingerprints_do_not_collide() {
        let sgx_submission = canonical_submission(sgx_remote_route(), false);
        let sgxgeth_submission = canonical_submission(sgxgeth_remote_route(), false);

        let sgx_fingerprint = batch_request_fingerprint("test", "raiko2-test", &sgx_submission)
            .expect("sgx fingerprint");
        let sgxgeth_fingerprint =
            batch_request_fingerprint("test", "raiko2-test", &sgxgeth_submission)
                .expect("sgxgeth fingerprint");

        assert_ne!(sgx_fingerprint, sgxgeth_fingerprint);
    }

    #[tokio::test]
    async fn aggregate_plan_keeps_one_graph_shape_when_a_proposal_is_cached() -> Result<()> {
        let route = native_local_route();
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
        let request_fingerprint = batch_request_fingerprint_for_test(&submission)?;
        let uncached_plan = build_submission_plan(&submission, &request_fingerprint)
            .expect("uncached submission plan");
        write_test_proof_artifact(
            &runtime,
            &submission.pair.key,
            &cached_ref,
            &valid_native_proof(),
        )
        .await
        .expect("write cached proof");

        let plan = build_submission_plan(&submission, &request_fingerprint)
            .expect("cached submission plan");
        assert_eq!(execution_plan(&plan), execution_plan(&uncached_plan));

        let recorder = Arc::new(RecordingEngine::new());
        recorder
            .attach_execution_plan(
                raiko2_queue::RootOwner::new("mixed-cache-root", uuid::Uuid::new_v4()),
                execution_plan(&plan),
            )
            .await
            .expect("attach execution plan");

        let proposals = recorder.proposals.lock().expect("proposal submissions");
        assert_eq!(proposals.len(), 2);
        assert_eq!(proposals[0].0.proposal_id, 7);
        assert_eq!(proposals[1].0.proposal_id, 8);
        assert!(
            proposals
                .iter()
                .all(|(_, dependencies)| dependencies.is_empty())
        );
        drop(proposals);

        let aggregate_inputs = recorder.aggregate_inputs.lock().expect("aggregate inputs");
        assert_eq!(aggregate_inputs.len(), 2);
        assert_eq!(
            aggregate_inputs[0],
            AggregateProofInput::PendingProofArtifact {
                artifact: proof_artifact_ref(
                    &submission.pair.key,
                    PipelineKey::ShastaNative,
                    PipelineKey::ShastaNative.route(),
                    &cached_ref
                ),
                dependency: Box::new(plan.proposals[0].task_id.clone()),
            }
        );
        assert_eq!(
            aggregate_inputs[1],
            AggregateProofInput::PendingProofArtifact {
                artifact: proof_artifact_ref(
                    &submission.pair.key,
                    PipelineKey::ShastaNative,
                    PipelineKey::ShastaNative.route(),
                    &plan.proposals[1].task_ref
                ),
                dependency: Box::new(plan.proposals[1].task_id.clone()),
            }
        );
        Ok(())
    }

    #[tokio::test]
    async fn aggregate_admission_does_not_depend_on_cached_artifact_lifetime() -> Result<()> {
        let route = native_local_route();
        let runtime = RuntimeManager::new(unique_test_runtime_root("cached-admission-race"))?;
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
        let proof_ref = proposal_task_ref(PipelineKey::ShastaNative, &first_request);
        write_test_proof_artifact(
            &runtime,
            &submission.pair.key,
            &proof_ref,
            &valid_native_proof(),
        )
        .await?;
        let fingerprint = batch_request_fingerprint_for_test(&submission)?;
        let plan = build_submission_plan(&submission, &fingerprint)
            .map_err(|error| anyhow!(error.message))?;
        let cached = runtime
            .get_proof_artifact(
                &submission.pair.key,
                PipelineKey::ShastaNative,
                PipelineKey::ShastaNative.route(),
                &proof_ref,
            )
            .await?
            .expect("cached artifact registration");
        assert!(matches!(
            runtime
                .invalidate_proof_artifact_descriptor_if_unowned(
                    &submission.pair.key,
                    PipelineKey::ShastaNative,
                    PipelineKey::ShastaNative.route(),
                    &proof_ref,
                    &cached.descriptor(),
                )
                .await?,
            raiko2_runtime::ProofArtifactInvalidationResult::Invalidated(_)
        ));

        let registration = build_batch_task_registration(&submission, &plan, &fingerprint)
            .map_err(|error| anyhow!(error.message))?;
        runtime.register_task_if_absent(registration).await?;
        assert!(
            runtime
                .get_task(&submission.public_task_id)
                .await?
                .is_some()
        );
        assert_eq!(execution_plan(&plan).proposals.len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn aggregate_recovery_preserves_the_original_execution_graph() -> Result<()> {
        let route = native_local_route();
        let runtime = Arc::new(
            RuntimeManager::new(unique_test_runtime_root("aggregate-recovery-artifacts"))
                .expect("runtime manager"),
        );
        let submission = canonical_multi_submission(route);
        let request_fingerprint = batch_request_fingerprint_for_test(&submission)?;
        let plan =
            build_submission_plan(&submission, &request_fingerprint).expect("submission plan");
        let mut metadata = build_task_metadata(
            &submission.pair,
            BuildTaskMetadataParams {
                network: &submission.pair.network,
                l1_network: &submission.pair.l1_network,
                proof_type: submission.route.proof_type(),
                requested_proof_type: Some(submission.requested_proof_type.as_str()),
                prover_type: submission.prover_type,
                execution_mode: submission.execution_mode,
                aggregate_requested: true,
            },
            &plan.proposals,
            plan.aggregate.as_ref(),
        );
        metadata.runtime.active_stage = Some("aggregate".to_string());
        let mut registration =
            build_batch_task_registration(&submission, &plan, &request_fingerprint)
                .map_err(|error| anyhow!(error.message))?;
        registration.metadata = serde_json::to_value(&metadata)?;
        let record = runtime.register_task(registration).await?;
        for proposal in &plan.proposals {
            write_test_proof_artifact(
                &runtime,
                &submission.pair.key,
                &proposal.task_ref,
                &valid_native_proof(),
            )
            .await?;
        }
        let recovery_plan = build_recovery_plan_from_metadata(&record, &metadata)
            .map_err(|error| anyhow!(error.message))?;
        assert_eq!(execution_plan(&recovery_plan), execution_plan(&plan));

        let recorder = Arc::new(RecordingEngine::new());
        let engine: Arc<dyn EngineHandle> = recorder.clone();
        let state = test_state(Arc::clone(&runtime), Arc::clone(&engine));
        reenqueue_existing_batch_task(&state, &record, &metadata)
            .await
            .expect("aggregate recovery should keep its canonical graph");

        let proposals = recorder.proposals.lock().expect("proposal submissions");
        assert_eq!(proposals.len(), 2);
        assert!(
            proposals
                .iter()
                .all(|(_, dependencies)| dependencies.is_empty())
        );
        drop(proposals);

        let aggregate_inputs = recorder.aggregate_inputs.lock().expect("aggregate inputs");
        assert_eq!(aggregate_inputs.len(), 2);
        assert!(aggregate_inputs.iter().all(|input| {
            matches!(
                input,
                AggregateProofInput::PendingProofArtifact { artifact, .. }
                    if artifact.proof_ref.starts_with("task_")
            )
        }));
        Ok(())
    }

    #[tokio::test]
    async fn aggregate_recovery_reenqueues_missing_proposal_artifacts() -> Result<()> {
        let route = native_local_route();
        let runtime = Arc::new(
            RuntimeManager::new(unique_test_runtime_root("aggregate-recovery-missing"))
                .expect("runtime manager"),
        );
        let submission = canonical_multi_submission(route);
        let request_fingerprint = batch_request_fingerprint_for_test(&submission)?;
        let plan =
            build_submission_plan(&submission, &request_fingerprint).expect("submission plan");
        let mut metadata = build_task_metadata(
            &submission.pair,
            BuildTaskMetadataParams {
                network: &submission.pair.network,
                l1_network: &submission.pair.l1_network,
                proof_type: submission.route.proof_type(),
                requested_proof_type: Some(submission.requested_proof_type.as_str()),
                prover_type: submission.prover_type,
                execution_mode: submission.execution_mode,
                aggregate_requested: true,
            },
            &plan.proposals,
            plan.aggregate.as_ref(),
        );
        metadata.runtime.active_stage = Some("aggregate".to_string());
        let mut registration =
            build_batch_task_registration(&submission, &plan, &request_fingerprint)
                .map_err(|error| anyhow!(error.message))?;
        registration.metadata = serde_json::to_value(&metadata)?;
        let record = runtime.register_task(registration).await?;

        let recorder = Arc::new(RecordingEngine::new());
        let engine: Arc<dyn EngineHandle> = recorder.clone();
        let state = test_state(Arc::clone(&runtime), Arc::clone(&engine));
        reenqueue_existing_batch_task(&state, &record, &metadata)
            .await
            .expect("missing proposal artifacts should be re-enqueued");

        let proposals = recorder.proposals.lock().expect("proposal submissions");
        assert_eq!(proposals.len(), 2);
        assert!(
            proposals
                .iter()
                .all(|(_, dependencies)| dependencies.is_empty())
        );
        let aggregate_inputs = recorder.aggregate_inputs.lock().expect("aggregate inputs");
        assert_eq!(aggregate_inputs.len(), 2);
        assert!(
            aggregate_inputs
                .iter()
                .all(|input| matches!(input, AggregateProofInput::PendingProofArtifact { .. }))
        );
        Ok(())
    }

    #[tokio::test]
    async fn corrupt_proof_artifact_does_not_change_plan_and_fails_read() -> Result<()> {
        let route = native_local_route();
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
        let publication = runtime
            .publish_proof_artifact_bytes(
                &submission.pair.key,
                PipelineKey::ShastaNative,
                PipelineKey::ShastaNative.route(),
                &proof_ref,
                b"{bad-json",
            )
            .await
            .expect("write corrupt artifact");
        let artifact = publication
            .try_object()
            .expect("proof publication should materialize content");
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: submission.pair.key.clone(),
                proof_ref,
                pipeline_key: PipelineKey::ShastaNative,
                route: "native/local".parse().expect("route"),
                proof_uri: artifact.proof_uri.clone(),
                content_hash: artifact.content_hash.clone(),
                generation: artifact.generation,
            })
            .await
            .expect("register corrupt artifact");

        let request_fingerprint = batch_request_fingerprint_for_test(&submission)?;
        let plan = build_submission_plan(&submission, &request_fingerprint)
            .map_err(|error| anyhow!(error.message))?;
        assert_eq!(execution_plan(&plan).proposals.len(), 1);
        let error = match load_proof_artifact_material(
            &runtime,
            &submission.pair.key,
            PipelineKey::ShastaNative,
            PipelineKey::ShastaNative.route(),
            &plan.proposals[0].task_ref,
            ProofArtifactPayload::Proposal,
        )
        .await
        {
            Ok(_) => panic!("corrupt canonical artifact must fail the cache read"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("invalid JSON"),
            "unexpected error: {}",
            error
        );
        Ok(())
    }

    #[tokio::test]
    async fn external_aggregate_inputs_are_persisted_as_artifacts() -> Result<()> {
        let route = native_local_route();
        let runtime = RuntimeManager::new(unique_test_runtime_root("external-agg-inputs"))
            .expect("runtime manager");
        let proof = valid_native_proof();
        let pair = resolved_pair();
        let prepared = prepare_external_aggregate_inputs(
            "taiko_dev/ethereum",
            route,
            "0xfingerprint",
            std::slice::from_ref(&proof),
        )
        .expect("prepare aggregate input artifacts");
        let inputs = prepared.inputs;
        let artifacts = prepared.artifacts;
        let bytes = prepared.bytes;
        let mut metadata = build_task_metadata(
            &pair,
            BuildTaskMetadataParams {
                network: &pair.network,
                l1_network: &pair.l1_network,
                proof_type: route.proof_type(),
                requested_proof_type: Some("native"),
                prover_type: None,
                execution_mode: None,
                aggregate_requested: true,
            },
            &[],
            None,
        );
        metadata.aggregate_input_artifacts = artifacts.clone();
        assert!(runtime.list_proof_artifacts().await?.is_empty());
        let owner = match runtime
            .register_task_if_absent(TaskRegistration {
                task_id: "external-aggregate-root".into(),
                pipeline_key: route.pipeline_key(),
                route: route.route,
                task_kind: "hoodi_aggregate".into(),
                network_pair: "taiko_dev/ethereum".into(),
                artifact_refs: std::iter::once("aggregate-root".into())
                    .chain(artifacts.iter().map(|artifact| artifact.proof_ref.clone()))
                    .collect(),
                metadata: serde_json::to_value(&metadata)?,
                request_fingerprint: "0xfingerprint".into(),
            })
            .await?
        {
            TaskRegistrationOutcome::Created(record) => record.incarnation_id,
            TaskRegistrationOutcome::Existing(_) => anyhow::bail!("unexpected existing task"),
        };
        let record = runtime
            .get_task("external-aggregate-root")
            .await?
            .expect("durable aggregate root");
        runtime
            .upsert_pending_proof_publication(
                "taiko_dev/ethereum",
                route.pipeline_key(),
                route.route,
                &artifacts[0].proof_ref,
                &bytes[0],
            )
            .await?;
        let error = recover_external_aggregate_input_artifacts(&runtime, &record, &metadata)
            .await
            .expect_err("raw pending bytes without durable ownership must be rejected");
        assert!(error.message.contains("no owned pending"));
        assert!(
            runtime
                .remove_pending_proof_publication_if_unowned(
                    "taiko_dev/ethereum",
                    route.pipeline_key(),
                    route.route,
                    &artifacts[0].proof_ref,
                )
                .await?
        );
        runtime
            .checkpoint_pending_proof_publication(
                "taiko_dev/ethereum",
                route.pipeline_key(),
                route.route,
                &artifacts[0].proof_ref,
                &[owner],
                &bytes[0],
            )
            .await?;
        recover_external_aggregate_input_artifacts(&runtime, &record, &metadata)
            .await
            .expect("recover aggregate input from raw pending outbox");
        assert!(
            runtime
                .get_pending_proof_publication(
                    "taiko_dev/ethereum",
                    route.pipeline_key(),
                    route.route,
                    &artifacts[0].proof_ref,
                )
                .await?
                .is_none()
        );
        assert_eq!(artifacts.len(), 1);
        assert_eq!(
            artifacts[0].proof_ref,
            aggregate_input_proof_ref("0xfingerprint", 0)
        );
        assert_eq!(
            inputs,
            aggregate_inputs_from_artifacts(
                "taiko_dev/ethereum",
                PipelineKey::ShastaNative,
                PipelineKey::ShastaNative.route(),
                &artifacts,
            )
        );

        let stored = load_proof_artifact_material(
            &runtime,
            "taiko_dev/ethereum",
            PipelineKey::ShastaNative,
            PipelineKey::ShastaNative.route(),
            &artifacts[0].proof_ref,
            ProofArtifactPayload::AggregateInput,
        )
        .await?
        .expect("stored aggregate input proof");
        assert_eq!(stored.proof, proof);
        recover_external_aggregate_input_artifacts(&runtime, &record, &metadata)
            .await
            .expect("recover exact active aggregate input registration");

        let active = runtime
            .get_proof_artifact(
                "taiko_dev/ethereum",
                route.pipeline_key(),
                route.route,
                &artifacts[0].proof_ref,
            )
            .await?
            .expect("active input registration");
        runtime
            .delete_proof_artifact(
                "taiko_dev/ethereum",
                route.pipeline_key(),
                route.route,
                &artifacts[0].proof_ref,
                active.generation,
                &active.content_hash,
            )
            .await?;
        runtime
            .publish_proof_artifact_bytes(
                "taiko_dev/ethereum",
                route.pipeline_key(),
                route.route,
                &artifacts[0].proof_ref,
                b"different-canonical-input",
            )
            .await?;
        let error = recover_external_aggregate_input_artifacts(&runtime, &record, &metadata)
            .await
            .expect_err("stale active registration must not adopt replacement bytes");
        assert!(error.message.contains("descriptor changed"));
        Ok(())
    }

    #[test]
    fn terminal_external_aggregate_duplicates_do_not_republish_inputs() {
        assert!(existing_external_aggregate_inputs_need_persistence(
            RuntimeRunnerStatus::Allocated
        ));
        assert!(!existing_external_aggregate_inputs_need_persistence(
            RuntimeRunnerStatus::Running
        ));
        assert!(existing_external_aggregate_inputs_need_persistence(
            RuntimeRunnerStatus::Failed
        ));
        assert!(!existing_external_aggregate_inputs_need_persistence(
            RuntimeRunnerStatus::Completed
        ));
        assert!(!existing_external_aggregate_inputs_need_persistence(
            RuntimeRunnerStatus::Cancelled
        ));
    }

    #[tokio::test]
    async fn published_artifact_without_lifecycle_registration_is_not_readable() -> Result<()> {
        let runtime =
            RuntimeManager::new(unique_test_runtime_root("artifact-registration-recovery"))?;
        let proof = valid_native_proof();
        runtime
            .publish_proof_artifact_bytes(
                "taiko_dev/ethereum",
                PipelineKey::ShastaNative,
                PipelineKey::ShastaNative.route(),
                "proposal-recovery",
                &serde_json::to_vec(&proof)?,
            )
            .await?;
        assert!(
            runtime
                .get_proof_artifact(
                    "taiko_dev/ethereum",
                    PipelineKey::ShastaNative,
                    PipelineKey::ShastaNative.route(),
                    "proposal-recovery",
                )
                .await?
                .is_none()
        );

        let recovered = load_proof_artifact_material(
            &runtime,
            "taiko_dev/ethereum",
            PipelineKey::ShastaNative,
            PipelineKey::ShastaNative.route(),
            "proposal-recovery",
            ProofArtifactPayload::Proposal,
        )
        .await?;

        assert!(recovered.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn sp1_network_artifact_without_lifecycle_registration_is_not_readable() -> Result<()> {
        let runtime = RuntimeManager::new(unique_test_runtime_root("sp1-network-artifact-route"))?;
        let route = CanonicalProofRoute::new(
            PipelineRoute::new(crate::config::GuestSystem::Sp1, RunnerKind::Network),
            PipelineKey::ShastaSp1,
        );
        let proof = Proof {
            proof: Some("0xproof".to_string()),
            input: Some(alloy_primitives::B256::ZERO),
            uuid: Some("sp1-proof-id".to_string()),
            extra_data: Some(serde_json::json!({ "sp1": true })),
            ..Proof::default()
        };
        runtime
            .publish_proof_artifact_bytes(
                "taiko_dev/ethereum",
                PipelineKey::ShastaSp1,
                route.route,
                "sp1-network-proposal",
                &serde_json::to_vec(&proof)?,
            )
            .await?;

        let recovered = load_proof_artifact_material(
            &runtime,
            "taiko_dev/ethereum",
            route.pipeline_key(),
            route.route,
            "sp1-network-proposal",
            ProofArtifactPayload::Proposal,
        )
        .await
        .map_err(|err| anyhow!(err.to_string()))?;

        assert!(recovered.is_none());
        Ok(())
    }

    #[test]
    fn v4_proposal_fingerprint_includes_effective_execution_route() {
        let mut local = canonical_submission(native_local_route(), false);
        local.prover_type = Some(ProverType::Mock);
        let mut remote = canonical_submission(sgx_remote_route(), false);
        remote.prover_type = Some(ProverType::Network);

        let base = v4::proposal_request_fingerprint_for_test(&local).expect("fingerprint");
        let after_config_change =
            v4::proposal_request_fingerprint_for_test(&remote).expect("fingerprint");
        assert_ne!(
            base, after_config_change,
            "route/prover_type must isolate work with different execution semantics"
        );

        // Sanity: client-visible request data still discriminates the fingerprint.
        let mut different_proposal = canonical_submission(native_local_route(), false);
        different_proposal.prover_type = Some(ProverType::Mock);
        different_proposal.proposals[0].proposal_id = 999;
        let changed =
            v4::proposal_request_fingerprint_for_test(&different_proposal).expect("fingerprint");
        assert_ne!(
            base, changed,
            "client request data must still change the v4 proposal fingerprint"
        );

        let mut different_l1 = canonical_submission(native_local_route(), false);
        different_l1.proposals[0].l1_inclusion_block_number = 999;
        let changed =
            v4::proposal_request_fingerprint_for_test(&different_l1).expect("fingerprint");
        assert_ne!(
            base, changed,
            "L1 inclusion block must affect the v4 proposal fingerprint"
        );

        let mut different_checkpoint = canonical_submission(native_local_route(), false);
        different_checkpoint.proposals[0].checkpoint = Some(raiko2_primitives::ShastaCheckpoint {
            block_number: 7,
            block_hash: alloy_primitives::B256::repeat_byte(0x11),
            state_root: alloy_primitives::B256::repeat_byte(0x22),
        });
        let changed =
            v4::proposal_request_fingerprint_for_test(&different_checkpoint).expect("fingerprint");
        assert_ne!(
            base, changed,
            "checkpoint must affect the v4 proposal fingerprint"
        );
    }

    #[tokio::test]
    async fn v4_clear_prover_authorizes_before_parsing_body() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "v4-clear-auth-before-body",
        ))?);
        let state = test_state_with_acl(Arc::clone(&runtime), []);
        let request = Request::builder()
            .method("POST")
            .uri("/v4/prover/clear")
            .header("content-type", "application/json")
            .body(Body::from("{"))?;

        let Err(err) = v4::clear_prover(State(state), HeaderMap::new(), request).await else {
            panic!("missing API key should fail before body parsing");
        };
        assert_eq!(err.status, StatusCode::UNAUTHORIZED);
        assert_eq!(err.code, "unauthorized");
        Ok(())
    }

    #[tokio::test]
    async fn v4_submit_is_open_when_submit_acl_feature_is_disabled() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "v4-submit-open-without-submit-acl",
        ))?);
        let state = test_state_with_acl(Arc::clone(&runtime), []);
        let request = Request::builder()
            .method("POST")
            .uri("/v4/proof/proposal")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "proof_type": "risc0",
                    "proposals": [
                        {
                            "proposal_id": 1,
                            "last_anchor_block_number": 10,
                            "l1_inclusion_block_number": 11,
                            "l2_block_number_start": 20,
                            "l2_block_number_end": 21
                        }
                    ]
                })
                .to_string(),
            ))?;

        let Err(err) =
            v4::request_proposal_proof(State(state.clone()), HeaderMap::new(), request).await
        else {
            panic!("unavailable backend should reject after optional ACL");
        };
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "unsupported_proof_type");

        let Err(err) = v4::get_task(
            State(state),
            HeaderMap::new(),
            Path("missing-task".to_string()),
        )
        .await
        else {
            panic!("missing task should reject after optional ACL");
        };
        assert_eq!(err.status, StatusCode::NOT_FOUND);
        assert_eq!(err.code, "task_not_found");
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
            proof_uri: None,
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
            proof_uri: None,
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
        config.runtime.namespace = "resolve-engine-network-config".to_string();
        let state = AppState::from_parts(
            Arc::new(config),
            Arc::new(factory),
            Arc::new(
                RuntimeManager::new(unique_test_runtime_root("resolve-engine-network-runtime"))
                    .expect("runtime manager"),
            ),
        );

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
    fn completed_engine_state_without_readable_artifact_is_failed() {
        let status = require_published_proof(
            EngineStatusView {
                status: ProofStatus::Completed,
                proof: None,
                error: None,
                extra_data: None,
            },
            "proposal-task",
        );

        assert!(matches!(status.status, ProofStatus::Failed));
        assert!(status.proof.is_none());
        assert_eq!(
            status.error.as_deref(),
            Some("proof publication incomplete: artifact proposal-task is not readable")
        );
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
    fn completed_multi_proposal_root_does_not_require_single_proof() {
        let proposals = vec![
            proposal_status(ProofStatus::Completed, Some("0xproposal-a")),
            proposal_status(ProofStatus::Completed, Some("0xproposal-b")),
        ];

        let root =
            resolve_root_task_state(RuntimeRunnerStatus::Completed, &proposals, None, true, None);

        assert!(matches!(root.status, ProofStatus::Completed));
        assert_eq!(root.proof, None);
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
}
