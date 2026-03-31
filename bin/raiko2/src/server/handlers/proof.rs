use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::StatusCode,
};
use raiko2_engine::{
    AggregationTaskRequest, EngineTaskId, EngineTaskKey, ProposalStage, ProposalTaskRequest,
};
use raiko2_pipeline::PipelineKey;
use raiko2_primitives::{L2BlockRange, Proof, ProofType};
use raiko2_prover::sp1::{
    ExecutionMode as Sp1ExecutionMode, Sp1ConfigOverrides, Sp1RequestContext,
};
use raiko2_queue::{decode_task_id, encode_task_id};
use raiko2_runtime::{RunnerStatus as RuntimeRunnerStatus, TaskRegistration};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;

use super::super::state::{AppState, EngineHandle, ProofStatus};
use super::errors::ApiError;
use crate::config::{GuestSystem, PipelineRoute, ResolvedNetworkPair, RunnerKind};
use crate::server::task_metadata::{
    HoodiProposalTask, HoodiRuntimeMetadata, HoodiTaskMetadata, HoodiTaskRuntimeMetadata,
};

static PUBLIC_TASK_COUNTER: AtomicU64 = AtomicU64::new(1);

const RESERVED_PROVER_ARG_KEYS: &[&str] = &[
    "proof_type",
    "network",
    "l1_network",
    "aggregate",
    "proposals",
    "proofs",
    "guest_system",
    "runner",
    "route",
    "l1_chain_id",
    "l2_chain_id",
];

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HoodiProofType {
    Native,
    Sp1,
    Risc0,
    ZkAny,
    Sgx,
}

impl HoodiProofType {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Sp1 => "sp1",
            Self::Risc0 => "risc0",
            Self::ZkAny => "zk_any",
            Self::Sgx => "sgx",
        }
    }

    const fn canonical_proof_type(self) -> Option<ProofType> {
        match self {
            Self::Native => Some(ProofType::Native),
            Self::Sp1 => Some(ProofType::Sp1),
            Self::Risc0 | Self::ZkAny => Some(ProofType::Risc0),
            Self::Sgx => None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ShastaProofRequest {
    pub proposals: Vec<ShastaProposal>,
    #[serde(default)]
    pub aggregate: bool,
    pub proof_type: HoodiProofType,
    pub network: String,
    pub l1_network: String,
    #[serde(default)]
    pub graffiti: Option<String>,
    #[serde(default)]
    pub prover: Option<String>,
    #[serde(default)]
    pub blob_proof_type: Option<String>,
    #[serde(flatten)]
    pub prover_args: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ShastaProposal {
    pub proposal_id: u64,
    #[serde(default)]
    pub checkpoint: Option<Value>,
    pub l1_inclusion_block_number: u64,
    pub l2_block_numbers: Vec<u64>,
    pub last_anchor_block_number: u64,
}

#[derive(Debug, Deserialize)]
pub struct AggregateProofRequest {
    pub proofs: Vec<Proof>,
    pub proof_type: HoodiProofType,
    pub network: String,
    pub l1_network: String,
    #[serde(default)]
    pub graffiti: Option<String>,
    #[serde(default)]
    pub prover: Option<String>,
    #[serde(default)]
    pub blob_proof_type: Option<String>,
    #[serde(flatten)]
    pub prover_args: BTreeMap<String, Value>,
}

#[derive(Serialize)]
pub(crate) struct HoodiSuccess<T> {
    status: &'static str,
    proof_type: String,
    data: T,
}

#[derive(Serialize)]
pub(crate) struct RegistrationData {
    status: &'static str,
    task_id: String,
}

#[derive(Serialize)]
pub(crate) struct HoodiTaskData {
    task_id: String,
    route: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    execution_mode: Option<String>,
    status: ProofStatus,
    network: String,
    l1_network: String,
    runtime: HoodiRootRuntimeView,
    current_index: Option<usize>,
    proposals: Vec<HoodiProposalStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    aggregate: Option<HoodiAggregateStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    proof: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct HoodiProposalStatus {
    index: usize,
    proposal_id: u64,
    task_id: String,
    status: ProofStatus,
    l1_inclusion_block_number: u64,
    l2_block_numbers: Vec<u64>,
    last_anchor_block_number: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    proof: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime: Option<HoodiTaskRuntimeView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extra_data: Option<Value>,
}

#[derive(Serialize)]
pub(crate) struct HoodiAggregateStatus {
    task_id: String,
    status: ProofStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    proof: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime: Option<HoodiTaskRuntimeView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extra_data: Option<Value>,
}

#[derive(Serialize)]
pub(crate) struct HoodiRootRuntimeView {
    runner_status: RuntimeRunnerStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_event: Option<String>,
    updated_at: i64,
    engine_state_present: bool,
}

#[derive(Serialize)]
pub(crate) struct HoodiTaskRuntimeView {
    updated_at: i64,
    engine_state_present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    remote_tx_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deployment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    offchain: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    quoted_mcycles_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    evaluated_mcycles_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sp1_network_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sp1_fulfillment_strategy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sp1_skip_simulation: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sp1_cycle_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sp1_timeout_secs: Option<u64>,
}

struct RouteSelection {
    route: PipelineRoute,
    pipeline_key: PipelineKey,
}

struct RootTaskState {
    status: ProofStatus,
    proof: Option<String>,
    error: Option<String>,
    current_index: Option<usize>,
}

struct TaskLookup {
    record: raiko2_runtime::RuntimeTaskRecord,
    metadata: HoodiTaskMetadata,
    engine: Arc<dyn EngineHandle>,
}

struct SubmissionContext {
    pair: ResolvedNetworkPair,
    selection: RouteSelection,
    engine: Arc<dyn EngineHandle>,
    proof_type: ProofType,
}

struct TaskMetadataParams {
    network: String,
    l1_network: String,
    proof_type: ProofType,
    api_proof_type: Option<String>,
    execution_mode: Option<Sp1ExecutionMode>,
    aggregate_requested: bool,
    proposals: Vec<HoodiProposalTask>,
    aggregate_task_id: Option<String>,
}

fn default_risc0_runner(state: &AppState) -> RunnerKind {
    match state.config.prover.route() {
        PipelineRoute {
            guest_system: GuestSystem::Risc0,
            runner,
        } => runner,
        _ => RunnerKind::Local,
    }
}

fn route_for_proof_type(
    state: &AppState,
    proof_type: HoodiProofType,
) -> Result<RouteSelection, ApiError> {
    let route = match proof_type {
        HoodiProofType::Native => PipelineRoute::new(GuestSystem::Native, RunnerKind::Local),
        HoodiProofType::Sp1 => PipelineRoute::new(GuestSystem::Sp1, RunnerKind::Local),
        HoodiProofType::Risc0 | HoodiProofType::ZkAny => {
            PipelineRoute::new(GuestSystem::Risc0, default_risc0_runner(state))
        }
        HoodiProofType::Sgx => {
            return Err(ApiError::bad_request("proof_type=sgx is not supported"));
        }
    };

    let pipeline_key = match route {
        PipelineRoute {
            guest_system: GuestSystem::Risc0,
            runner: RunnerKind::Local,
        } => PipelineKey::ShastaRisc0,
        PipelineRoute {
            guest_system: GuestSystem::Risc0,
            runner: RunnerKind::Boundless,
        } => PipelineKey::ShastaRisc0Boundless,
        PipelineRoute {
            guest_system: GuestSystem::Sp1,
            runner: RunnerKind::Local,
        } => PipelineKey::ShastaSp1,
        PipelineRoute {
            guest_system: GuestSystem::Native,
            runner: RunnerKind::Local,
        } => PipelineKey::ShastaNative,
        _ => {
            return Err(ApiError::bad_request("unsupported proving route"));
        }
    };

    Ok(RouteSelection {
        route,
        pipeline_key,
    })
}

fn parse_pipeline_key(raw: &str) -> Result<PipelineKey, ApiError> {
    match raw {
        "shasta-risc0-local" => Ok(PipelineKey::ShastaRisc0),
        "shasta-risc0-boundless" => Ok(PipelineKey::ShastaRisc0Boundless),
        "shasta-sp1-local" => Ok(PipelineKey::ShastaSp1),
        "shasta-native-local" => Ok(PipelineKey::ShastaNative),
        _ => Err(ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("unknown pipeline key '{raw}'"),
        }),
    }
}

fn generate_public_task_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = PUBLIC_TASK_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("task_{nanos:x}_{seq:x}")
}

fn validate_prover_args(prover_args: &BTreeMap<String, Value>) -> Result<Option<String>, ApiError> {
    for key in prover_args.keys() {
        if RESERVED_PROVER_ARG_KEYS.contains(&key.as_str()) {
            return Err(ApiError::bad_request(format!(
                "prover_args key '{key}' cannot override runner or spec selection"
            )));
        }
    }
    if prover_args.is_empty() {
        Ok(None)
    } else {
        serde_json::to_string(prover_args)
            .map(Some)
            .map_err(|err| ApiError::internal(format!("failed to serialize prover_args: {err}")))
    }
}

fn parse_sp1_prover_args(
    prover_args: &BTreeMap<String, Value>,
) -> Result<Option<Sp1ConfigOverrides>, ApiError> {
    prover_args
        .get("sp1")
        .cloned()
        .map(serde_json::from_value::<Sp1ConfigOverrides>)
        .transpose()
        .map_err(|err| ApiError::bad_request(format!("invalid sp1 prover args: {err}")))
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

fn validate_sp1_request(
    base_config: &raiko2_prover::sp1::Sp1Config,
    overrides: Option<&Sp1ConfigOverrides>,
    context: Sp1RequestContext,
) -> Result<(), ApiError> {
    base_config
        .resolve_request_config(overrides, context)
        .map(|_| ())
        .map_err(|err| ApiError::bad_request(err.to_string()))
}

fn validate_batch_request(state: &AppState, req: &ShastaProofRequest) -> Result<(), ApiError> {
    if req.proposals.is_empty() {
        return Err(ApiError::bad_request("proposals must not be empty"));
    }
    let sp1_args = parse_sp1_prover_args(&req.prover_args)?;
    if sp1_args.is_some() && !matches!(req.proof_type, HoodiProofType::Sp1) {
        return Err(ApiError::bad_request(
            "sp1 prover args require proof_type=sp1",
        ));
    }
    if matches!(req.proof_type, HoodiProofType::Sp1) {
        validate_sp1_request(
            &state.config.prover.sp1,
            sp1_args.as_ref(),
            Sp1RequestContext::ProposalBatch {
                aggregate: req.aggregate,
            },
        )?;
    }
    for proposal in &req.proposals {
        if proposal.checkpoint.is_some() {
            return Err(ApiError::bad_request(
                "proposal.checkpoint is not supported in this version",
            ));
        }
        let _ = validate_l2_block_numbers(&proposal.l2_block_numbers)?;
    }
    let _ = validate_prover_args(&req.prover_args)?;
    Ok(())
}

fn validate_aggregate_request(
    state: &AppState,
    req: &AggregateProofRequest,
) -> Result<(), ApiError> {
    if req.proofs.len() < 2 {
        return Err(ApiError::bad_request(
            "proofs must contain at least 2 entries",
        ));
    }
    let sp1_args = parse_sp1_prover_args(&req.prover_args)?;
    if sp1_args.is_some() && !matches!(req.proof_type, HoodiProofType::Sp1) {
        return Err(ApiError::bad_request(
            "sp1 prover args require proof_type=sp1",
        ));
    }
    if matches!(req.proof_type, HoodiProofType::Sp1) {
        validate_sp1_request(
            &state.config.prover.sp1,
            sp1_args.as_ref(),
            Sp1RequestContext::Aggregation,
        )?;
    }
    let _ = validate_prover_args(&req.prover_args)?;
    Ok(())
}

fn validate_external_proofs(pipeline_key: PipelineKey, proofs: &[Proof]) -> Result<(), ApiError> {
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
    pair: &ResolvedNetworkPair,
    selection: &RouteSelection,
) -> Result<Arc<dyn EngineHandle>, ApiError> {
    state
        .pipelines
        .get(&pair.key, selection.pipeline_key)
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "pipeline not available: {}",
                selection.pipeline_key.as_str()
            ))
        })
}

fn resolve_submission_context(
    state: &AppState,
    network: &str,
    l1_network: &str,
    proof_type: HoodiProofType,
) -> Result<SubmissionContext, ApiError> {
    let pair = resolved_pair(state, network, l1_network)?;
    let selection = route_for_proof_type(state, proof_type)?;
    let proof_type = proof_type
        .canonical_proof_type()
        .ok_or_else(|| ApiError::bad_request("proof_type=sgx is not supported"))?;
    let engine = resolve_engine(state, &pair, &selection)?;
    Ok(SubmissionContext {
        pair,
        selection,
        engine,
        proof_type,
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
    .map(|stage| {
        EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: *pipeline,
            request: request.clone(),
            stage,
        })
    })
    .collect()
}

fn summarize_proposal_task_state(
    stages: &[Option<super::super::state::EngineStatusView>],
) -> super::super::state::EngineStatusView {
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
                return super::super::state::EngineStatusView {
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
                    return super::super::state::EngineStatusView {
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
        super::super::state::EngineStatusView {
            status: ProofStatus::Proving,
            proof: None,
            error: pending_error,
            extra_data: None,
        }
    } else {
        super::super::state::EngineStatusView {
            status: ProofStatus::Pending,
            proof: None,
            error: pending_error,
            extra_data: None,
        }
    }
}

async fn register_batch_task(
    state: &AppState,
    public_task_id: &str,
    selection: &RouteSelection,
    proposal_tasks: &[HoodiProposalTask],
    metadata: HoodiTaskMetadata,
) -> Result<(), ApiError> {
    register_runtime_task(
        state,
        public_task_id,
        selection,
        "hoodi_batch",
        proposal_tasks.first().map(|task| task.proposal_id),
        proposal_tasks
            .iter()
            .map(|task| task.task_id.clone())
            .collect(),
        metadata,
    )
    .await
}

async fn register_aggregate_task(
    state: &AppState,
    public_task_id: &str,
    selection: &RouteSelection,
    metadata: HoodiTaskMetadata,
) -> Result<(), ApiError> {
    register_runtime_task(
        state,
        public_task_id,
        selection,
        "hoodi_aggregate",
        None,
        Vec::new(),
        metadata,
    )
    .await
}

fn execution_mode_for_sp1(
    proof_type: HoodiProofType,
    sp1_args: Option<&Sp1ConfigOverrides>,
) -> Option<Sp1ExecutionMode> {
    matches!(proof_type, HoodiProofType::Sp1).then(|| {
        sp1_args
            .and_then(|args| args.mode)
            .unwrap_or(Sp1ExecutionMode::Prove)
    })
}

fn api_proof_type_alias(
    proof_type: HoodiProofType,
    canonical_proof_type: ProofType,
) -> Option<String> {
    let proof_type = proof_type.as_str();
    let canonical_proof_type = canonical_proof_type.to_string();
    (proof_type != canonical_proof_type.as_str()).then(|| proof_type.to_string())
}

fn build_task_metadata(
    pair: &ResolvedNetworkPair,
    params: TaskMetadataParams,
) -> HoodiTaskMetadata {
    HoodiTaskMetadata {
        network_pair: pair.key.clone(),
        network: params.network,
        l1_network: params.l1_network,
        proof_type: params.proof_type,
        api_proof_type: params.api_proof_type,
        execution_mode: params.execution_mode,
        aggregate_requested: params.aggregate_requested,
        proposals: params.proposals,
        aggregate_task_id: params.aggregate_task_id,
        runtime: HoodiRuntimeMetadata::default(),
    }
}

async fn register_runtime_task(
    state: &AppState,
    public_task_id: &str,
    selection: &RouteSelection,
    task_kind: &str,
    proposal_id: Option<u64>,
    proof_ids: Vec<String>,
    metadata: HoodiTaskMetadata,
) -> Result<(), ApiError> {
    state
        .runtime
        .register_task(TaskRegistration {
            task_id: public_task_id.to_string(),
            pipeline_key: selection.pipeline_key.as_str().to_string(),
            route: selection.route.to_string(),
            guest_system: selection.route.guest_system.to_string(),
            runner: selection.route.runner.to_string(),
            task_kind: task_kind.to_string(),
            proposal_id,
            proof_ids,
            metadata: serde_json::to_value(metadata).map_err(|err| {
                ApiError::internal(format!("failed to serialize metadata: {err}"))
            })?,
        })
        .await
        .map(|_| ())
        .map_err(|err| ApiError::internal(format!("failed to register runtime task: {err}")))
}

fn build_proposal_task_request(
    req: &ShastaProofRequest,
    proposal: &ShastaProposal,
    prover_args_json: Option<&str>,
) -> Result<ProposalTaskRequest, ApiError> {
    Ok(ProposalTaskRequest {
        proposal_id: proposal.proposal_id,
        l2_block_range: Some(validate_l2_block_numbers(&proposal.l2_block_numbers)?),
        l1_inclusion_block_number: proposal.l1_inclusion_block_number,
        last_anchor_block_number: proposal.last_anchor_block_number,
        blob_proof_type: req.blob_proof_type.clone(),
        prover: req.prover.clone(),
        graffiti: req.graffiti.clone(),
        prover_args_json: prover_args_json.map(str::to_string),
    })
}

async fn enqueue_batch_proposals(
    engine: &Arc<dyn EngineHandle>,
    req: &ShastaProofRequest,
    prover_args_json: Option<&str>,
) -> Result<Vec<HoodiProposalTask>, ApiError> {
    let mut proposal_tasks = Vec::with_capacity(req.proposals.len());
    let mut previous = None;

    for proposal in &req.proposals {
        let task_request = build_proposal_task_request(req, proposal, prover_args_json)?;
        let task_id = engine
            .submit_proposal_proof_with_dependencies(
                task_request,
                previous.iter().cloned().collect(),
            )
            .await
            .map_err(|err| {
                ApiError::internal(format!("failed to enqueue proposal proof: {err}"))
            })?;
        let encoded_task_id = encode_task_id(&task_id)
            .map_err(|err| ApiError::internal(format!("failed to encode task id: {err}")))?;
        proposal_tasks.push(HoodiProposalTask {
            proposal_id: proposal.proposal_id,
            l1_inclusion_block_number: proposal.l1_inclusion_block_number,
            l2_block_numbers: proposal.l2_block_numbers.clone(),
            last_anchor_block_number: proposal.last_anchor_block_number,
            task_id: encoded_task_id,
        });
        previous = Some(task_id);
    }

    Ok(proposal_tasks)
}

async fn enqueue_aggregate_from_tasks(
    engine: &Arc<dyn EngineHandle>,
    public_task_id: &str,
    proposal_tasks: &[HoodiProposalTask],
) -> Result<String, ApiError> {
    let (request, proof_tasks) = prepare_aggregate_task(public_task_id, proposal_tasks)?;
    let task_id = engine
        .submit_aggregation_proof_from_tasks(request, proof_tasks)
        .await
        .map_err(|err| ApiError::internal(format!("failed to enqueue aggregation proof: {err}")))?;
    encode_task_id(&task_id)
        .map_err(|err| ApiError::internal(format!("failed to encode task id: {err}")))
}

async fn enqueue_aggregate_from_proofs(
    engine: &Arc<dyn EngineHandle>,
    public_task_id: &str,
    proofs: Vec<Proof>,
) -> Result<String, ApiError> {
    let task_id = engine
        .submit_aggregation_proof_from_proofs(
            AggregationTaskRequest {
                request_id: public_task_id.to_string(),
                proposal_ids: Vec::new(),
            },
            proofs,
        )
        .await
        .map_err(|err| ApiError::internal(format!("failed to enqueue aggregation proof: {err}")))?;
    encode_task_id(&task_id)
        .map_err(|err| ApiError::internal(format!("failed to encode task id: {err}")))
}

fn registration_response(
    proof_type: HoodiProofType,
    public_task_id: String,
) -> Json<HoodiSuccess<RegistrationData>> {
    Json(HoodiSuccess {
        status: "ok",
        proof_type: proof_type.as_str().to_string(),
        data: RegistrationData {
            status: "registered",
            task_id: public_task_id,
        },
    })
}

fn prepare_aggregate_task(
    public_task_id: &str,
    proposal_tasks: &[HoodiProposalTask],
) -> Result<(AggregationTaskRequest, Vec<EngineTaskId>), ApiError> {
    let request = AggregationTaskRequest {
        request_id: public_task_id.to_string(),
        proposal_ids: proposal_tasks.iter().map(|task| task.proposal_id).collect(),
    };
    let proof_tasks = proposal_tasks
        .iter()
        .map(|task| {
            decode_task_id::<EngineTaskKey>(&task.task_id).map_err(|err| {
                ApiError::internal(format!("failed to decode generated task id: {err}"))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok((request, proof_tasks))
}

const fn runner_status_to_proof_status(
    runner_status: RuntimeRunnerStatus,
    has_runtime_progress: bool,
) -> ProofStatus {
    match runner_status {
        RuntimeRunnerStatus::Cancelled => ProofStatus::Cancelled,
        RuntimeRunnerStatus::Failed => ProofStatus::Failed,
        RuntimeRunnerStatus::Completed => ProofStatus::Completed,
        RuntimeRunnerStatus::Running if has_runtime_progress => ProofStatus::Proving,
        RuntimeRunnerStatus::Allocated | RuntimeRunnerStatus::Running => ProofStatus::Pending,
    }
}

fn failed_runtime_error(status: &ProofStatus, runtime_error: Option<&str>) -> Option<String> {
    matches!(status, ProofStatus::Failed)
        .then_some(runtime_error.map(str::to_string))
        .flatten()
}

fn resolve_root_task_state(
    runner_status: RuntimeRunnerStatus,
    proposals: &[HoodiProposalStatus],
    aggregate: Option<&HoodiAggregateStatus>,
    runtime_has_progress: bool,
    runtime_error: Option<&str>,
) -> RootTaskState {
    let computed_root_status = summarize_root_status(proposals, aggregate);
    let status = match computed_root_status {
        ProofStatus::Pending => runner_status_to_proof_status(runner_status, runtime_has_progress),
        status => {
            if matches!(runner_status, RuntimeRunnerStatus::Cancelled)
                && !matches!(status, ProofStatus::Completed)
            {
                ProofStatus::Cancelled
            } else {
                status
            }
        }
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

fn fallback_status_without_engine(
    status: super::super::state::EngineStatusView,
    runner_status: RuntimeRunnerStatus,
    has_runtime_progress: bool,
    runtime_error: Option<&str>,
) -> super::super::state::EngineStatusView {
    if !matches!(status.status, ProofStatus::Pending) {
        return status;
    }

    let next_status = runner_status_to_proof_status(runner_status, has_runtime_progress);
    let error = failed_runtime_error(&next_status, runtime_error).or(status.error);
    super::super::state::EngineStatusView {
        status: next_status,
        proof: None,
        error,
        extra_data: None,
    }
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
    let engine = state
        .pipelines
        .get(&metadata.network_pair, pipeline_key)
        .ok_or_else(|| {
            ApiError::not_found(format!("pipeline not available: {}", record.pipeline_key))
        })?;

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
        let mut stage_statuses = Vec::with_capacity(3);

        for stage_id in proposal_task_chain_ids(&task_id) {
            let status = engine
                .get_status(stage_id)
                .await
                .map_err(|err| ApiError::internal(format!("failed to read task status: {err}")))?;
            stage_statuses.push(status);
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
        status.map_or(
            super::super::state::EngineStatusView {
                status: ProofStatus::Pending,
                proof: None,
                error: None,
                extra_data: None,
            },
            |status| status,
        ),
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

fn build_task_data(
    id: String,
    record: &raiko2_runtime::RuntimeTaskRecord,
    metadata: &HoodiTaskMetadata,
    root_state: RootTaskState,
    proposals: Vec<HoodiProposalStatus>,
    aggregate: Option<HoodiAggregateStatus>,
    root_engine_state_present: bool,
) -> HoodiTaskData {
    HoodiTaskData {
        task_id: id,
        route: record.route.clone(),
        execution_mode: metadata.execution_mode_str(),
        status: root_state.status,
        network: metadata.network.clone(),
        l1_network: metadata.l1_network.clone(),
        runtime: root_runtime_view(record, metadata, root_engine_state_present),
        current_index: root_state.current_index,
        proposals,
        aggregate,
        proof: root_state.proof,
        error: root_state.error,
    }
}

async fn cancel_registered_tasks(
    engine: &Arc<dyn EngineHandle>,
    metadata: &HoodiTaskMetadata,
) -> Result<(), ApiError> {
    for proposal in &metadata.proposals {
        let task_id = decode_task_id::<EngineTaskKey>(&proposal.task_id)
            .map_err(|err| ApiError::internal(format!("invalid stored proposal task id: {err}")))?;
        for stage_task_id in proposal_task_chain_ids(&task_id) {
            let _ = engine.cancel(stage_task_id).await;
        }
    }

    if let Some(task_id) = &metadata.aggregate_task_id {
        let task_id = decode_task_id::<EngineTaskKey>(task_id).map_err(|err| {
            ApiError::internal(format!("invalid stored aggregate task id: {err}"))
        })?;
        let _ = engine.cancel(task_id).await;
    }

    Ok(())
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

pub async fn request_batch_shasta_proof(
    State(state): State<AppState>,
    req: Result<Json<ShastaProofRequest>, JsonRejection>,
) -> Result<Json<HoodiSuccess<RegistrationData>>, ApiError> {
    let Json(req) = req.map_err(|err| ApiError::bad_request(err.to_string()))?;
    validate_batch_request(&state, &req)?;
    let SubmissionContext {
        pair,
        selection,
        engine,
        proof_type,
    } = resolve_submission_context(&state, &req.network, &req.l1_network, req.proof_type)?;
    let prover_args_json = validate_prover_args(&req.prover_args)?;
    let sp1_args = parse_sp1_prover_args(&req.prover_args)?;
    let public_task_id = generate_public_task_id();

    info!(
        "Received hoodi shasta batch request: task_id={}, proposals={}, aggregate={}, route={}, pair={}",
        public_task_id,
        req.proposals.len(),
        req.aggregate,
        selection.route,
        pair.key
    );

    let proposal_tasks =
        enqueue_batch_proposals(&engine, &req, prover_args_json.as_deref()).await?;

    let aggregate_task_id = if req.aggregate {
        Some(enqueue_aggregate_from_tasks(&engine, &public_task_id, &proposal_tasks).await?)
    } else {
        None
    };
    let metadata = build_task_metadata(
        &pair,
        TaskMetadataParams {
            network: req.network.clone(),
            l1_network: req.l1_network.clone(),
            proof_type,
            api_proof_type: api_proof_type_alias(req.proof_type, proof_type),
            execution_mode: execution_mode_for_sp1(req.proof_type, sp1_args.as_ref()),
            aggregate_requested: req.aggregate,
            proposals: proposal_tasks.clone(),
            aggregate_task_id: aggregate_task_id.clone(),
        },
    );

    register_batch_task(
        &state,
        &public_task_id,
        &selection,
        &proposal_tasks,
        metadata,
    )
    .await?;

    Ok(registration_response(req.proof_type, public_task_id))
}

pub async fn request_aggregation_proof(
    State(state): State<AppState>,
    req: Result<Json<AggregateProofRequest>, JsonRejection>,
) -> Result<Json<HoodiSuccess<RegistrationData>>, ApiError> {
    let Json(req) = req.map_err(|err| ApiError::bad_request(err.to_string()))?;
    validate_aggregate_request(&state, &req)?;
    let _ = (&req.graffiti, &req.prover, &req.blob_proof_type);
    let SubmissionContext {
        pair,
        selection,
        engine,
        proof_type,
    } = resolve_submission_context(&state, &req.network, &req.l1_network, req.proof_type)?;
    validate_external_proofs(selection.pipeline_key, &req.proofs)?;
    let public_task_id = generate_public_task_id();
    let aggregate_task_id =
        enqueue_aggregate_from_proofs(&engine, &public_task_id, req.proofs.clone()).await?;
    let metadata = build_task_metadata(
        &pair,
        TaskMetadataParams {
            network: req.network.clone(),
            l1_network: req.l1_network.clone(),
            proof_type,
            api_proof_type: api_proof_type_alias(req.proof_type, proof_type),
            execution_mode: execution_mode_for_sp1(req.proof_type, None),
            aggregate_requested: true,
            proposals: Vec::new(),
            aggregate_task_id: Some(aggregate_task_id.clone()),
        },
    );

    register_aggregate_task(&state, &public_task_id, &selection, metadata).await?;

    Ok(registration_response(req.proof_type, public_task_id))
}

pub async fn get_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<HoodiSuccess<HoodiTaskData>>, ApiError> {
    let TaskLookup {
        record,
        metadata,
        engine,
    } = load_task_lookup(&state, &id).await?;
    let (proposals, proposal_engine_state_present) =
        load_proposal_statuses(&engine, &metadata, &record).await?;
    let (aggregate, aggregate_engine_state_present) =
        load_aggregate_status(&engine, &metadata, &record).await?;
    let root_engine_state_present = proposal_engine_state_present || aggregate_engine_state_present;

    let root_state = resolve_root_task_state(
        record.runner_status,
        &proposals,
        aggregate.as_ref(),
        metadata.has_runtime_progress(),
        record.error.as_deref(),
    );

    state
        .runtime
        .sync_status(
            &id,
            runtime_status_from_proof_status(&root_state.status),
            root_state.error.clone(),
            None,
        )
        .await
        .map_err(|err| ApiError::internal(format!("failed to sync runtime status: {err}")))?;
    let proof_type = metadata.response_proof_type();
    let data = build_task_data(
        id,
        &record,
        &metadata,
        root_state,
        proposals,
        aggregate,
        root_engine_state_present,
    );

    Ok(Json(HoodiSuccess {
        status: "ok",
        proof_type,
        data,
    }))
}

pub async fn cancel_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<HoodiSuccess<HoodiTaskData>>, ApiError> {
    let TaskLookup {
        metadata, engine, ..
    } = load_task_lookup(&state, &id).await?;
    cancel_registered_tasks(&engine, &metadata).await?;

    state
        .runtime
        .sync_status(&id, RuntimeRunnerStatus::Cancelled, None, None)
        .await
        .map_err(|err| ApiError::internal(format!("failed to sync runtime cancellation: {err}")))?;

    get_task(State(state), Path(id)).await
}

fn summarize_root_status(
    proposals: &[HoodiProposalStatus],
    aggregate: Option<&HoodiAggregateStatus>,
) -> ProofStatus {
    if proposals.is_empty() {
        return aggregate.map_or(ProofStatus::Pending, |aggregate| aggregate.status.clone());
    }
    if root_has_status(proposals, aggregate, is_failed) {
        return ProofStatus::Failed;
    }
    if root_has_status(proposals, aggregate, is_proving) {
        return ProofStatus::Proving;
    }
    if proposals_all_status(proposals, is_completed) {
        return aggregate.map_or(ProofStatus::Completed, |aggregate| aggregate.status.clone());
    }
    if proposals_have_status(proposals, is_cancelled) {
        return ProofStatus::Cancelled;
    }
    ProofStatus::Pending
}

fn root_has_status(
    proposals: &[HoodiProposalStatus],
    aggregate: Option<&HoodiAggregateStatus>,
    predicate: fn(&ProofStatus) -> bool,
) -> bool {
    proposals_have_status(proposals, predicate)
        || aggregate.is_some_and(|aggregate| predicate(&aggregate.status))
}

fn proposals_have_status(
    proposals: &[HoodiProposalStatus],
    predicate: fn(&ProofStatus) -> bool,
) -> bool {
    proposals.iter().any(|proposal| predicate(&proposal.status))
}

fn proposals_all_status(
    proposals: &[HoodiProposalStatus],
    predicate: fn(&ProofStatus) -> bool,
) -> bool {
    proposals.iter().all(|proposal| predicate(&proposal.status))
}

const fn is_failed(status: &ProofStatus) -> bool {
    matches!(status, ProofStatus::Failed)
}

const fn is_proving(status: &ProofStatus) -> bool {
    matches!(status, ProofStatus::Proving)
}

const fn is_completed(status: &ProofStatus) -> bool {
    matches!(status, ProofStatus::Completed)
}

const fn is_cancelled(status: &ProofStatus) -> bool {
    matches!(status, ProofStatus::Cancelled)
}

const fn runtime_status_from_proof_status(status: &ProofStatus) -> RuntimeRunnerStatus {
    match status {
        ProofStatus::Pending | ProofStatus::Proving => RuntimeRunnerStatus::Running,
        ProofStatus::Completed => RuntimeRunnerStatus::Completed,
        ProofStatus::Failed => RuntimeRunnerStatus::Failed,
        ProofStatus::Cancelled => RuntimeRunnerStatus::Cancelled,
    }
}
