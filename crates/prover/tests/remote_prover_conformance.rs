#![allow(missing_docs)]

use std::{env, fs, path::PathBuf, str::FromStr};

use alloy_primitives::Address;
use raiko2_primitives_shasta::instance::{
    build_shasta_commitment_from_proof_carry_data_vec, shasta_aggregation_output,
};
use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
use raiko2_prover::remote_prover::protocol::{
    RAIKO2_PROOF_RESPONSE_SCHEMA, Raiko2ProofResponse, Raiko2ShastaAggregateRequest,
    Raiko2ShastaRequest,
};
use reqwest::{Client, Url};

#[tokio::test]
#[ignore = "requires RAIKO2_REMOTE_PROVER_BASE_URL"]
async fn remote_prover_conformance_proposal() {
    let base_url = remote_prover_base_url();
    let request_body = fs::read_to_string(proposal_fixture_path()).expect("read proposal fixture");
    let request: Raiko2ShastaRequest =
        serde_json::from_str(&request_body).expect("parse proposal fixture");

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

    let expected_input = hash_shasta_subproof_input(&request.payload.proof_carry_data);
    assert_eq!(result.input, format!("{expected_input:#x}"));
}

#[tokio::test]
#[ignore = "requires RAIKO2_REMOTE_PROVER_BASE_URL"]
async fn remote_prover_conformance_aggregate() {
    let base_url = remote_prover_base_url();
    let request_body =
        fs::read_to_string(aggregate_fixture_path()).expect("read aggregate fixture");
    let request: Raiko2ShastaAggregateRequest =
        serde_json::from_str(&request_body).expect("parse aggregate fixture");

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

fn proposal_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../tests/fixtures/remote_prover/shasta_request_v1_taiko_mainnet_proposal_2222_l2_5412225_5412416.json",
    )
}

fn aggregate_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../tests/fixtures/remote_prover/shasta_aggregate_request_v1_single_fixture_proof.json",
    )
}
