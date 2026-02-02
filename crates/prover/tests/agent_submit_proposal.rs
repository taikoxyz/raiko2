#![allow(missing_docs)]

use httpmock::Method::GET;
use httpmock::Method::POST;
use httpmock::MockServer;
use raiko2_pipeline::ProverBackend;
use raiko2_primitives::Proof;
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
    fn elf(&self, _stage: raiko2_pipeline::ProofStage) -> raiko2_primitives::RaikoResult<&'static [u8]> {
        Ok(&[])
    }
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

    let config = AgentConfig {
        base_url: server.url(""),
        prover_type: "boundless".to_string(),
        api_key: None,
        poll_interval_ms: 1,
        timeout_ms: 1000,
    };
    let prover = AgentProver::new(config);

    let proof: Proof = prover
        .prove_encoded(vec![1, 2, 3].into(), &serde_json::Value::Null, &TestBackend)
        .await
        .unwrap();

    assert_eq!(proof.proof.as_deref(), Some("0xaabb"));
    assert_eq!(proof.quote.as_deref(), Some("{\"receipt\":true}"));
}
