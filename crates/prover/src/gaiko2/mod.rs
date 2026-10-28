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
/// SGX provider health endpoint.
const SGX_HEALTH_PATH: &str = "/health";
/// SGXGETH (gaiko2) provider health endpoint; the provider registers only `/healthz`.
const SGXGETH_HEALTH_PATH: &str = "/healthz";
const SGX_LABEL: &str = "sgx";
const SGXGETH_LABEL: &str = "sgxgeth";
/// Upper bound on a readiness health probe, independent of the (much larger) proving timeout.
const HEALTH_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

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
    health_url: Url,
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
        let health_path = if label == SGX_LABEL {
            SGX_HEALTH_PATH
        } else {
            SGXGETH_HEALTH_PATH
        };
        let health_url = base_url.join(health_path).map_err(|err| {
            RaikoError::InvalidRequestConfig(format!("invalid gaiko2 health URL: {err}"))
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
            health_url,
            label,
        })
    }

    /// Probe the remote TEE prover's health endpoint (`/health` for SGX, `/healthz` for SGXGETH).
    ///
    /// Used by the host readiness check to confirm the SGX/SGXGETH accelerator backend is
    /// reachable and responsive. Uses a short, fixed timeout independent of the proving timeout
    /// so a hung backend fails readiness quickly.
    ///
    /// # Errors
    ///
    /// Returns an error if the request cannot be sent or the backend responds with a non-success
    /// status code.
    pub async fn health_check(&self) -> RaikoResult<()> {
        let response = self
            .client
            .get(self.health_url.clone())
            .timeout(HEALTH_PROBE_TIMEOUT)
            .send()
            .await
            .map_err(|err| {
                RaikoError::Guest(format!("{} health probe failed: {err}", self.label))
            })?;
        let status = response.status();
        if !status.is_success() {
            return Err(RaikoError::Guest(format!(
                "{} health probe returned status {status}",
                self.label
            )));
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::{Gaiko2Config, Gaiko2Prover};
    use raiko2_primitives::ProofType;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Spawn a one-shot HTTP server that answers `200 OK` only when the request targets
    /// `expected_path` (otherwise `404`), or `status_line` when the path matches.
    async fn spawn_health_server(status_line: &'static str, expected_path: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind health mock");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let read = socket.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..read]);
                let status_line = if request.starts_with(&format!("GET {expected_path} ")) {
                    status_line
                } else {
                    "404 Not Found"
                };
                let response = format!(
                    "HTTP/1.1 {status_line}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
            }
        });
        format!("http://{addr}")
    }

    fn prover_for(base_url: String, proof_type: ProofType) -> Gaiko2Prover {
        Gaiko2Prover::new_for_proof_type(
            &Gaiko2Config {
                base_url,
                timeout_ms: 300_000,
            },
            proof_type,
        )
        .expect("build gaiko2 prover")
    }

    #[tokio::test]
    async fn health_check_uses_health_for_sgx() {
        let base_url = spawn_health_server("200 OK", "/health").await;
        prover_for(base_url, ProofType::Sgx)
            .health_check()
            .await
            .expect("healthy sgx backend must pass the probe");
    }

    #[tokio::test]
    async fn health_check_uses_healthz_for_sgxgeth() {
        let base_url = spawn_health_server("200 OK", "/healthz").await;
        prover_for(base_url, ProofType::SgxGeth)
            .health_check()
            .await
            .expect("healthy sgxgeth backend must pass the probe");
    }

    #[tokio::test]
    async fn health_check_fails_when_path_is_not_served() {
        let base_url = spawn_health_server("200 OK", "/healthz").await;
        let err = prover_for(base_url, ProofType::Sgx)
            .health_check()
            .await
            .expect_err("sgx must not probe the sgxgeth path");
        assert!(err.to_string().contains("404"), "{err}");
    }

    #[tokio::test]
    async fn health_check_fails_on_error_status() {
        let base_url = spawn_health_server("503 Service Unavailable", "/health").await;
        let err = prover_for(base_url, ProofType::Sgx)
            .health_check()
            .await
            .expect_err("unhealthy backend must fail the probe");
        assert!(
            err.to_string().contains("503"),
            "error should surface the backend status: {err}"
        );
    }
}
