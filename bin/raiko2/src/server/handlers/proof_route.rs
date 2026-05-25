use alloy_primitives::keccak256;
use raiko2_engine::ProverTaskConfig;
use raiko2_pipeline::{PipelineKey, PipelineRoute, RunnerKind};
use raiko2_primitives::ProofType;
use raiko2_prover::sp1::{ProverMode as Sp1ProverMode, Sp1RequestContext};

use super::super::errors::ApiError;
use super::proof_types::{BatchProofType, BatchShastaRequest};
use crate::config::GuestSystem;
use crate::server::state::AppState;

#[derive(Debug, Clone, Copy)]
pub(super) struct CanonicalProofRoute {
    pub(super) route: PipelineRoute,
    pipeline_key: PipelineKey,
    proof_type: ProofType,
}

impl CanonicalProofRoute {
    pub(super) const fn new(
        route: PipelineRoute,
        pipeline_key: PipelineKey,
        proof_type: ProofType,
    ) -> Self {
        Self {
            route,
            pipeline_key,
            proof_type,
        }
    }

    pub(super) const fn pipeline_key(self) -> PipelineKey {
        self.pipeline_key
    }

    pub(super) const fn proof_type(self) -> ProofType {
        self.proof_type
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
    validate_hosted_proof_type(state.config.prover.route(), proof_type)?;

    let route = match proof_type {
        BatchProofType::Sp1 => PipelineRoute::new(
            GuestSystem::Sp1,
            sp1_runner_for_request(state, prover_config, sp1_context)?,
        ),
        BatchProofType::Risc0 => {
            PipelineRoute::new(GuestSystem::Risc0, default_risc0_runner(state))
        }
        BatchProofType::Tdx => PipelineRoute::new(GuestSystem::Tdx, RunnerKind::Local),
        BatchProofType::Sgx | BatchProofType::SgxGeth => {
            PipelineRoute::new(GuestSystem::Sgx, RunnerKind::Remote)
        }
        BatchProofType::Native => native_route_for_request(state)?,
        BatchProofType::Boundless => {
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

    let pipeline_key = match proof_type {
        BatchProofType::Sp1 => PipelineKey::ShastaSp1,
        BatchProofType::Risc0 => match route {
            PipelineRoute {
                guest_system: GuestSystem::Risc0,
                runner: RunnerKind::Local,
            } => PipelineKey::ShastaRisc0,
            PipelineRoute {
                guest_system: GuestSystem::Risc0,
                runner: RunnerKind::Network,
            } => PipelineKey::ShastaRisc0Network,
            _ => return Err(ApiError::bad_request("unsupported risc0 proving route")),
        },
        BatchProofType::Native => PipelineKey::ShastaNative,
        BatchProofType::Sgx => PipelineKey::ShastaSgx,
        BatchProofType::SgxGeth => PipelineKey::ShastaSgxGeth,
        BatchProofType::Tdx => PipelineKey::ShastaTdx,
        BatchProofType::Boundless | BatchProofType::ZkAny => {
            unreachable!("unsupported proof type is filtered before canonical route build")
        }
    };
    let canonical_proof_type = match proof_type {
        BatchProofType::Sp1 => ProofType::Sp1,
        BatchProofType::Risc0 => ProofType::Risc0,
        BatchProofType::Native => ProofType::Native,
        BatchProofType::Sgx => ProofType::Sgx,
        BatchProofType::SgxGeth => ProofType::SgxGeth,
        BatchProofType::Tdx => ProofType::Tdx,
        BatchProofType::Boundless | BatchProofType::ZkAny => {
            unreachable!("unsupported proof type is filtered before canonical route build")
        }
    };

    Ok(CanonicalProofRoute::new(
        route,
        pipeline_key,
        canonical_proof_type,
    ))
}

pub(super) fn validate_hosted_proof_type(
    route: PipelineRoute,
    proof_type: BatchProofType,
) -> Result<(), ApiError> {
    if matches!(
        route,
        PipelineRoute {
            guest_system: GuestSystem::Sgx,
            runner: RunnerKind::Remote,
        }
    ) && !matches!(proof_type, BatchProofType::Sgx | BatchProofType::SgxGeth)
    {
        return Err(ApiError::bad_request(format!(
            "proof_type={} is not supported when the server prover route is sgx/remote",
            proof_type.as_str()
        )));
    }

    Ok(())
}

fn native_route_for_request(state: &AppState) -> Result<PipelineRoute, ApiError> {
    let route = state.config.prover.route();
    if matches!(
        route,
        PipelineRoute {
            guest_system: GuestSystem::Native,
            runner: RunnerKind::Local,
        }
    ) {
        Ok(route)
    } else {
        Err(ApiError::bad_request(
            "proof_type=native is only supported when the server prover route is native/local",
        ))
    }
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
            ProofType::SgxGeth => Self::SgxGeth,
            ProofType::Risc0 => Self::Risc0,
            ProofType::Tdx => Self::Tdx,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BatchProofType, default_risc0_runner_for_route, route_for_proof_type};
    use crate::config::{Config, GuestSystem, PipelineRoute, RunnerKind};
    use crate::server::sampling::ZkAnySampler;
    use crate::server::state::{AppState, StaticPipelineFactory};
    use axum::http::StatusCode;
    use raiko2_engine::ProverTaskConfig;
    use raiko2_pipeline::PipelineKey;
    use raiko2_prover::sp1::Sp1RequestContext;
    use raiko2_runtime::RuntimeManager;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_state() -> AppState {
        test_state_with_config(Config::default())
    }

    fn test_state_with_config(config: Config) -> AppState {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let config = Arc::new(config);
        AppState {
            pipelines: Arc::new(StaticPipelineFactory::default()),
            runtime: Arc::new(
                RuntimeManager::new(
                    std::env::temp_dir().join(format!("raiko2-proof-route-tests-{nanos}")),
                )
                .expect("runtime manager"),
            ),
            zk_any_sampler: Arc::new(Mutex::new(ZkAnySampler::from_config(&config.prover.zk_any))),
            config,
        }
    }

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

    #[test]
    fn route_for_proof_type_selects_sgxgeth_remote_pipeline() {
        let state = test_state();
        let selection = route_for_proof_type(
            &state,
            BatchProofType::SgxGeth,
            &ProverTaskConfig::default(),
            Sp1RequestContext::ProposalBatch { aggregate: false },
        )
        .unwrap();

        assert_eq!(selection.route.to_string(), "sgx/remote");
        assert_eq!(selection.pipeline_key(), PipelineKey::ShastaSgxGeth);
        assert_eq!(
            selection.proof_type(),
            raiko2_primitives::ProofType::SgxGeth
        );
    }

    #[test]
    fn route_for_proof_type_keeps_native_on_native_local_route() {
        let mut config = Config::default();
        config.prover.guest_system = GuestSystem::Native;
        config.prover.runner = RunnerKind::Local;
        let state = test_state_with_config(config);

        let selection = route_for_proof_type(
            &state,
            BatchProofType::Native,
            &ProverTaskConfig::default(),
            Sp1RequestContext::ProposalBatch { aggregate: false },
        )
        .unwrap();

        assert_eq!(selection.route.to_string(), "native/local");
        assert_eq!(selection.pipeline_key(), PipelineKey::ShastaNative);
        assert_eq!(selection.proof_type(), raiko2_primitives::ProofType::Native);
    }

    #[test]
    fn route_for_proof_type_rejects_native_without_native_local_route() {
        let mut config = Config::default();
        config.prover.guest_system = GuestSystem::Risc0;
        config.prover.runner = RunnerKind::Network;
        let state = test_state_with_config(config);

        let error = route_for_proof_type(
            &state,
            BatchProofType::Native,
            &ProverTaskConfig::default(),
            Sp1RequestContext::ProposalBatch { aggregate: false },
        )
        .unwrap_err();

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(
            error.message,
            "proof_type=native is only supported when the server prover route is native/local"
        );
    }

    #[test]
    fn route_for_proof_type_rejects_sp1_on_remote_sgx_host() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let mut config = Config::default();
        config.prover.guest_system = GuestSystem::Sgx;
        config.prover.runner = RunnerKind::Remote;
        let state = AppState {
            pipelines: Arc::new(StaticPipelineFactory::default()),
            runtime: Arc::new(
                RuntimeManager::new(
                    std::env::temp_dir()
                        .join(format!("raiko2-proof-route-remote-sgx-tests-{nanos}")),
                )
                .expect("runtime manager"),
            ),
            zk_any_sampler: Arc::new(Mutex::new(ZkAnySampler::from_config(&config.prover.zk_any))),
            config: Arc::new(config),
        };

        let err = route_for_proof_type(
            &state,
            BatchProofType::Sp1,
            &ProverTaskConfig::default(),
            Sp1RequestContext::ProposalBatch { aggregate: false },
        )
        .expect_err("remote sgx host should reject sp1");

        assert!(
            err.message.contains("proof_type=sp1"),
            "unexpected error: {err:?}"
        );
    }
}
