#![allow(missing_docs)]

pub mod aggregation;
pub mod types;

use crate::agent::types::{AsyncProofRequestData, AsyncProofResponse, StatusResponse};
use alloy_primitives::hex::encode_prefixed;
use alloy_primitives::Bytes;
use raiko2_pipeline::ProverBackend;
use raiko2_primitives::proof::{
    AggregationInput, ProofEnvelope, ProofPayload, PublicInputs, VerifierArtifact,
};
use raiko2_primitives::{Proof, ProverConfig, RaikoError, RaikoResult};
use raiko2_primitives_shasta::GuestInput;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct AgentConfig {
    pub base_url: String,
    pub prover_type: String,
    pub api_key: Option<String>,
    pub poll_interval_ms: u64,
    pub timeout_ms: u64,
}

#[derive(Clone)]
pub struct AgentClient {
    config: AgentConfig,
    http: Client,
}

impl AgentClient {
    #[must_use]
    pub fn new(config: AgentConfig) -> Self {
        Self {
            config,
            http: Client::new(),
        }
    }

    /// Submit a proof request to the agent.
    ///
    /// # Errors
    /// Returns an error if the request cannot be sent or the response cannot be parsed.
    pub async fn submit_proof(
        &self,
        request: &AsyncProofRequestData,
    ) -> RaikoResult<AsyncProofResponse> {
        let url = format!("{}/proof", self.config.base_url.trim_end_matches('/'));
        let mut builder = self.http.post(url).json(request);
        if let Some(key) = &self.config.api_key {
            builder = builder.header("x-api-key", key);
        }
        let response = builder.send().await.map_err(|e| {
            RaikoError::InvalidRequestConfig(format!("Failed to submit proof: {e}"))
        })?;
        response.json::<AsyncProofResponse>().await.map_err(|e| {
            RaikoError::InvalidRequestConfig(format!("Failed to parse proof response: {e}"))
        })
    }

    /// Poll the agent for the status of a request.
    ///
    /// # Errors
    /// Returns an error if the request cannot be sent or the response cannot be parsed.
    pub async fn poll_status(&self, request_id: &str) -> RaikoResult<StatusResponse> {
        let url = format!(
            "{}/status/{}",
            self.config.base_url.trim_end_matches('/'),
            request_id
        );
        let mut builder = self.http.get(url);
        if let Some(key) = &self.config.api_key {
            builder = builder.header("x-api-key", key);
        }
        let response = builder.send().await.map_err(|e| {
            RaikoError::InvalidRequestConfig(format!("Failed to poll status: {e}"))
        })?;
        response.json::<StatusResponse>().await.map_err(|e| {
            RaikoError::InvalidRequestConfig(format!("Failed to parse status response: {e}"))
        })
    }
}

#[derive(Clone)]
pub struct AgentProver {
    client: AgentClient,
}

impl AgentProver {
    #[must_use]
    pub fn new(config: AgentConfig) -> Self {
        Self {
            client: AgentClient::new(config),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Risc0Response {
    seal: Vec<u8>,
    journal: Vec<u8>,
    receipt: Option<String>,
}

#[async_trait::async_trait]
impl<B> crate::Prover<B> for AgentProver
where
    B: ProverBackend,
{
    type GuestInput = GuestInput;

    fn encode(&self, input: &Self::GuestInput, _config: &ProverConfig) -> RaikoResult<Bytes> {
        let bytes = bincode::serialize(input)
            .map_err(|e| RaikoError::InvalidRequestConfig(format!("Encode failed: {e}")))?;
        Ok(Bytes::from(bytes))
    }

    async fn prove_encoded(
        &self,
        input: Bytes,
        _config: &ProverConfig,
        _backend: &B,
    ) -> RaikoResult<Proof> {
        let request = AsyncProofRequestData::new(
            &self.client.config.prover_type,
            "batch",
            input.to_vec(),
            Vec::new(),
        );
        let proof_data = self.submit_and_wait(&request).await?;
        decode_risc0_proof(&proof_data)
    }

    async fn aggregate(
        &self,
        input: raiko2_primitives::AggregationGuestInput,
        _config: &ProverConfig,
        _backend: &B,
    ) -> RaikoResult<Proof> {
        let agg = AggregationInput {
            proofs: input
                .proofs
                .into_iter()
                .map(proof_to_envelope)
                .collect(),
            expected_image_id: None,
            metadata: None,
        };

        let bytes = aggregation::build_risc0_aggregation_input(&agg)?;
        let request = AsyncProofRequestData::new(
            &self.client.config.prover_type,
            "aggregate",
            bytes,
            Vec::new(),
        );
        let proof_data = self.submit_and_wait(&request).await?;
        decode_risc0_proof(&proof_data)
    }
}

impl AgentProver {
    async fn submit_and_wait(&self, request: &AsyncProofRequestData) -> RaikoResult<Vec<u8>> {
        let response = self.client.submit_proof(request).await?;
        let start = Instant::now();
        let poll_interval = Duration::from_millis(self.client.config.poll_interval_ms);
        let timeout = Duration::from_millis(self.client.config.timeout_ms);

        loop {
            let status = self.client.poll_status(&response.request_id).await?;
            let status_lower = status.status.to_lowercase();
            if status_lower == "fulfilled" {
                return status.proof_data.ok_or_else(|| {
                    RaikoError::InvalidRequestConfig("Missing proof_data".to_string())
                });
            }
            if status_lower == "failed" {
                return Err(RaikoError::InvalidRequestConfig(
                    status.error.unwrap_or_else(|| "Agent failed".to_string()),
                ));
            }
            if start.elapsed() > timeout {
                return Err(RaikoError::InvalidRequestConfig(
                    "Agent polling timed out".to_string(),
                ));
            }
            tokio::time::sleep(poll_interval).await;
        }
    }
}

fn decode_risc0_proof(proof_data: &[u8]) -> RaikoResult<Proof> {
    let decoded: Risc0Response = bincode::deserialize(proof_data).map_err(|e| {
        RaikoError::InvalidRequestConfig(format!("Failed to decode proof: {e}"))
    })?;
    Ok(Proof {
        proof: Some(encode_prefixed(decoded.journal)),
        quote: decoded.receipt,
        input: None,
        uuid: None,
        kzg_proof: None,
        extra_data: None,
    })
}

fn proof_to_envelope(proof: Proof) -> ProofEnvelope {
    let mut artifacts = Vec::new();
    if let Some(receipt) = proof.quote {
        artifacts.push(VerifierArtifact {
            kind: "receipt_json".to_string(),
            value: serde_json::Value::String(receipt),
        });
    }

    ProofEnvelope {
        backend: "risc0".to_string(),
        public_inputs: PublicInputs {
            input_hash: proof.input.map(|value| encode_prefixed(value.as_slice())),
            instance_hash: None,
        },
        payload: ProofPayload {
            payload_kind: "risc0_journal".to_string(),
            bytes: Vec::new(),
        },
        verifier_artifacts: artifacts,
        carry_data: None,
        metadata: None,
    }
}
