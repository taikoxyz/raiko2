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

use crate::remote_prover::{
    adapter::{
        build_shasta_aggregate_request, build_shasta_packet, build_shasta_packet_with_guest_input,
    },
    protocol::{
        RAIKO2_PROOF_RESPONSE_SCHEMA, RAIKO2_SHASTA_AGGREGATE_REQUEST_SCHEMA,
        RAIKO2_SHASTA_REQUEST_SCHEMA, Raiko2ProofResponse, Raiko2ProofResult, Raiko2ProofStatus,
        Raiko2ShastaRequest,
    },
};

pub mod adapter;
pub mod protocol;

const SHASTA_PROPOSAL_PATH: &str = "/prove/shasta";
const SHASTA_AGGREGATE_PATH: &str = "/prove/shasta-aggregate";

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
    aggregate_url: Url,
    request_encoding: ShastaRequestEncoding,
}

#[derive(Clone, Copy)]
enum ShastaRequestEncoding {
    ReplayPacket,
    GuestInput,
}

impl Gaiko2Prover {
    /// # Errors
    ///
    /// Returns an error when the gaiko2 base URL is empty, malformed, or the HTTP client
    /// cannot be constructed.
    pub fn new(config: &Gaiko2Config) -> RaikoResult<Self> {
        Self::new_with_encoding(config, ShastaRequestEncoding::ReplayPacket)
    }

    /// # Errors
    ///
    /// Returns an error when the remote SGX base URL is empty, malformed, or the HTTP client
    /// cannot be constructed.
    pub fn new_for_guest_input(config: &Gaiko2Config) -> RaikoResult<Self> {
        Self::new_with_encoding(config, ShastaRequestEncoding::GuestInput)
    }

    fn new_with_encoding(
        config: &Gaiko2Config,
        request_encoding: ShastaRequestEncoding,
    ) -> RaikoResult<Self> {
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
        let aggregate_url = base_url.join(SHASTA_AGGREGATE_PATH).map_err(|err| {
            RaikoError::InvalidRequestConfig(format!("invalid gaiko2 aggregate URL: {err}"))
        })?;
        let client = Client::builder()
            .timeout(config.timeout())
            .build()
            .map_err(|err| {
                RaikoError::InvalidRequestConfig(format!("failed to build gaiko2 client: {err}"))
            })?;

        Ok(Self {
            client,
            prove_url,
            aggregate_url,
            request_encoding,
        })
    }
}

impl GuestInputCodec<GuestInput> for Gaiko2Prover {
    fn encode(&self, input: &GuestInput, _config: &ProverConfig) -> RaikoResult<Bytes> {
        let packet = match self.request_encoding {
            ShastaRequestEncoding::ReplayPacket => build_shasta_packet(input)?,
            ShastaRequestEncoding::GuestInput => build_shasta_packet_with_guest_input(input)?,
        };
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
        let packet: Raiko2ShastaRequest = serde_json::from_slice(input.as_ref())
            .map_err(|err| RaikoError::Guest(format!("failed to decode gaiko2 packet: {err}")))?;
        if packet.schema != RAIKO2_SHASTA_REQUEST_SCHEMA {
            return Err(RaikoError::Guest(format!(
                "unsupported remote prover request schema: {}",
                packet.schema
            )));
        }

        let (envelope, result) = self
            .post_request(self.prove_url.clone(), input.to_vec())
            .await?;
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
        input: AggregationGuestInput,
        _config: &ProverConfig,
        _backend: &B,
    ) -> RaikoResult<Proof> {
        let packet = build_shasta_aggregate_request(&input.proofs)?;
        if packet.schema != RAIKO2_SHASTA_AGGREGATE_REQUEST_SCHEMA {
            return Err(RaikoError::Guest(format!(
                "unsupported remote prover request schema: {}",
                packet.schema
            )));
        }

        let payload = serde_json::to_vec(&packet).map_err(|err| {
            RaikoError::Guest(format!("failed to encode gaiko2 aggregate packet: {err}"))
        })?;
        let (envelope, result) = self
            .post_request(self.aggregate_url.clone(), payload)
            .await?;
        let input_hash = B256::from_str(&result.input).map_err(|err| {
            RaikoError::Guest(format!(
                "invalid gaiko2 input hash '{}': {err}",
                result.input
            ))
        })?;
        let metadata = gaiko2_metadata(&envelope.schema, &result);

        Ok(Proof {
            proof: result.proof,
            input: Some(input_hash),
            quote: result.quote,
            extra_data: Some(serde_json::json!({
                "gaiko2": metadata,
            })),
            ..Default::default()
        })
    }
}

impl Gaiko2Prover {
    async fn post_request(
        &self,
        url: Url,
        body: Vec<u8>,
    ) -> RaikoResult<(Raiko2ProofResponse, Raiko2ProofResult)> {
        let response = self
            .client
            .post(url)
            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .body(body)
            .send()
            .await
            .map_err(|err| RaikoError::Guest(format!("gaiko2 request failed: {err}")))?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|err| RaikoError::Guest(format!("gaiko2 read failed: {err}")))?;
        let envelope: Raiko2ProofResponse = serde_json::from_slice(&body).map_err(|err| {
            RaikoError::Guest(format!(
                "gaiko2 response decode failed (status {status}): {err}"
            ))
        })?;

        if envelope.schema != RAIKO2_PROOF_RESPONSE_SCHEMA {
            return Err(RaikoError::Guest(format!(
                "unsupported remote prover response schema: {}",
                envelope.schema
            )));
        }

        if !status.is_success() || envelope.status == Raiko2ProofStatus::Error {
            let error = envelope.error.as_ref().ok_or_else(|| {
                RaikoError::Guest(format!(
                    "gaiko2 request failed with status {status} and no error payload"
                ))
            })?;
            return Err(RaikoError::Guest(format!(
                "gaiko2 {}: {}",
                error.code, error.message
            )));
        }

        let result = envelope.result.clone().ok_or_else(|| {
            RaikoError::Guest("gaiko2 response missing result payload".to_string())
        })?;

        Ok((envelope, result))
    }
}

fn gaiko2_metadata(
    response_schema: &str,
    result: &crate::remote_prover::protocol::Raiko2ProofResult,
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
