use std::{str::FromStr, time::Duration};

use alloy_primitives::{Address, B256, Bytes};
use reqwest::{
    Client, Url,
    header::{CONTENT_TYPE, HeaderValue},
};
use serde_json::{Map, Value};

use raiko2_pipeline::ProverBackend;
use raiko2_primitives::{
    AggregationGuestInput, Proof, ProofType, ProverConfig, RaikoError, RaikoResult,
};
use raiko2_primitives_shasta::GuestInput;
use raiko2_protocol_shasta::shasta::ProofCarryData;

use crate::{
    GuestInputCodec, Prover, ensure_shasta_aggregate_input_matches_carries,
    ensure_shasta_proposal_input_matches_carry, with_shasta_extra_data,
};

use crate::remote_prover::{
    adapter::{build_shasta_aggregate_request, build_shasta_packet},
    protocol::{
        RAIKO2_PROOF_RESPONSE_SCHEMA, RAIKO2_SHASTA_AGGREGATE_REQUEST_SCHEMA,
        RAIKO2_SHASTA_REQUEST_SCHEMA, Raiko2ProofResponse, Raiko2ProofResult, Raiko2ProofStatus,
        Raiko2ShastaRequest,
    },
};

const SHASTA_PROPOSAL_PATH: &str = "/prove/shasta";
const SHASTA_AGGREGATE_PATH: &str = "/prove/shasta-aggregate";
const SGX_LABEL: &str = "sgx";
const SGXGETH_LABEL: &str = "sgxgeth";

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
    label: &'static str,
}

impl Gaiko2Prover {
    /// # Errors
    ///
    /// Returns an error when the gaiko2 base URL is empty, malformed, or the HTTP client
    /// cannot be constructed.
    pub fn new(config: &Gaiko2Config) -> RaikoResult<Self> {
        Self::new_with_label(config, SGXGETH_LABEL)
    }

    /// # Errors
    ///
    /// Returns an error when `proof_type` is not backed by the gaiko2 remote SGX adapter, or
    /// when the gaiko2 base URL is empty, malformed, or the HTTP client cannot be constructed.
    pub fn new_for_proof_type(config: &Gaiko2Config, proof_type: ProofType) -> RaikoResult<Self> {
        let label = match proof_type {
            ProofType::Sgx => SGX_LABEL,
            ProofType::SgxGeth => SGXGETH_LABEL,
            _ => {
                return Err(RaikoError::InvalidRequestConfig(format!(
                    "gaiko2 prover does not support proof type {proof_type}"
                )));
            }
        };

        Self::new_with_label(config, label)
    }

    fn new_with_label(config: &Gaiko2Config, label: &'static str) -> RaikoResult<Self> {
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
            label,
        })
    }
}

impl GuestInputCodec<GuestInput> for Gaiko2Prover {
    fn encode(&self, input: &GuestInput, _config: &ProverConfig) -> RaikoResult<Bytes> {
        let payload = serde_json::to_vec(&build_shasta_packet(input)?).map_err(|err| {
            RaikoError::Guest(format!("failed to encode {} packet: {err}", self.label))
        })?;
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
        let proof_carry_data = self.proposal_carry_data_from_encoded(&input)?;

        let (envelope, result) = self
            .post_request(self.prove_url.clone(), input.to_vec())
            .await?;
        let input_hash = B256::from_str(&result.input).map_err(|err| {
            RaikoError::Guest(format!(
                "invalid {} input hash '{}': {err}",
                self.label, result.input
            ))
        })?;
        ensure_shasta_proposal_input_matches_carry(input_hash, &proof_carry_data, self.label)?;

        let extra_data = with_shasta_extra_data(
            &proof_carry_data,
            self.label,
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
            RaikoError::Guest(format!(
                "failed to encode {} aggregate packet: {err}",
                self.label
            ))
        })?;
        let (envelope, result) = self
            .post_request(self.aggregate_url.clone(), payload)
            .await?;
        let input_hash = B256::from_str(&result.input).map_err(|err| {
            RaikoError::Guest(format!(
                "invalid {} input hash '{}': {err}",
                self.label, result.input
            ))
        })?;
        let instance_address = parse_aggregate_instance_address(&result, self.label)?;
        let carries = packet
            .payload
            .proofs
            .iter()
            .map(|proof| proof.proof_carry_data.clone())
            .collect::<Vec<_>>();
        ensure_shasta_aggregate_input_matches_carries(
            input_hash,
            &carries,
            instance_address,
            self.label,
        )?;
        let metadata = gaiko2_metadata(&envelope.schema, &result);
        let mut extra_data = Map::new();
        extra_data.insert(self.label.to_string(), metadata);

        Ok(Proof {
            proof: result.proof,
            input: Some(input_hash),
            quote: result.quote,
            extra_data: Some(Value::Object(extra_data)),
            ..Default::default()
        })
    }
}

impl Gaiko2Prover {
    fn proposal_carry_data_from_encoded(&self, input: &Bytes) -> RaikoResult<ProofCarryData> {
        let packet: Raiko2ShastaRequest =
            serde_json::from_slice(input.as_ref()).map_err(|err| {
                RaikoError::Guest(format!("failed to decode {} packet: {err}", self.label))
            })?;
        if packet.schema != RAIKO2_SHASTA_REQUEST_SCHEMA {
            return Err(RaikoError::Guest(format!(
                "unsupported remote prover request schema: {}",
                packet.schema
            )));
        }
        Ok(packet.payload.guest_input.proof_carry_data)
    }

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
            .map_err(|err| RaikoError::Guest(format!("{} request failed: {err}", self.label)))?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|err| RaikoError::Guest(format!("{} read failed: {err}", self.label)))?;
        let envelope: Raiko2ProofResponse = serde_json::from_slice(&body).map_err(|err| {
            RaikoError::Guest(format!(
                "{} response decode failed (status {status}): {err}",
                self.label
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
                    "{} request failed with status {status} and no error payload",
                    self.label
                ))
            })?;
            return Err(RaikoError::Guest(format!(
                "{} {}: {}",
                self.label, error.code, error.message
            )));
        }

        let result = envelope.result.clone().ok_or_else(|| {
            RaikoError::Guest(format!("{} response missing result payload", self.label))
        })?;

        Ok((envelope, result))
    }
}

fn parse_aggregate_instance_address(
    result: &Raiko2ProofResult,
    label: &str,
) -> RaikoResult<Address> {
    let raw = result.instance_address.as_deref().ok_or_else(|| {
        RaikoError::Guest(format!(
            "{label} aggregate response missing instance_address"
        ))
    })?;

    Address::from_str(raw).map_err(|err| {
        RaikoError::Guest(format!(
            "invalid {label} aggregate instance_address '{raw}': {err}"
        ))
    })
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
