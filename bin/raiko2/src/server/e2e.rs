//! End-to-end (in-process) API tests.
//!
//! These tests exercise the HTTP handlers + engine orchestration without relying on
//! external RPC endpoints. A minimal JSON-RPC server is spun up only for `/ready`.

use std::sync::{Arc, Mutex};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use raiko2_engine::{Engine, EngineTaskId, EngineTaskKey, ProposalStage, ProposalTaskRequest};
use raiko2_pipeline::PipelineKey;
use raiko2_prover::{BoundlessSubmissionProgress, sp1::ProverMode as Sp1ProverMode};
use raiko2_queue::encode_task_id;
use raiko2_runtime::{RunnerStatus, TaskRegistration};
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};
use tower::ServiceExt;

use super::app;
use super::fixture::{
    app_with_engine, app_with_native_fixture_engine, base_config, native_fixture_engine,
    risc0_boundless_fixture_engine, risc0_fixture_engine, sp1_fixture_engine, spawn_chain_id_rpc,
    unique_runtime_root,
};
use super::sampling::ZkAnySampler;
use super::state::{AppState, StaticPipelineFactory};
use super::task_metadata::{HoodiProposalTask, HoodiRuntimeMetadata, HoodiTaskMetadata};
use crate::config::{GuestSystem, RunnerKind};
use raiko2_runtime::RuntimeManager;

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

fn sp1_fixture_app() -> (
    Router,
    raiko2_engine::Engine<super::fixture::Sp1FixtureSpec>,
) {
    let mut config = base_config();
    config.prover.guest_system = GuestSystem::Sp1;
    config.prover.runner = RunnerKind::Local;
    config.prover.sp1.prover = Sp1ProverMode::Local;

    let engine = sp1_fixture_engine(json!({}));
    let state = app_with_engine(
        config,
        "taiko_dev/ethereum",
        PipelineKey::ShastaSp1,
        engine.clone(),
    );
    (app::build_router(state), engine)
}

fn risc0_boundless_fixture_app() -> (
    Router,
    raiko2_engine::Engine<super::fixture::Risc0FixtureSpec>,
) {
    let mut config = base_config();
    config.prover.guest_system = GuestSystem::Risc0;
    config.prover.runner = RunnerKind::Boundless;

    let engine = risc0_boundless_fixture_engine(json!({}));
    let state = app_with_engine(
        config,
        "taiko_dev/ethereum",
        PipelineKey::ShastaRisc0Boundless,
        engine.clone(),
    );
    (app::build_router(state), engine)
}

fn sp1_external_proof(proof_hex: String) -> Value {
    json!({
        "proof": proof_hex,
        "input": format!("{:#066x}", 0),
        "uuid": "fixture-sp1-vk",
        "extra_data": {
            "sp1": {
                "proof_carry_data": {
                    "chain_id": 167001
                }
            }
        }
    })
}

fn risc0_boundless_external_proof() -> Value {
    json!({
        "quote": "0xfixture-boundless-receipt"
    })
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

#[tokio::test]
async fn e2e_ready_ok_with_matching_chain_id() {
    let (l1_rpc, l1_handle) = match spawn_chain_id_rpc(1).await {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(err) => panic!("bind mock l1 rpc listener: {err}"),
    };
    let (l2_rpc, l2_handle) = match spawn_chain_id_rpc(167_001).await {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(err) => panic!("bind mock l2 rpc listener: {err}"),
    };

    let mut config = base_config();
    config.rpc.pairs[0].l1_rpc = Some(l1_rpc);
    config.rpc.pairs[0].l2_rpc = Some(l2_rpc);
    let zk_any_sampler = Arc::new(Mutex::new(ZkAnySampler::from_config(&config.prover.zk_any)));

    let state = AppState {
        config: Arc::new(config),
        pipelines: Arc::new(StaticPipelineFactory::default()),
        runtime: Arc::new(
            RuntimeManager::new(unique_runtime_root("raiko2-e2e-ready-runtime"))
                .expect("runtime manager"),
        ),
        zk_any_sampler,
    };
    let app = app::build_router(state);

    let (status, body) = get_json(&app, "/ready").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["reth"]["ok"], true);
    assert_eq!(body["queue"]["ok"], true);
    assert_eq!(body["prover"]["ok"], true);

    l1_handle.abort();
    l2_handle.abort();
}

#[tokio::test]
async fn e2e_ready_fails_when_l1_chain_id_mismatches() {
    let (l1_rpc, l1_handle) = match spawn_chain_id_rpc(11_155_111).await {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(err) => panic!("bind mock l1 rpc listener: {err}"),
    };
    let (l2_rpc, l2_handle) = match spawn_chain_id_rpc(167_001).await {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(err) => panic!("bind mock l2 rpc listener: {err}"),
    };

    let mut config = base_config();
    config.rpc.pairs[0].l1_rpc = Some(l1_rpc);
    config.rpc.pairs[0].l2_rpc = Some(l2_rpc);
    let zk_any_sampler = Arc::new(Mutex::new(ZkAnySampler::from_config(&config.prover.zk_any)));

    let state = AppState {
        config: Arc::new(config),
        pipelines: Arc::new(StaticPipelineFactory::default()),
        runtime: Arc::new(
            RuntimeManager::new(unique_runtime_root("raiko2-e2e-ready-l1-mismatch-runtime"))
                .expect("runtime manager"),
        ),
        zk_any_sampler,
    };
    let app = app::build_router(state);

    let (status, body) = get_json(&app, "/ready").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["status"], "error");
    assert_eq!(body["reth"]["ok"], false);
    assert!(
        body["reth"]["error"]
            .as_str()
            .expect("reth error")
            .contains("l1 chain_id mismatch")
    );

    l1_handle.abort();
    l2_handle.abort();
}

#[tokio::test]
async fn e2e_ready_fails_when_boundless_signer_is_invalid() {
    let (l1_rpc, l1_handle) = match spawn_chain_id_rpc(1).await {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(err) => panic!("bind mock l1 rpc listener: {err}"),
    };
    let (l2_rpc, l2_handle) = match spawn_chain_id_rpc(167_001).await {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(err) => panic!("bind mock l2 rpc listener: {err}"),
    };

    let mut config = base_config();
    config.rpc.pairs[0].l1_rpc = Some(l1_rpc);
    config.rpc.pairs[0].l2_rpc = Some(l2_rpc);
    config.prover.guest_system = GuestSystem::Risc0;
    config.prover.runner = RunnerKind::Boundless;
    config.prover.boundless.rpc_url = "https://base-rpc.publicnode.com".to_string();
    config.prover.boundless.signer_key = "not-a-private-key".to_string();
    let zk_any_sampler = Arc::new(Mutex::new(ZkAnySampler::from_config(&config.prover.zk_any)));

    let state = AppState {
        config: Arc::new(config),
        pipelines: Arc::new(StaticPipelineFactory::default()),
        runtime: Arc::new(
            RuntimeManager::new(unique_runtime_root("raiko2-e2e-ready-boundless-runtime"))
                .expect("runtime manager"),
        ),
        zk_any_sampler,
    };
    let app = app::build_router(state);

    let (status, body) = get_json(&app, "/ready").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["status"], "error");
    assert_eq!(body["reth"]["ok"], true);
    assert_eq!(body["queue"]["ok"], true);
    assert_eq!(body["prover"]["ok"], false);
    assert!(
        body["prover"]["error"]
            .as_str()
            .expect("prover error")
            .contains("boundless signer_key is invalid")
    );

    l1_handle.abort();
    l2_handle.abort();
}

#[tokio::test]
async fn e2e_ready_fails_when_l2_witness_chain_id_mismatches() {
    let (l1_rpc, l1_handle) = match spawn_chain_id_rpc(1).await {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(err) => panic!("bind mock l1 rpc listener: {err}"),
    };
    let (l2_rpc, l2_handle) = match spawn_chain_id_rpc(167_001).await {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(err) => panic!("bind mock l2 rpc listener: {err}"),
    };
    let (l2_witness_rpc, l2_witness_handle) = match spawn_chain_id_rpc(167_013).await {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(err) => panic!("bind mock l2 witness rpc listener: {err}"),
    };

    let mut config = base_config();
    config.rpc.pairs[0].l1_rpc = Some(l1_rpc);
    config.rpc.pairs[0].l2_rpc = Some(l2_rpc);
    config.rpc.pairs[0].l2_witness_rpc = Some(l2_witness_rpc);
    let zk_any_sampler = Arc::new(Mutex::new(ZkAnySampler::from_config(&config.prover.zk_any)));

    let state = AppState {
        config: Arc::new(config),
        pipelines: Arc::new(StaticPipelineFactory::default()),
        runtime: Arc::new(
            RuntimeManager::new(unique_runtime_root("raiko2-e2e-ready-l2-witness-runtime"))
                .expect("runtime manager"),
        ),
        zk_any_sampler,
    };
    let app = app::build_router(state);

    let (status, body) = get_json(&app, "/ready").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["status"], "error");
    assert_eq!(body["reth"]["ok"], false);
    assert!(
        body["reth"]["error"]
            .as_str()
            .expect("reth error")
            .contains("l2_witness chain_id mismatch")
    );

    l1_handle.abort();
    l2_handle.abort();
    l2_witness_handle.abort();
}

#[tokio::test]
async fn e2e_ready_fails_when_sp1_verification_is_disabled() {
    let (l1_rpc, l1_handle) = match spawn_chain_id_rpc(1).await {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(err) => panic!("bind mock l1 rpc listener: {err}"),
    };
    let (l2_rpc, l2_handle) = match spawn_chain_id_rpc(167_001).await {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(err) => panic!("bind mock l2 rpc listener: {err}"),
    };

    let mut config = base_config();
    config.rpc.pairs[0].l1_rpc = Some(l1_rpc);
    config.rpc.pairs[0].l2_rpc = Some(l2_rpc);
    config.prover.guest_system = GuestSystem::Sp1;
    config.prover.runner = RunnerKind::Local;
    config.prover.sp1.prover = Sp1ProverMode::Local;
    config.prover.sp1.verify = false;
    let zk_any_sampler = Arc::new(Mutex::new(ZkAnySampler::from_config(&config.prover.zk_any)));

    let state = AppState {
        config: Arc::new(config),
        pipelines: Arc::new(StaticPipelineFactory::default()),
        runtime: Arc::new(
            RuntimeManager::new(unique_runtime_root("raiko2-e2e-ready-sp1-verify-runtime"))
                .expect("runtime manager"),
        ),
        zk_any_sampler,
    };
    let app = app::build_router(state);

    let (status, body) = get_json(&app, "/ready").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["status"], "error");
    assert_eq!(body["prover"]["ok"], false);
    let prover_error = body["prover"]["error"].as_str().expect("prover error");
    assert!(
        prover_error.contains("prover.sp1.verify must be true when prover.sp1.mode=prove"),
        "unexpected prover error: {prover_error}"
    );

    l1_handle.abort();
    l2_handle.abort();
}

#[tokio::test]
async fn e2e_ready_checks_sp1_even_when_risc0_boundless_is_default() {
    let (l1_rpc, l1_handle) = match spawn_chain_id_rpc(1).await {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(err) => panic!("bind mock l1 rpc listener: {err}"),
    };
    let (l2_rpc, l2_handle) = match spawn_chain_id_rpc(167_001).await {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(err) => panic!("bind mock l2 rpc listener: {err}"),
    };

    let mut config = base_config();
    config.rpc.pairs[0].l1_rpc = Some(l1_rpc);
    config.rpc.pairs[0].l2_rpc = Some(l2_rpc);
    config.prover.guest_system = GuestSystem::Risc0;
    config.prover.runner = RunnerKind::Boundless;
    config.prover.boundless.rpc_url = "https://base-rpc.publicnode.com".to_string();
    config.prover.boundless.signer_key =
        "0x45f40b61ccb3a68af7eca7d54035df42ec3786c940387d3a14dea058ac68ef3b".to_string();
    config.prover.sp1.prover = Sp1ProverMode::Local;
    config.prover.sp1.verify = false;
    let zk_any_sampler = Arc::new(Mutex::new(ZkAnySampler::from_config(&config.prover.zk_any)));

    let state = AppState {
        config: Arc::new(config),
        pipelines: Arc::new(StaticPipelineFactory::default()),
        runtime: Arc::new(
            RuntimeManager::new(unique_runtime_root("raiko2-e2e-ready-multi-zk-runtime"))
                .expect("runtime manager"),
        ),
        zk_any_sampler,
    };
    let app = app::build_router(state);

    let (status, body) = get_json(&app, "/ready").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["status"], "error");
    assert_eq!(body["prover"]["ok"], false);
    let prover_error = body["prover"]["error"].as_str().expect("prover error");
    assert!(
        prover_error.contains("configured proving capabilities are invalid"),
        "unexpected prover error: {prover_error}"
    );
    assert!(
        prover_error.contains("prover.sp1.verify must be true when prover.sp1.mode=prove"),
        "unexpected prover error: {prover_error}"
    );

    l1_handle.abort();
    l2_handle.abort();
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
async fn e2e_zk_any_returns_not_drawn_when_ballot_is_disabled() {
    let mut config = base_config();
    config.prover.zk_any = Default::default();
    let engine = native_fixture_engine();
    let state = app_with_engine(
        config,
        "taiko_dev/ethereum",
        PipelineKey::ShastaNative,
        engine,
    );
    let app = app::build_router(state);

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
            "proof_type": "zk_any",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["status"], "ok");
    assert_eq!(res["proof_type"], "native");
    assert_eq!(res["data"]["status"], "zk_any_not_drawn");
    assert!(res["data"].get("task_id").is_none(), "{res}");
}

#[tokio::test]
async fn e2e_zk_any_draws_sp1_and_registers_sp1_task() {
    let mut config = base_config();
    config.prover.zk_any.sp1 = Some(crate::config::ZkAnyTargetConfig {
        probability: 1.0,
        per_day: 0,
    });
    let engine = sp1_fixture_engine(json!({}));
    let state = app_with_engine(
        config,
        "taiko_dev/ethereum",
        PipelineKey::ShastaSp1,
        engine.clone(),
    );
    let app = app::build_router(state);

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
            "proof_type": "zk_any",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["proof_type"], "sp1");
    assert_eq!(res["data"]["status"], "registered");
    let id = res["data"]["task_id"]
        .as_str()
        .expect("response task_id")
        .to_string();

    drive_engine_to_idle(&engine).await;

    let (status, res) = get_json(&app, &format!("/v3/tasks/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["data"]["route"], "sp1/local");
    assert_eq!(res["data"]["status"], "completed");
}

#[tokio::test]
async fn e2e_zk_any_rejects_prover_args() {
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
            "proof_type": "zk_any",
            "network": "taiko_dev",
            "l1_network": "ethereum",
            "sp1": {
                "mode": "execute"
            }
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        res["message"],
        "proof_type=zk_any does not support prover args"
    );
}

#[tokio::test]
async fn e2e_sp1_execute_returns_execution_metadata() {
    let mut config = base_config();
    config.prover.guest_system = GuestSystem::Sp1;
    config.prover.runner = RunnerKind::Local;
    config.prover.sp1.prover = Sp1ProverMode::Local;

    let engine = sp1_fixture_engine(json!({}));
    let state = app_with_engine(
        config,
        "taiko_dev/ethereum",
        PipelineKey::ShastaSp1,
        engine.clone(),
    );
    let app = app::build_router(state);

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
            "proof_type": "sp1",
            "network": "taiko_dev",
            "l1_network": "ethereum",
            "sp1": {
                "mode": "execute",
                "prover": "local"
            }
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
    assert_eq!(res["data"]["route"], "sp1/local");
    assert_eq!(res["data"]["execution_mode"], "execute");
    assert_eq!(res["data"]["status"], "completed");
    assert!(
        res["data"]["proof"].is_null() || res["data"].get("proof").is_none(),
        "unexpected root proof payload: {res}"
    );
    assert_eq!(res["data"]["proposals"][0]["status"], "completed");
    assert!(
        res["data"]["proposals"][0]["proof"].is_null()
            || res["data"]["proposals"][0].get("proof").is_none(),
        "unexpected proposal proof payload: {res}"
    );
    assert_eq!(
        res["data"]["proposals"][0]["extra_data"]["sp1"]["mode"],
        "execute"
    );
    assert_eq!(
        res["data"]["proposals"][0]["extra_data"]["sp1"]["zkvm"],
        "sp1"
    );
    assert!(
        res["data"]["proposals"][0]["extra_data"]["sp1"]["public_values"]
            .as_str()
            .is_some(),
        "missing public values: {res}"
    );
}

#[tokio::test]
async fn e2e_batch_aggregate_sp1_completes_from_fixture() {
    let (app, engine) = sp1_fixture_app();

    let (status, res) = post_json(
        &app,
        "/v3/proof/batch/shasta",
        json!({
            "proposals": [
                {
                    "proposal_id": 3,
                    "l1_inclusion_block_number": 1,
                    "l2_block_numbers": [3],
                    "last_anchor_block_number": 0
                },
                {
                    "proposal_id": 3,
                    "l1_inclusion_block_number": 1,
                    "l2_block_numbers": [3],
                    "last_anchor_block_number": 0
                }
            ],
            "aggregate": true,
            "proof_type": "sp1",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    let id = res["data"]["task_id"]
        .as_str()
        .expect("response task_id")
        .to_string();

    drive_engine_to_idle(&engine).await;

    let (status, res) = get_json(&app, &format!("/v3/tasks/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["data"]["status"], "completed");
    assert_eq!(res["data"]["aggregate"]["status"], "completed");
    assert_eq!(res["data"]["proof"], "0xfixture-sp1-aggregation");
}

#[tokio::test]
async fn e2e_aggregate_sp1_external_proofs_completes_from_fixture() {
    let (app, engine) = sp1_fixture_app();

    let (status, res) = post_json(
        &app,
        "/v3/proof/aggregate",
        json!({
            "proofs": [
                sp1_external_proof("0xfixture-proof-a".to_string()),
                sp1_external_proof("0xfixture-proof-b".to_string())
            ],
            "proof_type": "sp1",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    let id = res["data"]["task_id"]
        .as_str()
        .expect("response task_id")
        .to_string();

    drive_engine_to_idle(&engine).await;

    let (status, res) = get_json(&app, &format!("/v3/tasks/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["data"]["status"], "completed");
    assert_eq!(res["data"]["aggregate"]["status"], "completed");
    assert_eq!(res["data"]["proof"], "0xfixture-sp1-aggregation");
}

#[tokio::test]
async fn e2e_aggregate_risc0_boundless_external_proofs_completes_from_fixture() {
    let (app, engine) = risc0_boundless_fixture_app();

    let (status, res) = post_json(
        &app,
        "/v3/proof/aggregate",
        json!({
            "proofs": [
                risc0_boundless_external_proof(),
                risc0_boundless_external_proof()
            ],
            "proof_type": "risc0",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    let id = res["data"]["task_id"]
        .as_str()
        .expect("response task_id")
        .to_string();

    drive_engine_to_idle(&engine).await;

    let (status, res) = get_json(&app, &format!("/v3/tasks/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["data"]["route"], "risc0/boundless");
    assert_eq!(res["data"]["status"], "completed");
    assert_eq!(res["data"]["aggregate"]["status"], "completed");
    assert_eq!(res["data"]["proof"], "0xfixture-risc0-aggregation");
}

#[tokio::test]
async fn e2e_aggregate_rejects_zk_any() {
    let (app, _engine) = sp1_fixture_app();

    let (status, res) = post_json(
        &app,
        "/v3/proof/aggregate",
        json!({
            "proofs": [
                sp1_external_proof("0xfixture-proof-a".to_string()),
                sp1_external_proof("0xfixture-proof-b".to_string())
            ],
            "proof_type": "zk_any",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{res}");
    assert_eq!(
        res["message"],
        "proof_type=zk_any is not supported for aggregate requests"
    );
}

#[tokio::test]
async fn e2e_report_and_list_expose_root_tasks_only() {
    let config = base_config();
    let engine = native_fixture_engine();
    let app = app_with_native_fixture_engine(config, engine.clone());

    let payload = |proposal_id| {
        json!({
            "proposals": [{
                "proposal_id": proposal_id,
                "l1_inclusion_block_number": 1,
                "l2_block_numbers": [proposal_id],
                "last_anchor_block_number": 0
            }],
            "aggregate": false,
            "proof_type": "native",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        })
    };

    let (status, first) = post_json(&app, "/v3/proof/batch/shasta", payload(3)).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let first_id = first["data"]["task_id"]
        .as_str()
        .expect("first task id")
        .to_string();

    drive_engine_to_idle(&engine).await;

    let (status, second) = post_json(&app, "/v3/proof/batch/shasta", payload(4)).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    let second_id = second["data"]["task_id"]
        .as_str()
        .expect("second task id")
        .to_string();

    let (status, report) = get_json(&app, "/proof/report").await;
    assert_eq!(status, StatusCode::OK);
    let report = report.as_array().expect("report array");
    assert_eq!(report.len(), 2);
    assert!(report.iter().any(|entry| entry["task_id"] == first_id));
    assert!(report.iter().any(|entry| entry["task_id"] == second_id));

    let (status, list) = get_json(&app, "/v3/proof/list").await;
    assert_eq!(status, StatusCode::OK);
    let list = list.as_array().expect("list array");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["task_id"], first_id);
}

#[tokio::test]
async fn e2e_prune_clears_runtime_and_alias_routes() {
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
    assert_eq!(status, StatusCode::OK, "{res}");
    let id = res["data"]["task_id"]
        .as_str()
        .expect("response task_id")
        .to_string();

    let (status, prune) = post_json(&app, "/proof/prune", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(prune["status"], "ok");

    let (status, report) = get_json(&app, "/v3/proof/report").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report.as_array().map(Vec::len), Some(0));

    let (status, task) = get_json(&app, &format!("/v3/tasks/{id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(task["status"], "error");
}

#[tokio::test]
async fn e2e_sp1_execute_rejects_aggregate_requests() {
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
            "aggregate": true,
            "proof_type": "sp1",
            "network": "taiko_dev",
            "l1_network": "ethereum",
            "sp1": {
                "mode": "execute",
                "prover": "local"
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        res["message"],
        "sp1.mode=execute does not support aggregate=true"
    );
}

#[tokio::test]
async fn e2e_sp1_hosted_api_rejects_unverified_prove_requests() {
    let mut config = base_config();
    config.prover.guest_system = GuestSystem::Sp1;
    config.prover.runner = RunnerKind::Local;
    config.prover.sp1.prover = Sp1ProverMode::Local;
    config.prover.sp1.verify = false;

    let engine = sp1_fixture_engine(json!({}));
    let state = app_with_engine(config, "taiko_dev/ethereum", PipelineKey::ShastaSp1, engine);
    let app = app::build_router(state);

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
            "proof_type": "sp1",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{res}");
    assert_eq!(
        res["message"],
        "sp1.mode=prove requires sp1.verify=true on the hosted API"
    );
}

#[tokio::test]
async fn e2e_sp1_hosted_api_rejects_network_verify_when_pair_not_enabled() {
    let mut config = base_config();
    config.prover.guest_system = GuestSystem::Sp1;
    config.prover.runner = RunnerKind::Local;
    config.prover.sp1.prover = Sp1ProverMode::Network;
    config.prover.sp1.verify = true;

    let engine = sp1_fixture_engine(json!({}));
    let state = app_with_engine(config, "taiko_dev/ethereum", PipelineKey::ShastaSp1, engine);
    let app = app::build_router(state);

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
            "proof_type": "sp1",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{res}");
    assert_eq!(
        res["message"],
        "sp1 network verification is not enabled for network pair taiko_dev/ethereum"
    );
}

#[tokio::test]
async fn e2e_sp1_hosted_api_accepts_network_verify_when_pair_enabled() {
    let mut config = base_config();
    config.prover.guest_system = GuestSystem::Sp1;
    config.prover.runner = RunnerKind::Local;
    config.prover.sp1.prover = Sp1ProverMode::Network;
    config.prover.sp1.verify = true;
    config.rpc.pairs[0].sp1_verifier_rpc_url = Some("https://verifier.example.com".to_string());
    config.rpc.pairs[0].sp1_verifier_address =
        Some("0x0000000000000000000000000000000000000001".to_string());

    let engine = sp1_fixture_engine(json!({}));
    let state = app_with_engine(config, "taiko_dev/ethereum", PipelineKey::ShastaSp1, engine);
    let app = app::build_router(state);

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
            "proof_type": "sp1",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["proof_type"], "sp1");
    assert_eq!(res["data"]["status"], "registered");
    assert!(res["data"]["task_id"].as_str().is_some());
}

#[tokio::test]
async fn e2e_sp1_network_settings_require_network_prover() {
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
            "proof_type": "sp1",
            "network": "taiko_dev",
            "l1_network": "ethereum",
            "sp1": {
                "prover": "local",
                "network_mode": "mainnet"
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        res["message"],
        "sp1 network-only settings require sp1.prover=network"
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
async fn e2e_cancel_does_not_cancel_shared_engine_task_for_other_root_task() {
    let config = base_config();
    let engine = native_fixture_engine();
    let app = app_with_native_fixture_engine(config, engine);
    let payload = json!({
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
    });

    let (status, first) = post_json(&app, "/v3/proof/batch/shasta", payload.clone()).await;
    assert_eq!(status, StatusCode::OK);
    let first_id = first["data"]["task_id"]
        .as_str()
        .expect("first response task_id")
        .to_string();

    let (status, second) = post_json(&app, "/v3/proof/batch/shasta", payload).await;
    assert_eq!(status, StatusCode::OK);
    let second_id = second["data"]["task_id"]
        .as_str()
        .expect("second response task_id")
        .to_string();

    let (status, first_task) = get_json(&app, &format!("/v3/tasks/{first_id}")).await;
    assert_eq!(status, StatusCode::OK);
    let (status, second_task) = get_json(&app, &format!("/v3/tasks/{second_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        first_task["data"]["proposals"][0]["task_id"],
        second_task["data"]["proposals"][0]["task_id"]
    );

    let (status, cancelled) =
        post_json(&app, &format!("/v3/tasks/{first_id}/cancel"), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cancelled["data"]["status"], "cancelled");

    let (status, remaining) = get_json(&app, &format!("/v3/tasks/{second_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(remaining["data"]["status"], "pending");
    assert_ne!(remaining["data"]["runtime"]["runner_status"], "cancelled");
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
async fn e2e_task_status_falls_back_to_runtime_metadata_without_mutating_runtime_store() {
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
            checkpoint: None,
            blob_proof_type: None,
            prover: None,
            graffiti: None,
            prover_config: Default::default(),
        },
        stage: ProposalStage::Prove,
    });
    let encoded_task_id = encode_task_id(&proposal_task_id).expect("encode task id");
    let mut metadata = HoodiTaskMetadata {
        network_pair: "taiko_dev/ethereum".to_string(),
        network: "taiko_dev".to_string(),
        l1_network: "ethereum".to_string(),
        proof_type: raiko2_primitives::ProofType::Native,
        execution_mode: None,
        aggregate_requested: false,
        proposals: vec![HoodiProposalTask {
            proposal_id: 3,
            checkpoint: None,
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
            quoted_mcycles_count: Some(6_000),
            evaluated_mcycles_count: Some(12_345),
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
    assert_eq!(res["data"]["runtime"]["runner_status"], "allocated");
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
        res["data"]["proposals"][0]["runtime"]["quoted_mcycles_count"],
        6000
    );
    assert_eq!(
        res["data"]["proposals"][0]["runtime"]["evaluated_mcycles_count"],
        12345
    );
    assert_eq!(
        res["data"]["proposals"][0]["runtime"]["engine_state_present"],
        false
    );

    let runtime_task = state
        .runtime
        .get_task("task_runtime_fallback")
        .await
        .expect("read runtime task")
        .expect("runtime task exists");
    assert_eq!(runtime_task.runner_status, RunnerStatus::Allocated);
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
    assert_eq!(runtime_task.runner_status, RunnerStatus::Allocated);
    assert_eq!(runtime_task.error, None);
}
