//! Axum server for the dedicated SGX prover.

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, FromRef, State, rejection::JsonRejection},
    routing::{get, post},
};

use crate::{
    aggregation::aggregate_request, config::ServiceConfig, proposal::prove_request,
    protocol::RequestFailure, tee::TeeProvider,
};

const MAX_REQUEST_BODY_BYTES: usize = 128 * 1024 * 1024;

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

async fn prove_shasta<P>(
    State(state): State<SgxProver<P>>,
    request: Result<Json<raiko2_prover::gaiko2::protocol::Gaiko2ShastaRequest>, JsonRejection>,
) -> (
    axum::http::StatusCode,
    Json<raiko2_prover::gaiko2::protocol::Gaiko2ProofResponse>,
)
where
    P: TeeProvider + Clone + Send + Sync + 'static,
{
    match request {
        Ok(Json(request)) => {
            match prove_request(&state.provider, state.service_config.instance_id, &request) {
                Ok(response) => (axum::http::StatusCode::OK, Json(response)),
                Err(err) => err.into_response(),
            }
        }
        Err(err) => RequestFailure::invalid_json(err.body_text()).into_response(),
    }
}

async fn prove_shasta_aggregate<P>(
    State(state): State<SgxProver<P>>,
    request: Result<
        Json<raiko2_prover::gaiko2::protocol::Gaiko2ShastaAggregateRequest>,
        JsonRejection,
    >,
) -> (
    axum::http::StatusCode,
    Json<raiko2_prover::gaiko2::protocol::Gaiko2ProofResponse>,
)
where
    P: TeeProvider + Clone + Send + Sync + 'static,
{
    match request {
        Ok(Json(request)) => {
            match aggregate_request(&state.provider, state.service_config.instance_id, &request) {
                Ok(response) => (axum::http::StatusCode::OK, Json(response)),
                Err(err) => err.into_response(),
            }
        }
        Err(err) => RequestFailure::invalid_json(err.body_text()).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, B256};
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use raiko2_primitives::{ChainSpec, ExecutionWitness, StatelessInput};
    use raiko2_protocol_shasta::shasta::{Checkpoint, ProofCarryData};
    use raiko2_prover::gaiko2::protocol::{
        GAIKO2_SHASTA_REQUEST_SCHEMA, Gaiko2ShastaAggregatePayload, Gaiko2ShastaAggregateRequest,
        Gaiko2ShastaPayload, Gaiko2ShastaRequest,
    };
    use reth_ethereum_primitives::Block;
    use secp256k1::SecretKey;
    use tower::util::ServiceExt;

    use super::{SgxProver, router};
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

    fn u48(value: u64) -> alloy_primitives::Uint<48, 1> {
        alloy_primitives::Uint::from_limbs([value])
    }

    fn request_fixture() -> Gaiko2ShastaRequest {
        let mut carry = ProofCarryData {
            chain_id: 167_013,
            ..ProofCarryData::default()
        };
        carry.transition_input.parent_block_hash = B256::from([0x11; 32]);
        carry.transition_input.checkpoint = Checkpoint {
            blockNumber: u48(42),
            blockHash: B256::from([0x22; 32]),
            stateRoot: B256::from([0x33; 32]),
        };

        let mut stateless = StatelessInput {
            block: Block::default(),
            chain_spec: ChainSpec::default(),
            witness: ExecutionWitness::default(),
            accounts: Default::default(),
        };
        stateless.block.header.number = 42;
        stateless.block.header.parent_hash = B256::from([0x11; 32]);
        stateless.block.header.state_root = B256::from([0x33; 32]);
        stateless.chain_spec.chain_id = 167_013;

        Gaiko2ShastaRequest {
            schema: GAIKO2_SHASTA_REQUEST_SCHEMA.to_string(),
            payload: Gaiko2ShastaPayload {
                chain_id: 167_013,
                blocks: vec![stateless.into()],
                proof_carry_data: carry,
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
    async fn prove_shasta_route_returns_json_envelope() {
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

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn prove_shasta_route_accepts_large_valid_request_bodies() {
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

        assert_eq!(response.status(), StatusCode::OK);
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
        let body = serde_json::to_vec(&Gaiko2ShastaAggregateRequest {
            schema: GAIKO2_SHASTA_REQUEST_SCHEMA.to_string(),
            payload: Gaiko2ShastaAggregatePayload { proofs: vec![] },
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
}
