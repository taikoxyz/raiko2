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
use raiko2_engine::{Engine, EngineTaskId, EngineTaskKey, ProposalStage, ProposalTaskRequest};
use raiko2_pipeline::{
    NativeBackend, PipelineKey, Risc0ShastaBackend,
    forks::shasta::{RISC0_SHASTA_BACKEND, ShastaSpec},
};
use raiko2_primitives::{ProofContext, ProofRequest, RaikoError, RaikoResult};
use raiko2_primitives_shasta::{GuestInput, build_proof_carry_data};
use raiko2_prover::BoundlessSubmissionProgress;
use raiko2_prover::native::NativeProver;
use raiko2_prover::risc0::{Risc0Config, Risc0Prover};
use raiko2_provider::Provider;
use raiko2_queue::{MemoryStore, RetryPolicy, SchedulerConfig, encode_task_id};
use raiko2_runtime::{RunnerStatus, RuntimeManager, TaskRegistration};
use serde_json::{Value, json};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;
use tower::ServiceExt;

use super::app;
use super::state::{AppState, StaticPipelineFactory};
use super::task_metadata::{HoodiProposalTask, HoodiRuntimeMetadata, HoodiTaskMetadata};
use crate::config::{Config, GuestSystem, NetworkPairConfig, RunnerKind};

type NativeFixtureSpec = ShastaSpec<NativeProver, NativeBackend, FixtureProvider>;
type NativeFixtureEngine = Engine<NativeFixtureSpec>;
type Risc0FixtureSpec = ShastaSpec<Risc0Prover, Risc0ShastaBackend, FixtureProvider>;
type Risc0FixtureEngine = Engine<Risc0FixtureSpec>;

fn unique_runtime_root(prefix: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "{prefix}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ))
}

#[derive(Clone)]
struct FixtureProvider {
    input: Arc<GuestInput>,
}

impl FixtureProvider {
    fn from_repo_test_json() -> Self {
        // NOTE: Keep this pinned to the repo's `test.json` so the e2e tests also validate the
        // real-world fixture used by benchmarks and roundtrip tests.
        let raw = include_str!("../../../../test.json");
        let mut input: GuestInput =
            serde_json::from_str(raw).expect("parse test.json as GuestInput");
        if input.proof_carry_data == Default::default() && !input.witnesses.is_empty() {
            input.proof_carry_data = build_proof_carry_data(&input);
        }
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
    ) -> RaikoResult<Vec<raiko2_primitives::ExecutionWitness>> {
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
    config.prover.guest_system = GuestSystem::Native;
    config.prover.runner = RunnerKind::Local;
    config.rpc.pairs = vec![NetworkPairConfig {
        network: "taiko_dev".to_string(),
        l1_network: "ethereum".to_string(),
        l1_rpc: Some("http://localhost:8545".to_string()),
        l2_rpc: Some("http://localhost:9545".to_string()),
    }];
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

fn risc0_fixture_engine(context_config: serde_json::Value) -> Risc0FixtureEngine {
    let provider = FixtureProvider::from_repo_test_json();
    let spec = ShastaSpec::new(
        PipelineKey::ShastaRisc0,
        Risc0Prover::new(Risc0Config {
            bonsai: false,
            snark: false,
            mock: true,
            profile: false,
            execution_po2: 20,
            verify: true,
        }),
        RISC0_SHASTA_BACKEND,
        provider,
    );
    let ctx = ProofContext::new(
        ProofRequest {
            l1_chain_id: 1,
            l2_chain_id: 167_001,
            proposal_id: 0,
            l2_block_range: None,
            proof_type: "risc0".to_string(),
            blob_proof_type: None,
            prover: None,
            graffiti: None,
        },
        context_config,
    );
    Engine::with_store_and_scheduler_config(
        spec,
        ctx,
        MemoryStore::new(),
        SchedulerConfig {
            lease_duration: Duration::from_secs(60),
            retry: RetryPolicy::None,
        },
    )
}

fn app_with_engine<S>(
    config: Config,
    network_pair: &str,
    pipeline_key: PipelineKey,
    engine: Engine<S>,
) -> AppState
where
    S: raiko2_pipeline::PipelineSpec + Send + Sync + 'static,
    S::Prover: raiko2_prover::Prover<S::Backend, GuestInput = S::GuestInput> + 'static,
    S::Backend: raiko2_pipeline::ProverBackend + 'static,
    S::Provider: raiko2_provider::Provider + 'static,
{
    let mut factory = StaticPipelineFactory::default();
    factory.insert(network_pair.to_string(), pipeline_key, Arc::new(engine));
    AppState {
        config: Arc::new(config),
        pipelines: Arc::new(factory),
        runtime: Arc::new(
            RuntimeManager::new(unique_runtime_root("raiko2-e2e-runtime"))
                .expect("runtime manager"),
        ),
    }
}

fn app_with_native_fixture_engine(config: Config, engine: NativeFixtureEngine) -> Router {
    let state = app_with_engine(
        config,
        "taiko_dev/ethereum",
        PipelineKey::ShastaNative,
        engine,
    );
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

async fn drive_engine_to_idle<S>(engine: &Engine<S>)
where
    S: raiko2_pipeline::PipelineSpec,
    S::Prover: raiko2_prover::Prover<S::Backend, GuestInput = S::GuestInput>,
    S::Backend: raiko2_pipeline::ProverBackend,
    S::Provider: raiko2_provider::Provider,
{
    for _ in 0..32 {
        let progressed = engine.run_one("e2e").await.expect("run_one");
        if !progressed {
            return;
        }
    }
    panic!("engine did not drain after 32 steps");
}

async fn spawn_chain_id_rpc(
    chain_id: u64,
) -> Result<(String, tokio::task::JoinHandle<()>), std::io::Error> {
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

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr().expect("listener local_addr");
    let url = format!("http://{addr}");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve mock rpc");
    });
    Ok((url, handle))
}

#[tokio::test]
async fn e2e_ready_ok_with_matching_chain_id() {
    let chain_id = 167_001;
    let (l2_rpc, handle) = match spawn_chain_id_rpc(chain_id).await {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(err) => panic!("bind mock rpc listener: {err}"),
    };

    let mut config = base_config();
    config.rpc.pairs[0].l2_rpc = Some(l2_rpc);

    let state = AppState {
        config: Arc::new(config),
        pipelines: Arc::new(StaticPipelineFactory::default()),
        runtime: Arc::new(
            RuntimeManager::new(unique_runtime_root("raiko2-e2e-ready-runtime"))
                .expect("runtime manager"),
        ),
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

    let (status, res) = post_json(
        &app,
        "/v3/proof/batch/shasta",
        json!({
            "proposals": [{
                "proposal_id": 3,
                "l1_inclusion_block_number": 1,
                "l2_block_numbers": [3],
                "last_anchor_block_number": 0
            }],
            "aggregate": false,
            "proof_type": "native",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = res["data"]["task_id"]
        .as_str()
        .expect("response task_id")
        .to_string();

    drive_engine_to_idle(&engine).await;

    let (status, res) = get_json(&app, &format!("/v3/tasks/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["data"]["status"], "completed");
    assert!(
        res["data"].get("error").is_none(),
        "unexpected error: {res}"
    );
}

#[tokio::test]
async fn e2e_cancel_marks_task_cancelled_without_workers() {
    let config = base_config();
    let engine = native_fixture_engine();
    let app = app_with_native_fixture_engine(config, engine);

    let (status, res) = post_json(
        &app,
        "/v3/proof/batch/shasta",
        json!({
            "proposals": [{
                "proposal_id": 3,
                "l1_inclusion_block_number": 1,
                "l2_block_numbers": [3],
                "last_anchor_block_number": 0
            }],
            "aggregate": false,
            "proof_type": "native",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = res["data"]["task_id"]
        .as_str()
        .expect("response task_id")
        .to_string();

    let (status, res) = post_json(&app, &format!("/v3/tasks/{id}/cancel"), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["data"]["status"], "cancelled");
}

#[tokio::test]
async fn e2e_task_status_turns_proving_after_preflight_progress() {
    let config = base_config();
    let engine = native_fixture_engine();
    let app = app_with_native_fixture_engine(config, engine.clone());

    let (status, res) = post_json(
        &app,
        "/v3/proof/batch/shasta",
        json!({
            "proposals": [{
                "proposal_id": 3,
                "l1_inclusion_block_number": 1,
                "l2_block_numbers": [3],
                "last_anchor_block_number": 0
            }],
            "aggregate": false,
            "proof_type": "native",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = res["data"]["task_id"]
        .as_str()
        .expect("response task_id")
        .to_string();

    let ran = engine.run_one("e2e-worker").await.expect("run one task");
    assert!(ran, "expected preflight task to run");

    let (status, res) = get_json(&app, &format!("/v3/tasks/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["data"]["route"], "native/local");
    assert_eq!(res["data"]["status"], "proving");
    assert_eq!(res["data"]["current_index"], 0);
    assert_eq!(res["data"]["proposals"][0]["status"], "proving");
    assert!(
        res["data"]["proposals"][0].get("runtime").is_none(),
        "unexpected per-proposal runtime while engine state is present: {res}"
    );
}

#[tokio::test]
async fn e2e_task_status_falls_back_to_runtime_metadata_without_engine_state() {
    let config = base_config();
    let engine = native_fixture_engine();
    let state = app_with_engine(
        config,
        "taiko_dev/ethereum",
        PipelineKey::ShastaNative,
        engine,
    );
    let app = app::build_router(state.clone());

    let proposal_task_id = EngineTaskId::new(EngineTaskKey::Proposal {
        pipeline: PipelineKey::ShastaNative,
        request: ProposalTaskRequest {
            proposal_id: 3,
            l2_block_range: None,
            l1_inclusion_block_number: 1,
            last_anchor_block_number: 0,
            blob_proof_type: None,
            prover: None,
            graffiti: None,
            prover_args_json: None,
        },
        stage: ProposalStage::Prove,
    });
    let encoded_task_id = encode_task_id(&proposal_task_id).expect("encode task id");
    let mut metadata = HoodiTaskMetadata {
        network_pair: "taiko_dev/ethereum".to_string(),
        network: "taiko_dev".to_string(),
        l1_network: "ethereum".to_string(),
        proof_type: "native".to_string(),
        aggregate_requested: false,
        proposals: vec![HoodiProposalTask {
            proposal_id: 3,
            l1_inclusion_block_number: 1,
            l2_block_numbers: vec![3],
            last_anchor_block_number: 0,
            task_id: encoded_task_id.clone(),
        }],
        aggregate_task_id: None,
        runtime: HoodiRuntimeMetadata {
            active_stage: Some("prove".to_string()),
            last_event: Some("submission_registered".to_string()),
            ..Default::default()
        },
    };
    let updated_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_secs() as i64;
    metadata.upsert_proposal_runtime(
        &encoded_task_id,
        &BoundlessSubmissionProgress {
            provider_request_id: "0x1234".to_string(),
            remote_tx_hash: Some("0xabcd".to_string()),
            image_ref: "0ximage".to_string(),
            deployment: "base".to_string(),
            offchain: false,
        },
        updated_at,
    );

    state
        .runtime
        .register_task(TaskRegistration {
            task_id: "task_runtime_fallback".to_string(),
            pipeline_key: PipelineKey::ShastaNative.as_str().to_string(),
            route: "native/local".to_string(),
            guest_system: "native".to_string(),
            runner: "local".to_string(),
            task_kind: "hoodi_batch".to_string(),
            proposal_id: Some(3),
            proof_ids: vec![encoded_task_id.clone()],
            metadata: serde_json::to_value(metadata).expect("serialize metadata"),
        })
        .await
        .expect("register task");

    let mut record = state
        .runtime
        .get_task("task_runtime_fallback")
        .await
        .expect("read task")
        .expect("task exists");
    record.runner_status = RunnerStatus::Running;
    record.updated_at = updated_at;
    state
        .runtime
        .upsert_task(&record)
        .await
        .expect("upsert task");

    let (status, res) = get_json(&app, "/v3/tasks/task_runtime_fallback").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["data"]["route"], "native/local");
    assert_eq!(res["data"]["status"], "proving");
    assert_eq!(res["data"]["runtime"]["runner_status"], "running");
    assert_eq!(res["data"]["runtime"]["active_stage"], "prove");
    assert_eq!(
        res["data"]["runtime"]["last_event"],
        "submission_registered"
    );
    assert_eq!(res["data"]["runtime"]["engine_state_present"], false);
    assert_eq!(res["data"]["proposals"][0]["status"], "proving");
    assert_eq!(
        res["data"]["proposals"][0]["runtime"]["provider_request_id"],
        "0x1234"
    );
    assert_eq!(
        res["data"]["proposals"][0]["runtime"]["remote_tx_hash"],
        "0xabcd"
    );
    assert_eq!(
        res["data"]["proposals"][0]["runtime"]["image_ref"],
        "0ximage"
    );
    assert_eq!(res["data"]["proposals"][0]["runtime"]["deployment"], "base");
    assert_eq!(res["data"]["proposals"][0]["runtime"]["offchain"], false);
    assert_eq!(
        res["data"]["proposals"][0]["runtime"]["engine_state_present"],
        false
    );
}

#[tokio::test]
async fn e2e_risc0_mock_failure_propagates_guest_error_to_status_and_runtime() {
    let mut config = base_config();
    config.prover.guest_system = GuestSystem::Risc0;
    config.prover.runner = RunnerKind::Local;

    let engine = risc0_fixture_engine(json!({
        "shasta_data_sources": [{
            "tx_data_from_calldata": [],
            "tx_data_from_blob": [[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]],
            "blob_commitments": [],
            "blob_proofs": [],
            "is_forced_inclusion": false
        }]
    }));
    let state = app_with_engine(
        config,
        "taiko_dev/ethereum",
        PipelineKey::ShastaRisc0,
        engine.clone(),
    );
    let app = app::build_router(state.clone());

    let (status, res) = post_json(
        &app,
        "/v3/proof/batch/shasta",
        json!({
            "proposals": [{
                "proposal_id": 3,
                "l1_inclusion_block_number": 1,
                "l2_block_numbers": [3],
                "last_anchor_block_number": 0
            }],
            "aggregate": false,
            "proof_type": "risc0",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = res["data"]["task_id"]
        .as_str()
        .expect("response task_id")
        .to_string();

    drive_engine_to_idle(&engine).await;

    let (status, res) = get_json(&app, &format!("/v3/tasks/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["data"]["status"], "failed");
    let error = res["data"]["error"].as_str().expect("error message");
    assert!(error.contains("RISC0 proposal mock execution failed"));
    assert!(error.contains("proposal mode blob usage verification failed"));
    assert!(
        res["data"].get("proof").is_none(),
        "unexpected proof in failure response: {res}"
    );

    let runtime_task = state
        .runtime
        .get_task(&id)
        .await
        .expect("read runtime task")
        .expect("runtime task exists");
    assert_eq!(runtime_task.runner_status, RunnerStatus::Failed);
    assert_eq!(runtime_task.error.as_deref(), Some(error));
}
