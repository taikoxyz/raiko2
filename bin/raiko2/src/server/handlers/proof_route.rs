use alloy_primitives::keccak256;
use axum::http::StatusCode;
use raiko2_pipeline::PipelineKey;
use raiko2_primitives::ProofType;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::super::errors::ApiError;
use super::proof_types::{BatchShastaRequest, HoodiProofType};
use crate::config::{GuestSystem, PipelineRoute, RunnerKind};
use crate::server::state::AppState;

static PUBLIC_TASK_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy)]
pub(super) struct CanonicalProofRoute {
    pub(super) route: PipelineRoute,
    pub(super) pipeline_key: PipelineKey,
    pub(super) proof_type: ProofType,
}

pub(super) enum BatchProofDecision {
    Selected(HoodiProofType),
    NotDrawn,
}

pub(super) fn generate_public_task_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = PUBLIC_TASK_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("task_{nanos:x}_{seq:x}")
}

pub(super) fn route_for_proof_type(
    state: &AppState,
    proof_type: HoodiProofType,
) -> Result<CanonicalProofRoute, ApiError> {
    let route = match proof_type {
        HoodiProofType::Native => PipelineRoute::new(GuestSystem::Native, RunnerKind::Local),
        HoodiProofType::Sp1 => PipelineRoute::new(GuestSystem::Sp1, RunnerKind::Local),
        HoodiProofType::Risc0 => {
            PipelineRoute::new(GuestSystem::Risc0, default_risc0_runner(state))
        }
        HoodiProofType::Sgx => {
            return Err(ApiError::bad_request("proof_type=sgx is not supported"));
        }
        HoodiProofType::ZkAny => {
            return Err(ApiError::bad_request(
                "proof_type=zk_any must be resolved before route selection",
            ));
        }
    };

    let (pipeline_key, proof_type) = match route {
        PipelineRoute {
            guest_system: GuestSystem::Risc0,
            runner: RunnerKind::Local,
        } => (PipelineKey::ShastaRisc0, ProofType::Risc0),
        PipelineRoute {
            guest_system: GuestSystem::Risc0,
            runner: RunnerKind::Boundless,
        } => (PipelineKey::ShastaRisc0Boundless, ProofType::Risc0),
        PipelineRoute {
            guest_system: GuestSystem::Sp1,
            runner: RunnerKind::Local,
        } => (PipelineKey::ShastaSp1, ProofType::Sp1),
        PipelineRoute {
            guest_system: GuestSystem::Native,
            runner: RunnerKind::Local,
        } => (PipelineKey::ShastaNative, ProofType::Native),
        _ => return Err(ApiError::bad_request("unsupported proving route")),
    };

    Ok(CanonicalProofRoute {
        route,
        pipeline_key,
        proof_type,
    })
}

pub(super) fn parse_pipeline_key(raw: &str) -> Result<PipelineKey, ApiError> {
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

pub(super) fn decide_batch_proof_type(
    state: &AppState,
    req: &BatchShastaRequest,
) -> Result<BatchProofDecision, ApiError> {
    if !matches!(req.proof_type, HoodiProofType::ZkAny) {
        return Ok(BatchProofDecision::Selected(req.proof_type));
    }

    let Some(first_proposal) = req.proposals.first() else {
        return Err(ApiError::bad_request("proposals must not be empty"));
    };
    let seed = keccak256(
        format!(
            "proposal:{}/{}",
            first_proposal.proposal_id, first_proposal.l1_inclusion_block_number
        )
        .as_bytes(),
    );
    let mut sampler = state
        .zk_any_sampler
        .lock()
        .map_err(|_| ApiError::internal("failed to lock zk_any sampler"))?;
    let selected = sampler
        .draw(seed)
        .map(HoodiProofType::from_canonical)
        .map_or(BatchProofDecision::NotDrawn, BatchProofDecision::Selected);
    Ok(selected)
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

impl HoodiProofType {
    pub(super) const fn from_canonical(proof_type: ProofType) -> Self {
        match proof_type {
            ProofType::Native => Self::Native,
            ProofType::Sp1 => Self::Sp1,
            ProofType::Sgx => Self::Sgx,
            ProofType::Risc0 => Self::Risc0,
        }
    }
}
