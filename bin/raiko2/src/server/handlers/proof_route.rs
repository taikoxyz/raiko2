use alloy_primitives::keccak256;
use raiko2_pipeline::{PipelineRoute, RunnerKind};
use raiko2_primitives::ProofType;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::super::errors::ApiError;
use super::proof_types::{BatchShastaRequest, HoodiProofType};
use crate::config::GuestSystem;
use crate::server::state::AppState;

static PUBLIC_TASK_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy)]
pub(super) struct CanonicalProofRoute {
    pub(super) route: PipelineRoute,
}

impl CanonicalProofRoute {
    fn new(route: PipelineRoute) -> Result<Self, ApiError> {
        route.pipeline_key().map_err(ApiError::bad_request)?;
        Ok(Self { route })
    }

    pub(super) fn pipeline_key(self) -> raiko2_pipeline::PipelineKey {
        self.route
            .pipeline_key()
            .expect("canonical proof route should always be supported")
    }

    pub(super) const fn proof_type(self) -> ProofType {
        self.route.proof_type()
    }
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
        HoodiProofType::Sgx | HoodiProofType::SgxGeth => {
            return Err(ApiError::bad_request(format!(
                "proof_type={} is not supported",
                proof_type.as_str()
            )));
        }
        HoodiProofType::ZkAny => {
            return Err(ApiError::bad_request(
                "proof_type=zk_any must be resolved before route selection",
            ));
        }
    };

    CanonicalProofRoute::new(route)
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
    default_risc0_runner_for_route(state.config.prover.route())
}

const fn default_risc0_runner_for_route(route: PipelineRoute) -> RunnerKind {
    match route {
        PipelineRoute {
            guest_system: GuestSystem::Risc0,
            runner: RunnerKind::Boundless,
        } => {
            #[cfg(feature = "boundless")]
            {
                RunnerKind::Boundless
            }

            #[cfg(not(feature = "boundless"))]
            {
                RunnerKind::Local
            }
        }
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

#[cfg(test)]
mod tests {
    use super::default_risc0_runner_for_route;
    use crate::config::{GuestSystem, PipelineRoute, RunnerKind};

    #[test]
    fn default_risc0_runner_keeps_local_routes_local() {
        assert_eq!(
            default_risc0_runner_for_route(PipelineRoute::new(
                GuestSystem::Risc0,
                RunnerKind::Local,
            )),
            RunnerKind::Local
        );
    }

    #[cfg(feature = "boundless")]
    #[test]
    fn default_risc0_runner_keeps_boundless_when_feature_enabled() {
        assert_eq!(
            default_risc0_runner_for_route(PipelineRoute::new(
                GuestSystem::Risc0,
                RunnerKind::Boundless,
            )),
            RunnerKind::Boundless
        );
    }

    #[cfg(not(feature = "boundless"))]
    #[test]
    fn default_risc0_runner_falls_back_to_local_when_feature_disabled() {
        assert_eq!(
            default_risc0_runner_for_route(PipelineRoute::new(
                GuestSystem::Risc0,
                RunnerKind::Boundless,
            )),
            RunnerKind::Local
        );
    }
}
