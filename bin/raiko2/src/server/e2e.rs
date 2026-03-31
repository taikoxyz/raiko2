//! End-to-end (in-process) API tests.
//!
//! These tests exercise the HTTP handlers + engine orchestration without relying on
//! external RPC endpoints. A minimal JSON-RPC server is spun up only for `/ready`.

use std::sync::Arc;

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
    native_fixture_engine_with_pipeline, risc0_fixture_engine_with_pipeline, sp1_fixture_engine,
    spawn_chain_id_rpc, unique_runtime_root,
};
use super::state::{AppState, StaticPipelineFactory};
use super::task_metadata::{HoodiProposalTask, HoodiRuntimeMetadata, HoodiTaskMetadata};
use crate::config::{GuestSystem, RunnerKind};
use raiko2_runtime::RuntimeManager;

/// Which batch proof route + pipeline namespace the in-process e2e exercises.
#[derive(Clone, Copy, Debug)]
enum E2eBatchFamily {
    Shasta,
    Uzen,
}

impl E2eBatchFamily {
    fn native_pipeline_key(self) -> PipelineKey {
        match self {
            E2eBatchFamily::Shasta => PipelineKey::ShastaNative,
            E2eBatchFamily::Uzen => PipelineKey::UzenNative,
        }
    }

    fn risc0_pipeline_key(self) -> PipelineKey {
        match self {
            E2eBatchFamily::Shasta => PipelineKey::ShastaRisc0,
            E2eBatchFamily::Uzen => PipelineKey::UzenRisc0,
        }
    }

    fn batch_proof_path(self) -> &'static str {
        match self {
            E2eBatchFamily::Shasta => "/v3/proof/batch/shasta",
            E2eBatchFamily::Uzen => "/v3/proof/batch/uzen",
        }
    }
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
    for family in [E2eBatchFamily::Shasta, E2eBatchFamily::Uzen] {
        let config = base_config();
        let engine = native_fixture_engine_with_pipeline(family.native_pipeline_key());
        let app = app_with_native_fixture_engine(
            config,
            engine.clone(),
            family.native_pipeline_key(),
        );

        let (status, res) = post_json(
            &app,
            family.batch_proof_path(),
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
        assert_eq!(status, StatusCode::OK, "family={family:?}");
        let id = res["data"]["task_id"]
            .as_str()
            .expect("response task_id")
            .to_string();

        drive_engine_to_idle(&engine).await;

        let (status, res) = get_json(&app, &format!("/v3/tasks/{id}")).await;
        assert_eq!(status, StatusCode::OK, "family={family:?}");
        assert_eq!(res["data"]["status"], "completed", "family={family:?}");
        assert!(
            res["data"].get("error").is_none(),
            "unexpected error: {res} family={family:?}"
        );
    }
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
async fn e2e_sp1_execute_rejects_aggregate_requests() {
    let config = base_config();
    let engine = native_fixture_engine();
    let app = app_with_native_fixture_engine(config, engine, PipelineKey::ShastaNative);

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
async fn e2e_sp1_network_settings_require_network_prover() {
    let config = base_config();
    let engine = native_fixture_engine();
    let app = app_with_native_fixture_engine(config, engine, PipelineKey::ShastaNative);

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
    for family in [E2eBatchFamily::Shasta, E2eBatchFamily::Uzen] {
        let config = base_config();
        let engine = native_fixture_engine_with_pipeline(family.native_pipeline_key());
        let app = app_with_native_fixture_engine(
            config,
            engine,
            family.native_pipeline_key(),
        );

        let (status, res) = post_json(
            &app,
            family.batch_proof_path(),
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
        assert_eq!(status, StatusCode::OK, "family={family:?}");
        let id = res["data"]["task_id"]
            .as_str()
            .expect("response task_id")
            .to_string();

        let (status, res) = post_json(&app, &format!("/v3/tasks/{id}/cancel"), json!({})).await;
        assert_eq!(status, StatusCode::OK, "family={family:?}");
        assert_eq!(res["data"]["status"], "cancelled", "family={family:?}");
    }
}

#[tokio::test]
async fn e2e_task_status_turns_proving_after_preflight_progress() {
    let config = base_config();
    let engine = native_fixture_engine();
    let app = app_with_native_fixture_engine(config, engine.clone(), PipelineKey::ShastaNative);

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
        proof_type: raiko2_primitives::ProofType::Native,
        execution_mode: None,
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
}

#[tokio::test]
async fn e2e_risc0_mock_failure_propagates_guest_error_to_status_and_runtime() {
    for family in [E2eBatchFamily::Shasta, E2eBatchFamily::Uzen] {
        let mut config = base_config();
        config.prover.guest_system = GuestSystem::Risc0;
        config.prover.runner = RunnerKind::Local;

        let engine = risc0_fixture_engine_with_pipeline(
            family.risc0_pipeline_key(),
            json!({
                "shasta_data_sources": [{
                    "tx_data_from_calldata": [],
                    "tx_data_from_blob": [[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]],
                    "blob_commitments": [],
                    "blob_proofs": [],
                    "is_forced_inclusion": false
                }]
            }),
        );
        let state = app_with_engine(
            config,
            "taiko_dev/ethereum",
            family.risc0_pipeline_key(),
            engine.clone(),
        );
        let app = app::build_router(state.clone());

        let (status, res) = post_json(
            &app,
            family.batch_proof_path(),
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
        assert_eq!(status, StatusCode::OK, "family={family:?}");
        let id = res["data"]["task_id"]
            .as_str()
            .expect("response task_id")
            .to_string();

        drive_engine_to_idle(&engine).await;

        let (status, res) = get_json(&app, &format!("/v3/tasks/{id}")).await;
        assert_eq!(status, StatusCode::OK, "family={family:?}");
        assert_eq!(res["data"]["status"], "failed", "family={family:?}");
        let error = res["data"]["error"].as_str().expect("error message");
        assert!(error.contains("RISC0 proposal mock execution failed"));
        assert!(error.contains("proposal mode blob usage verification failed"));
        assert!(
            res["data"].get("proof").is_none(),
            "unexpected proof in failure response: {res} family={family:?}"
        );

        let runtime_task = state
            .runtime
            .get_task(&id)
            .await
            .expect("read runtime task")
            .expect("runtime task exists");
        assert_eq!(
            runtime_task.runner_status,
            RunnerStatus::Failed,
            "family={family:?}"
        );
        assert_eq!(
            runtime_task.error.as_deref(),
            Some(error),
            "family={family:?}"
        );
    }
}
