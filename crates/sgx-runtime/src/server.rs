//! Axum server for the dedicated SGX prover.

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, FromRef, State, rejection::JsonRejection},
    routing::{get, post},
};
use tracing::{info, warn};

use crate::{
    aggregation::aggregate_request, config::ServiceConfig, proposal::prove_request,
    protocol::RequestFailure, tee::TeeProvider,
};

const MAX_REQUEST_BODY_BYTES: usize = 10_000 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct SgxProver<P> {
    pub(crate) provider: P,
    pub(crate) service_config: ServiceConfig,
}

impl<P> FromRef<SgxProver<P>> for ServiceConfig
where
    P: Clone,
{
    fn from_ref(input: &SgxProver<P>) -> Self {
        input.service_config.clone()
    }
}

pub(crate) fn router<P>(state: SgxProver<P>) -> Router
where
    P: TeeProvider + Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/health", get(health))
        .route("/prove/shasta", post(prove_shasta::<P>))
        .route("/prove/shasta-aggregate", post(prove_shasta_aggregate::<P>))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
        .with_state(state)
}

pub(crate) async fn serve<P>(provider: P, service_config: ServiceConfig) -> Result<()>
where
    P: TeeProvider + Clone + Send + Sync + 'static,
{
    let listener = tokio::net::TcpListener::bind(&service_config.listen_addr)
        .await
        .with_context(|| format!("bind {}", service_config.listen_addr))?;
    let local_addr = listener
        .local_addr()
        .context("read local SGX listener address")?;
    info!(
        listen = %local_addr,
        fork = %service_config.fork,
        instance_id = service_config.instance_id,
        "raiko2 sgx provider listening"
    );
    axum::serve(
        listener,
        router(SgxProver {
            provider,
            service_config,
        }),
    )
    .await
    .context("run SGX server")
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

const fn proposal_id_from_request(
    request: &raiko2_prover::remote_prover::protocol::Raiko2ShastaRequest,
) -> u64 {
    request
        .payload
        .proof_carry_data
        .transition_input
        .proposal_id
}

fn aggregate_proposal_id_summary(
    request: &raiko2_prover::remote_prover::protocol::Raiko2ShastaAggregateRequest,
) -> String {
    if request.payload.proofs.is_empty() {
        return "none".to_string();
    }

    let first = request.payload.proofs[0]
        .proof_carry_data
        .transition_input
        .proposal_id;
    let last = request.payload.proofs[request.payload.proofs.len() - 1]
        .proof_carry_data
        .transition_input
        .proposal_id;
    if first == last {
        first.to_string()
    } else {
        format!("{first}..{last}")
    }
}

async fn prove_shasta<P>(
    State(state): State<SgxProver<P>>,
    request: Result<
        Json<raiko2_prover::remote_prover::protocol::Raiko2ShastaRequest>,
        JsonRejection,
    >,
) -> (
    axum::http::StatusCode,
    Json<raiko2_prover::remote_prover::protocol::Raiko2ProofResponse>,
)
where
    P: TeeProvider + Clone + Send + Sync + 'static,
{
    match request {
        Ok(Json(request)) => {
            match prove_request(&state.provider, state.service_config.instance_id, &request) {
                Ok(response) => {
                    if response.result.is_some() {
                        info!(
                            schema = %request.schema,
                            proposal_id = proposal_id_from_request(&request),
                            chain_id = request.payload.chain_id,
                            block_count = request.payload.blocks.len(),
                            instance_id = state.service_config.instance_id,
                            "completed sgx shasta prove request"
                        );
                    }
                    (axum::http::StatusCode::OK, Json(response))
                }
                Err(err) => {
                    warn!(
                        schema = %request.schema,
                        chain_id = request.payload.chain_id,
                        block_count = request.payload.blocks.len(),
                        instance_id = state.service_config.instance_id,
                        code = err.code,
                        message = %err.message,
                        "sgx shasta prove request failed"
                    );
                    err.into_response()
                }
            }
        }
        Err(err) => {
            let failure = RequestFailure::invalid_json(err.body_text());
            warn!(
                instance_id = state.service_config.instance_id,
                code = failure.code,
                message = %failure.message,
                "sgx shasta prove request failed"
            );
            failure.into_response()
        }
    }
}

async fn prove_shasta_aggregate<P>(
    State(state): State<SgxProver<P>>,
    request: Result<
        Json<raiko2_prover::remote_prover::protocol::Raiko2ShastaAggregateRequest>,
        JsonRejection,
    >,
) -> (
    axum::http::StatusCode,
    Json<raiko2_prover::remote_prover::protocol::Raiko2ProofResponse>,
)
where
    P: TeeProvider + Clone + Send + Sync + 'static,
{
    match request {
        Ok(Json(request)) => {
            match aggregate_request(&state.provider, state.service_config.instance_id, &request) {
                Ok(response) => {
                    info!(
                        schema = %request.schema,
                        proposal_ids = %aggregate_proposal_id_summary(&request),
                        proof_count = request.payload.proofs.len(),
                        instance_id = state.service_config.instance_id,
                        "completed sgx shasta aggregate request"
                    );
                    (axum::http::StatusCode::OK, Json(response))
                }
                Err(err) => {
                    warn!(
                        schema = %request.schema,
                        proof_count = request.payload.proofs.len(),
                        instance_id = state.service_config.instance_id,
                        code = err.code,
                        message = %err.message,
                        "sgx shasta aggregate request failed"
                    );
                    err.into_response()
                }
            }
        }
        Err(err) => {
            let failure = RequestFailure::invalid_json(err.body_text());
            warn!(
                instance_id = state.service_config.instance_id,
                code = failure.code,
                message = %failure.message,
                "sgx shasta aggregate request failed"
            );
            failure.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::Address;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use raiko2_protocol_shasta::shasta::ProofCarryData;
    use raiko2_prover::remote_prover::protocol::{
        RAIKO2_SHASTA_AGGREGATE_REQUEST_SCHEMA, RAIKO2_SHASTA_REQUEST_SCHEMA,
        Raiko2ShastaAggregatePayload, Raiko2ShastaAggregateRequest, Raiko2ShastaPayload,
        Raiko2ShastaRequest,
    };
    use secp256k1::SecretKey;
    use tower::util::ServiceExt;

    use super::{SgxProver, aggregate_proposal_id_summary, proposal_id_from_request, router};
    use crate::config::ServiceConfig;
    use crate::tee::TeeProvider;

    #[derive(Clone)]
    struct FakeProvider;

    impl TeeProvider for FakeProvider {
        fn save_private_key(&self, _key: &SecretKey) -> anyhow::Result<()> {
            unreachable!("unused in tests")
        }

        fn load_private_key(&self) -> anyhow::Result<SecretKey> {
            SecretKey::from_slice(&[12u8; 32]).map_err(Into::into)
        }

        fn load_quote(&self, _instance_address: Address) -> anyhow::Result<Vec<u8>> {
            Ok(vec![0xAA])
        }
    }

    fn request_fixture() -> Raiko2ShastaRequest {
        let mut carry = ProofCarryData {
            chain_id: 167_013,
            ..ProofCarryData::default()
        };
        carry.transition_input.proposal_id = 42;

        Raiko2ShastaRequest {
            schema: RAIKO2_SHASTA_REQUEST_SCHEMA.to_string(),
            payload: Raiko2ShastaPayload {
                chain_id: 167_013,
                blocks: Vec::new(),
                proof_carry_data: carry,
                guest_input: None,
            },
        }
    }

    #[tokio::test]
    async fn health_route_responds_ok() {
        let app = router(SgxProver {
            provider: FakeProvider,
            service_config: ServiceConfig {
                listen_addr: "127.0.0.1:0".to_string(),
                fork: "shasta".to_string(),
                instance_id: 99,
            },
        });

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("health response");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn prove_shasta_route_rejects_request_without_guest_input() {
        let app = router(SgxProver {
            provider: FakeProvider,
            service_config: ServiceConfig {
                listen_addr: "127.0.0.1:0".to_string(),
                fork: "shasta".to_string(),
                instance_id: 99,
            },
        });
        let body = serde_json::to_vec(&request_fixture()).expect("request body");

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/prove/shasta")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("prove response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn prove_shasta_route_accepts_large_request_bodies_before_validation() {
        let app = router(SgxProver {
            provider: FakeProvider,
            service_config: ServiceConfig {
                listen_addr: "127.0.0.1:0".to_string(),
                fork: "shasta".to_string(),
                instance_id: 99,
            },
        });
        let mut body = vec![b' '; 3 * 1024 * 1024];
        body.extend(serde_json::to_vec(&request_fixture()).expect("request body"));

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/prove/shasta")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("prove response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn prove_shasta_aggregate_rejects_empty_request() {
        let app = router(SgxProver {
            provider: FakeProvider,
            service_config: ServiceConfig {
                listen_addr: "127.0.0.1:0".to_string(),
                fork: "shasta".to_string(),
                instance_id: 99,
            },
        });
        let body = serde_json::to_vec(&Raiko2ShastaAggregateRequest {
            schema: RAIKO2_SHASTA_AGGREGATE_REQUEST_SCHEMA.to_string(),
            payload: Raiko2ShastaAggregatePayload { proofs: vec![] },
        })
        .expect("request body");

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/prove/shasta-aggregate")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("aggregate response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn proposal_log_summary_uses_business_identifier() {
        let mut request = request_fixture();
        request
            .payload
            .proof_carry_data
            .transition_input
            .proposal_id = 2_222;

        assert_eq!(proposal_id_from_request(&request), 2_222);
    }

    #[test]
    fn aggregate_log_summary_summarizes_proposal_ids() {
        let mut first = request_fixture().payload.proof_carry_data;
        first.transition_input.proposal_id = 2_222;
        let aggregate = Raiko2ShastaAggregateRequest {
            schema: RAIKO2_SHASTA_AGGREGATE_REQUEST_SCHEMA.to_string(),
            payload: Raiko2ShastaAggregatePayload {
                proofs: vec![
                    raiko2_prover::remote_prover::protocol::Raiko2AggregateProof {
                        input: "0x1".to_string(),
                        proof: "0x2".to_string(),
                        proof_carry_data: first,
                    },
                ],
            },
        };

        assert_eq!(aggregate_proposal_id_summary(&aggregate), "2222");
    }

    #[test]
    fn aggregate_log_summary_reports_none_for_empty_proofs() {
        let aggregate = Raiko2ShastaAggregateRequest {
            schema: RAIKO2_SHASTA_AGGREGATE_REQUEST_SCHEMA.to_string(),
            payload: Raiko2ShastaAggregatePayload { proofs: vec![] },
        };

        assert_eq!(aggregate_proposal_id_summary(&aggregate), "none");
    }

    #[test]
    fn aggregate_log_summary_summarizes_proposal_id_range() {
        let mut first = request_fixture().payload.proof_carry_data;
        first.transition_input.proposal_id = 2_222;

        let mut last = request_fixture().payload.proof_carry_data;
        last.transition_input.proposal_id = 2_333;

        let aggregate = Raiko2ShastaAggregateRequest {
            schema: RAIKO2_SHASTA_AGGREGATE_REQUEST_SCHEMA.to_string(),
            payload: Raiko2ShastaAggregatePayload {
                proofs: vec![
                    raiko2_prover::remote_prover::protocol::Raiko2AggregateProof {
                        input: "0x1".to_string(),
                        proof: "0x2".to_string(),
                        proof_carry_data: first,
                    },
                    raiko2_prover::remote_prover::protocol::Raiko2AggregateProof {
                        input: "0x3".to_string(),
                        proof: "0x4".to_string(),
                        proof_carry_data: last,
                    },
                ],
            },
        };

        assert_eq!(aggregate_proposal_id_summary(&aggregate), "2222..2333");
    }
}
