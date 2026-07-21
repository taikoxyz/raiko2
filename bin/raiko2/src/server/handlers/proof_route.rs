use alloy_primitives::keccak256;
use raiko2_engine::ProverTaskConfig;
use raiko2_pipeline::{GuestSystem, PipelineKey, PipelineRoute, RunnerKind};
use raiko2_primitives::ProofType;
use raiko2_prover::sp1_config::{ProverMode as Sp1ProverMode, Sp1RequestContext};

use super::super::errors::ApiError;
use super::proof_types::{BatchProofType, BatchShastaRequest};
use crate::server::request_identity::RequestFingerprint;
use crate::server::state::AppState;

#[derive(Debug, Clone, Copy)]
pub(super) struct CanonicalProofRoute {
    pub(super) route: PipelineRoute,
    pipeline_key: PipelineKey,
}

impl CanonicalProofRoute {
    pub(super) const fn new(route: PipelineRoute, pipeline_key: PipelineKey) -> Self {
        Self {
            route,
            pipeline_key,
        }
    }

    pub(super) const fn pipeline_key(self) -> PipelineKey {
        self.pipeline_key
    }

    pub(super) const fn proof_type(self) -> ProofType {
        self.pipeline_key.proof_type()
    }
}

pub(super) enum BatchProofDecision {
    Selected(BatchProofType),
    NotDrawn,
}

pub(super) fn public_task_id_from_fingerprint(request_fingerprint: &str) -> String {
    RequestFingerprint::from_hex(request_fingerprint).public_task_id()
}

pub(super) fn route_for_proof_type(
    state: &AppState,
    proof_type: BatchProofType,
    prover_config: &ProverTaskConfig,
    sp1_context: Sp1RequestContext,
) -> Result<CanonicalProofRoute, ApiError> {
    let concrete_proof_type = match proof_type {
        BatchProofType::Sp1 => ProofType::Sp1,
        BatchProofType::Risc0 => ProofType::Risc0,
        BatchProofType::Native => ProofType::Native,
        BatchProofType::Sgx => ProofType::Sgx,
        BatchProofType::SgxGeth => ProofType::SgxGeth,
        BatchProofType::Boundless => return Err(unsupported_proof_type(proof_type)),
        BatchProofType::ZkAny => {
            return Err(ApiError::bad_request(
                "proof_type=zk_any must be resolved before route selection",
            ));
        }
    };
    let runner = state
        .config
        .prover
        .routes
        .runner(concrete_proof_type)
        .ok_or_else(|| unsupported_proof_type(proof_type))?;

    if matches!(proof_type, BatchProofType::Sp1) {
        validate_sp1_runner_for_request(state, prover_config, sp1_context, runner)?;
    }

    let (guest_system, pipeline_key) = match (proof_type, runner) {
        (BatchProofType::Risc0, RunnerKind::Local) => {
            (GuestSystem::Risc0, PipelineKey::ShastaRisc0)
        }
        (BatchProofType::Risc0, RunnerKind::Network) => {
            (GuestSystem::Risc0, PipelineKey::ShastaRisc0Network)
        }
        (BatchProofType::Sp1, RunnerKind::Local | RunnerKind::Network) => {
            (GuestSystem::Sp1, PipelineKey::ShastaSp1)
        }
        (BatchProofType::Native, RunnerKind::Local) => {
            (GuestSystem::Native, PipelineKey::ShastaNative)
        }
        (BatchProofType::Sgx, RunnerKind::Remote) => (GuestSystem::Sgx, PipelineKey::ShastaSgx),
        (BatchProofType::SgxGeth, RunnerKind::Remote) => {
            (GuestSystem::SgxGeth, PipelineKey::ShastaSgxGeth)
        }
        _ => {
            return Err(ApiError::internal(format!(
                "configured prover route {}/{} is invalid",
                proof_type.as_str(),
                runner
            )));
        }
    };
    let route = PipelineRoute::new(guest_system, runner);
    Ok(CanonicalProofRoute::new(route, pipeline_key))
}

pub(super) fn unsupported_proof_type(proof_type: BatchProofType) -> ApiError {
    ApiError::bad_request(format!(
        "proof_type={} is not supported",
        proof_type.as_str()
    ))
}

fn validate_sp1_runner_for_request(
    state: &AppState,
    prover_config: &ProverTaskConfig,
    sp1_context: Sp1RequestContext,
    configured_runner: RunnerKind,
) -> Result<(), ApiError> {
    let effective_config = state
        .config
        .prover
        .sp1
        .resolve_request_config(prover_config.sp1.as_ref(), sp1_context)
        .map_err(|err| ApiError::bad_request(err.to_string()))?;
    let runner = match effective_config.prover {
        Sp1ProverMode::Network => RunnerKind::Network,
        Sp1ProverMode::Mock | Sp1ProverMode::Local => RunnerKind::Local,
    };
    if runner != configured_runner {
        return Err(ApiError::bad_request(format!(
            "request-scoped sp1 prover route {} is unavailable; this server is configured for {}",
            PipelineRoute::new(GuestSystem::Sp1, runner),
            PipelineRoute::new(GuestSystem::Sp1, configured_runner),
        )));
    }
    Ok(())
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

impl BatchProofType {
    pub(super) const fn from_canonical(proof_type: ProofType) -> Self {
        match proof_type {
            ProofType::Native => Self::Native,
            ProofType::Sp1 => Self::Sp1,
            ProofType::Sgx => Self::Sgx,
            ProofType::SgxGeth => Self::SgxGeth,
            ProofType::Risc0 => Self::Risc0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BatchProofType, route_for_proof_type};
    use crate::config::Config;
    use crate::server::state::{AppState, StaticPipelineFactory};
    use axum::http::StatusCode;
    use raiko2_engine::ProverTaskConfig;
    use raiko2_pipeline::{GuestSystem, PipelineKey, PipelineRoute, RunnerKind};
    use raiko2_prover::sp1_config::{
        ProverMode as Sp1ProverMode, Sp1ConfigOverrides, Sp1RequestContext,
    };
    use raiko2_runtime::RuntimeManager;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_state_with_config(config: Config) -> AppState {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let config = Arc::new(config);
        AppState::from_parts(
            config,
            Arc::new(StaticPipelineFactory::default()),
            Arc::new(
                RuntimeManager::new(
                    std::env::temp_dir().join(format!("raiko2-proof-route-tests-{nanos}")),
                )
                .expect("runtime manager"),
            ),
        )
    }

    fn config_with_routes(routes: &str) -> Config {
        let mut config = Config::default();
        config.prover.routes = routes.parse().expect("valid prover routes");
        config
    }

    fn assert_unsupported(error: super::ApiError, proof_type: BatchProofType) {
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(
            error.message,
            format!("proof_type={} is not supported", proof_type.as_str())
        );
    }

    #[test]
    fn risc0_route_does_not_enable_sp1() {
        let state = test_state_with_config(config_with_routes("risc0/local"));

        let risc0 = route_for_proof_type(
            &state,
            BatchProofType::Risc0,
            &ProverTaskConfig::default(),
            Sp1RequestContext::ProposalBatch { aggregate: false },
        )
        .expect("configured RISC0 route");
        assert_eq!(risc0.route, PipelineKey::ShastaRisc0.route());
        assert_eq!(risc0.pipeline_key(), PipelineKey::ShastaRisc0);

        let error = route_for_proof_type(
            &state,
            BatchProofType::Sp1,
            &ProverTaskConfig::default(),
            Sp1RequestContext::ProposalBatch { aggregate: false },
        );
        assert_unsupported(
            error.expect_err("SP1 must remain disabled"),
            BatchProofType::Sp1,
        );
    }

    #[test]
    fn sp1_route_does_not_enable_risc0() {
        let mut config = config_with_routes("sp1/local");
        config.prover.sp1.prover = Sp1ProverMode::Local;
        let state = test_state_with_config(config);

        let sp1 = route_for_proof_type(
            &state,
            BatchProofType::Sp1,
            &ProverTaskConfig::default(),
            Sp1RequestContext::ProposalBatch { aggregate: false },
        )
        .expect("configured SP1 route");
        assert_eq!(sp1.route, PipelineKey::ShastaSp1.route());
        assert_eq!(sp1.pipeline_key(), PipelineKey::ShastaSp1);

        let error = route_for_proof_type(
            &state,
            BatchProofType::Risc0,
            &ProverTaskConfig::default(),
            Sp1RequestContext::ProposalBatch { aggregate: false },
        );
        assert_unsupported(
            error.expect_err("RISC0 must remain disabled"),
            BatchProofType::Risc0,
        );
    }

    #[test]
    fn sgxgeth_request_uses_distinct_public_route() {
        let state = test_state_with_config(config_with_routes("sgxgeth/remote"));

        let selection = route_for_proof_type(
            &state,
            BatchProofType::SgxGeth,
            &ProverTaskConfig::default(),
            Sp1RequestContext::ProposalBatch { aggregate: false },
        )
        .expect("configured SGXGETH route");

        assert_eq!(selection.route.to_string(), "sgxgeth/remote");
        assert_ne!(
            selection.route,
            "sgx/remote"
                .parse::<PipelineRoute>()
                .expect("parse sgx route")
        );
        assert_eq!(selection.pipeline_key(), PipelineKey::ShastaSgxGeth);
    }

    #[test]
    fn all_production_routes_resolve_independently_on_one_host() {
        let mut config =
            config_with_routes("risc0/network,sp1/network,native/local,sgx/remote,sgxgeth/remote");
        config.prover.sp1.prover = Sp1ProverMode::Network;
        let state = test_state_with_config(config);

        for (proof_type, expected_route, expected_pipeline) in [
            (
                BatchProofType::Risc0,
                PipelineRoute::new(GuestSystem::Risc0, RunnerKind::Network),
                PipelineKey::ShastaRisc0Network,
            ),
            (
                BatchProofType::Sp1,
                PipelineRoute::new(GuestSystem::Sp1, RunnerKind::Network),
                PipelineKey::ShastaSp1,
            ),
            (
                BatchProofType::Native,
                PipelineRoute::new(GuestSystem::Native, RunnerKind::Local),
                PipelineKey::ShastaNative,
            ),
            (
                BatchProofType::Sgx,
                PipelineRoute::new(GuestSystem::Sgx, RunnerKind::Remote),
                PipelineKey::ShastaSgx,
            ),
            (
                BatchProofType::SgxGeth,
                PipelineRoute::new(GuestSystem::SgxGeth, RunnerKind::Remote),
                PipelineKey::ShastaSgxGeth,
            ),
        ] {
            let selection = route_for_proof_type(
                &state,
                proof_type,
                &ProverTaskConfig::default(),
                Sp1RequestContext::ProposalBatch { aggregate: false },
            )
            .unwrap_or_else(|error| panic!("failed to route {proof_type:?}: {error:?}"));

            assert_eq!(selection.route, expected_route);
            assert_eq!(selection.pipeline_key(), expected_pipeline);
            assert_eq!(selection.proof_type(), expected_pipeline.proof_type());
        }
    }

    #[test]
    fn route_for_proof_type_rejects_sp1_request_route_flip() {
        for (routes, configured, requested) in [
            ("sp1/network", Sp1ProverMode::Network, Sp1ProverMode::Local),
            ("sp1/local", Sp1ProverMode::Local, Sp1ProverMode::Network),
        ] {
            let mut config = config_with_routes(routes);
            config.prover.sp1.prover = configured;
            let state = test_state_with_config(config);
            let prover_config = ProverTaskConfig {
                sp1: Some(Sp1ConfigOverrides {
                    prover: Some(requested),
                    ..Sp1ConfigOverrides::default()
                }),
                sp1_system: None,
            };

            for sp1_context in [
                Sp1RequestContext::ProposalBatch { aggregate: false },
                Sp1RequestContext::Aggregation,
            ] {
                let error =
                    route_for_proof_type(&state, BatchProofType::Sp1, &prover_config, sp1_context)
                        .expect_err("request-level route flip must be rejected");

                assert_eq!(error.status, StatusCode::BAD_REQUEST);
                assert!(error.message.contains("request-scoped sp1 prover route"));
            }
        }
    }

    #[test]
    fn sp1_local_route_allows_local_and_mock_request_modes() {
        for requested in [Sp1ProverMode::Local, Sp1ProverMode::Mock] {
            let mut config = config_with_routes("sp1/local");
            config.prover.sp1.prover = Sp1ProverMode::Local;
            let state = test_state_with_config(config);
            let prover_config = ProverTaskConfig {
                sp1: Some(Sp1ConfigOverrides {
                    prover: Some(requested),
                    ..Sp1ConfigOverrides::default()
                }),
                sp1_system: None,
            };

            let selection = route_for_proof_type(
                &state,
                BatchProofType::Sp1,
                &prover_config,
                Sp1RequestContext::Aggregation,
            )
            .unwrap_or_else(|error| panic!("failed to route {requested:?}: {error:?}"));

            assert_eq!(selection.route, PipelineKey::ShastaSp1.route());
            assert_eq!(selection.pipeline_key(), PipelineKey::ShastaSp1);
        }
    }
}
