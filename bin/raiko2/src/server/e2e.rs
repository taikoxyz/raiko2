//! End-to-end (in-process) API tests.
//!
//! These tests exercise the HTTP handlers + engine orchestration without relying on
//! external RPC endpoints. A minimal JSON-RPC server is spun up only for `/ready`.

mod invalidation_prefix;
mod invalidation_range;

use std::sync::Arc;

use alloy_primitives::{hex, keccak256};
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use raiko2_engine::{Engine, ProposalTaskRequest, ProverTaskConfig};
use raiko2_pipeline::{PipelineKey, PipelineRoute};
use raiko2_primitives::Proof;
use raiko2_primitives_shasta::encode_proof_carry_data;
use raiko2_protocol_shasta::shasta::ProofCarryData;
use raiko2_prover::{BoundlessSubmissionProgress, sp1::ProverMode as Sp1ProverMode};
use raiko2_runtime::{RunnerStatus, TaskRegistration};
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};
use tower::ServiceExt;

use super::app;
use super::fixture::app_with_observed_risc0_boundless_fixture_engine;
use super::fixture::{
    Sp1FixtureEngine, app_with_engine, app_with_observed_native_fixture_engine,
    app_with_observed_risc0_fixture_engine, app_with_observed_sp1_fixture_engine,
    app_with_risc0_fixture_engine, base_config, engine_observer,
    native_fixture_engine_for_pipeline, risc0_fixture_engine, risc0_fixture_engine_with_observer,
    sp1_fixture_engine, spawn_chain_id_rpc, state_with_observed_risc0_fixture_engine,
    state_with_observed_sp1_fixture_engine, unique_runtime_root,
};
use super::state::{AppState, StaticPipelineFactory};
use super::task_metadata::{
    ProposalTask, RuntimeMetadata, TaskMetadata, TaskRuntimeMetadata, proposal_proof_artifact_refs,
    proposal_task_ref, publication_proof_artifact_refs, root_proof_artifact_refs,
};
use crate::config::{Config, ServerAclFeature, ServerAclKey};
use raiko2_runtime::test_support::{MemoryProofArtifactStore, RuntimeStore};
use raiko2_runtime::{ProofArtifactRegistration, RuntimeManager};

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

async fn read_text(res: axum::response::Response) -> (StatusCode, String) {
    let status = res.status();
    let bytes = res
        .into_body()
        .collect()
        .await
        .expect("read response body")
        .to_bytes();
    let body = String::from_utf8(bytes.to_vec()).expect("parse text response");
    (status, body)
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

fn acl_key(id: &str, key: &str, allow: Vec<ServerAclFeature>) -> ServerAclKey {
    acl_key_with_rate_limit(id, key, allow, None)
}

fn acl_key_with_rate_limit(
    id: &str,
    key: &str,
    allow: Vec<ServerAclFeature>,
    rate_limit_per_minute: Option<u32>,
) -> ServerAclKey {
    ServerAclKey {
        id: id.to_string(),
        key: key.to_string(),
        allow,
        rate_limit_per_minute,
    }
}

async fn get_json_with_api_key(app: &Router, uri: &str, key: &str) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .header("x-api-key", key)
        .body(Body::empty())
        .expect("build authenticated GET request");
    let res = app.clone().oneshot(req).await.expect("dispatch request");
    read_json(res).await
}

async fn get_text(app: &Router, uri: &str) -> (StatusCode, String) {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("build GET request");
    let res = app.clone().oneshot(req).await.expect("dispatch request");
    read_text(res).await
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

async fn post_json_with_api_key(
    app: &Router,
    uri: &str,
    key: &str,
    payload: Value,
) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("x-api-key", key)
        .body(Body::from(payload.to_string()))
        .expect("build authenticated POST request");
    let res = app.clone().oneshot(req).await.expect("dispatch request");
    read_json(res).await
}

async fn post_raw_json_text_with_optional_api_key(
    app: &Router,
    uri: &str,
    key: Option<&str>,
    body: &str,
) -> (StatusCode, String) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(key) = key {
        builder = builder.header("x-api-key", key);
    }
    let req = builder
        .body(Body::from(body.to_string()))
        .expect("build raw POST request");
    let res = app.clone().oneshot(req).await.expect("dispatch request");
    read_text(res).await
}

async fn write_e2e_proof_artifact(
    state: &AppState,
    network_pair: &str,
    proof_ref: &str,
    pipeline_key: PipelineKey,
    route: PipelineRoute,
    proof: &Proof,
) -> String {
    let publication = state
        .runtime
        .publish_proof_artifact_bytes(
            network_pair,
            pipeline_key,
            route,
            proof_ref,
            &serde_json::to_vec(proof).expect("serialize proof"),
        )
        .await
        .expect("write proof artifact");
    let artifact = publication
        .try_object()
        .expect("proof publication should materialize content");
    state
        .runtime
        .upsert_proof_artifact(ProofArtifactRegistration {
            network_pair: network_pair.to_string(),
            proof_ref: proof_ref.to_string(),
            pipeline_key,
            route,
            proof_uri: artifact.proof_uri.clone(),
            content_hash: artifact.content_hash.clone(),
            generation: artifact.generation,
        })
        .await
        .expect("register proof artifact");
    artifact.proof_uri.clone()
}

async fn replace_e2e_proof_artifact(
    state: &AppState,
    network_pair: &str,
    proof_ref: &str,
    pipeline_key: PipelineKey,
    route: PipelineRoute,
    proof: &Proof,
) {
    let old = state
        .runtime
        .get_proof_artifact(network_pair, pipeline_key, route, proof_ref)
        .await
        .expect("get proof artifact")
        .expect("existing proof artifact");
    state
        .runtime
        .delete_proof_artifact(
            network_pair,
            pipeline_key,
            route,
            proof_ref,
            old.generation,
            &old.content_hash,
        )
        .await
        .expect("delete proof artifact manifest");
    state
        .runtime
        .remove_proof_artifact_if_descriptor(
            network_pair,
            pipeline_key,
            route,
            proof_ref,
            &old.descriptor(),
        )
        .await
        .expect("remove proof artifact record");
    write_e2e_proof_artifact(state, network_pair, proof_ref, pipeline_key, route, proof).await;
}

async fn write_e2e_pending_publication(
    state: &AppState,
    network_pair: &str,
    proof_ref: &str,
    pipeline_key: PipelineKey,
    route: PipelineRoute,
) {
    state
        .runtime
        .upsert_pending_proof_publication(
            network_pair,
            pipeline_key,
            route,
            proof_ref,
            br#"{"proof":"0xpending"}"#,
        )
        .await
        .expect("write pending proof publication");
}

async fn assert_e2e_pending_publication(
    state: &AppState,
    network_pair: &str,
    proof_ref: &str,
    pipeline_key: PipelineKey,
    route: PipelineRoute,
    expected: bool,
) {
    assert_eq!(
        state
            .runtime
            .get_pending_proof_publication(network_pair, pipeline_key, route, proof_ref)
            .await
            .expect("get pending proof publication")
            .is_some(),
        expected,
        "unexpected pending publication state for {proof_ref}"
    );
}

async fn report_task_ids(app: &Router) -> Vec<String> {
    let (status, report) = get_json(app, "/v3/proof/report").await;
    assert_eq!(status, StatusCode::OK, "{report}");
    report
        .as_array()
        .expect("report array")
        .iter()
        .map(|entry| {
            entry["task_id"]
                .as_str()
                .expect("report task_id")
                .to_string()
        })
        .collect()
}

async fn single_report_task_id(app: &Router) -> String {
    let ids = report_task_ids(app).await;
    assert_eq!(ids.len(), 1, "expected one runtime task, got {ids:?}");
    ids.into_iter().next().expect("single task id")
}

fn duplicate_request_fingerprint(
    pair_key: &str,
    route: &str,
    aggregate_requested: bool,
    proposals: &[Value],
) -> String {
    let payload = json!({
        "pair_key": pair_key,
        "route": route,
        "prover_type": "local",
        "aggregate_requested": aggregate_requested,
        "execution_mode": Value::Null,
        "blob_proof_type": Value::Null,
        "prover": Value::Null,
        "graffiti": Value::Null,
        "prover_config": ProverTaskConfig::default(),
        "proposals": proposals,
    });
    hex::encode_prefixed(
        keccak256(serde_json::to_vec(&payload).expect("serialize duplicate fingerprint"))
            .as_slice(),
    )
}

fn deterministic_test_private_key(label: &str) -> String {
    hex::encode_prefixed(keccak256(label.as_bytes()).as_slice())
}

fn set_prover_routes(config: &mut Config, routes: &str) {
    config
        .prover
        .apply_routes_override(&routes.parse().expect("valid prover routes"));
}

fn sp1_fixture_app() -> (
    Router,
    raiko2_engine::Engine<super::fixture::Sp1FixtureSpec>,
) {
    let mut config = base_config();
    set_prover_routes(&mut config, "sp1/local");
    config.prover.sp1.prover = Sp1ProverMode::Local;

    app_with_observed_sp1_fixture_engine(config)
}

fn risc0_boundless_fixture_app() -> (
    Router,
    raiko2_engine::Engine<super::fixture::Risc0FixtureSpec>,
) {
    let mut config = base_config();
    set_prover_routes(&mut config, "risc0/network");

    app_with_observed_risc0_boundless_fixture_engine(config)
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
    let extra_data =
        encode_proof_carry_data(&ProofCarryData::default()).expect("encode proof carry data");
    json!({
        "quote": "0xfixture-boundless-receipt",
        "extra_data": extra_data
    })
}

fn fixture_external_aggregate_proof() -> Proof {
    Proof {
        proof: Some(format!("0x{}", "11".repeat(89))),
        input: Some(alloy_primitives::B256::ZERO),
        extra_data: Some(json!({
            "shasta": {
                "proof_carry_data": {}
            }
        })),
        ..Proof::default()
    }
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

fn v4_acl_app(keys: Vec<ServerAclKey>) -> Router {
    let mut config = base_config();
    config.server.acl.keys = keys;
    let state = AppState::from_parts(
        Arc::new(config),
        Arc::new(StaticPipelineFactory::default()),
        Arc::new(
            RuntimeManager::new(unique_runtime_root("raiko2-e2e-v4-submit-acl"))
                .expect("runtime manager"),
        ),
    );
    app::build_router_with_legacy_v3_for_tests(state)
}

fn app_with_default_config(runtime_label: &str) -> Router {
    let state = AppState::from_parts(
        Arc::new(Config::default()),
        Arc::new(StaticPipelineFactory::default()),
        Arc::new(RuntimeManager::new(unique_runtime_root(runtime_label)).expect("runtime manager")),
    );
    app::build_router(state)
}

fn v4_submit_acl_app(rate_limit_per_minute: Option<u32>) -> Router {
    v4_acl_app(vec![acl_key_with_rate_limit(
        "submit",
        "submit-secret",
        vec![ServerAclFeature::ProverSubmit],
        rate_limit_per_minute,
    )])
}

fn v4_proposal_request() -> Value {
    json!({
        "proof_type": "risc0",
        "proposals": [
            {
                "proposal_id": 1,
                "last_anchor_block_number": 10,
                "l1_inclusion_block_number": 11,
                "l2_block_number_start": 20,
                "l2_block_number_end": 21
            }
        ]
    })
}

fn v4_sp1_proposal_request(proposal_id: u64) -> Value {
    v4_sp1_batch_request(proposal_id, proposal_id, false)
}

fn v4_sp1_batch_request(proposal_id_start: u64, proposal_id_end: u64, aggregate: bool) -> Value {
    let proposals = (proposal_id_start..=proposal_id_end)
        .map(|proposal_id| {
            json!({
                "proposal_id": proposal_id,
                "last_anchor_block_number": proposal_id.saturating_sub(1),
                "l1_inclusion_block_number": proposal_id,
                "l2_block_number_start": proposal_id,
                "l2_block_number_end": proposal_id
            })
        })
        .collect::<Vec<_>>();
    json!({
        "proof_type": "sp1",
        "aggregate": aggregate,
        "proposals": proposals
    })
}

fn v4_sp1_aggregation_request(proposal_id_start: u64, proposal_id_end: u64) -> Value {
    v4_sp1_batch_request(proposal_id_start, proposal_id_end, true)
}

fn assert_v4_submit_data_has_root_task_only(body: &Value) -> &str {
    let task_id = body["data"]["task_id"].as_str().expect("v4 task_id");
    assert!(task_id.starts_with("task_"), "{body}");
    assert_eq!(task_id.len(), "task_".len() + 64, "{body}");
    assert!(
        task_id["task_".len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit()),
        "{body}"
    );
    assert!(body["data"].get("current_index").is_none(), "{body}");
    assert!(body["data"].get("proposals").is_none(), "{body}");
    assert!(body["data"].get("aggregate").is_none(), "{body}");
    task_id
}

fn v4_sp1_acl_app() -> (Router, Sp1FixtureEngine) {
    let (app, engine, _) = v4_sp1_acl_state_app_with_clear_rate_limit(None);
    (app, engine)
}

fn v4_sp1_acl_app_with_clear_rate_limit(
    clear_rate_limit_per_minute: Option<u32>,
) -> (Router, Sp1FixtureEngine) {
    let (app, engine, _) = v4_sp1_acl_state_app_with_clear_rate_limit(clear_rate_limit_per_minute);
    (app, engine)
}

fn v4_sp1_acl_state_app_with_clear_rate_limit(
    clear_rate_limit_per_minute: Option<u32>,
) -> (Router, Sp1FixtureEngine, AppState) {
    let mut config = base_config();
    set_prover_routes(&mut config, "sp1/local");
    config.prover.sp1.prover = Sp1ProverMode::Local;
    config.server.acl.keys = vec![
        acl_key(
            "submit",
            "submit-secret",
            vec![ServerAclFeature::ProverSubmit],
        ),
        acl_key_with_rate_limit(
            "clear",
            "clear-secret",
            vec![ServerAclFeature::ProverClear],
            clear_rate_limit_per_minute,
        ),
        acl_key("admin", "admin-secret", vec![ServerAclFeature::Admin]),
    ];

    let (state, engine) = state_with_observed_sp1_fixture_engine(config);
    (
        app::build_router_with_legacy_v3_for_tests(state.clone()),
        engine,
        state,
    )
}

async fn complete_v4_sp1_proposal(app: &Router, engine: &Sp1FixtureEngine, proposal_id: u64) {
    let proposal_payload = v4_sp1_proposal_request(proposal_id);

    let (status, first) = post_json_with_api_key(
        app,
        "/v4/proof/proposal",
        "submit-secret",
        proposal_payload.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["status"], "ok");
    assert_eq!(first["proof_type"], "sp1");
    assert_eq!(first["proposal_id_start"], proposal_id);
    assert_eq!(first["proposal_id_end"], proposal_id);
    let task_id = assert_v4_submit_data_has_root_task_only(&first).to_string();
    assert_eq!(first["data"]["status"], "registered");
    assert!(first["data"]["proof"].is_null(), "{first}");

    Box::pin(drive_engine_to_idle(engine)).await;

    let (status, completed) =
        post_json_with_api_key(app, "/v4/proof/proposal", "submit-secret", proposal_payload).await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    assert_eq!(
        assert_v4_submit_data_has_root_task_only(&completed),
        task_id
    );
    assert_eq!(completed["data"]["status"], "completed");
    assert_eq!(completed["data"]["proof"], "0xfixture-sp1-proof");
}

async fn complete_v4_sp1_aggregation(
    app: &Router,
    engine: &Sp1FixtureEngine,
    proposal_id_start: u64,
    proposal_id_end: u64,
) {
    let aggregation_payload = v4_sp1_aggregation_request(proposal_id_start, proposal_id_end);

    let (status, first) = post_json_with_api_key(
        app,
        "/v4/proof/proposal",
        "submit-secret",
        aggregation_payload.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["status"], "ok");
    assert_eq!(first["proof_type"], "sp1");
    assert_eq!(first["proposal_id_start"], proposal_id_start);
    assert_eq!(first["proposal_id_end"], proposal_id_end);
    let task_id = assert_v4_submit_data_has_root_task_only(&first).to_string();
    assert_eq!(first["data"]["status"], "registered");
    assert!(first["data"]["proof"].is_null(), "{first}");

    Box::pin(drive_engine_to_idle(engine)).await;

    let (status, completed) = post_json_with_api_key(
        app,
        "/v4/proof/proposal",
        "submit-secret",
        aggregation_payload,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    assert_eq!(
        assert_v4_submit_data_has_root_task_only(&completed),
        task_id
    );
    assert_eq!(completed["data"]["status"], "completed");
    assert_eq!(completed["data"]["proof"], "0xfixture-sp1-aggregation");
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
    let state = AppState::from_parts(
        Arc::new(config),
        Arc::new(StaticPipelineFactory::default()),
        Arc::new(
            RuntimeManager::new(unique_runtime_root("raiko2-e2e-ready-runtime"))
                .expect("runtime manager"),
        ),
    );
    let app = app::build_router_with_legacy_v3_for_tests(state);

    let (status, body) = get_json(&app, "/ready").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["reth"]["ok"], true);
    assert_eq!(body["runtime"]["ok"], true);
    assert_eq!(body["queue"]["ok"], true);
    assert_eq!(body["prover"]["ok"], true);

    l1_handle.abort();
    l2_handle.abort();
}

#[tokio::test]
async fn e2e_v4_submit_requires_submit_acl_key() {
    let app = v4_submit_acl_app(None);

    let (status, body) =
        post_raw_json_text_with_optional_api_key(&app, "/v4/proof/proposal", None, "{not-json")
            .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.contains("\"error\":\"unauthorized\""), "{body}");

    let (status, body) = post_json(&app, "/v4/proof/proposal", v4_proposal_request()).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["status"], "error");
    assert_eq!(body["error"], "unauthorized");

    let (status, body) =
        post_json(&app, "/v4/proof/proposal", v4_sp1_aggregation_request(1, 1)).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["status"], "error");
    assert_eq!(body["error"], "unauthorized");
}

#[tokio::test]
async fn e2e_v3_routes_are_disabled_by_default() {
    let app = app_with_default_config("raiko2-e2e-v3-disabled");

    let (status, body) =
        post_raw_json_text_with_optional_api_key(&app, "/v3/proof/batch/shasta", None, "{}").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let (status, body) =
        post_raw_json_text_with_optional_api_key(&app, "/proof/batch/shasta", None, "{}").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let (status, body) = post_json(&app, "/v4/proof/proposal", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "error", "{body}");
    assert_eq!(body["error"], "missing_proof_type", "{body}");
}

#[tokio::test]
async fn e2e_v4_submit_is_open_when_acl_feature_is_disabled() {
    let app = v4_acl_app(vec![acl_key(
        "clear",
        "clear-secret",
        vec![ServerAclFeature::ProverClear],
    )]);

    let (status, body) = post_json(&app, "/v4/proof/proposal", v4_proposal_request()).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "error");
    assert_eq!(body["error"], "unsupported_proof_type");

    assert!(report_task_ids(&app).await.is_empty());
}

#[tokio::test]
async fn e2e_v4_submit_rejects_unavailable_backend_without_registering_task() {
    let app = v4_submit_acl_app(None);

    let (status, body) = post_json_with_api_key(
        &app,
        "/v4/proof/proposal",
        "submit-secret",
        v4_proposal_request(),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "error");
    assert_eq!(body["error"], "unsupported_proof_type");

    assert!(report_task_ids(&app).await.is_empty());
}

#[tokio::test]
async fn e2e_v4_proposal_rejects_multi_proposal_without_aggregation() {
    let (app, _engine) = v4_sp1_acl_app();

    let (status, body) = post_json_with_api_key(
        &app,
        "/v4/proof/proposal",
        "submit-secret",
        v4_sp1_batch_request(1, 2, false),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "error", "{body}");
    assert_eq!(body["error"], "invalid_request", "{body}");
    assert_eq!(
        body["message"], "aggregate=false requires exactly one proposal",
        "{body}"
    );

    assert!(report_task_ids(&app).await.is_empty());
}

#[tokio::test]
async fn e2e_v4_proposal_terminal_task_accepts_corrected_resubmission() {
    // A corrected resubmission with a different fingerprint (e.g. an L1 reorg changed
    // l1_inclusion_block_number) must get a new root task instead of wedging the proposal range with
    // a permanent request_conflict. The fixture engine stays quiescent until driven, so the task
    // stays `registered` until `clear` cancels it.
    let (app, _engine) = v4_sp1_acl_app();

    let original = json!({
        "proof_type": "sp1",
        "proposals": [
            {
                "proposal_id": 1,
                "last_anchor_block_number": 0,
                "l1_inclusion_block_number": 1,
                "l2_block_number_start": 1,
                "l2_block_number_end": 1
            }
        ]
    });

    // 1) Register the proposal task.
    let (status, first) = post_json_with_api_key(
        &app,
        "/v4/proof/proposal",
        "submit-secret",
        original.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let original_task_id = assert_v4_submit_data_has_root_task_only(&first).to_string();
    assert_eq!(first["data"]["status"], "registered");

    // 2) Cancel it via clear -> task becomes terminal (Cancelled) but is NOT removed.
    let (status, cleared) = post_json_with_api_key(
        &app,
        "/v4/prover/clear",
        "clear-secret",
        json!({ "proof_type": "sp1" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    // Engine child tasks are not runtime_tasks rows, so the sp1-scoped clear cancels one root task.
    assert_eq!(cleared["data"]["cancelled"], 1, "{cleared}");

    // 3) Resubmit a corrected payload for the same slot: different l1_inclusion_block_number ->
    //    different fingerprint and therefore a different root task id.
    let corrected = json!({
        "proof_type": "sp1",
        "proposals": [
            {
                "proposal_id": 1,
                "last_anchor_block_number": 0,
                "l1_inclusion_block_number": 2,
                "l2_block_number_start": 1,
                "l2_block_number_end": 1
            }
        ]
    });
    let (status, replaced) =
        post_json_with_api_key(&app, "/v4/proof/proposal", "submit-secret", corrected).await;
    assert_eq!(status, StatusCode::OK, "{replaced}");
    let corrected_task_id = assert_v4_submit_data_has_root_task_only(&replaced);
    assert_ne!(corrected_task_id, original_task_id);
    assert_eq!(replaced["data"]["status"], "registered");
}

#[tokio::test]
async fn e2e_v4_proposal_completed_task_accepts_corrected_resubmission_with_new_task_id() {
    // Corrected proof input must not overwrite a completed proof, but it also must not be blocked by
    // the old completed row. The fingerprint-derived root task id gives the corrected payload a new
    // task while keeping the original completed task addressable.
    let (app, engine) = v4_sp1_acl_app();

    // Drive proposal (1,1) to completion (l1_inclusion_block_number = 1 in v4_sp1_proposal_request).
    complete_v4_sp1_proposal(&app, &engine, 1).await;
    let original_task_id = single_report_task_id(&app).await;

    // Resubmit the same slot with a different l1_inclusion_block_number -> different fingerprint.
    let corrected = json!({
        "proof_type": "sp1",
        "proposals": [
            {
                "proposal_id": 1,
                "last_anchor_block_number": 0,
                "l1_inclusion_block_number": 999,
                "l2_block_number_start": 1,
                "l2_block_number_end": 1
            }
        ]
    });
    let (status, body) =
        post_json_with_api_key(&app, "/v4/proof/proposal", "submit-secret", corrected).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "ok", "{body}");
    let corrected_task_id = assert_v4_submit_data_has_root_task_only(&body);
    assert_ne!(corrected_task_id, original_task_id);
    assert_eq!(body["data"]["status"], "registered");
}

#[tokio::test]
async fn e2e_v4_aggregation_registers_missing_proposals_from_same_request() {
    let (app, engine) = v4_sp1_acl_app();

    let (status, body) = post_json_with_api_key(
        &app,
        "/v4/proof/proposal",
        "submit-secret",
        v4_sp1_aggregation_request(7, 7),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "ok");
    let task_id = assert_v4_submit_data_has_root_task_only(&body).to_string();
    assert_eq!(body["data"]["status"], "registered");

    Box::pin(drive_engine_to_idle(&engine)).await;

    let (status, completed) = post_json_with_api_key(
        &app,
        "/v4/proof/proposal",
        "submit-secret",
        v4_sp1_aggregation_request(7, 7),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    assert_eq!(
        assert_v4_submit_data_has_root_task_only(&completed),
        task_id
    );
    assert_eq!(completed["data"]["status"], "completed");
    assert_eq!(completed["data"]["proof"], "0xfixture-sp1-aggregation");
}

#[tokio::test]
async fn e2e_v4_aggregation_endpoint_is_not_a_submit_route() {
    let (app, _engine) = v4_sp1_acl_app();

    let (status, _body) = post_raw_json_text_with_optional_api_key(
        &app,
        "/v4/proof/aggregation",
        Some("submit-secret"),
        &v4_sp1_aggregation_request(7, 7).to_string(),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn e2e_v4_aggregation_completed_row_repeated_post_fast_returns() {
    // Proposal-side aggregation should poll by repeating the same batch request, matching the
    // proposal path's idempotency and requeue semantics.
    let (app, engine) = v4_sp1_acl_app();
    complete_v4_sp1_aggregation(&app, &engine, 5, 5).await;

    let (status, again) = post_json_with_api_key(
        &app,
        "/v4/proof/proposal",
        "submit-secret",
        v4_sp1_aggregation_request(5, 5),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_v4_submit_data_has_root_task_only(&again);
    assert_eq!(again["data"]["status"], "completed", "{again}");
    assert_eq!(
        again["data"]["proof"], "0xfixture-sp1-aggregation",
        "{again}"
    );
}

#[tokio::test]
async fn e2e_v4_submit_rate_limits_acl_key() {
    let app = v4_submit_acl_app(Some(1));

    let (first_status, _) = post_json_with_api_key(
        &app,
        "/v4/proof/proposal",
        "submit-secret",
        v4_proposal_request(),
    )
    .await;
    assert_ne!(first_status, StatusCode::TOO_MANY_REQUESTS);

    let (status, body) = post_json_with_api_key(
        &app,
        "/v4/proof/proposal",
        "submit-secret",
        v4_proposal_request(),
    )
    .await;

    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["status"], "error");
    assert_eq!(body["error"], "rate_limited");

    let (status, body) = post_json_with_api_key(
        &app,
        "/v4/proof/proposal",
        "submit-secret",
        v4_sp1_aggregation_request(1, 1),
    )
    .await;

    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["status"], "error");
    assert_eq!(body["error"], "rate_limited");
}

#[tokio::test]
async fn e2e_v4_native_proposal_and_aggregate_complete_from_fixture() {
    let mut config = base_config();
    set_prover_routes(&mut config, "native/local");
    let (app, engine) = app_with_observed_native_fixture_engine(config);
    let payload = json!({
        "proof_type": "native",
        "aggregate": true,
        "proposals": [{
            "proposal_id": 23077,
            "l1_inclusion_block_number": 25585003,
            "l2_block_number_start": 9051439,
            "l2_block_number_end": 9051630,
            "last_anchor_block_number": 25584933
        }]
    });

    let (status, first) = post_json(&app, "/v4/proof/proposal", payload.clone()).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["proof_type"], "native");
    assert_eq!(first["data"]["status"], "registered");
    let task_id = assert_v4_submit_data_has_root_task_only(&first).to_string();

    drive_engine_to_idle(&engine).await;

    let (status, completed) = post_json(&app, "/v4/proof/proposal", payload).await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    assert_eq!(completed["data"]["task_id"], task_id);
    assert_eq!(completed["data"]["status"], "completed", "{completed}");
    assert!(completed["data"]["proof"].as_str().is_some(), "{completed}");

    let (status, task) = get_json(&app, &format!("/v4/tasks/{task_id}")).await;
    assert_eq!(status, StatusCode::OK, "{task}");
    assert_eq!(task["data"]["route"], "native/local");
    assert_eq!(task["data"]["status"], "completed");
}

#[tokio::test]
async fn e2e_v4_proposal_poll_and_task_lookup_complete_from_fixture() {
    let (app, engine) = v4_sp1_acl_app();
    complete_v4_sp1_proposal(&app, &engine, 3).await;
    let task_id = single_report_task_id(&app).await;
    let task_path = format!("/v4/tasks/{task_id}");

    let (status, task) = get_json(&app, &task_path).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{task}");
    assert_eq!(task["error"], "unauthorized");

    let (status, task) = get_json_with_api_key(&app, &task_path, "clear-secret").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{task}");
    assert_eq!(task["error"], "forbidden");

    let (status, task) = get_json_with_api_key(&app, &task_path, "submit-secret").await;
    assert_eq!(status, StatusCode::OK, "{task}");
    assert_eq!(task["status"], "ok");
    assert_eq!(task["data"]["task_id"], task_id);
    assert_eq!(task["data"]["status"], "completed");
    assert_eq!(task["data"]["proof"], "0xfixture-sp1-proof");

    let (status, task) = get_json_with_api_key(&app, &task_path, "admin-secret").await;
    assert_eq!(status, StatusCode::OK, "{task}");
    assert_eq!(task["data"]["proof"], "0xfixture-sp1-proof");
}

#[tokio::test]
async fn e2e_v4_aggregation_status_and_clear_complete_from_fixture() {
    let (app, engine) = v4_sp1_acl_app();
    complete_v4_sp1_aggregation(&app, &engine, 3, 3).await;

    let (status, status_body) = get_json(&app, "/v4/prover/status?proof_type=sp1").await;
    assert_eq!(status, StatusCode::OK, "{status_body}");
    assert_eq!(status_body["status"], "ok");
    assert_eq!(status_body["proof_type"], "sp1");

    let (status, clear_body) = post_json(
        &app,
        "/v4/prover/clear",
        json!({
            "proof_type": "sp1"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{clear_body}");

    let (status, clear_body) = post_json_with_api_key(
        &app,
        "/v4/prover/clear",
        "submit-secret",
        json!({
            "proof_type": "sp1"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{clear_body}");

    let (status, clear_body) = post_json_with_api_key(
        &app,
        "/v4/prover/clear",
        "clear-secret",
        json!({
            "proof_type": "sp1"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{clear_body}");
    assert_eq!(clear_body["status"], "ok");
    assert_eq!(clear_body["proof_type"], "sp1");
    assert!(clear_body["data"].get("status").is_none());
    assert_eq!(clear_body["data"]["cancelled"], 0);
    assert_eq!(clear_body["data"]["failed"], 0);
}

#[tokio::test]
async fn e2e_v4_clear_rate_limits_acl_key() {
    let (app, _engine) = v4_sp1_acl_app_with_clear_rate_limit(Some(1));

    let payload = json!({ "proof_type": "sp1" });
    let (status, body) =
        post_json_with_api_key(&app, "/v4/prover/clear", "clear-secret", payload.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) =
        post_json_with_api_key(&app, "/v4/prover/clear", "clear-secret", payload).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["error"], "rate_limited");
}

#[tokio::test]
async fn e2e_v4_invalidate_artifacts_removes_completed_cache() {
    let (app, engine) = v4_sp1_acl_app();
    let payload = v4_sp1_proposal_request(11);

    let (status, first) =
        post_json_with_api_key(&app, "/v4/proof/proposal", "submit-secret", payload.clone()).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["data"]["status"], "registered");

    drive_engine_to_idle(&engine).await;

    let (status, completed) =
        post_json_with_api_key(&app, "/v4/proof/proposal", "submit-secret", payload.clone()).await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    assert_eq!(completed["data"]["status"], "completed");
    assert_eq!(completed["data"]["proof"], "0xfixture-sp1-proof");

    let request = json!({
        "proof_type": "sp1",
        "dry_run": true
    });
    let (status, dry_run) = post_json_with_api_key(
        &app,
        "/v4/prover/invalidate-artifacts",
        "clear-secret",
        request,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{dry_run}");
    assert_eq!(dry_run["status"], "ok");
    assert_eq!(dry_run["proof_type"], "sp1");
    assert_eq!(dry_run["data"]["dry_run"], true);
    assert_eq!(dry_run["data"]["artifacts"]["matched"], 1);
    assert_eq!(dry_run["data"]["artifacts"]["removed"], 0);
    assert_eq!(dry_run["data"]["tasks"]["matched"], 1);
    assert_eq!(dry_run["data"]["tasks"]["removed"], 0);

    let request = json!({
        "proof_type": "sp1"
    });
    let (status, invalidated) = post_json_with_api_key(
        &app,
        "/v4/prover/invalidate-artifacts",
        "clear-secret",
        request,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{invalidated}");
    assert_eq!(invalidated["data"]["dry_run"], false);
    assert_eq!(invalidated["data"]["artifacts"]["matched"], 1);
    assert_eq!(invalidated["data"]["artifacts"]["removed"], 1);
    assert_eq!(invalidated["data"]["artifacts"]["manifests_removed"], 1);
    assert_eq!(invalidated["data"]["artifacts"]["manifests_missing"], 0);
    assert_eq!(invalidated["data"]["tasks"]["matched"], 1);
    assert_eq!(invalidated["data"]["tasks"]["removed"], 1);

    let (status, resubmitted) =
        post_json_with_api_key(&app, "/v4/proof/proposal", "submit-secret", payload).await;
    assert_eq!(status, StatusCode::OK, "{resubmitted}");
    assert_eq!(resubmitted["data"]["status"], "registered");
    assert!(resubmitted["data"]["proof"].is_null(), "{resubmitted}");
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
    let state = AppState::from_parts(
        Arc::new(config),
        Arc::new(StaticPipelineFactory::default()),
        Arc::new(
            RuntimeManager::new(unique_runtime_root("raiko2-e2e-ready-l1-mismatch-runtime"))
                .expect("runtime manager"),
        ),
    );
    let app = app::build_router_with_legacy_v3_for_tests(state);

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
async fn e2e_ready_fails_after_runtime_begins_draining() {
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
    let store: Arc<dyn RuntimeStore> = Arc::new(
        MemoryProofArtifactStore::new("test".into(), "ready-draining".into())
            .expect("memory artifact store"),
    );
    let runtime = Arc::new(RuntimeManager::with_store(store));
    let permit = runtime
        .acquire_submission_checkpoint_permit()
        .expect("pre-drain checkpoint permit");
    runtime.start_draining();
    let state = AppState::from_parts(
        Arc::new(config),
        Arc::new(StaticPipelineFactory::default()),
        Arc::clone(&runtime),
    );
    let app = app::build_router_with_legacy_v3_for_tests(state);

    let (status, body) = get_json(&app, "/ready").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["status"], "error");
    assert_eq!(body["runtime"]["ok"], false);
    assert!(
        body["runtime"]["error"]
            .as_str()
            .expect("runtime error")
            .contains("runtime is draining")
    );
    assert_eq!(body["reth"]["ok"], true);
    assert_eq!(body["queue"]["ok"], true);
    assert_eq!(body["prover"]["ok"], true);

    drop(permit);
    runtime.begin_draining().await;
    let (status, body) = get_json(&app, "/ready").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["runtime"]["ok"], false);

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
    set_prover_routes(&mut config, "risc0/network");
    config.prover.risc0.boundless.rpc_url = "https://base-rpc.publicnode.com".to_string();
    config.prover.risc0.boundless.signer_key = "not-a-private-key".to_string();
    let state = AppState::from_parts(
        Arc::new(config),
        Arc::new(StaticPipelineFactory::default()),
        Arc::new(
            RuntimeManager::new(unique_runtime_root("raiko2-e2e-ready-boundless-runtime"))
                .expect("runtime manager"),
        ),
    );
    let app = app::build_router_with_legacy_v3_for_tests(state);

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
    let state = AppState::from_parts(
        Arc::new(config),
        Arc::new(StaticPipelineFactory::default()),
        Arc::new(
            RuntimeManager::new(unique_runtime_root("raiko2-e2e-ready-l2-witness-runtime"))
                .expect("runtime manager"),
        ),
    );
    let app = app::build_router_with_legacy_v3_for_tests(state);

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
    set_prover_routes(&mut config, "sp1/local");
    config.prover.sp1.prover = Sp1ProverMode::Local;
    config.prover.sp1.verify = false;
    let state = AppState::from_parts(
        Arc::new(config),
        Arc::new(StaticPipelineFactory::default()),
        Arc::new(
            RuntimeManager::new(unique_runtime_root("raiko2-e2e-ready-sp1-verify-runtime"))
                .expect("runtime manager"),
        ),
    );
    let app = app::build_router_with_legacy_v3_for_tests(state);

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
    set_prover_routes(&mut config, "risc0/network,sp1/local");
    config.prover.risc0.boundless.rpc_url = "https://base-rpc.publicnode.com".to_string();
    config.prover.risc0.boundless.signer_key =
        deterministic_test_private_key("raiko2:e2e-ready-boundless");
    config.prover.sp1.prover = Sp1ProverMode::Local;
    config.prover.sp1.verify = false;
    let state = AppState::from_parts(
        Arc::new(config),
        Arc::new(StaticPipelineFactory::default()),
        Arc::new(
            RuntimeManager::new(unique_runtime_root("raiko2-e2e-ready-multi-zk-runtime"))
                .expect("runtime manager"),
        ),
    );
    let app = app::build_router_with_legacy_v3_for_tests(state);

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
async fn e2e_proposal_proof_risc0_completes_from_fixture() {
    let config = base_config();
    let (app, engine) = app_with_observed_risc0_fixture_engine(config);

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
    assert_eq!(res["data"]["status"], "registered", "{res}");
    assert!(res["data"].get("task_id").is_none(), "{res}");
    let id = single_report_task_id(&app).await;

    drive_engine_to_idle(&engine).await;

    let (status, res) = get_json(&app, &format!("/v3/tasks/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["data"]["status"], "completed", "{res}");
    assert!(
        res["data"].get("error").is_none(),
        "unexpected error: {res}"
    );
}

#[tokio::test]
async fn e2e_shasta_disabled_route_wins_over_registered_fixture_pipeline() {
    let mut config = base_config();
    set_prover_routes(&mut config, "sp1/local");
    config.prover.sp1.prover = Sp1ProverMode::Local;
    let engine = risc0_fixture_engine(json!({}));
    let app = app_with_risc0_fixture_engine(config, engine);

    let (status, body) = post_json(&app, "/v4/proof/proposal", v4_proposal_request()).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "error", "{body}");
    assert_eq!(body["error"], "unsupported_proof_type", "{body}");
    assert_eq!(body["message"], "proof_type=risc0 is not supported");
    assert!(report_task_ids(&app).await.is_empty());
}

#[tokio::test]
async fn e2e_shasta_proposal_and_external_aggregate_use_same_enabled_route() {
    let mut config = base_config();
    set_prover_routes(&mut config, "sgx/remote");
    let engine = native_fixture_engine_for_pipeline(PipelineKey::ShastaSgx, None);
    let state = app_with_engine(config, "taiko_dev/ethereum", PipelineKey::ShastaSgx, engine);
    let app = app::build_router_with_legacy_v3_for_tests(state);

    let (status, proposal) = post_json(
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
            "proof_type": "sgx",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{proposal}");
    assert_eq!(proposal["data"]["status"], "registered", "{proposal}");
    let proposal_task_id = single_report_task_id(&app).await;

    let (status, aggregate) = post_json(
        &app,
        "/v3/proof/aggregate",
        json!({
            "proofs": [
                fixture_external_aggregate_proof(),
                fixture_external_aggregate_proof()
            ],
            "proof_type": "sgx",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{aggregate}");
    assert_eq!(aggregate["data"]["status"], "registered", "{aggregate}");

    let task_ids = report_task_ids(&app).await;
    let aggregate_task_id = task_ids
        .into_iter()
        .find(|task_id| task_id != &proposal_task_id)
        .expect("aggregate task id");
    for task_id in [proposal_task_id, aggregate_task_id] {
        let (status, task) = get_json(&app, &format!("/v3/tasks/{task_id}")).await;
        assert_eq!(status, StatusCode::OK, "{task}");
        assert_eq!(task["data"]["route"], "sgx/remote", "{task}");
        assert_eq!(task["proof_type"], "sgx", "{task}");
    }
}

#[tokio::test]
async fn e2e_shasta_rejects_boundless_public_proof_type() {
    let engine = risc0_fixture_engine(json!({}));
    let app = app_with_risc0_fixture_engine(base_config(), engine);

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
            "proof_type": "boundless",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["status"], "error");
    assert_eq!(res["error"], "invalid_request_config");
    assert_eq!(res["message"], "proof_type=boundless is not supported");
    assert!(report_task_ids(&app).await.is_empty());
}

#[tokio::test]
async fn e2e_shasta_request_is_compatible_with_taiko_client_shape() {
    let config = base_config();
    let (app, engine) = app_with_observed_risc0_fixture_engine(config);

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
            "prover": "1111111111111111111111111111111111111111"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["data"]["status"], "registered", "{res}");
    assert!(res["data"].get("task_id").is_none(), "{res}");
    let id = single_report_task_id(&app).await;

    drive_engine_to_idle(&engine).await;

    let (status, res) = get_json(&app, &format!("/v3/tasks/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["data"]["status"], "completed", "{res}");
    assert_eq!(res["data"]["network"], "taiko_dev");
    assert_eq!(res["data"]["l1_network"], "ethereum");
}

#[tokio::test]
async fn e2e_shasta_request_rejects_partial_network_pair() {
    let config = base_config();
    let engine = risc0_fixture_engine(json!({}));
    let app = app_with_risc0_fixture_engine(config, engine);

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
            "network": "taiko_dev"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["status"], "error");
    assert_eq!(res["error"], "invalid_request_config");
    assert_eq!(
        res["message"],
        "network and l1_network must be provided together"
    );
}

#[tokio::test]
async fn e2e_shasta_request_rejects_unknown_fields() {
    let config = base_config();
    let engine = risc0_fixture_engine(json!({}));
    let app = app_with_risc0_fixture_engine(config, engine);

    let (status, res) = post_json(
        &app,
        "/v3/proof/batch/shasta",
        json!({
            "proposals": [{
                "proposal_id": 3,
                "l1_inclusion_block_number": 1,
                "l2_block_numbers": [3],
                "last_anchor_block_number": 0,
                "unexpected_proposal_field": true
            }],
            "aggregate": false,
            "proof_type": "risc0",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["status"], "error");
    assert_eq!(res["error"], "invalid_request_config");
    assert!(
        res["message"]
            .as_str()
            .is_some_and(|message| message.contains("unexpected_proposal_field")),
        "{res}"
    );
    assert!(report_task_ids(&app).await.is_empty());
}

#[tokio::test]
async fn e2e_shasta_reports_unregistered_sgxgeth_pipeline() {
    let mut config = base_config();
    set_prover_routes(&mut config, "sgxgeth/remote");
    let engine = risc0_fixture_engine(json!({}));
    let app = app_with_risc0_fixture_engine(config, engine);

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
            "proof_type": "sgxgeth",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["status"], "error");
    assert_eq!(res["error"], "not_found");
    assert!(report_task_ids(&app).await.is_empty());
}

#[tokio::test]
async fn e2e_duplicate_shasta_post_reuses_same_root_task() {
    let config = base_config();
    let engine = risc0_fixture_engine(json!({}));
    let app = app_with_risc0_fixture_engine(config, engine);
    let payload = json!({
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
    });

    let (status, first) = post_json(&app, "/v3/proof/batch/shasta", payload.clone()).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["data"]["status"], "registered");
    assert!(first["data"].get("task_id").is_none(), "{first}");
    let first_id = single_report_task_id(&app).await;

    let (status, second) = post_json(&app, "/v3/proof/batch/shasta", payload).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["status"], "ok");
    assert_eq!(second["data"]["status"], "registered");
    assert!(second["data"].get("task_id").is_none(), "{second}");
    assert_eq!(single_report_task_id(&app).await, first_id);
}

#[tokio::test]
async fn e2e_batch_public_task_id_is_scoped_to_runtime_namespace() {
    let payload = json!({
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
    });

    let first_app = app_with_risc0_fixture_engine(base_config(), risc0_fixture_engine(json!({})));
    let (status, first) = post_json(&first_app, "/v3/proof/batch/shasta", payload.clone()).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let first_id = single_report_task_id(&first_app).await;

    let second_app = app_with_risc0_fixture_engine(base_config(), risc0_fixture_engine(json!({})));
    let (status, second) = post_json(&second_app, "/v3/proof/batch/shasta", payload).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    let second_id = single_report_task_id(&second_app).await;

    assert_ne!(second_id, first_id);
}

#[tokio::test]
async fn e2e_duplicate_shasta_post_returns_work_in_progress_when_runtime_has_progress() {
    let config = base_config();
    let engine = risc0_fixture_engine(json!({}));
    let state = app_with_engine(
        config,
        "taiko_dev/ethereum",
        PipelineKey::ShastaRisc0,
        engine,
    );
    let app = app::build_router_with_legacy_v3_for_tests(state.clone());
    let payload = json!({
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
    });

    let (status, first) = post_json(&app, "/v3/proof/batch/shasta", payload.clone()).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["data"]["status"], "registered");
    assert!(first["data"].get("task_id").is_none(), "{first}");
    let task_id = single_report_task_id(&app).await;

    let mut record = state
        .runtime
        .get_task(&task_id)
        .await
        .expect("read task")
        .expect("task exists");
    let mut metadata: TaskMetadata =
        serde_json::from_value(record.metadata.clone()).expect("deserialize metadata");
    metadata.runtime.active_stage = Some("prove".to_string());
    metadata.runtime.last_event = Some("submission_registered".to_string());
    record.metadata = serde_json::to_value(metadata).expect("serialize metadata");
    state
        .runtime
        .upsert_task(&record)
        .await
        .expect("upsert task");

    let (status, second) = post_json(&app, "/v3/proof/batch/shasta", payload).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["status"], "ok");
    assert_eq!(second["data"]["status"], "work_in_progress");
    assert!(second["data"].get("task_id").is_none(), "{second}");
    assert_eq!(single_report_task_id(&app).await, task_id);
}

#[tokio::test]
async fn e2e_duplicate_shasta_post_recovers_stale_runtime_progress_after_restart() {
    let config = base_config();
    let (state, _engine) = state_with_observed_risc0_fixture_engine(config);
    let app = app::build_router_with_legacy_v3_for_tests(state.clone());
    let payload = json!({
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
    });

    let (status, first) = post_json(&app, "/v3/proof/batch/shasta", payload.clone()).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["data"]["status"], "registered");
    let task_id = single_report_task_id(&app).await;

    let mut record = state
        .runtime
        .get_task(&task_id)
        .await
        .expect("read task")
        .expect("task exists");
    let mut metadata: TaskMetadata =
        serde_json::from_value(record.metadata.clone()).expect("deserialize metadata");
    metadata.runtime.active_stage = Some("prove".to_string());
    metadata.runtime.last_event = Some("started:engine-3".to_string());
    record.metadata = serde_json::to_value(metadata).expect("serialize metadata");
    state
        .runtime
        .upsert_task(&record)
        .await
        .expect("upsert task");

    let restarted_engine = risc0_fixture_engine_with_observer(
        json!({}),
        Some(engine_observer(
            Arc::clone(&state.runtime),
            PipelineKey::ShastaRisc0.route(),
        )),
    );
    let mut factory = StaticPipelineFactory::default();
    factory.insert(
        "taiko_dev/ethereum".to_string(),
        PipelineKey::ShastaRisc0,
        Arc::new(restarted_engine.clone()),
    );
    let restarted_state = AppState::from_parts(
        Arc::clone(&state.config),
        Arc::new(factory),
        Arc::clone(&state.runtime),
    );
    let restarted_app = app::build_router_with_legacy_v3_for_tests(restarted_state);

    let (status, second) = post_json(&restarted_app, "/v3/proof/batch/shasta", payload).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["data"]["status"], "work_in_progress");
    assert_eq!(single_report_task_id(&restarted_app).await, task_id);

    drive_engine_to_idle(&restarted_engine).await;

    let (status, task) = get_json(&restarted_app, &format!("/v3/tasks/{task_id}")).await;
    assert_eq!(status, StatusCode::OK, "{task}");
    assert_eq!(task["data"]["status"], "completed");
}

#[tokio::test]
async fn e2e_duplicate_shasta_post_returns_completed_legacy_proof() {
    let config = base_config();
    let (app, engine) = app_with_observed_risc0_fixture_engine(config);
    let payload = json!({
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
    });

    let (status, first) = post_json(&app, "/v3/proof/batch/shasta", payload.clone()).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["data"]["status"], "registered");
    assert!(first["data"].get("task_id").is_none(), "{first}");

    drive_engine_to_idle(&engine).await;

    let (status, second) = post_json(&app, "/v3/proof/batch/shasta", payload).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["status"], "ok");
    assert!(second["data"]["proof"]["proof"].is_string(), "{second}");
    assert!(second["data"]["proof"].get("status").is_none(), "{second}");
}

#[tokio::test]
async fn e2e_duplicate_shasta_post_recovers_registered_task_without_engine_children() {
    let config = base_config();
    let (state, engine) = state_with_observed_risc0_fixture_engine(config);
    let app = app::build_router_with_legacy_v3_for_tests(state.clone());
    let request_proposals = vec![json!({
        "proposal_id": 3,
        "l1_inclusion_block_number": 1,
        "l2_block_numbers": [3],
        "last_anchor_block_number": 0
    })];
    let payload = json!({
        "proposals": request_proposals,
        "aggregate": false,
        "proof_type": "risc0",
        "network": "taiko_dev",
        "l1_network": "ethereum"
    });
    let proposal_request = ProposalTaskRequest {
        proposal_id: 3,
        l2_block_range: Some(raiko2_primitives::L2BlockRange { start: 3, end: 3 }),
        l1_inclusion_block_number: 1,
        last_anchor_block_number: 0,
        checkpoint: None,
        blob_proof_type: None,
        prover: None,
        graffiti: None,
        prover_config: ProverTaskConfig::default(),
    };
    let proposal_ref = proposal_task_ref(PipelineKey::ShastaRisc0, &proposal_request);
    let metadata = TaskMetadata {
        network_pair: "taiko_dev/ethereum".to_string(),
        network: "taiko_dev".to_string(),
        l1_network: "ethereum".to_string(),
        proof_type: raiko2_primitives::ProofType::Risc0,
        requested_proof_type: None,
        prover_type: None,
        execution_mode: None,
        aggregate_requested: false,
        proposals: vec![ProposalTask {
            proposal_id: 3,
            checkpoint: None,
            l1_inclusion_block_number: 1,
            l2_block_numbers: vec![3],
            last_anchor_block_number: 0,
            task_id: proposal_ref,
            request: proposal_request,
        }],
        aggregate_task_id: None,
        aggregate_request: None,
        aggregate_input_artifacts: Vec::new(),
        runtime: RuntimeMetadata::default(),
    };
    let canonical_proposals = vec![json!({
        "proposal_id": 3,
        "checkpoint": Value::Null,
        "l1_inclusion_block_number": 1,
        "l2_block_numbers": [3],
        "l2_block_range": {
            "start": 3,
            "end": 3
        },
        "last_anchor_block_number": 0
    })];
    let request_fingerprint = duplicate_request_fingerprint(
        "taiko_dev/ethereum",
        "risc0/local",
        false,
        &canonical_proposals,
    );
    let artifact_refs = publication_proof_artifact_refs(&metadata, PipelineKey::ShastaRisc0);
    state
        .runtime
        .register_task(TaskRegistration {
            task_id: "task_orphan_registered".to_string(),
            pipeline_key: PipelineKey::ShastaRisc0,
            route: "risc0/local".parse::<PipelineRoute>().expect("parse route"),
            task_kind: "hoodi_batch".to_string(),
            network_pair: "taiko_dev/ethereum".into(),
            artifact_refs,
            metadata: serde_json::to_value(metadata).expect("serialize orphan metadata"),
            request_fingerprint,
        })
        .await
        .expect("register orphan task");

    let (status, second) = post_json(&app, "/v3/proof/batch/shasta", payload).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["data"]["status"], "registered");
    assert!(second["data"].get("task_id").is_none(), "{second}");

    drive_engine_to_idle(&engine).await;

    let (status, task) = get_json(&app, "/v3/tasks/task_orphan_registered").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(task["data"]["status"], "completed");
}

#[tokio::test]
async fn e2e_duplicate_shasta_post_keeps_completed_without_root_proof_as_success() {
    let (app, engine) = sp1_fixture_app();
    let payload = json!({
        "proposals": [
            {
                "proposal_id": 3,
                "l1_inclusion_block_number": 1,
                "l2_block_numbers": [3],
                "last_anchor_block_number": 0
            },
            {
                "proposal_id": 4,
                "l1_inclusion_block_number": 1,
                "l2_block_numbers": [4],
                "last_anchor_block_number": 0
            }
        ],
        "aggregate": false,
        "proof_type": "sp1",
        "network": "taiko_dev",
        "l1_network": "ethereum"
    });

    let (status, first) = post_json(&app, "/v3/proof/batch/shasta", payload.clone()).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["data"]["status"], "registered");
    assert!(first["data"].get("task_id").is_none(), "{first}");

    drive_engine_to_idle(&engine).await;

    let (status, second) = post_json(&app, "/v3/proof/batch/shasta", payload).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["status"], "ok");
    assert_eq!(second["proof_type"], "sp1");
    assert!(second["data"]["proof"].is_object(), "{second}");
    assert!(second["data"]["proof"]["proof"].is_null(), "{second}");
}

#[tokio::test]
async fn e2e_duplicate_shasta_post_recovers_failed_task_before_remote_submission() {
    let config = base_config();
    let engine = risc0_fixture_engine(json!({}));
    let state = app_with_engine(
        config,
        "taiko_dev/ethereum",
        PipelineKey::ShastaRisc0,
        engine,
    );
    let app = app::build_router_with_legacy_v3_for_tests(state.clone());
    let payload = json!({
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
    });

    let (status, first) = post_json(&app, "/v3/proof/batch/shasta", payload.clone()).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["data"]["status"], "registered");
    assert!(first["data"].get("task_id").is_none(), "{first}");
    let task_id = single_report_task_id(&app).await;

    let mut registered = state
        .runtime
        .get_task(&task_id)
        .await
        .expect("get registered task")
        .expect("registered task");
    registered.runner_status = RunnerStatus::Failed;
    registered.error = Some("fixture failed".to_string());
    state
        .runtime
        .upsert_task(&registered)
        .await
        .expect("store failed task fixture");

    let (status, second) = post_json(&app, "/v3/proof/batch/shasta", payload).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["status"], "ok");
    assert_eq!(second["proof_type"], "risc0");
    assert_eq!(second["data"]["status"], "registered");
    assert_eq!(single_report_task_id(&app).await, task_id);
}

#[tokio::test]
async fn e2e_duplicate_aggregate_shasta_post_recovers_failed_root_before_remote_submission() {
    let mut config = base_config();
    set_prover_routes(&mut config, "sp1/local");
    config.prover.sp1.prover = Sp1ProverMode::Local;

    let engine = sp1_fixture_engine(json!({}));
    let state = app_with_engine(config, "taiko_dev/ethereum", PipelineKey::ShastaSp1, engine);
    let app = app::build_router_with_legacy_v3_for_tests(state.clone());
    let payload = json!({
        "proposals": [
            {
                "proposal_id": 3,
                "l1_inclusion_block_number": 1,
                "l2_block_numbers": [3],
                "last_anchor_block_number": 0
            },
            {
                "proposal_id": 4,
                "l1_inclusion_block_number": 1,
                "l2_block_numbers": [4],
                "last_anchor_block_number": 0
            }
        ],
        "aggregate": true,
        "proof_type": "sp1",
        "network": "taiko_dev",
        "l1_network": "ethereum"
    });

    let (status, first) = post_json(&app, "/v3/proof/batch/shasta", payload.clone()).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["data"]["status"], "registered");
    assert!(first["data"].get("task_id").is_none(), "{first}");
    let task_id = single_report_task_id(&app).await;

    let mut registered = state
        .runtime
        .get_task(&task_id)
        .await
        .expect("get registered task")
        .expect("registered task");
    registered.runner_status = RunnerStatus::Failed;
    registered.error = Some("fixture aggregate failed".to_string());
    state
        .runtime
        .upsert_task(&registered)
        .await
        .expect("store failed aggregate task fixture");

    let (status, second) = post_json(&app, "/v3/proof/batch/shasta", payload).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["status"], "ok");
    assert_eq!(second["proof_type"], "sp1");
    assert_eq!(second["data"]["status"], "registered");
    assert_eq!(single_report_task_id(&app).await, task_id);
}

#[tokio::test]
async fn e2e_duplicate_aggregate_shasta_post_returns_aggregate_proof() {
    let (app, engine) = sp1_fixture_app();
    let payload = json!({
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
    });

    let (status, first) = post_json(&app, "/v3/proof/batch/shasta", payload.clone()).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["data"]["status"], "registered");
    assert!(first["data"].get("task_id").is_none(), "{first}");

    drive_engine_to_idle(&engine).await;

    let (status, second) = post_json(&app, "/v3/proof/batch/shasta", payload).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["status"], "ok");
    assert_eq!(second["proof_type"], "sp1");
    assert_eq!(
        second["data"]["proof"]["proof"],
        "0xfixture-sp1-aggregation"
    );
}

#[tokio::test]
async fn e2e_metrics_endpoint_exposes_key_metric_families() {
    let config = base_config();
    let (app, engine) = app_with_observed_risc0_fixture_engine(config);
    let request = json!({
        "proposals": [{
            "proposal_id": 7,
            "l1_inclusion_block_number": 1,
            "l2_block_numbers": [7],
            "last_anchor_block_number": 0
        }],
        "aggregate": false,
        "proof_type": "risc0",
        "network": "taiko_dev",
        "l1_network": "ethereum"
    });

    let (status, _res) = post_json(&app, "/v3/proof/batch/shasta", request.clone()).await;
    assert_eq!(status, StatusCode::OK);

    drive_engine_to_idle(&engine).await;

    let (status, duplicate) = post_json(&app, "/v3/proof/batch/shasta", request).await;
    assert_eq!(status, StatusCode::OK, "{duplicate}");

    let (status, body) = get_text(&app, "/metrics").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("raiko2_request_registrations_total"),
        "{body}"
    );
    assert!(
        body.contains("raiko2_stage_task_duration_seconds_bucket"),
        "{body}"
    );
    assert!(body.contains("raiko2_duplicate_requests_total"), "{body}");
    assert!(
        body.contains(
            "raiko2_duplicate_requests_total{aggregate=\"false\",pair=\"taiko_dev/ethereum\",proof_type=\"risc0\",route=\"risc0/local\",runner_status=\"completed\"}"
        ),
        "{body}"
    );
    assert!(body.contains("pair=\"taiko_dev/ethereum\""), "{body}");
}

#[tokio::test]
async fn e2e_zk_any_returns_not_drawn_when_ballot_is_disabled() {
    let mut config = base_config();
    config.prover.zk_any = Default::default();
    let engine = risc0_fixture_engine(json!({}));
    let state = app_with_engine(
        config,
        "taiko_dev/ethereum",
        PipelineKey::ShastaRisc0,
        engine,
    );
    let app = app::build_router_with_legacy_v3_for_tests(state);

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
    assert_eq!(res["proof_type"], "zk_any");
    assert_eq!(res["data"]["status"], "zk_any_not_drawn");
    assert_eq!(res["batch_id"], 3);
    assert!(res["data"].get("task_id").is_none(), "{res}");
}

#[tokio::test]
async fn e2e_zk_any_still_validates_request_when_not_drawn() {
    let mut config = base_config();
    config.prover.zk_any = Default::default();
    let engine = risc0_fixture_engine(json!({}));
    let state = app_with_engine(
        config,
        "taiko_dev/ethereum",
        PipelineKey::ShastaRisc0,
        engine,
    );
    let app = app::build_router_with_legacy_v3_for_tests(state);

    let (status, res) = post_json(
        &app,
        "/v3/proof/batch/shasta",
        json!({
            "proposals": [{
                "proposal_id": 3,
                "l1_inclusion_block_number": 1,
                "l2_block_numbers": [],
                "last_anchor_block_number": 0
            }],
            "aggregate": false,
            "proof_type": "zk_any",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["status"], "error");
    assert_eq!(res["error"], "invalid_request_config");
    assert_eq!(
        res["message"],
        "proposal.l2_block_numbers must not be empty"
    );
}

#[tokio::test]
async fn e2e_zk_any_draws_sp1_and_registers_sp1_task() {
    let mut config = base_config();
    set_prover_routes(&mut config, "sp1/local");
    config.prover.zk_any.sp1 = Some(crate::config::ZkAnyTargetConfig {
        probability: 1.0,
        per_day: 0,
    });
    let (state, engine) = state_with_observed_sp1_fixture_engine(config);
    let app = app::build_router_with_legacy_v3_for_tests(state);

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
    assert_eq!(res["data"]["status"], "registered", "{res}");
    assert!(res["data"].get("task_id").is_none(), "{res}");
    let id = single_report_task_id(&app).await;

    drive_engine_to_idle(&engine).await;

    let (status, res) = get_json(&app, &format!("/v3/tasks/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["data"]["route"], "sp1/local");
    assert_eq!(res["data"]["prover_type"], "local");
    assert_eq!(res["data"]["status"], "completed", "{res}");
}

#[tokio::test]
async fn e2e_zk_any_rejects_prover_args() {
    let config = base_config();
    let engine = risc0_fixture_engine(json!({}));
    let app = app_with_risc0_fixture_engine(config, engine);

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

    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["status"], "error");
    assert_eq!(res["error"], "invalid_request_config");
    assert_eq!(
        res["message"],
        "proof_type=zk_any does not support prover args"
    );
}

#[tokio::test]
async fn e2e_admin_ballot_authenticates_before_body_parse() {
    let config = base_config();
    let engine = risc0_fixture_engine(json!({}));
    let app = app_with_risc0_fixture_engine(config, engine);

    let (status, body) =
        post_raw_json_text_with_optional_api_key(&app, "/admin/ballot", None, "{not-json").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let mut config = base_config();
    config.server.acl.keys = vec![acl_key(
        "ops-admin",
        "secret-admin-key",
        vec![ServerAclFeature::AdminBallotWrite],
    )];
    let engine = risc0_fixture_engine(json!({}));
    let app = app_with_risc0_fixture_engine(config, engine);

    let (status, body) =
        post_raw_json_text_with_optional_api_key(&app, "/admin/ballot", None, "{not-json").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    let (status, body) =
        post_raw_json_text_with_optional_api_key(&app, "/admin/ballot", Some("wrong"), "{not-json")
            .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    let (status, body) = post_raw_json_text_with_optional_api_key(
        &app,
        "/admin/ballot",
        Some("secret-admin-key"),
        "{not-json",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
async fn e2e_admin_ballot_requires_key_and_updates_sampler() {
    let mut config = base_config();
    set_prover_routes(&mut config, "risc0/local,sp1/local");
    config.server.acl.keys = vec![
        acl_key(
            "root-admin",
            "secret-root-key",
            vec![ServerAclFeature::Admin],
        ),
        acl_key(
            "ops-admin",
            "secret-admin-key",
            vec![
                ServerAclFeature::AdminBallotRead,
                ServerAclFeature::AdminBallotWrite,
            ],
        ),
        acl_key(
            "ops-clear",
            "secret-clear-key",
            vec![ServerAclFeature::ProverClear],
        ),
    ];
    let (state, engine) = state_with_observed_sp1_fixture_engine(config);
    let app = app::build_router_with_legacy_v3_for_tests(state);

    let (status, res) = get_json(&app, "/admin/ballot").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{res}");

    let (status, res) = get_json_with_api_key(&app, "/admin/ballot", "secret-clear-key").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{res}");

    let (status, res) = post_json_with_api_key(
        &app,
        "/admin/ballot",
        "secret-admin-key",
        json!({
            "Sp1": [1.0, 0],
            "Risc0": [0.0, 0]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["status"], "ok");

    let (status, res) = get_json_with_api_key(&app, "/admin/ballot", "secret-admin-key").await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["Sp1"][0], 1.0);
    assert_eq!(res["Sp1"][1], 0);
    assert_eq!(res["Risc0"][0], 0.0);
    assert_eq!(res["Risc0"][1], 0);

    let (status, res) = get_json_with_api_key(&app, "/admin/ballot", "secret-root-key").await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["Sp1"][0], 1.0);

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

    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["proof_type"], "sp1");
    assert_eq!(res["data"]["status"], "registered", "{res}");
    drive_engine_to_idle(&engine).await;
}

#[tokio::test]
async fn e2e_admin_ballot_rate_limits_acl_key() {
    let mut config = base_config();
    config.server.acl.keys = vec![acl_key_with_rate_limit(
        "ops-admin",
        "secret-admin-key",
        vec![ServerAclFeature::AdminBallotRead],
        Some(1),
    )];
    let engine = risc0_fixture_engine(json!({}));
    let app = app_with_risc0_fixture_engine(config, engine);

    let (status, res) = get_json_with_api_key(&app, "/admin/ballot", "secret-admin-key").await;
    assert_eq!(status, StatusCode::OK, "{res}");

    let (status, res) = get_json_with_api_key(&app, "/admin/ballot", "secret-admin-key").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{res}");
    assert_eq!(res["error"], "rate_limited");
}

#[tokio::test]
async fn e2e_admin_ballot_rejects_unsupported_proof_type() {
    let mut config = base_config();
    config.server.acl.keys = vec![acl_key(
        "ops-admin",
        "secret-admin-key",
        vec![ServerAclFeature::AdminBallotWrite],
    )];
    let engine = risc0_fixture_engine(json!({}));
    let app = app_with_risc0_fixture_engine(config, engine);

    let (status, res) = post_json_with_api_key(
        &app,
        "/admin/ballot",
        "secret-admin-key",
        json!({
            "Native": [1.0, 0]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{res}");
    assert!(
        res["message"]
            .as_str()
            .expect("message")
            .contains("not supported for zk_any")
    );
}

#[tokio::test]
async fn e2e_prover_clear_requires_clear_api_key() {
    let config = base_config();
    let engine = risc0_fixture_engine(json!({}));
    let app = app_with_risc0_fixture_engine(config, engine);

    let (status, res) = post_json(&app, "/v3/prover/clear", json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{res}");

    let mut config = base_config();
    config.server.acl.keys = vec![
        acl_key(
            "root-admin",
            "secret-root-key",
            vec![ServerAclFeature::Admin],
        ),
        acl_key(
            "ops-admin",
            "secret-admin-key",
            vec![ServerAclFeature::AdminBallotRead],
        ),
        acl_key(
            "ops-clear",
            "secret-clear-key",
            vec![ServerAclFeature::ProverClear],
        ),
    ];
    let engine = risc0_fixture_engine(json!({}));
    let app = app_with_risc0_fixture_engine(config, engine);

    let (status, res) = post_json(&app, "/v3/prover/clear", json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{res}");

    let (status, res) = post_json_with_api_key(&app, "/v3/prover/clear", "wrong", json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{res}");

    let (status, res) =
        post_json_with_api_key(&app, "/v3/prover/clear", "secret-admin-key", json!({})).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{res}");

    let (status, res) =
        post_json_with_api_key(&app, "/v3/prover/clear", "secret-root-key", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["status"], "ok");

    let (status, res) =
        post_json_with_api_key(&app, "/v3/prover/clear", "secret-clear-key", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["status"], "ok");
}

#[tokio::test]
async fn e2e_v3_clear_rate_limits_acl_key() {
    let mut config = base_config();
    config.server.acl.keys = vec![acl_key_with_rate_limit(
        "ops-clear",
        "secret-clear-key",
        vec![ServerAclFeature::ProverClear],
        Some(1),
    )];
    let engine = risc0_fixture_engine(json!({}));
    let app = app_with_risc0_fixture_engine(config, engine);

    let (status, res) =
        post_json_with_api_key(&app, "/v3/prover/clear", "secret-clear-key", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{res}");

    let (status, res) =
        post_json_with_api_key(&app, "/v3/prover/clear", "secret-clear-key", json!({})).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{res}");
    assert_eq!(res["error"], "rate_limited");
}

#[tokio::test]
async fn e2e_sp1_execute_returns_execution_metadata() {
    let mut config = base_config();
    set_prover_routes(&mut config, "sp1/local");
    config.prover.sp1.prover = Sp1ProverMode::Local;

    let (state, _engine) = state_with_observed_sp1_fixture_engine(config);
    let app = app::build_router_with_legacy_v3_for_tests(state);

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
    assert_eq!(res["error"], "invalid_request_config", "{res}");
    assert!(
        res["message"]
            .as_str()
            .is_some_and(|message| message.contains("sp1.mode=execute")),
        "{res}"
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
    assert_eq!(res["data"]["status"], "registered", "{res}");
    assert!(res["data"].get("task_id").is_none(), "{res}");
    let id = single_report_task_id(&app).await;

    drive_engine_to_idle(&engine).await;

    let (status, res) = get_json(&app, &format!("/v3/tasks/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["data"]["status"], "completed");
    assert_eq!(res["data"]["aggregate"]["status"], "completed");
    assert_eq!(res["data"]["proof"], "0xfixture-sp1-aggregation");
}

#[tokio::test]
async fn e2e_batch_single_proof_aggregate_sp1_completes_from_fixture() {
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
    assert_eq!(res["data"]["status"], "registered", "{res}");
    assert!(res["data"].get("task_id").is_none(), "{res}");
    let id = single_report_task_id(&app).await;

    drive_engine_to_idle(&engine).await;

    let (status, res) = get_json(&app, &format!("/v3/tasks/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["data"]["status"], "completed");
    assert_eq!(res["data"]["aggregate"]["status"], "completed");
    assert_eq!(res["data"]["proof"], "0xfixture-sp1-aggregation", "{res}");
}

#[tokio::test]
async fn e2e_batch_aggregate_sp1_reuses_cached_proposal_proof() {
    let mut config = base_config();
    set_prover_routes(&mut config, "sp1/local");
    config.prover.sp1.prover = Sp1ProverMode::Local;

    let (state, engine) = state_with_observed_sp1_fixture_engine(config);
    let proposal_request = ProposalTaskRequest {
        proposal_id: 3,
        l2_block_range: Some(raiko2_primitives::L2BlockRange { start: 3, end: 3 }),
        l1_inclusion_block_number: 1,
        last_anchor_block_number: 0,
        checkpoint: None,
        blob_proof_type: None,
        prover: None,
        graffiti: None,
        prover_config: ProverTaskConfig::default(),
    };
    let proof_ref = proposal_task_ref(PipelineKey::ShastaSp1, &proposal_request);
    let proof: Proof = serde_json::from_value(sp1_external_proof("0xcached-sp1-proof".to_string()))
        .expect("cached proof");
    let proof_uri = write_e2e_proof_artifact(
        &state,
        "taiko_dev/ethereum",
        &proof_ref,
        PipelineKey::ShastaSp1,
        PipelineKey::ShastaSp1.route(),
        &proof,
    )
    .await;

    let app = app::build_router_with_legacy_v3_for_tests(state);
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
    let id = single_report_task_id(&app).await;

    assert!(
        engine
            .run_one("e2e")
            .await
            .expect("recover cached proposal")
    );
    assert!(engine.run_one("e2e").await.expect("run aggregate"));
    assert!(
        !engine.run_one("e2e").await.expect("queue drained"),
        "cached proposal recovery and aggregation should drain the canonical graph"
    );

    let (status, res) = get_json(&app, &format!("/v3/tasks/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["data"]["status"], "completed", "{res}");
    assert_eq!(res["data"]["proposals"][0]["status"], "completed");
    assert_eq!(res["data"]["aggregate"]["status"], "completed");
    assert_eq!(res["data"]["proof"], "0xfixture-sp1-aggregation");
    assert_eq!(res["data"]["proposals"][0]["proof_ref"], proof_ref);
    assert_eq!(res["data"]["proposals"][0]["proof_uri"], proof_uri);
    assert_eq!(
        res["data"]["proof_ref"],
        res["data"]["aggregate"]["proof_ref"]
    );
    assert_eq!(
        res["data"]["proof_uri"],
        res["data"]["aggregate"]["proof_uri"]
    );
}

#[tokio::test]
async fn e2e_batch_aggregate_rejects_zk_any() {
    let (app, _engine) = sp1_fixture_app();

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
                }
            ],
            "aggregate": true,
            "proof_type": "zk_any",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["status"], "error");
    assert_eq!(res["error"], "invalid_request_config");
    assert_eq!(
        res["message"],
        "proof_type=zk_any is not supported for aggregate requests"
    );
    assert!(report_task_ids(&app).await.is_empty());
}

#[tokio::test]
async fn e2e_batch_aggregate_zk_any_rejection_does_not_consume_ballot() {
    let mut config = base_config();
    config.prover.zk_any.sp1 = Some(crate::config::ZkAnyTargetConfig {
        probability: 1.0,
        per_day: 1,
    });
    set_prover_routes(&mut config, "sp1/local");
    config.prover.sp1.prover = Sp1ProverMode::Local;

    let engine = sp1_fixture_engine(json!({}));
    let state = app_with_engine(config, "taiko_dev/ethereum", PipelineKey::ShastaSp1, engine);
    let app = app::build_router_with_legacy_v3_for_tests(state);

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
                }
            ],
            "aggregate": true,
            "proof_type": "zk_any",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["status"], "error");
    assert_eq!(
        res["message"],
        "proof_type=zk_any is not supported for aggregate requests"
    );
    assert!(report_task_ids(&app).await.is_empty());

    let (status, res) = post_json(
        &app,
        "/v3/proof/batch/shasta",
        json!({
            "proposals": [
                {
                    "proposal_id": 4,
                    "l1_inclusion_block_number": 1,
                    "l2_block_numbers": [4],
                    "last_anchor_block_number": 0
                }
            ],
            "aggregate": false,
            "proof_type": "zk_any",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["status"], "ok");
    assert_eq!(res["proof_type"], "sp1");
    assert_eq!(res["data"]["status"], "registered");
    assert_eq!(report_task_ids(&app).await.len(), 1);
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
    assert_eq!(res["data"]["status"], "registered");
    assert!(res["data"].get("task_id").is_none(), "{res}");
    let id = single_report_task_id(&app).await;

    drive_engine_to_idle(&engine).await;

    let (status, res) = get_json(&app, &format!("/v3/tasks/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["data"]["status"], "completed");
    assert_eq!(res["data"]["aggregate"]["status"], "completed");
    assert_eq!(res["data"]["proof"], "0xfixture-sp1-aggregation");
}

#[tokio::test]
async fn e2e_aggregate_request_accepts_legacy_aggregation_ids() {
    let (app, engine) = sp1_fixture_app();

    let (status, res) = post_json(
        &app,
        "/v3/proof/aggregate",
        json!({
            "aggregation_ids": [10, 11],
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
    assert_eq!(res["data"]["status"], "registered");
    assert!(res["data"].get("task_id").is_none(), "{res}");
    let id = single_report_task_id(&app).await;

    drive_engine_to_idle(&engine).await;

    let (status, res) = get_json(&app, &format!("/v3/tasks/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["data"]["status"], "completed");
    assert_eq!(res["data"]["aggregate"]["status"], "completed");
    assert_eq!(res["data"]["proof"], "0xfixture-sp1-aggregation");
}

#[tokio::test]
async fn e2e_duplicate_aggregate_post_reuses_same_root_task() {
    let (app, _engine) = sp1_fixture_app();
    let payload = json!({
        "aggregation_ids": [10, 11],
        "proofs": [
            sp1_external_proof("0xfixture-proof-a".to_string()),
            sp1_external_proof("0xfixture-proof-b".to_string())
        ],
        "proof_type": "sp1",
        "network": "taiko_dev",
        "l1_network": "ethereum"
    });

    let (status, first) = post_json(&app, "/v3/proof/aggregate", payload.clone()).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["data"]["status"], "registered");
    assert!(first["data"].get("task_id").is_none(), "{first}");
    let first_id = single_report_task_id(&app).await;

    let (status, second) = post_json(&app, "/v3/proof/aggregate", payload).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["status"], "ok");
    assert_eq!(second["data"]["status"], "registered");
    assert!(second["data"].get("task_id").is_none(), "{second}");
    assert_eq!(single_report_task_id(&app).await, first_id);
}

#[tokio::test]
async fn e2e_duplicate_aggregate_post_returns_work_in_progress_when_runtime_has_progress() {
    let mut config = base_config();
    set_prover_routes(&mut config, "sp1/network");
    config.prover.sp1.prover = Sp1ProverMode::Network;
    config.prover.sp1.verify = true;
    config.rpc.pairs[0].sp1_verifier_rpc_url = Some("https://verifier.example.com".to_string());
    config.rpc.pairs[0].sp1_verifier_address =
        Some("0x0000000000000000000000000000000000000001".to_string());
    let engine = sp1_fixture_engine(json!({}));
    let state = app_with_engine(config, "taiko_dev/ethereum", PipelineKey::ShastaSp1, engine);
    let app = app::build_router_with_legacy_v3_for_tests(state.clone());
    let payload = json!({
        "aggregation_ids": [10, 11],
        "proofs": [
            sp1_external_proof("0xfixture-proof-a".to_string()),
            sp1_external_proof("0xfixture-proof-b".to_string())
        ],
        "proof_type": "sp1",
        "network": "taiko_dev",
        "l1_network": "ethereum"
    });

    let (status, first) = post_json(&app, "/v3/proof/aggregate", payload.clone()).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["data"]["status"], "registered");
    assert!(first["data"].get("task_id").is_none(), "{first}");
    let task_id = single_report_task_id(&app).await;

    let mut record = state
        .runtime
        .get_task(&task_id)
        .await
        .expect("read task")
        .expect("task exists");
    let mut metadata: TaskMetadata =
        serde_json::from_value(record.metadata.clone()).expect("deserialize metadata");
    metadata.runtime.active_stage = Some("aggregate".to_string());
    metadata.runtime.last_event = Some("submission_registered".to_string());
    metadata.runtime.aggregate = Some(TaskRuntimeMetadata {
        updated_at: 1,
        provider_request_id: Some("0xsp1-aggregate".to_string()),
        expires_at: Some(7_201),
        submitted_at: Some(1),
        rebid_attempt: Some(1),
        sp1_network_mode: Some(raiko2_prover::Sp1NetworkMode::Reserved),
        sp1_fulfillment_strategy: Some(raiko2_prover::Sp1FulfillmentStrategy::Reserved),
        sp1_skip_simulation: Some(false),
        sp1_cycle_limit: Some(1_000_000),
        sp1_timeout_secs: Some(7_200),
        ..TaskRuntimeMetadata::default()
    });
    record.metadata = serde_json::to_value(metadata).expect("serialize metadata");
    state
        .runtime
        .upsert_task(&record)
        .await
        .expect("upsert task");

    let (status, second) = post_json(&app, "/v3/proof/aggregate", payload).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["status"], "ok");
    assert_eq!(second["data"]["status"], "work_in_progress");
    assert!(second["data"].get("task_id").is_none(), "{second}");
    assert_eq!(single_report_task_id(&app).await, task_id);
}

#[tokio::test]
async fn e2e_duplicate_aggregate_post_returns_completed_legacy_proof() {
    let (app, engine) = sp1_fixture_app();
    let payload = json!({
        "aggregation_ids": [10, 11],
        "proofs": [
            sp1_external_proof("0xfixture-proof-a".to_string()),
            sp1_external_proof("0xfixture-proof-b".to_string())
        ],
        "proof_type": "sp1",
        "network": "taiko_dev",
        "l1_network": "ethereum"
    });

    let (status, first) = post_json(&app, "/v3/proof/aggregate", payload.clone()).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["data"]["status"], "registered");
    assert!(first["data"].get("task_id").is_none(), "{first}");

    drive_engine_to_idle(&engine).await;

    let (status, second) = post_json(&app, "/v3/proof/aggregate", payload).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["status"], "ok");
    assert!(second["data"]["proof"]["proof"].is_string(), "{second}");
    assert!(second["data"]["proof"].get("status").is_none(), "{second}");
}

#[tokio::test]
async fn e2e_aggregate_single_sp1_external_proof_completes_from_fixture() {
    let (app, engine) = sp1_fixture_app();

    let (status, res) = post_json(
        &app,
        "/v3/proof/aggregate",
        json!({
            "proofs": [
                sp1_external_proof("0xfixture-proof-a".to_string())
            ],
            "proof_type": "sp1",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["data"]["status"], "registered");
    assert!(res["data"].get("task_id").is_none(), "{res}");
    let id = single_report_task_id(&app).await;

    drive_engine_to_idle(&engine).await;

    let (status, res) = get_json(&app, &format!("/v3/tasks/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["data"]["status"], "completed");
    assert_eq!(res["data"]["aggregate"]["status"], "completed");
    assert_eq!(res["data"]["proof"], "0xfixture-sp1-aggregation");
}

#[tokio::test]
async fn e2e_aggregate_request_uses_default_pair_when_network_fields_are_omitted() {
    let (app, engine) = sp1_fixture_app();

    let (status, res) = post_json(
        &app,
        "/v3/proof/aggregate",
        json!({
            "proofs": [
                sp1_external_proof("0xfixture-proof-a".to_string()),
                sp1_external_proof("0xfixture-proof-b".to_string())
            ],
            "proof_type": "sp1"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["data"]["status"], "registered");
    assert!(res["data"].get("task_id").is_none(), "{res}");
    let id = single_report_task_id(&app).await;

    drive_engine_to_idle(&engine).await;

    let (status, res) = get_json(&app, &format!("/v3/tasks/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["data"]["status"], "completed");
    assert_eq!(res["data"]["network"], "taiko_dev");
    assert_eq!(res["data"]["l1_network"], "ethereum");
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
    assert_eq!(res["data"]["status"], "registered", "{res}");
    assert!(res["data"].get("task_id").is_none(), "{res}");
    let id = single_report_task_id(&app).await;

    drive_engine_to_idle(&engine).await;

    let (status, res) = get_json(&app, &format!("/v3/tasks/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["data"]["route"], "risc0/network");
    assert_eq!(res["data"]["prover_type"], "network");
    assert_eq!(res["data"]["status"], "completed", "{res}");
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

    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["status"], "error");
    assert_eq!(res["error"], "invalid_request_config");
    assert_eq!(
        res["message"],
        "proof_type=zk_any is not supported for aggregate requests"
    );
}

#[tokio::test]
async fn e2e_aggregate_reports_unregistered_sgxgeth_pipeline() {
    let mut config = base_config();
    set_prover_routes(&mut config, "sgxgeth/remote");
    let engine = sp1_fixture_engine(json!({}));
    let state = app_with_engine(config, "taiko_dev/ethereum", PipelineKey::ShastaSp1, engine);
    let app = app::build_router_with_legacy_v3_for_tests(state);

    let (status, res) = post_json(
        &app,
        "/v3/proof/aggregate",
        json!({
            "proofs": [
                sp1_external_proof("0xfixture-proof-a".to_string()),
                sp1_external_proof("0xfixture-proof-b".to_string())
            ],
            "proof_type": "sgxgeth",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["status"], "error");
    assert_eq!(res["error"], "not_found");
    assert!(report_task_ids(&app).await.is_empty());
}

#[tokio::test]
async fn e2e_aggregate_rejects_boundless_public_proof_type() {
    let (app, _engine) = sp1_fixture_app();

    let (status, res) = post_json(
        &app,
        "/v3/proof/aggregate",
        json!({
            "proofs": [
                sp1_external_proof("0xfixture-proof-a".to_string()),
                sp1_external_proof("0xfixture-proof-b".to_string())
            ],
            "proof_type": "boundless",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["status"], "error");
    assert_eq!(res["error"], "invalid_request_config");
    assert_eq!(res["message"], "proof_type=boundless is not supported");
    assert!(report_task_ids(&app).await.is_empty());
}

#[tokio::test]
async fn e2e_aggregate_rejects_native_public_proof_type() {
    let (app, _engine) = sp1_fixture_app();

    let (status, res) = post_json(
        &app,
        "/v3/proof/aggregate",
        json!({
            "proofs": [
                sp1_external_proof("0xfixture-proof-a".to_string()),
                sp1_external_proof("0xfixture-proof-b".to_string())
            ],
            "proof_type": "native",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["status"], "error");
    assert_eq!(res["error"], "invalid_request_config");
    assert_eq!(res["message"], "proof_type=native is not supported");
    assert!(report_task_ids(&app).await.is_empty());
}

#[tokio::test]
async fn e2e_report_and_list_expose_root_tasks_only() {
    let config = base_config();
    let (app, engine) = app_with_observed_risc0_fixture_engine(config);

    let payload = |proposal_id| {
        json!({
            "proposals": [{
                "proposal_id": proposal_id,
                "l1_inclusion_block_number": 1,
                "l2_block_numbers": [proposal_id],
                "last_anchor_block_number": 0
            }],
            "aggregate": false,
            "proof_type": "risc0",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        })
    };

    let (status, first) = post_json(&app, "/v3/proof/batch/shasta", payload(3)).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["data"]["status"], "registered");
    assert!(first["data"].get("task_id").is_none(), "{first}");
    let first_id = single_report_task_id(&app).await;

    drive_engine_to_idle(&engine).await;

    let (status, second) = post_json(&app, "/v3/proof/batch/shasta", payload(4)).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["data"]["status"], "registered");
    assert!(second["data"].get("task_id").is_none(), "{second}");
    let ids = report_task_ids(&app).await;
    let second_id = ids
        .into_iter()
        .find(|id| id != &first_id)
        .expect("second task id");

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
    let engine = risc0_fixture_engine(json!({}));
    let app = app_with_risc0_fixture_engine(config, engine);

    let (status, res) = post_json(&app, "/proof/prune", json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{res}");

    let mut config = base_config();
    config.server.acl.keys = vec![
        acl_key(
            "ops-clear",
            "secret-clear-key",
            vec![ServerAclFeature::ProverClear],
        ),
        acl_key(
            "root-admin",
            "secret-admin-key",
            vec![ServerAclFeature::Admin],
        ),
    ];
    let engine = risc0_fixture_engine(json!({}));
    let app = app_with_risc0_fixture_engine(config, engine);

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
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["data"]["status"], "registered");
    assert!(res["data"].get("task_id").is_none(), "{res}");
    let id = single_report_task_id(&app).await;

    let (status, res) = post_json(&app, "/v3/proof/prune", json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{res}");

    let (status, res) = post_json_with_api_key(&app, "/proof/prune", "wrong", json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{res}");

    let (status, res) =
        post_json_with_api_key(&app, "/proof/prune", "secret-clear-key", json!({})).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{res}");

    let (status, prune) =
        post_json_with_api_key(&app, "/proof/prune", "secret-admin-key", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{prune}");
    assert_eq!(prune["status"], "ok");

    let (status, report) = get_json(&app, "/v3/proof/report").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report.as_array().map(Vec::len), Some(0));

    let (status, task) = get_json(&app, &format!("/v3/tasks/{id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(task["status"], "error");
}

#[tokio::test]
async fn e2e_prune_rate_limits_admin_key() {
    let mut config = base_config();
    config.server.acl.keys = vec![acl_key_with_rate_limit(
        "root-admin",
        "secret-admin-key",
        vec![ServerAclFeature::Admin],
        Some(1),
    )];
    let engine = risc0_fixture_engine(json!({}));
    let app = app_with_risc0_fixture_engine(config, engine);

    let (status, res) =
        post_json_with_api_key(&app, "/v3/proof/prune", "secret-admin-key", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{res}");

    let (status, res) =
        post_json_with_api_key(&app, "/v3/proof/prune", "secret-admin-key", json!({})).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{res}");
    assert_eq!(res["error"], "rate_limited");
}

#[tokio::test]
async fn e2e_sp1_execute_rejects_aggregate_requests() {
    let mut config = base_config();
    set_prover_routes(&mut config, "sp1/local");
    let engine = risc0_fixture_engine(json!({}));
    let app = app_with_risc0_fixture_engine(config, engine);

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
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["status"], "error");
    assert_eq!(res["error"], "invalid_request_config");
    assert_eq!(
        res["message"],
        "sp1.mode=execute does not support aggregate=true"
    );
}

#[tokio::test]
async fn e2e_sp1_hosted_api_rejects_unverified_prove_requests() {
    let mut config = base_config();
    set_prover_routes(&mut config, "sp1/local");
    config.prover.sp1.prover = Sp1ProverMode::Local;
    config.prover.sp1.verify = false;

    let engine = sp1_fixture_engine(json!({}));
    let state = app_with_engine(config, "taiko_dev/ethereum", PipelineKey::ShastaSp1, engine);
    let app = app::build_router_with_legacy_v3_for_tests(state);

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
    assert_eq!(res["status"], "error");
    assert_eq!(res["error"], "invalid_request_config");
    assert_eq!(
        res["message"],
        "sp1.mode=prove requires sp1.verify=true on the hosted API"
    );
}

#[tokio::test]
async fn e2e_sp1_hosted_api_rejects_network_verify_when_pair_not_enabled() {
    let mut config = base_config();
    set_prover_routes(&mut config, "sp1/network");
    config.prover.sp1.prover = Sp1ProverMode::Network;
    config.prover.sp1.verify = true;

    let engine = sp1_fixture_engine(json!({}));
    let state = app_with_engine(config, "taiko_dev/ethereum", PipelineKey::ShastaSp1, engine);
    let app = app::build_router_with_legacy_v3_for_tests(state);

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
    assert_eq!(res["status"], "error");
    assert_eq!(res["error"], "invalid_request_config");
    assert_eq!(
        res["message"],
        "sp1 network verification is not enabled for network pair taiko_dev/ethereum"
    );
}

#[tokio::test]
async fn e2e_sp1_hosted_api_accepts_network_verify_when_pair_enabled() {
    let mut config = base_config();
    set_prover_routes(&mut config, "sp1/network");
    config.prover.sp1.prover = Sp1ProverMode::Network;
    config.prover.sp1.verify = true;
    config.rpc.pairs[0].sp1_verifier_rpc_url = Some("https://verifier.example.com".to_string());
    config.rpc.pairs[0].sp1_verifier_address =
        Some("0x0000000000000000000000000000000000000001".to_string());

    let engine = sp1_fixture_engine(json!({}));
    let state = app_with_engine(config, "taiko_dev/ethereum", PipelineKey::ShastaSp1, engine);
    let app = app::build_router_with_legacy_v3_for_tests(state);

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
    assert!(res["data"].get("task_id").is_none(), "{res}");
    let id = single_report_task_id(&app).await;

    let (status, res) = get_json(&app, &format!("/v3/tasks/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["data"]["route"], "sp1/network");
    assert_eq!(res["data"]["prover_type"], "network");
}

#[tokio::test]
async fn e2e_sgx_batch_accepts_aggregate_requests() {
    let mut config = base_config();
    set_prover_routes(&mut config, "sgx/remote");
    let engine = native_fixture_engine_for_pipeline(PipelineKey::ShastaSgx, None);
    let state = app_with_engine(config, "taiko_dev/ethereum", PipelineKey::ShastaSgx, engine);
    let app = app::build_router_with_legacy_v3_for_tests(state);

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
                    "proposal_id": 4,
                    "l1_inclusion_block_number": 1,
                    "l2_block_numbers": [4],
                    "last_anchor_block_number": 0
                }
            ],
            "aggregate": true,
            "proof_type": "sgx",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {res}");
    assert_eq!(res["data"]["status"], "registered", "{res}");
    assert!(res["data"].get("task_id").is_none(), "{res}");
}

#[tokio::test]
async fn e2e_sgx_accepts_aggregate_proof_requests() {
    let mut config = base_config();
    set_prover_routes(&mut config, "sgx/remote");
    let engine = native_fixture_engine_for_pipeline(PipelineKey::ShastaSgx, None);
    let state = app_with_engine(config, "taiko_dev/ethereum", PipelineKey::ShastaSgx, engine);
    let app = app::build_router_with_legacy_v3_for_tests(state);

    let (status, res) = post_json(
        &app,
        "/v3/proof/aggregate",
        json!({
            "proofs": [
                fixture_external_aggregate_proof(),
                fixture_external_aggregate_proof()
            ],
            "proof_type": "sgx",
            "network": "taiko_dev",
            "l1_network": "ethereum"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {res}");
    assert_eq!(res["data"]["status"], "registered", "{res}");
    assert!(res["data"].get("task_id").is_none(), "{res}");
}

#[tokio::test]
async fn e2e_sp1_network_settings_require_network_prover() {
    let mut config = base_config();
    set_prover_routes(&mut config, "sp1/local");
    let engine = risc0_fixture_engine(json!({}));
    let app = app_with_risc0_fixture_engine(config, engine);

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
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["status"], "error");
    assert_eq!(res["error"], "invalid_request_config");
    assert_eq!(
        res["message"],
        "sp1 network-only settings require sp1.prover=network"
    );
}

#[tokio::test]
async fn e2e_cancel_marks_task_cancelled_without_workers() {
    let config = base_config();
    let engine = risc0_fixture_engine(json!({}));
    let app = app_with_risc0_fixture_engine(config, engine);

    let (status, res) = post_json(&app, "/v3/tasks/missing/cancel", json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{res}");

    let mut config = base_config();
    config.server.acl.keys = vec![
        acl_key(
            "ops-clear",
            "secret-clear-key",
            vec![ServerAclFeature::ProverClear],
        ),
        acl_key(
            "ballot-admin",
            "secret-ballot-key",
            vec![ServerAclFeature::AdminBallotWrite],
        ),
        acl_key(
            "root-admin",
            "secret-admin-key",
            vec![ServerAclFeature::Admin],
        ),
    ];
    let engine = risc0_fixture_engine(json!({}));
    let app = app_with_risc0_fixture_engine(config, engine);

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
    assert_eq!(res["data"]["status"], "registered");
    assert!(res["data"].get("task_id").is_none(), "{res}");
    let id = single_report_task_id(&app).await;

    let cancel_uri = format!("/v3/tasks/{id}/cancel");

    let (status, res) = post_json(&app, &cancel_uri, json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{res}");

    let (status, res) = post_json_with_api_key(&app, &cancel_uri, "wrong", json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{res}");

    let (status, res) =
        post_json_with_api_key(&app, &cancel_uri, "secret-clear-key", json!({})).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{res}");

    let (status, res) =
        post_json_with_api_key(&app, &cancel_uri, "secret-ballot-key", json!({})).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{res}");

    let (status, res) =
        post_json_with_api_key(&app, &cancel_uri, "secret-admin-key", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["data"]["status"], "cancelled");
}

#[tokio::test]
async fn e2e_duplicate_batch_request_reuses_existing_root_task() {
    let config = base_config();
    let engine = risc0_fixture_engine(json!({}));
    let app = app_with_risc0_fixture_engine(config, engine);
    let payload = json!({
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
    });

    let (status, first) = post_json(&app, "/v3/proof/batch/shasta", payload.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["data"]["status"], "registered");
    assert!(first["data"].get("task_id").is_none(), "{first}");
    let first_id = single_report_task_id(&app).await;

    let (status, second) = post_json(&app, "/v3/proof/batch/shasta", payload).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(second["status"], "ok");
    assert_eq!(second["data"]["status"], "registered");
    assert!(second["data"].get("task_id").is_none(), "{second}");
    assert_eq!(single_report_task_id(&app).await, first_id);
}

#[tokio::test]
async fn e2e_task_status_completes_after_single_proposal_task() {
    let config = base_config();
    let (app, engine) = app_with_observed_risc0_fixture_engine(config);

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
    assert_eq!(res["data"]["status"], "registered");
    assert!(res["data"].get("task_id").is_none(), "{res}");
    let id = single_report_task_id(&app).await;

    let ran = engine.run_one("e2e-worker").await.expect("run one task");
    assert!(ran, "expected proposal task to run");

    let (status, res) = get_json(&app, &format!("/v3/tasks/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["data"]["route"], "risc0/local");
    assert_eq!(res["data"]["prover_type"], "local");
    assert_eq!(res["data"]["status"], "completed");
    assert!(res["data"]["current_index"].is_null());
    assert_eq!(res["data"]["proposals"][0]["status"], "completed");
    assert!(
        res["data"]["proposals"][0].get("runtime").is_none(),
        "unexpected per-proposal runtime while engine state is present: {res}"
    );
}

#[tokio::test]
async fn e2e_task_status_falls_back_to_runtime_metadata_without_mutating_runtime_store() {
    let config = base_config();
    let engine = risc0_fixture_engine(json!({}));
    let state = app_with_engine(
        config,
        "taiko_dev/ethereum",
        PipelineKey::ShastaRisc0Network,
        engine,
    );
    let app = app::build_router_with_legacy_v3_for_tests(state.clone());

    let proposal_request = ProposalTaskRequest {
        proposal_id: 3,
        l2_block_range: Some(raiko2_primitives::L2BlockRange { start: 3, end: 3 }),
        l1_inclusion_block_number: 1,
        last_anchor_block_number: 0,
        checkpoint: None,
        blob_proof_type: None,
        prover: None,
        graffiti: None,
        prover_config: Default::default(),
    };
    let proposal_ref = proposal_task_ref(PipelineKey::ShastaRisc0Network, &proposal_request);
    let mut metadata = TaskMetadata {
        network_pair: "taiko_dev/ethereum".to_string(),
        network: "taiko_dev".to_string(),
        l1_network: "ethereum".to_string(),
        proof_type: raiko2_primitives::ProofType::Risc0,
        requested_proof_type: None,
        prover_type: None,
        execution_mode: None,
        aggregate_requested: false,
        proposals: vec![ProposalTask {
            proposal_id: 3,
            checkpoint: None,
            l1_inclusion_block_number: 1,
            l2_block_numbers: vec![3],
            last_anchor_block_number: 0,
            task_id: proposal_ref.clone(),
            request: proposal_request,
        }],
        aggregate_task_id: None,
        aggregate_request: None,
        aggregate_input_artifacts: Vec::new(),
        runtime: RuntimeMetadata {
            active_stage: Some("prove".to_string()),
            last_event: Some("submission_registered".to_string()),
            ..Default::default()
        },
    };
    let updated_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_secs() as i64;
    metadata
        .upsert_proposal_runtime(
            &proposal_ref,
            &BoundlessSubmissionProgress {
                provider_request_id: "0x1234".to_string(),
                remote_tx_hash: Some("0xabcd".to_string()),
                expires_at: 123_456,
                lock_expires_at: 123_300,
                submitted_at: 123_000,
                image_ref: "0ximage".to_string(),
                deployment: "base".to_string(),
                offchain: false,
                quoted_mcycles_count: Some(6_000),
                evaluated_mcycles_count: Some(12_345),
                max_price_multiplier: 4,
                max_price_wei: Some("9000000000000".to_string()),
                rebid_attempt: 2,
            },
            updated_at,
        )
        .expect("persist canonical Boundless progress");
    let artifact_refs = publication_proof_artifact_refs(&metadata, PipelineKey::ShastaRisc0Network);

    state
        .runtime
        .register_task(TaskRegistration {
            task_id: "task_runtime_fallback".to_string(),
            pipeline_key: PipelineKey::ShastaRisc0Network,
            route: "risc0/network"
                .parse::<PipelineRoute>()
                .expect("parse route"),
            task_kind: "hoodi_batch".to_string(),
            network_pair: "taiko_dev/ethereum".into(),
            artifact_refs,
            metadata: serde_json::to_value(metadata).expect("serialize metadata"),
            request_fingerprint: "task-runtime-fallback".into(),
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
    assert_eq!(res["data"]["route"], "risc0/network");
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
    assert_eq!(res["data"]["proposals"][0]["runtime"]["expires_at"], 123456);
    assert_eq!(
        res["data"]["proposals"][0]["runtime"]["quoted_mcycles_count"],
        6000
    );
    assert_eq!(
        res["data"]["proposals"][0]["runtime"]["evaluated_mcycles_count"],
        12345
    );
    assert_eq!(
        res["data"]["proposals"][0]["runtime"]["submitted_at"],
        123000
    );
    assert_eq!(
        res["data"]["proposals"][0]["runtime"]["max_price_multiplier"],
        4
    );
    assert_eq!(
        res["data"]["proposals"][0]["runtime"]["max_price_wei"],
        "9000000000000"
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
async fn e2e_completed_task_recovers_root_proof_from_artifact_store() {
    let config = base_config();
    let engine = risc0_fixture_engine(json!({}));
    let state = app_with_engine(
        config,
        "taiko_dev/ethereum",
        PipelineKey::ShastaRisc0,
        engine,
    );
    let app = app::build_router_with_legacy_v3_for_tests(state.clone());

    let proposal_request = ProposalTaskRequest {
        proposal_id: 3,
        l2_block_range: Some(raiko2_primitives::L2BlockRange { start: 3, end: 3 }),
        l1_inclusion_block_number: 1,
        last_anchor_block_number: 0,
        checkpoint: None,
        blob_proof_type: None,
        prover: None,
        graffiti: None,
        prover_config: Default::default(),
    };
    let proof_ref = proposal_task_ref(PipelineKey::ShastaRisc0, &proposal_request);
    let metadata = TaskMetadata {
        network_pair: "taiko_dev/ethereum".to_string(),
        network: "taiko_dev".to_string(),
        l1_network: "ethereum".to_string(),
        proof_type: raiko2_primitives::ProofType::Risc0,
        requested_proof_type: None,
        prover_type: None,
        execution_mode: None,
        aggregate_requested: false,
        proposals: vec![ProposalTask {
            proposal_id: 3,
            checkpoint: None,
            l1_inclusion_block_number: 1,
            l2_block_numbers: vec![3],
            last_anchor_block_number: 0,
            task_id: proof_ref.clone(),
            request: proposal_request,
        }],
        aggregate_task_id: None,
        aggregate_request: None,
        aggregate_input_artifacts: Vec::new(),
        runtime: RuntimeMetadata {
            last_event: Some("completed".to_string()),
            ..Default::default()
        },
    };
    let artifact_refs = publication_proof_artifact_refs(&metadata, PipelineKey::ShastaRisc0);

    state
        .runtime
        .register_task(TaskRegistration {
            task_id: "task_persisted_proof".to_string(),
            pipeline_key: PipelineKey::ShastaRisc0,
            route: "risc0/local".parse::<PipelineRoute>().expect("parse route"),
            task_kind: "hoodi_batch".to_string(),
            network_pair: "taiko_dev/ethereum".into(),
            artifact_refs,
            metadata: serde_json::to_value(metadata).expect("serialize metadata"),
            request_fingerprint: "task-persisted-proof".into(),
        })
        .await
        .expect("register task");

    let mut record = state
        .runtime
        .get_task("task_persisted_proof")
        .await
        .expect("read task")
        .expect("task exists");
    let proof = raiko2_primitives::Proof {
        proof: Some("0xpersisted-proof".to_string()),
        ..Default::default()
    };
    let proof_uri = write_e2e_proof_artifact(
        &state,
        "taiko_dev/ethereum",
        &proof_ref,
        record.pipeline_key,
        record.route,
        &proof,
    )
    .await;
    record.runner_status = RunnerStatus::Completed;
    record.proof_uri = Some(proof_uri);
    state
        .runtime
        .upsert_task(&record)
        .await
        .expect("upsert task");

    let (status, res) = get_json(&app, "/v3/tasks/task_persisted_proof").await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["data"]["status"], "completed");
    assert_eq!(res["data"]["proof"], "0xpersisted-proof");

    let (status, list) = get_json(&app, "/v3/proof/list").await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list.as_array().expect("proof list").len(), 1);
    assert_eq!(list[0]["proof"], "0xpersisted-proof");
}

#[tokio::test]
async fn e2e_risc0_mock_failure_propagates_guest_error_to_status_and_runtime() {
    let mut config = base_config();
    set_prover_routes(&mut config, "risc0/local");

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
    let app = app::build_router_with_legacy_v3_for_tests(state.clone());

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
    assert_eq!(res["data"]["status"], "registered");
    assert!(res["data"].get("task_id").is_none(), "{res}");
    let id = single_report_task_id(&app).await;

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
