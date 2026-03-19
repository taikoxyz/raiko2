use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::StatusCode,
};
use raiko2_engine::{
    AggregationTaskRequest, EngineTaskId, EngineTaskKey, ProposalStage, ProposalTaskRequest,
};
use raiko2_pipeline::PipelineKey;
use raiko2_primitives::{L2BlockRange, Proof};
use raiko2_queue::{decode_task_id, encode_task_id};
use raiko2_runtime::{RunnerStatus as RuntimeRunnerStatus, TaskRegistration};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;

use super::super::state::{AppState, ProofStatus};
use super::errors::ApiError;
use crate::config::{GuestSystem, PipelineRoute, ResolvedNetworkPair, RunnerKind};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HoodiTaskMetadata {
    network_pair: String,
    network: String,
    l1_network: String,
    proof_type: String,
    aggregate_requested: bool,
    proposals: Vec<HoodiProposalTask>,
    aggregate_task_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HoodiProposalTask {
    proposal_id: u64,
    l1_inclusion_block_number: u64,
    l2_block_numbers: Vec<u64>,
    last_anchor_block_number: u64,
    task_id: String,
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
    status: ProofStatus,
    network: String,
    l1_network: String,
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
    extra_data: Option<Value>,
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

fn validate_batch_request(req: &ShastaProofRequest) -> Result<(), ApiError> {
    if req.proposals.is_empty() {
        return Err(ApiError::bad_request("proposals must not be empty"));
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

fn validate_aggregate_request(req: &AggregateProofRequest) -> Result<(), ApiError> {
    if req.proofs.len() < 2 {
        return Err(ApiError::bad_request(
            "proofs must contain at least 2 entries",
        ));
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

async fn register_batch_task(
    state: &AppState,
    public_task_id: &str,
    selection: &RouteSelection,
    req: &ShastaProofRequest,
    pair: &ResolvedNetworkPair,
    proposal_tasks: &[HoodiProposalTask],
    aggregate_task_id: Option<String>,
) -> Result<(), ApiError> {
    let metadata = HoodiTaskMetadata {
        network_pair: pair.key.clone(),
        network: req.network.clone(),
        l1_network: req.l1_network.clone(),
        proof_type: req.proof_type.as_str().to_string(),
        aggregate_requested: req.aggregate,
        proposals: proposal_tasks.to_vec(),
        aggregate_task_id: aggregate_task_id.clone(),
    };
    let proposal_id = proposal_tasks.first().map(|task| task.proposal_id);
    let proof_ids = proposal_tasks
        .iter()
        .map(|task| task.task_id.clone())
        .collect();

    state
        .runtime
        .register_task(TaskRegistration {
            task_id: public_task_id.to_string(),
            pipeline_key: selection.pipeline_key.as_str().to_string(),
            route: selection.route.to_string(),
            guest_system: selection.route.guest_system.to_string(),
            runner: selection.route.runner.to_string(),
            task_kind: "hoodi_batch".to_string(),
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

fn resolve_root_task_state(
    runner_status: RuntimeRunnerStatus,
    proposals: &[HoodiProposalStatus],
    aggregate: Option<&HoodiAggregateStatus>,
) -> RootTaskState {
    let computed_root_status = summarize_root_status(proposals, aggregate);
    let status = if matches!(runner_status, RuntimeRunnerStatus::Cancelled)
        && !matches!(computed_root_status, ProofStatus::Completed)
    {
        ProofStatus::Cancelled
    } else {
        computed_root_status
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
        .or_else(|| proposals.iter().find_map(|proposal| proposal.error.clone()));
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

pub async fn request_batch_shasta_proof(
    State(state): State<AppState>,
    req: Result<Json<ShastaProofRequest>, JsonRejection>,
) -> Result<Json<HoodiSuccess<RegistrationData>>, ApiError> {
    let Json(req) = req.map_err(|err| ApiError::bad_request(err.to_string()))?;
    validate_batch_request(&req)?;
    let pair = resolved_pair(&state, &req.network, &req.l1_network)?;
    let selection = route_for_proof_type(&state, req.proof_type)?;
    let engine = state
        .pipelines
        .get(&pair.key, selection.pipeline_key)
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "pipeline not available: {}",
                selection.pipeline_key.as_str()
            ))
        })?;
    let prover_args_json = validate_prover_args(&req.prover_args)?;
    let public_task_id = generate_public_task_id();

    info!(
        "Received hoodi shasta batch request: task_id={}, proposals={}, aggregate={}, route={}, pair={}",
        public_task_id,
        req.proposals.len(),
        req.aggregate,
        selection.route,
        pair.key
    );

    let mut proposal_tasks = Vec::with_capacity(req.proposals.len());
    let mut previous = None;
    for proposal in &req.proposals {
        let task_request = ProposalTaskRequest {
            proposal_id: proposal.proposal_id,
            l2_block_range: Some(validate_l2_block_numbers(&proposal.l2_block_numbers)?),
            l1_inclusion_block_number: proposal.l1_inclusion_block_number,
            last_anchor_block_number: proposal.last_anchor_block_number,
            blob_proof_type: req.blob_proof_type.clone(),
            prover: req.prover.clone(),
            graffiti: req.graffiti.clone(),
            prover_args_json: prover_args_json.clone(),
        };
        let task_id = engine
            .submit_proposal_proof_with_dependencies(
                task_request,
                previous.into_iter().collect::<Vec<_>>(),
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

    let aggregate_task_id = if req.aggregate {
        let (request, proof_tasks) = prepare_aggregate_task(&public_task_id, &proposal_tasks)?;
        let task_id = engine
            .submit_aggregation_proof_from_tasks(request, proof_tasks)
            .await
            .map_err(|err| {
                ApiError::internal(format!("failed to enqueue aggregation proof: {err}"))
            })?;
        Some(
            encode_task_id(&task_id)
                .map_err(|err| ApiError::internal(format!("failed to encode task id: {err}")))?,
        )
    } else {
        None
    };

    register_batch_task(
        &state,
        &public_task_id,
        &selection,
        &req,
        &pair,
        &proposal_tasks,
        aggregate_task_id.clone(),
    )
    .await?;

    Ok(Json(HoodiSuccess {
        status: "ok",
        proof_type: req.proof_type.as_str().to_string(),
        data: RegistrationData {
            status: "registered",
            task_id: public_task_id,
        },
    }))
}

pub async fn request_aggregation_proof(
    State(state): State<AppState>,
    req: Result<Json<AggregateProofRequest>, JsonRejection>,
) -> Result<Json<HoodiSuccess<RegistrationData>>, ApiError> {
    let Json(req) = req.map_err(|err| ApiError::bad_request(err.to_string()))?;
    validate_aggregate_request(&req)?;
    let _ = (&req.graffiti, &req.prover, &req.blob_proof_type);
    let pair = resolved_pair(&state, &req.network, &req.l1_network)?;
    let selection = route_for_proof_type(&state, req.proof_type)?;
    validate_external_proofs(selection.pipeline_key, &req.proofs)?;
    let engine = state
        .pipelines
        .get(&pair.key, selection.pipeline_key)
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "pipeline not available: {}",
                selection.pipeline_key.as_str()
            ))
        })?;
    let public_task_id = generate_public_task_id();

    let aggregate_engine_task_id = engine
        .submit_aggregation_proof_from_proofs(
            AggregationTaskRequest {
                request_id: public_task_id.clone(),
                proposal_ids: Vec::new(),
            },
            req.proofs.clone(),
        )
        .await
        .map_err(|err| ApiError::internal(format!("failed to enqueue aggregation proof: {err}")))?;
    let aggregate_task_id = encode_task_id(&aggregate_engine_task_id)
        .map_err(|err| ApiError::internal(format!("failed to encode task id: {err}")))?;

    let metadata = HoodiTaskMetadata {
        network_pair: pair.key.clone(),
        network: req.network.clone(),
        l1_network: req.l1_network.clone(),
        proof_type: req.proof_type.as_str().to_string(),
        aggregate_requested: true,
        proposals: Vec::new(),
        aggregate_task_id: Some(aggregate_task_id),
    };

    state
        .runtime
        .register_task(TaskRegistration {
            task_id: public_task_id.clone(),
            pipeline_key: selection.pipeline_key.as_str().to_string(),
            route: selection.route.to_string(),
            guest_system: selection.route.guest_system.to_string(),
            runner: selection.route.runner.to_string(),
            task_kind: "hoodi_aggregate".to_string(),
            proposal_id: None,
            proof_ids: Vec::new(),
            metadata: serde_json::to_value(metadata).map_err(|err| {
                ApiError::internal(format!("failed to serialize metadata: {err}"))
            })?,
        })
        .await
        .map_err(|err| ApiError::internal(format!("failed to register runtime task: {err}")))?;

    Ok(Json(HoodiSuccess {
        status: "ok",
        proof_type: req.proof_type.as_str().to_string(),
        data: RegistrationData {
            status: "registered",
            task_id: public_task_id,
        },
    }))
}

pub async fn get_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<HoodiSuccess<HoodiTaskData>>, ApiError> {
    let record = state
        .runtime
        .get_task(&id)
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

    let mut proposals = Vec::with_capacity(metadata.proposals.len());
    for (index, proposal) in metadata.proposals.iter().enumerate() {
        let task_id = decode_task_id::<EngineTaskKey>(&proposal.task_id)
            .map_err(|err| ApiError::internal(format!("invalid stored proposal task id: {err}")))?;
        let status = engine
            .get_status(task_id)
            .await
            .map_err(|err| ApiError::internal(format!("failed to read task status: {err}")))?;
        proposals.push(HoodiProposalStatus {
            index,
            proposal_id: proposal.proposal_id,
            task_id: proposal.task_id.clone(),
            status: status
                .as_ref()
                .map_or(ProofStatus::Pending, |status| status.status.clone()),
            l1_inclusion_block_number: proposal.l1_inclusion_block_number,
            l2_block_numbers: proposal.l2_block_numbers.clone(),
            last_anchor_block_number: proposal.last_anchor_block_number,
            proof: status.as_ref().and_then(|status| status.proof.clone()),
            error: status.as_ref().and_then(|status| status.error.clone()),
            extra_data: status.as_ref().and_then(|status| status.extra_data.clone()),
        });
    }

    let aggregate = if let Some(task_id) = metadata.aggregate_task_id.as_ref() {
        let task_id = decode_task_id::<EngineTaskKey>(task_id).map_err(|err| {
            ApiError::internal(format!("invalid stored aggregate task id: {err}"))
        })?;
        let status = engine.get_status(task_id).await.map_err(|err| {
            ApiError::internal(format!("failed to read aggregation status: {err}"))
        })?;
        Some(HoodiAggregateStatus {
            task_id: metadata.aggregate_task_id.clone().expect("checked"),
            status: status
                .as_ref()
                .map_or(ProofStatus::Pending, |status| status.status.clone()),
            proof: status.as_ref().and_then(|status| status.proof.clone()),
            error: status.as_ref().and_then(|status| status.error.clone()),
            extra_data: status.as_ref().and_then(|status| status.extra_data.clone()),
        })
    } else {
        None
    };

    let root_state = resolve_root_task_state(record.runner_status, &proposals, aggregate.as_ref());

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

    Ok(Json(HoodiSuccess {
        status: "ok",
        proof_type: metadata.proof_type,
        data: HoodiTaskData {
            task_id: id,
            status: root_state.status,
            network: metadata.network,
            l1_network: metadata.l1_network,
            current_index: root_state.current_index,
            proposals,
            aggregate,
            proof: root_state.proof,
            error: root_state.error,
        },
    }))
}

pub async fn cancel_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<HoodiSuccess<HoodiTaskData>>, ApiError> {
    let record = state
        .runtime
        .get_task(&id)
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

const fn runtime_status_from_proof_status(status: &ProofStatus) -> RuntimeRunnerStatus {
    match status {
        ProofStatus::Pending | ProofStatus::Proving => RuntimeRunnerStatus::Running,
        ProofStatus::Completed => RuntimeRunnerStatus::Completed,
        ProofStatus::Failed => RuntimeRunnerStatus::Failed,
        ProofStatus::Cancelled => RuntimeRunnerStatus::Cancelled,
    }
}
