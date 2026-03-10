//! End-to-end (in-process) API tests.
//!
//! These tests exercise the HTTP handlers + engine orchestration without relying on
//! external RPC endpoints. A minimal JSON-RPC server is spun up only for `/ready`.

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    extract::Json,
    http::{Request, StatusCode},
    routing::post,
};
use http_body_util::BodyExt;
use raiko2_engine::Engine;
use raiko2_pipeline::{NativeBackend, PipelineKey, forks::shasta::ShastaSpec};
use raiko2_primitives::{ProofContext, ProofRequest, RaikoError, RaikoResult};
use raiko2_primitives_shasta::GuestInput;
use raiko2_prover::native::NativeProver;
use raiko2_provider::Provider;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tower::ServiceExt;

use super::app;
use super::state::{AppState, StaticPipelineFactory};
use crate::config::{Config, ProverType};

type NativeFixtureSpec = ShastaSpec<NativeProver, NativeBackend, FixtureProvider>;
type NativeFixtureEngine = Engine<NativeFixtureSpec>;

#[derive(Clone)]
struct FixtureProvider {
    input: Arc<GuestInput>,
}

impl FixtureProvider {
    fn from_repo_test_json() -> Self {
        // NOTE: Keep this pinned to the repo's `test.json` so the e2e tests also validate the
        // real-world fixture used by benchmarks and roundtrip tests.
        let raw = include_str!("../../../../test.json");
        let input: GuestInput = serde_json::from_str(raw).expect("parse test.json as GuestInput");
        Self {
            input: Arc::new(input),
        }
    }

    fn witness_for_block(&self, block_number: u64) -> Option<&raiko2_primitives::StatelessInput> {
        self.input
            .witnesses
            .iter()
            .find(|w| w.block.header.number == block_number)
    }
}

#[async_trait::async_trait]
impl Provider for FixtureProvider {
    async fn batch_blocks(
        &self,
        block_numbers: &[u64],
    ) -> RaikoResult<Vec<reth_ethereum_primitives::Block>> {
        let mut out = Vec::with_capacity(block_numbers.len());
        for block_number in block_numbers {
            let witness = self
                .witness_for_block(*block_number)
                .ok_or_else(|| RaikoError::RPC(format!("fixture missing block {block_number}")))?;
            out.push(witness.block.clone());
        }
        Ok(out)
    }

    async fn batch_accounts(
        &self,
        block_numbers: &[u64],
        _accounts: &[Vec<alloy_primitives::Address>],
    ) -> RaikoResult<Vec<alloy_primitives::map::AddressMap<alloy_trie::TrieAccount>>> {
        let mut out = Vec::with_capacity(block_numbers.len());
        for block_number in block_numbers {
            let witness = self.witness_for_block(*block_number).ok_or_else(|| {
                RaikoError::RPC(format!("fixture missing accounts for block {block_number}"))
            })?;
            out.push(witness.accounts.clone());
        }
        Ok(out)
    }

    async fn batch_witnesses(
        &self,
        block_numbers: &[u64],
    ) -> RaikoResult<Vec<reth_stateless::ExecutionWitness>> {
        let mut out = Vec::with_capacity(block_numbers.len());
        for block_number in block_numbers {
            let witness = self.witness_for_block(*block_number).ok_or_else(|| {
                RaikoError::RPC(format!("fixture missing witness for block {block_number}"))
            })?;
            out.push(witness.witness.clone());
        }
        Ok(out)
    }
}

fn base_config() -> Config {
    let mut config = Config::default();
    config.prover.prover_type = ProverType::Native;
    // The e2e tests don't hit L1/L2 RPC except via `/ready` (which uses a dedicated mock server).
    config.rpc.l2_chain_id = 167_001;
    config
}

fn native_fixture_engine() -> NativeFixtureEngine {
    let provider = FixtureProvider::from_repo_test_json();
    let spec = ShastaSpec::new(
        PipelineKey::ShastaNative,
        NativeProver,
        NativeBackend,
        provider,
    );
    let ctx = ProofContext::new(
        ProofRequest {
            l1_chain_id: 1,
            l2_chain_id: 167_001,
            proposal_id: 0,
            l2_block_range: None,
            proof_type: "native".to_string(),
            blob_proof_type: None,
            prover: None,
            graffiti: None,
        },
        raiko2_primitives::ProverConfig::default(),
    );
    Engine::new(spec, ctx)
}

fn app_with_native_fixture_engine(config: Config, engine: NativeFixtureEngine) -> Router {
    let mut factory = StaticPipelineFactory::default();
    factory.insert(PipelineKey::ShastaNative, Arc::new(engine));
    let state = AppState {
        config: Arc::new(config),
        pipelines: Arc::new(factory),
    };
    app::build_router(state)
}

async fn read_json(res: axum::response::Response) -> (StatusCode, Value) {
    let status = res.status();
    let bytes = res
        .into_body()
        .collect()
        .await
        .expect("read response body")
        .to_bytes();
    let json: Value = serde_json::from_slice(&bytes).expect("parse JSON response");
    (status, json)
}

async fn get_json(app: &Router, uri: &str) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("build GET request");
    let res = app.clone().oneshot(req).await.expect("dispatch request");
    read_json(res).await
}

async fn post_json(app: &Router, uri: &str, payload: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .expect("build POST request");
    let res = app.clone().oneshot(req).await.expect("dispatch request");
    read_json(res).await
}

async fn drive_engine_to_idle(engine: &NativeFixtureEngine) {
    for _ in 0..32 {
        let progressed = engine.run_one("e2e").await.expect("run_one");
        if !progressed {
            return;
        }
    }
    panic!("engine did not drain after 32 steps");
}

async fn spawn_chain_id_rpc(chain_id: u64) -> (String, tokio::task::JoinHandle<()>) {
    let app = Router::new().route(
        "/",
        post(move |Json(req): Json<Value>| async move {
            let id = req.get("id").cloned().unwrap_or(Value::Null);
            let method = req
                .get("method")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if method == "eth_chainId" {
                Json(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": format!("0x{:x}", chain_id),
                }))
            } else {
                Json(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": format!("method not found: {method}") },
                }))
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock rpc listener");
    let addr = listener.local_addr().expect("listener local_addr");
    let url = format!("http://{addr}");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve mock rpc");
    });
    (url, handle)
}

#[tokio::test]
async fn e2e_ready_ok_with_matching_chain_id() {
    let chain_id = 167_001;
    let (l2_rpc, handle) = spawn_chain_id_rpc(chain_id).await;

    let mut config = base_config();
    config.rpc.l2_rpc = l2_rpc;
    config.rpc.l2_chain_id = chain_id;

    let state = AppState {
        config: Arc::new(config),
        pipelines: Arc::new(StaticPipelineFactory::default()),
    };
    let app = app::build_router(state);

    let (status, body) = get_json(&app, "/ready").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["reth"]["ok"], true);
    assert_eq!(body["queue"]["ok"], true);

    handle.abort();
}

#[tokio::test]
async fn e2e_proposal_proof_native_completes_from_fixture() {
    let config = base_config();
    let engine = native_fixture_engine();
    let app = app_with_native_fixture_engine(config, engine.clone());

    let (status, res) = post_json(&app, "/v1/proof/proposal", json!({"proposal_id": 3})).await;
    assert_eq!(status, StatusCode::OK);
    let id = res["id"].as_str().expect("response id").to_string();

    drive_engine_to_idle(&engine).await;

    let (status, res) = get_json(&app, &format!("/v1/proof/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["status"], "completed");
    assert!(res.get("error").is_none(), "unexpected error: {res}");
}

#[tokio::test]
async fn e2e_cancel_marks_task_cancelled_without_workers() {
    let config = base_config();
    let engine = native_fixture_engine();
    let app = app_with_native_fixture_engine(config, engine);

    let (status, res) = post_json(
        &app,
        "/v1/proof/proposal",
        json!({"proposal_id": 3, "prover_type": "native"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = res["id"].as_str().expect("response id").to_string();

    let (status, res) = post_json(&app, &format!("/v1/proof/{id}/cancel"), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["status"], "cancelled");
}
