use std::{str::FromStr, time::Duration};

use alloy_primitives::{B256, Bytes};
use reqwest::{
    Client, Url,
    header::{CONTENT_TYPE, HeaderValue},
};
use serde_json::{Map, Value};

use raiko2_pipeline::ProverBackend;
use raiko2_primitives::{AggregationGuestInput, Proof, ProverConfig, RaikoError, RaikoResult};
use raiko2_primitives_shasta::GuestInput;

use crate::{GuestInputCodec, Prover, with_shasta_extra_data};

use self::{
    adapter::build_shasta_packet,
    protocol::{
        GAIKO2_PROOF_RESPONSE_SCHEMA, GAIKO2_SHASTA_REQUEST_SCHEMA, Gaiko2ProofResponse,
        Gaiko2ProofStatus, Gaiko2ShastaRequest,
    },
};

pub mod adapter;
pub mod protocol;

const SHASTA_PROPOSAL_PATH: &str = "/internal/prove/shasta-proposal";

#[derive(Debug, Clone, Default)]
pub struct Gaiko2Config {
    pub base_url: String,
    pub timeout_ms: u64,
}

impl Gaiko2Config {
    #[must_use]
    pub const fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms)
    }
}

#[derive(Clone)]
pub struct Gaiko2Prover {
    client: Client,
    prove_url: Url,
}

impl Gaiko2Prover {
    pub fn new(config: Gaiko2Config) -> RaikoResult<Self> {
        if config.base_url.trim().is_empty() {
            return Err(RaikoError::InvalidRequestConfig(
                "gaiko2.base_url must not be empty".to_string(),
            ));
        }

        let base_url = Url::parse(&config.base_url).map_err(|err| {
            RaikoError::InvalidRequestConfig(format!("invalid gaiko2.base_url: {err}"))
        })?;
        let prove_url = base_url.join(SHASTA_PROPOSAL_PATH).map_err(|err| {
            RaikoError::InvalidRequestConfig(format!("invalid gaiko2 prove URL: {err}"))
        })?;
        let client = Client::builder()
            .timeout(config.timeout())
            .build()
            .map_err(|err| {
                RaikoError::InvalidRequestConfig(format!("failed to build gaiko2 client: {err}"))
            })?;

        Ok(Self { client, prove_url })
    }
}

impl GuestInputCodec<GuestInput> for Gaiko2Prover {
    fn encode(&self, input: &GuestInput, _config: &ProverConfig) -> RaikoResult<Bytes> {
        let packet = build_shasta_packet(input)?;
        let payload = serde_json::to_vec(&packet)
            .map_err(|err| RaikoError::Guest(format!("failed to encode gaiko2 packet: {err}")))?;
        Ok(Bytes::from(payload))
    }
}

#[async_trait::async_trait]
impl<B> Prover<B> for Gaiko2Prover
where
    B: ProverBackend,
{
    type GuestInput = GuestInput;

    fn encode(&self, input: &Self::GuestInput, config: &ProverConfig) -> RaikoResult<Bytes> {
        GuestInputCodec::encode(self, input, config)
    }

    async fn prove_encoded(
        &self,
        input: Bytes,
        _config: &ProverConfig,
        _backend: &B,
    ) -> RaikoResult<Proof> {
        let packet: Gaiko2ShastaRequest = serde_json::from_slice(input.as_ref())
            .map_err(|err| RaikoError::Guest(format!("failed to decode gaiko2 packet: {err}")))?;
        if packet.schema != GAIKO2_SHASTA_REQUEST_SCHEMA {
            return Err(RaikoError::Guest(format!(
                "unsupported gaiko2 request schema: {}",
                packet.schema
            )));
        }

        let response = self
            .client
            .post(self.prove_url.clone())
            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .body(input.to_vec())
            .send()
            .await
            .map_err(|err| RaikoError::Guest(format!("gaiko2 request failed: {err}")))?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|err| RaikoError::Guest(format!("gaiko2 read failed: {err}")))?;
        let envelope: Gaiko2ProofResponse = serde_json::from_slice(&body).map_err(|err| {
            RaikoError::Guest(format!(
                "gaiko2 response decode failed (status {}): {err}",
                status
            ))
        })?;

        if envelope.schema != GAIKO2_PROOF_RESPONSE_SCHEMA {
            return Err(RaikoError::Guest(format!(
                "unsupported gaiko2 response schema: {}",
                envelope.schema
            )));
        }

        if !status.is_success() || envelope.status == Gaiko2ProofStatus::Error {
            let error = envelope.error.ok_or_else(|| {
                RaikoError::Guest(format!(
                    "gaiko2 request failed with status {} and no error payload",
                    status
                ))
            })?;
            return Err(RaikoError::Guest(format!(
                "gaiko2 {}: {}",
                error.code, error.message
            )));
        }

        let result = envelope.result.ok_or_else(|| {
            RaikoError::Guest("gaiko2 response missing result payload".to_string())
        })?;
        let input_hash = B256::from_str(&result.input).map_err(|err| {
            RaikoError::Guest(format!(
                "invalid gaiko2 input hash '{}': {err}",
                result.input
            ))
        })?;

        let extra_data = with_shasta_extra_data(
            &packet.payload.proof_carry_data,
            "gaiko2",
            Some(gaiko2_metadata(&envelope.schema, &result)),
        )?;

        Ok(Proof {
            proof: result.proof,
            input: Some(input_hash),
            quote: result.quote,
            extra_data,
            ..Default::default()
        })
    }

    async fn aggregate(
        &self,
        _input: AggregationGuestInput,
        _config: &ProverConfig,
        _backend: &B,
    ) -> RaikoResult<Proof> {
        Err(RaikoError::InvalidRequestConfig(
            "gaiko2 aggregation is not supported".to_string(),
        ))
    }
}

fn gaiko2_metadata(
    response_schema: &str,
    result: &protocol::Gaiko2ProofResult,
) -> serde_json::Value {
    let mut metadata = Map::from_iter([(
        "schema".to_string(),
        Value::String(response_schema.to_string()),
    )]);
    if let Some(public_key) = result.public_key.clone() {
        metadata.insert("public_key".to_string(), Value::String(public_key));
    }
    if let Some(instance_address) = result.instance_address.clone() {
        metadata.insert(
            "instance_address".to_string(),
            Value::String(instance_address),
        );
    }
    Value::Object(metadata)
}
