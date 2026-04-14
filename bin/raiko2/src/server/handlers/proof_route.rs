use alloy_primitives::keccak256;
use raiko2_engine::ProverTaskConfig;
use raiko2_pipeline::{PipelineRoute, RunnerKind};
use raiko2_primitives::ProofType;
use raiko2_prover::sp1::{ProverMode as Sp1ProverMode, Sp1RequestContext};

use super::super::errors::ApiError;
use super::proof_types::{BatchProofType, BatchShastaRequest};
use crate::config::GuestSystem;
use crate::server::state::AppState;

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
    Selected(BatchProofType),
    NotDrawn,
}

pub(super) fn public_task_id_from_fingerprint(request_fingerprint: &str) -> String {
    let fingerprint = request_fingerprint
        .strip_prefix("0x")
        .unwrap_or(request_fingerprint);
    format!("task_{fingerprint}")
}

pub(super) fn route_for_proof_type(
    state: &AppState,
    proof_type: BatchProofType,
    prover_config: &ProverTaskConfig,
    sp1_context: Sp1RequestContext,
) -> Result<CanonicalProofRoute, ApiError> {
    let route = match proof_type {
        BatchProofType::Sp1 => PipelineRoute::new(
            GuestSystem::Sp1,
            sp1_runner_for_request(state, prover_config, sp1_context)?,
        ),
        BatchProofType::Risc0 => {
            PipelineRoute::new(GuestSystem::Risc0, default_risc0_runner(state))
        }
        BatchProofType::Sgx => PipelineRoute::new(GuestSystem::Sgx, RunnerKind::Remote),
        BatchProofType::Native | BatchProofType::Boundless | BatchProofType::SgxGeth => {
            return Err(ApiError::bad_request(format!(
                "proof_type={} is not supported",
                proof_type.as_str()
            )));
        }
        BatchProofType::ZkAny => {
            return Err(ApiError::bad_request(
                "proof_type=zk_any must be resolved before route selection",
            ));
        }
    };

    CanonicalProofRoute::new(route)
}

fn sp1_runner_for_request(
    state: &AppState,
    prover_config: &ProverTaskConfig,
    sp1_context: Sp1RequestContext,
) -> Result<RunnerKind, ApiError> {
    let effective_config = state
        .config
        .prover
        .sp1
        .resolve_request_config(prover_config.sp1.as_ref(), sp1_context)
        .map_err(|err| ApiError::bad_request(err.to_string()))?;
    Ok(match effective_config.prover {
        Sp1ProverMode::Network => RunnerKind::Network,
        Sp1ProverMode::Mock | Sp1ProverMode::Local => RunnerKind::Local,
    })
}

pub(super) fn decide_batch_proof_type(
    state: &AppState,
    req: &BatchShastaRequest,
) -> Result<BatchProofDecision, ApiError> {
    if !matches!(req.proof_type, BatchProofType::ZkAny) {
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
        .map(BatchProofType::from_canonical)
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
            runner: RunnerKind::Network,
        } => RunnerKind::Network,
        PipelineRoute {
            guest_system: GuestSystem::Risc0,
            runner,
        } => runner,
        _ => RunnerKind::Local,
    }
}

impl BatchProofType {
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

    #[test]
    fn default_risc0_runner_keeps_network_routes_network() {
        assert_eq!(
            default_risc0_runner_for_route(PipelineRoute::new(
                GuestSystem::Risc0,
                RunnerKind::Network,
            )),
            RunnerKind::Network
        );
    }
}
