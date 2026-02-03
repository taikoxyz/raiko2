#![allow(missing_docs)]

use httpmock::Method::GET;
use httpmock::Method::POST;
use httpmock::MockServer;
use raiko2_pipeline::ProverBackend;
use raiko2_primitives::Proof;
use raiko2_prover::Prover;
use raiko2_prover::agent::{AgentConfig, AgentProver};
use serde_json::json;

fn proof_response_bytes() -> Vec<u8> {
    #[derive(serde::Serialize)]
    struct Risc0Response {
        seal: Vec<u8>,
        journal: Vec<u8>,
        receipt: Option<String>,
    }

    let response = Risc0Response {
        seal: vec![9, 9],
        journal: vec![0xaa, 0xbb],
        receipt: Some("{\"receipt\":true}".to_string()),
    };

    bincode::serialize(&response).unwrap()
}

struct TestBackend;

impl ProverBackend for TestBackend {
    fn elf(
        &self,
        _stage: raiko2_pipeline::ProofStage,
    ) -> raiko2_primitives::RaikoResult<&'static [u8]> {
        Ok(&[])
    }
}

fn prover_for(server: &MockServer) -> AgentProver {
    let config = AgentConfig {
        base_url: server.url(""),
        prover_type: "boundless".to_string(),
        api_key: None,
        poll_interval_ms: 1,
        timeout_ms: 5,
    };
    AgentProver::new(config)
}

#[tokio::test]
async fn agent_submit_proposal_returns_proof() {
    let server = MockServer::start();

    let proof_bytes = proof_response_bytes();

    let _submit = server.mock(|when, then| {
        when.method(POST).path("/proof");
        then.status(200).json_body(json!({
            "request_id": "req_1",
            "prover_type": "boundless",
            "status": "preparing",
            "message": "ok"
        }));
    });

    let _status = server.mock(|when, then| {
        when.method(GET).path("/status/req_1");
        then.status(200).json_body(json!({
            "request_id": "req_1",
            "prover_type": "boundless",
            "status": "fulfilled",
            "status_message": "done",
            "proof_data": proof_bytes
        }));
    });

    let prover = prover_for(&server);

    let proof: Proof = prover
        .prove_encoded(vec![1, 2, 3].into(), &serde_json::Value::Null, &TestBackend)
        .await
        .unwrap();

    assert_eq!(proof.proof.as_deref(), Some("0xaabb"));
    assert_eq!(proof.quote.as_deref(), Some("{\"receipt\":true}"));
}

#[tokio::test]
async fn agent_submit_proposal_returns_failed_status_error() {
    let server = MockServer::start();

    let _submit = server.mock(|when, then| {
        when.method(POST).path("/proof");
        then.status(200).json_body(json!({
            "request_id": "req_fail",
            "prover_type": "boundless",
            "status": "preparing",
            "message": "ok"
        }));
    });

    let _status = server.mock(|when, then| {
        when.method(GET).path("/status/req_fail");
        then.status(200).json_body(json!({
            "request_id": "req_fail",
            "prover_type": "boundless",
            "status": "failed",
            "status_message": "boom",
            "error": "boom"
        }));
    });

    let prover = prover_for(&server);
    let err = prover
        .prove_encoded(vec![1].into(), &serde_json::Value::Null, &TestBackend)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("boom"));
}

#[tokio::test]
async fn agent_submit_proposal_times_out() {
    let server = MockServer::start();

    let _submit = server.mock(|when, then| {
        when.method(POST).path("/proof");
        then.status(200).json_body(json!({
            "request_id": "req_timeout",
            "prover_type": "boundless",
            "status": "preparing",
            "message": "ok"
        }));
    });

    let _status = server.mock(|when, then| {
        when.method(GET).path("/status/req_timeout");
        then.status(200).json_body(json!({
            "request_id": "req_timeout",
            "prover_type": "boundless",
            "status": "running",
            "status_message": "wait"
        }));
    });

    let prover = prover_for(&server);
    let err = prover
        .prove_encoded(vec![1].into(), &serde_json::Value::Null, &TestBackend)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("timed out"));
}

#[tokio::test]
async fn agent_submit_proposal_missing_proof_data() {
    let server = MockServer::start();

    let _submit = server.mock(|when, then| {
        when.method(POST).path("/proof");
        then.status(200).json_body(json!({
            "request_id": "req_missing",
            "prover_type": "boundless",
            "status": "preparing",
            "message": "ok"
        }));
    });

    let _status = server.mock(|when, then| {
        when.method(GET).path("/status/req_missing");
        then.status(200).json_body(json!({
            "request_id": "req_missing",
            "prover_type": "boundless",
            "status": "fulfilled",
            "status_message": "done",
            "proof_data": null
        }));
    });

    let prover = prover_for(&server);
    let err = prover
        .prove_encoded(vec![1].into(), &serde_json::Value::Null, &TestBackend)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("Missing proof_data"));
}

#[tokio::test]
async fn agent_submit_proposal_submission_error() {
    let server = MockServer::start();

    let _submit = server.mock(|when, then| {
        when.method(POST).path("/proof");
        then.status(500).body("oops");
    });

    let prover = prover_for(&server);
    let err = prover
        .prove_encoded(vec![1].into(), &serde_json::Value::Null, &TestBackend)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("Failed to parse proof response"));
}

#[tokio::test]
async fn agent_submit_proposal_polling_error() {
    let server = MockServer::start();

    let _submit = server.mock(|when, then| {
        when.method(POST).path("/proof");
        then.status(200).json_body(json!({
            "request_id": "req_poll",
            "prover_type": "boundless",
            "status": "preparing",
            "message": "ok"
        }));
    });

    let _status = server.mock(|when, then| {
        when.method(GET).path("/status/req_poll");
        then.status(500).body("oops");
    });

    let prover = prover_for(&server);
    let err = prover
        .prove_encoded(vec![1].into(), &serde_json::Value::Null, &TestBackend)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("Failed to parse status response"));
}
