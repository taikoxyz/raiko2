#![allow(missing_docs)]

use std::{env, fs, path::PathBuf, str::FromStr};

use alloy_primitives::Address;
use raiko2_primitives_shasta::GuestInput;
use raiko2_primitives_shasta::instance::{
    build_shasta_commitment_from_proof_carry_data_vec, shasta_aggregation_output,
};
use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
use raiko2_prover::remote_prover::{
    adapter::build_shasta_packet,
    protocol::{
        RAIKO2_PROOF_RESPONSE_SCHEMA, Raiko2ProofResponse, Raiko2ShastaAggregateRequest,
        Raiko2ShastaRequest,
    },
};
use reqwest::{Client, Url};

#[test]
fn live_aggregate_request_reuses_proposal_proof_from_same_provider() {
    let (request, _) = proposal_request();
    let response = Raiko2ProofResponse {
        schema: RAIKO2_PROOF_RESPONSE_SCHEMA.to_string(),
        status: raiko2_prover::remote_prover::protocol::Raiko2ProofStatus::Ok,
        result: Some(raiko2_prover::remote_prover::protocol::Raiko2ProofResult {
            proof: Some("0xdeadbeef".to_string()),
            quote: None,
            public_key: Some("0xpub".to_string()),
            instance_address: Some("0xaddr".to_string()),
            input: "0x1234".to_string(),
        }),
        error: None,
    };

    let aggregate = build_live_aggregate_request(&request, &response);

    assert_eq!(aggregate.schema, "raiko2-shasta-aggregate-request-v1");
    assert_eq!(aggregate.payload.proofs.len(), 1);
    assert_eq!(aggregate.payload.proofs[0].proof, "0xdeadbeef");
    assert_eq!(aggregate.payload.proofs[0].input, "0x1234");
    assert_eq!(
        aggregate.payload.proofs[0].proof_carry_data,
        request.payload.guest_input.proof_carry_data
    );
}

#[tokio::test]
#[ignore = "requires RAIKO2_REMOTE_PROVER_BASE_URL"]
async fn remote_prover_conformance_proposal() {
    let base_url = remote_prover_base_url();
    let (request, request_body) = proposal_request();

    let response = post_fixture(&base_url, "/prove/shasta", &request_body).await;

    assert_eq!(response.schema, RAIKO2_PROOF_RESPONSE_SCHEMA);
    assert_eq!(response.status.as_str(), "ok");
    assert!(response.error.is_none());

    let result = response.result.expect("proposal result");
    assert!(result.proof.as_ref().is_some_and(|proof| !proof.is_empty()));
    assert!(
        result
            .public_key
            .as_ref()
            .is_some_and(|public_key| !public_key.is_empty())
    );
    assert!(
        result
            .instance_address
            .as_ref()
            .is_some_and(|instance| !instance.is_empty())
    );
    if let Some(quote) = result.quote.as_ref() {
        assert!(!quote.is_empty());
    }

    let expected_input = hash_shasta_subproof_input(&request.payload.guest_input.proof_carry_data);
    assert_eq!(result.input, format!("{expected_input:#x}"));
}

#[tokio::test]
#[ignore = "requires RAIKO2_REMOTE_PROVER_BASE_URL"]
async fn remote_prover_conformance_aggregate() {
    let base_url = remote_prover_base_url();
    let (proposal_request, proposal_body) = proposal_request();
    let proposal_response = post_fixture(&base_url, "/prove/shasta", &proposal_body).await;
    let request = build_live_aggregate_request(&proposal_request, &proposal_response);
    let request_body = serde_json::to_string(&request).expect("serialize live aggregate request");

    let response = post_fixture(&base_url, "/prove/shasta-aggregate", &request_body).await;

    assert_eq!(response.schema, RAIKO2_PROOF_RESPONSE_SCHEMA);
    assert_eq!(response.status.as_str(), "ok");
    assert!(response.error.is_none());

    let result = response.result.expect("aggregate result");
    assert!(result.proof.as_ref().is_some_and(|proof| !proof.is_empty()));
    assert!(
        result
            .public_key
            .as_ref()
            .is_some_and(|public_key| !public_key.is_empty())
    );
    let instance_address = result
        .instance_address
        .as_ref()
        .filter(|instance| !instance.is_empty())
        .expect("aggregate instance address");
    if let Some(quote) = result.quote.as_ref() {
        assert!(!quote.is_empty());
    }

    let carries = request
        .payload
        .proofs
        .iter()
        .map(|proof| proof.proof_carry_data.clone())
        .collect::<Vec<_>>();
    let commitment =
        build_shasta_commitment_from_proof_carry_data_vec(&carries).expect("aggregate commitment");
    let instance_address = Address::from_str(instance_address).expect("decode instance address");
    let first = carries.first().expect("aggregate fixture carries");
    let expected_input = shasta_aggregation_output(
        &commitment,
        first.chain_id,
        first.verifier,
        instance_address,
    );
    assert_eq!(result.input, format!("{expected_input:#x}"));
}

fn remote_prover_base_url() -> Url {
    let raw = env::var("RAIKO2_REMOTE_PROVER_BASE_URL")
        .expect("set RAIKO2_REMOTE_PROVER_BASE_URL to a provider endpoint");
    Url::parse(&raw).expect("parse remote prover base URL")
}

async fn post_fixture(base_url: &Url, path: &str, body: &str) -> Raiko2ProofResponse {
    let url = base_url.join(path).expect("join conformance path");
    let response = Client::new()
        .post(url)
        .header("content-type", "application/json")
        .body(body.to_owned())
        .send()
        .await
        .expect("post remote prover fixture");

    let status = response.status();
    let body = response.text().await.expect("read remote prover response");
    assert!(
        status.is_success(),
        "remote prover returned {status}: {body}"
    );
    serde_json::from_str(&body).expect("parse remote prover response")
}

fn shared_guest_input_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_23077_l2_9051439_9051630.json",
    )
}

fn proposal_request() -> (Raiko2ShastaRequest, String) {
    let raw = fs::read_to_string(shared_guest_input_path()).expect("read shared guest input");
    let guest_input: GuestInput = serde_json::from_str(&raw).expect("parse shared guest input");
    let request = build_shasta_packet(&guest_input).expect("build proposal request");
    let body = serde_json::to_string(&request).expect("serialize proposal request");
    (request, body)
}

fn build_live_aggregate_request(
    proposal_request: &Raiko2ShastaRequest,
    proposal_response: &Raiko2ProofResponse,
) -> Raiko2ShastaAggregateRequest {
    let result = proposal_response
        .result
        .as_ref()
        .expect("proposal response result");
    let proof = result.proof.as_ref().expect("proposal response proof");

    Raiko2ShastaAggregateRequest {
        schema: "raiko2-shasta-aggregate-request-v1".to_string(),
        payload: raiko2_prover::remote_prover::protocol::Raiko2ShastaAggregatePayload {
            proofs: vec![
                raiko2_prover::remote_prover::protocol::Raiko2AggregateProof {
                    input: result.input.clone(),
                    proof: proof.clone(),
                    proof_carry_data: proposal_request
                        .payload
                        .guest_input
                        .proof_carry_data
                        .clone(),
                },
            ],
        },
    }
}
