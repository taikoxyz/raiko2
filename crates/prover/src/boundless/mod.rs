#![allow(missing_docs)]

pub mod aggregation;

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use alloy_primitives::{
    Bytes, U256,
    utils::{parse_ether, parse_units},
};
use alloy_signer_local::PrivateKeySigner;
use boundless_market::{
    Client, ProofRequest,
    contracts::RequestStatus,
    deployments::{BASE, Deployment, SEPOLIA},
    input::GuestEnv,
    request_builder::OfferParams,
};
use raiko2_pipeline::{ProofStage, ProverBackend};
use raiko2_primitives::proof::{
    AggregationInput, ProofEnvelope, ProofPayload, PublicInputs, VerifierArtifact,
};
use raiko2_primitives::{AggregationGuestInput, Proof, ProverConfig, RaikoError, RaikoResult};
use raiko2_primitives_shasta::GuestInput;
use risc0_ethereum_contracts_boundless::receipt::{Receipt as ContractReceipt, decode_seal};
use risc0_zkvm::{Digest, Journal, compute_image_id, local_executor};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use url::Url;

use crate::{
    BoundlessSubmissionProgress, ProverProgress, ProverProgressObserver, RISC0_SEAL_PAYLOAD_KIND,
    decode_hex_payload, parse_shasta_aggregation_input_hash, parse_shasta_proposal_input_hash,
};

const MILLION_CYCLES: u64 = 1_000_000;
const STAKE_TOKEN_DECIMALS: u8 = 18;
const BATCH_QUOTED_MCYCLES_MIN: u32 = 2_000;
const BATCH_QUOTED_MCYCLES_STEP: u32 = 1_000;
const AGGREGATION_QUOTED_MCYCLES: u32 = 200;

fn user_cycles_to_mcycles(user_cycles: u64) -> u32 {
    let mcycles = user_cycles.div_ceil(MILLION_CYCLES);
    u32::try_from(mcycles).unwrap_or(u32::MAX)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DeploymentType {
    Sepolia,
    Base,
}

impl FromStr for DeploymentType {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_lowercase().as_str() {
            "sepolia" => Ok(Self::Sepolia),
            "base" => Ok(Self::Base),
            _ => Err(format!("Invalid boundless deployment_type: {value}")),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BoundlessOfferParams {
    pub ramp_up_start_sec: u32,
    pub ramp_up_period_blocks: u32,
    pub lock_timeout_ms_per_mcycle: u32,
    pub timeout_ms_per_mcycle: u32,
    pub max_price_per_mcycle: String,
    #[serde(default)]
    pub min_price_per_mcycle: Option<String>,
    pub lock_collateral: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OfferParamsConfig {
    pub batch: BoundlessOfferParams,
    pub aggregation: BoundlessOfferParams,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeploymentConfig {
    pub deployment_type: Option<DeploymentType>,
    pub overrides: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchQuoteStrategy {
    #[default]
    RaikoAgent,
    Evaluated,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BoundlessConfig {
    #[serde(default = "default_execution_po2")]
    pub execution_po2: u32,
    #[serde(default)]
    pub offchain: bool,
    pub rpc_url: String,
    pub signer_key: String,
    #[serde(default)]
    pub deployment: Option<DeploymentConfig>,
    #[serde(default)]
    pub batch_quoted_mcycles: Option<u32>,
    #[serde(default)]
    pub batch_quote_strategy: BatchQuoteStrategy,
    pub offer_params: OfferParamsConfig,
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

impl Default for BoundlessConfig {
    fn default() -> Self {
        Self {
            execution_po2: default_execution_po2(),
            offchain: false,
            rpc_url: "https://base-rpc.publicnode.com".to_string(),
            signer_key: String::new(),
            deployment: Some(DeploymentConfig {
                deployment_type: Some(DeploymentType::Base),
                overrides: Some(serde_json::json!({
                    "order_stream_url": "https://base-mainnet.boundless.network"
                })),
            }),
            batch_quoted_mcycles: None,
            batch_quote_strategy: BatchQuoteStrategy::default(),
            offer_params: OfferParamsConfig {
                batch: default_batch_offer_params(),
                aggregation: default_aggregation_offer_params(),
            },
            poll_interval_ms: default_poll_interval_ms(),
            timeout_ms: default_timeout_ms(),
        }
    }
}

fn default_execution_po2() -> u32 {
    crate::risc0::Risc0Config::default().execution_po2
}

const fn quote_batch_mcycles(evaluated_mcycles: u32) -> u32 {
    let rounded = if evaluated_mcycles == 0 {
        0
    } else {
        evaluated_mcycles.div_ceil(BATCH_QUOTED_MCYCLES_STEP) * BATCH_QUOTED_MCYCLES_STEP
    };
    if rounded < BATCH_QUOTED_MCYCLES_MIN {
        BATCH_QUOTED_MCYCLES_MIN
    } else {
        rounded
    }
}

impl BoundlessConfig {
    #[must_use]
    pub fn get_deployment_type(&self) -> DeploymentType {
        self.deployment
            .as_ref()
            .and_then(|deployment| deployment.deployment_type.clone())
            .unwrap_or(DeploymentType::Base)
    }

    #[must_use]
    pub fn get_effective_deployment(&self) -> Deployment {
        let mut deployment = match self.get_deployment_type() {
            DeploymentType::Sepolia => SEPOLIA,
            DeploymentType::Base => BASE,
        };

        if let Some(overrides) = self
            .deployment
            .as_ref()
            .and_then(|deployment| deployment.overrides.as_ref())
            && let Some(order_stream_url) = overrides
                .get("order_stream_url")
                .and_then(serde_json::Value::as_str)
        {
            deployment.order_stream_url =
                Some(std::borrow::Cow::Owned(order_stream_url.to_string()));
        }

        deployment
    }

    #[must_use]
    pub fn block_time_sec(&self) -> u32 {
        match self.get_deployment_type() {
            DeploymentType::Base => 2,
            DeploymentType::Sepolia => 12,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ElfType {
    Batch,
    Aggregation,
}

#[derive(Clone, Debug)]
struct UploadedProgram {
    image_id: Digest,
    url: Url,
    refresh_at: SystemTime,
}

#[derive(Clone, Debug)]
struct Submission {
    market_request_id: U256,
    provider_request_id: String,
    remote_tx_hash: Option<String>,
    expires_at: u64,
}

#[derive(Debug)]
struct ValidatedOfferParams {
    max_price: U256,
    min_price: U256,
    lock_collateral: U256,
    lock_timeout: u32,
    timeout: u32,
    ramp_up_period: u32,
    bidding_start: u64,
}

pub struct BoundlessProver {
    config: BoundlessConfig,
    deployment: Deployment,
    programs: Arc<RwLock<HashMap<ElfType, UploadedProgram>>>,
}

impl BoundlessProver {
    #[must_use]
    pub fn new(config: BoundlessConfig) -> Self {
        Self {
            deployment: config.get_effective_deployment(),
            config,
            programs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn create_client(&self) -> RaikoResult<Client> {
        let rpc_url = Url::parse(&self.config.rpc_url).map_err(|e| {
            RaikoError::InvalidRequestConfig(format!("Invalid boundless rpc_url: {e}"))
        })?;
        let signer: PrivateKeySigner = self.config.signer_key.parse().map_err(|e| {
            RaikoError::InvalidRequestConfig(format!("Invalid boundless signer_key: {e}"))
        })?;
        Client::builder()
            .with_rpc_url(rpc_url)
            .with_deployment(Some(self.deployment.clone()))
            .with_storage_provider(boundless_market::storage::storage_provider_from_env().ok())
            .with_private_key(signer)
            .build()
            .await
            .map_err(|e| {
                RaikoError::InvalidRequestConfig(format!("Failed to build boundless client: {e}"))
            })
    }

    async fn ensure_uploaded(
        &self,
        client: &Client,
        elf_type: ElfType,
        elf: &[u8],
    ) -> RaikoResult<UploadedProgram> {
        let image_id = compute_image_id(elf)
            .map_err(|e| RaikoError::Guest(format!("Failed to compute image id: {e}")))?;

        if let Some(program) = self.programs.read().await.get(&elf_type).cloned()
            && program.image_id == image_id
            && SystemTime::now() < program.refresh_at
        {
            return Ok(program);
        }

        let url = client.upload_program(elf).await.map_err(|e| {
            RaikoError::InvalidRequestConfig(format!("Failed to upload boundless program: {e}"))
        })?;
        let expires_secs = url
            .query_pairs()
            .find(|(key, _)| key.eq_ignore_ascii_case("X-Amz-Expires"))
            .and_then(|(_, value)| value.parse::<u64>().ok())
            .unwrap_or(3600);
        let program = UploadedProgram {
            image_id,
            url,
            refresh_at: SystemTime::now()
                + Duration::from_secs(expires_secs.saturating_sub(120).max(60)),
        };
        self.programs
            .write()
            .await
            .insert(elf_type, program.clone());
        Ok(program)
    }

    fn process_input(input: &[u8]) -> RaikoResult<(GuestEnv, Vec<u8>)> {
        let guest_env = GuestEnv::builder().write_frame(input).build_env();
        let guest_env_bytes = guest_env.encode().map_err(|e| {
            RaikoError::InvalidRequestConfig(format!("Failed to encode guest environment: {e}"))
        })?;
        Ok((guest_env, guest_env_bytes))
    }

    async fn evaluate_guest(
        input: Vec<u8>,
        execution_po2: u32,
        elf: Vec<u8>,
    ) -> RaikoResult<(u32, Vec<u8>)> {
        tokio::task::spawn_blocking(move || {
            let executor_env = risc0_zkvm::ExecutorEnv::builder()
                .write_frame(input.as_slice())
                .segment_limit_po2(execution_po2)
                .build()
                .map_err(|e| {
                    RaikoError::Guest(format!("Failed to build boundless execution env: {e}"))
                })?;
            let session = local_executor()
                .execute(executor_env, &elf)
                .map_err(|e| RaikoError::Guest(format!("Boundless dry-run failed: {e}")))?;
            Ok((
                user_cycles_to_mcycles(session.cycles()),
                session.journal.bytes,
            ))
        })
        .await
        .map_err(|err| RaikoError::Guest(format!("Boundless dry-run task failed: {err}")))?
    }

    #[allow(clippy::too_many_arguments)]
    async fn build_request(
        &self,
        client: &Client,
        guest_env: GuestEnv,
        guest_env_bytes: &[u8],
        elf: &[u8],
        program: &UploadedProgram,
        offer_spec: &BoundlessOfferParams,
        mcycles_count: u32,
        journal: Vec<u8>,
    ) -> RaikoResult<ProofRequest> {
        let offer = validate_offer_params(offer_spec, mcycles_count, self.config.block_time_sec())?;
        let input_url = client.upload_input(guest_env_bytes).await.map_err(|e| {
            RaikoError::InvalidRequestConfig(format!("Failed to upload boundless input: {e}"))
        })?;
        let mut request_params = client
            .new_request()
            .with_program(elf.to_vec())
            .with_program_url(program.url.clone())
            .expect("with_program_url is infallible for valid URLs")
            .with_groth16_proof()
            .with_env(guest_env)
            .with_cycles(u64::from(mcycles_count) * MILLION_CYCLES)
            .with_image_id(program.image_id)
            .with_journal(Journal::new(journal))
            .with_offer(
                OfferParams::builder()
                    .ramp_up_period(offer.ramp_up_period)
                    .lock_timeout(offer.lock_timeout)
                    .timeout(offer.timeout)
                    .max_price(offer.max_price)
                    .min_price(offer.min_price)
                    .lock_collateral(offer.lock_collateral)
                    .bidding_start(offer.bidding_start),
            );
        request_params = request_params
            .with_input_url(input_url)
            .expect("with_input_url is infallible for valid URLs");
        client.build_request(request_params).await.map_err(|e| {
            RaikoError::InvalidRequestConfig(format!("Failed to build boundless request: {e:?}"))
        })
    }

    async fn submit_request(
        &self,
        client: &Client,
        request: &ProofRequest,
    ) -> RaikoResult<Submission> {
        if self.config.offchain {
            let market_request_id = client
                .submit_request_offchain(request)
                .await
                .map_err(|e| RaikoError::Guest(format!("Failed to submit boundless request: {e}")))?
                .0;
            return Ok(Submission {
                market_request_id,
                provider_request_id: format!("0x{market_request_id:x}"),
                remote_tx_hash: None,
                expires_at: request.expires_at(),
            });
        }

        let signer = client.signer.as_ref().ok_or_else(|| {
            RaikoError::InvalidRequestConfig("Boundless signer is not configured".to_string())
        })?;
        let balance = client
            .boundless_market
            .balance_of(request.client_address())
            .await
            .map_err(|e| RaikoError::Guest(format!("Failed to query boundless balance: {e}")))?;
        let max_price = U256::from(request.offer.maxPrice);
        let value = if balance > max_price {
            U256::ZERO
        } else {
            max_price - balance
        };
        let chain_id =
            client.boundless_market.get_chain_id().await.map_err(|e| {
                RaikoError::Guest(format!("Failed to query boundless chain id: {e}"))
            })?;
        let market_addr = *client.boundless_market.instance().address();
        let client_sig = request
            .sign_request(signer, market_addr, chain_id)
            .await
            .map_err(|e| RaikoError::Guest(format!("Failed to sign boundless request: {e}")))?;
        let pending_tx = client
            .boundless_market
            .instance()
            .submitRequest(request.clone(), client_sig.as_bytes().into())
            .from(client.boundless_market.caller())
            .value(value)
            .send()
            .await
            .map_err(|e| RaikoError::Guest(format!("Failed to submit boundless request: {e}")))?;
        Ok(Submission {
            market_request_id: request.id,
            provider_request_id: format!("0x{:x}", request.id),
            remote_tx_hash: Some(format!("0x{:x}", pending_tx.tx_hash())),
            expires_at: request.expires_at(),
        })
    }

    async fn poll_until_fulfilled(
        &self,
        client: &Client,
        submission: &Submission,
        proof_type: &'static str,
        image_id: Digest,
        quoted_mcycles_count: u32,
        evaluated_mcycles_count: u32,
    ) -> RaikoResult<Proof> {
        let poll_interval = Duration::from_millis(self.config.poll_interval_ms.max(1));
        let timeout = Duration::from_millis(self.config.timeout_ms.max(1));
        let started_at = Instant::now();

        loop {
            match client
                .boundless_market
                .get_status(submission.market_request_id, Some(submission.expires_at))
                .await
                .map_err(|e| RaikoError::Guest(format!("Failed to read boundless status: {e}")))?
            {
                RequestStatus::Unknown | RequestStatus::Locked => {}
                RequestStatus::Expired => {
                    return Err(RaikoError::Guest(
                        "Boundless request expired before fulfillment".to_string(),
                    ));
                }
                RequestStatus::Fulfilled => {
                    let fulfillment = client
                        .boundless_market
                        .get_request_fulfillment(submission.market_request_id)
                        .await
                        .map_err(|e| {
                            RaikoError::Guest(format!("Failed to read boundless fulfillment: {e}"))
                        })?;
                    let fulfillment_data = fulfillment.data().map_err(|e| {
                        RaikoError::Guest(format!(
                            "Failed to decode boundless fulfillment payload: {e}"
                        ))
                    })?;
                    let journal = fulfillment_data.journal().ok_or_else(|| {
                        RaikoError::Guest("Boundless fulfillment is missing journal".to_string())
                    })?;
                    let seal = fulfillment.seal.clone();
                    let receipt_json = if proof_type == "proposal" {
                        match decode_seal(seal.clone(), image_id, journal.to_vec()) {
                            Ok(ContractReceipt::Base(receipt)) => {
                                serde_json::to_string(&receipt).ok()
                            }
                            Ok(ContractReceipt::SetInclusion(_)) | Err(_) => None,
                        }
                    } else {
                        None
                    };
                    let input_hash = match proof_type {
                        "proposal" => parse_shasta_proposal_input_hash(journal)?,
                        _ => parse_shasta_aggregation_input_hash(journal),
                    };
                    return Ok(Proof {
                        proof: Some(alloy_primitives::hex::encode_prefixed(seal)),
                        input: Some(input_hash),
                        quote: receipt_json,
                        uuid: Some(alloy_primitives::hex::encode_prefixed(image_id.as_bytes())),
                        kzg_proof: None,
                        extra_data: Some(serde_json::json!({
                            "zkvm": "risc0",
                            "runner": "boundless",
                            "proof_type": proof_type,
                            "mcycles_count": quoted_mcycles_count,
                            "quoted_mcycles_count": quoted_mcycles_count,
                            "evaluated_mcycles_count": evaluated_mcycles_count,
                            "boundless": {
                                "provider_request_id": submission.provider_request_id,
                                "remote_tx_hash": submission.remote_tx_hash,
                                "image_id": alloy_primitives::hex::encode_prefixed(image_id.as_bytes()),
                                "deployment": format!("{:?}", self.config.get_deployment_type()).to_lowercase(),
                                "offchain": self.config.offchain,
                            }
                        })),
                    });
                }
            }

            if started_at.elapsed() > timeout {
                return Err(RaikoError::Guest("Boundless request timed out".to_string()));
            }
            tokio::time::sleep(poll_interval).await;
        }
    }

    async fn prove_boundless(
        &self,
        elf_type: ElfType,
        offer_spec: &BoundlessOfferParams,
        proof_type: &'static str,
        input: Bytes,
        elf: &[u8],
        observer: Option<Arc<dyn ProverProgressObserver>>,
    ) -> RaikoResult<Proof> {
        let client = self.create_client().await?;
        let program = self.ensure_uploaded(&client, elf_type, elf).await?;
        let (guest_env, guest_env_bytes) = Self::process_input(input.as_ref())?;
        // Local RISC0 dry-run can take seconds to minutes for large inputs and must not
        // occupy the async runtime threads that serve health/readiness probes.
        let (evaluated_mcycles_count, journal) =
            Self::evaluate_guest(input.to_vec(), self.config.execution_po2, elf.to_vec()).await?;
        let quoted_mcycles_count = self.quoted_mcycles_count(elf_type, evaluated_mcycles_count);
        let request = self
            .build_request(
                &client,
                guest_env,
                &guest_env_bytes,
                elf,
                &program,
                offer_spec,
                quoted_mcycles_count,
                journal,
            )
            .await?;
        let submission = self.submit_request(&client, &request).await?;
        if let Some(observer) = observer {
            observer
                .on_progress(&ProverProgress::BoundlessSubmission(
                    BoundlessSubmissionProgress {
                        provider_request_id: submission.provider_request_id.clone(),
                        remote_tx_hash: submission.remote_tx_hash.clone(),
                        image_ref: alloy_primitives::hex::encode_prefixed(
                            program.image_id.as_bytes(),
                        ),
                        deployment: format!("{:?}", self.config.get_deployment_type())
                            .to_lowercase(),
                        offchain: self.config.offchain,
                        quoted_mcycles_count: Some(quoted_mcycles_count),
                        evaluated_mcycles_count: Some(evaluated_mcycles_count),
                    },
                ))
                .await;
        }
        self.poll_until_fulfilled(
            &client,
            &submission,
            proof_type,
            program.image_id,
            quoted_mcycles_count,
            evaluated_mcycles_count,
        )
        .await
    }
}

impl crate::GuestInputCodec<GuestInput> for BoundlessProver {
    fn encode(&self, input: &GuestInput, _config: &ProverConfig) -> RaikoResult<Bytes> {
        bincode::serialize(input)
            .map(Bytes::from)
            .map_err(|e| RaikoError::InvalidRequestConfig(format!("Encode failed: {e}")))
    }
}

#[async_trait::async_trait]
impl<B> crate::Prover<B> for BoundlessProver
where
    B: ProverBackend,
{
    type GuestInput = GuestInput;

    fn encode(&self, input: &Self::GuestInput, config: &ProverConfig) -> RaikoResult<Bytes> {
        crate::GuestInputCodec::encode(self, input, config)
    }

    async fn prove_encoded(
        &self,
        input: Bytes,
        _config: &ProverConfig,
        backend: &B,
    ) -> RaikoResult<Proof> {
        let elf = backend.elf(ProofStage::Proposal)?.to_vec();
        Box::pin(self.prove_boundless(
            ElfType::Batch,
            &self.config.offer_params.batch,
            "proposal",
            input,
            &elf,
            None,
        ))
        .await
    }

    async fn prove_encoded_with_observer(
        &self,
        input: Bytes,
        _config: &ProverConfig,
        backend: &B,
        observer: Option<Arc<dyn ProverProgressObserver>>,
    ) -> RaikoResult<Proof> {
        let elf = backend.elf(ProofStage::Proposal)?.to_vec();
        Box::pin(self.prove_boundless(
            ElfType::Batch,
            &self.config.offer_params.batch,
            "proposal",
            input,
            &elf,
            observer,
        ))
        .await
    }

    async fn aggregate(
        &self,
        input: AggregationGuestInput,
        _config: &ProverConfig,
        backend: &B,
    ) -> RaikoResult<Proof> {
        let proposal_elf = backend.elf(ProofStage::Proposal)?;
        let proposal_image_id = compute_image_id(proposal_elf)
            .map_err(|e| RaikoError::Guest(format!("Failed to compute proposal image id: {e}")))?;
        let agg = AggregationInput {
            proofs: input.proofs.into_iter().map(proof_to_envelope).collect(),
            expected_image_id: Some(alloy_primitives::hex::encode_prefixed(
                proposal_image_id.as_bytes(),
            )),
            metadata: None,
        };
        let aggregation_input = aggregation::build_risc0_aggregation_input(&agg)?;
        let aggregation_elf = backend.elf(ProofStage::Aggregation)?.to_vec();
        Box::pin(self.prove_boundless(
            ElfType::Aggregation,
            &self.config.offer_params.aggregation,
            "aggregation",
            Bytes::from(aggregation_input),
            &aggregation_elf,
            None,
        ))
        .await
    }

    async fn aggregate_with_observer(
        &self,
        input: AggregationGuestInput,
        _config: &ProverConfig,
        backend: &B,
        observer: Option<Arc<dyn ProverProgressObserver>>,
    ) -> RaikoResult<Proof> {
        let proposal_elf = backend.elf(ProofStage::Proposal)?;
        let proposal_image_id = compute_image_id(proposal_elf)
            .map_err(|e| RaikoError::Guest(format!("Failed to compute proposal image id: {e}")))?;
        let agg = AggregationInput {
            proofs: input.proofs.into_iter().map(proof_to_envelope).collect(),
            expected_image_id: Some(alloy_primitives::hex::encode_prefixed(
                proposal_image_id.as_bytes(),
            )),
            metadata: None,
        };
        let aggregation_input = aggregation::build_risc0_aggregation_input(&agg)?;
        let aggregation_elf = backend.elf(ProofStage::Aggregation)?.to_vec();
        Box::pin(self.prove_boundless(
            ElfType::Aggregation,
            &self.config.offer_params.aggregation,
            "aggregation",
            Bytes::from(aggregation_input),
            &aggregation_elf,
            observer,
        ))
        .await
    }
}

const fn default_poll_interval_ms() -> u64 {
    10_000
}

const fn default_timeout_ms() -> u64 {
    3_600_000
}

fn default_batch_offer_params() -> BoundlessOfferParams {
    BoundlessOfferParams {
        ramp_up_start_sec: 20,
        ramp_up_period_blocks: 60,
        lock_timeout_ms_per_mcycle: 200,
        timeout_ms_per_mcycle: 410,
        max_price_per_mcycle: "0.000000085".to_string(),
        min_price_per_mcycle: Some("0.000000010".to_string()),
        lock_collateral: "20".to_string(),
    }
}

fn default_aggregation_offer_params() -> BoundlessOfferParams {
    BoundlessOfferParams {
        ramp_up_start_sec: 20,
        ramp_up_period_blocks: 60,
        lock_timeout_ms_per_mcycle: 3000,
        timeout_ms_per_mcycle: 6000,
        max_price_per_mcycle: "0.00000006".to_string(),
        min_price_per_mcycle: Some("0.000000006".to_string()),
        lock_collateral: "20".to_string(),
    }
}

impl BoundlessProver {
    // Keep boundless order pricing aligned with the legacy raiko-agent strategy.
    const fn quoted_mcycles_count(&self, elf_type: ElfType, evaluated_mcycles_count: u32) -> u32 {
        match elf_type {
            ElfType::Batch => {
                if let Some(batch_quoted_mcycles) = self.config.batch_quoted_mcycles {
                    batch_quoted_mcycles
                } else {
                    match self.config.batch_quote_strategy {
                        BatchQuoteStrategy::RaikoAgent => {
                            quote_batch_mcycles(evaluated_mcycles_count)
                        }
                        BatchQuoteStrategy::Evaluated => evaluated_mcycles_count,
                    }
                }
            }
            ElfType::Aggregation => AGGREGATION_QUOTED_MCYCLES,
        }
    }
}

fn parse_staking_token(value: &str) -> RaikoResult<U256> {
    parse_units(value, STAKE_TOKEN_DECIMALS)
        .map(Into::into)
        .map_err(|e| {
            RaikoError::InvalidRequestConfig(format!(
                "Failed to parse lock_collateral {value}: {e}"
            ))
        })
}

fn validate_offer_params(
    offer_spec: &BoundlessOfferParams,
    mcycles_count: u32,
    block_time_sec: u32,
) -> RaikoResult<ValidatedOfferParams> {
    let max_price = parse_ether(&offer_spec.max_price_per_mcycle).map_err(|e| {
        RaikoError::InvalidRequestConfig(format!(
            "Failed to parse max_price_per_mcycle {}: {e}",
            offer_spec.max_price_per_mcycle
        ))
    })? * U256::from(mcycles_count);
    let min_price_value = offer_spec.min_price_per_mcycle.as_deref().unwrap_or("0");
    let min_price = parse_ether(min_price_value).map_err(|e| {
        RaikoError::InvalidRequestConfig(format!(
            "Failed to parse min_price_per_mcycle {min_price_value}: {e}"
        ))
    })? * U256::from(mcycles_count);
    if min_price > max_price {
        return Err(RaikoError::InvalidRequestConfig(
            "min_price_per_mcycle cannot exceed max_price_per_mcycle".to_string(),
        ));
    }

    let lock_timeout = offer_spec.lock_timeout_ms_per_mcycle * mcycles_count / 1000;
    let timeout = offer_spec.timeout_ms_per_mcycle * mcycles_count / 1000;
    if timeout <= lock_timeout {
        return Err(RaikoError::InvalidRequestConfig(
            "timeout must be greater than lock_timeout".to_string(),
        ));
    }
    let ramp_up_seconds = offer_spec
        .ramp_up_period_blocks
        .saturating_mul(block_time_sec);
    if ramp_up_seconds > lock_timeout {
        return Err(RaikoError::InvalidRequestConfig(format!(
            "ramp_up_period_blocks={} exceeds lock timeout for {} mcycles",
            offer_spec.ramp_up_period_blocks, mcycles_count
        )));
    }

    Ok(ValidatedOfferParams {
        max_price,
        min_price,
        lock_collateral: parse_staking_token(&offer_spec.lock_collateral)?,
        lock_timeout,
        timeout,
        ramp_up_period: offer_spec.ramp_up_period_blocks,
        bidding_start: SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            + u64::from(offer_spec.ramp_up_start_sec),
    })
}

fn proof_to_envelope(proof: Proof) -> ProofEnvelope {
    let mut verifier_artifacts = Vec::new();
    if let Some(receipt) = proof.quote {
        verifier_artifacts.push(VerifierArtifact {
            kind: "receipt_json".to_string(),
            value: serde_json::Value::String(receipt),
        });
    }
    let payload_bytes = decode_hex_payload(proof.proof.as_deref());

    ProofEnvelope {
        backend: "risc0".to_string(),
        public_inputs: PublicInputs {
            input_hash: proof
                .input
                .map(|value| alloy_primitives::hex::encode_prefixed(value.as_slice())),
            instance_hash: None,
        },
        payload: ProofPayload {
            payload_kind: RISC0_SEAL_PAYLOAD_KIND.to_string(),
            bytes: payload_bytes,
        },
        verifier_artifacts,
        carry_data: None,
        metadata: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AGGREGATION_QUOTED_MCYCLES, BatchQuoteStrategy, BoundlessConfig, BoundlessProver,
        DeploymentType, ElfType, default_batch_offer_params, proof_to_envelope,
        quote_batch_mcycles, user_cycles_to_mcycles, validate_offer_params,
    };
    use raiko2_primitives::Proof;

    fn sample_offer() -> super::BoundlessOfferParams {
        default_batch_offer_params()
    }

    #[test]
    fn default_config_uses_base_deployment() {
        let config = BoundlessConfig::default();
        assert_eq!(config.get_deployment_type(), DeploymentType::Base);
        assert!(!config.offchain);
    }

    #[test]
    fn default_batch_offer_matches_tolba_deployment() {
        let batch = BoundlessConfig::default().offer_params.batch;
        assert_eq!(batch.ramp_up_start_sec, 20);
        assert_eq!(batch.ramp_up_period_blocks, 60);
        assert_eq!(batch.lock_timeout_ms_per_mcycle, 200);
        assert_eq!(batch.timeout_ms_per_mcycle, 410);
        assert_eq!(batch.max_price_per_mcycle, "0.000000085");
        assert_eq!(batch.min_price_per_mcycle.as_deref(), Some("0.000000010"));
        assert_eq!(batch.lock_collateral, "20");
    }

    #[test]
    fn default_aggregation_offer_matches_tolba_deployment() {
        let aggregation = BoundlessConfig::default().offer_params.aggregation;
        assert_eq!(aggregation.ramp_up_start_sec, 20);
        assert_eq!(aggregation.ramp_up_period_blocks, 60);
        assert_eq!(aggregation.lock_timeout_ms_per_mcycle, 3000);
        assert_eq!(aggregation.timeout_ms_per_mcycle, 6000);
        assert_eq!(aggregation.max_price_per_mcycle, "0.00000006");
        assert_eq!(
            aggregation.min_price_per_mcycle.as_deref(),
            Some("0.000000006")
        );
        assert_eq!(aggregation.lock_collateral, "20");
    }

    #[test]
    fn quoted_mcycles_count_matches_raiko_agent_strategy() {
        let prover = BoundlessProver::new(BoundlessConfig::default());
        assert_eq!(prover.quoted_mcycles_count(ElfType::Batch, 1_491), 2_000);
        assert_eq!(
            prover.quoted_mcycles_count(ElfType::Aggregation, 123),
            AGGREGATION_QUOTED_MCYCLES
        );
    }

    #[test]
    fn quoted_mcycles_count_can_use_evaluated_cycles_directly() {
        let mut config = BoundlessConfig::default();
        config.batch_quote_strategy = BatchQuoteStrategy::Evaluated;
        let prover = BoundlessProver::new(config);
        assert_eq!(prover.quoted_mcycles_count(ElfType::Batch, 1_188), 1_188);
    }

    #[test]
    fn quoted_mcycles_count_can_use_fixed_override() {
        let mut config = BoundlessConfig::default();
        config.batch_quoted_mcycles = Some(1_500);
        config.batch_quote_strategy = BatchQuoteStrategy::Evaluated;
        let prover = BoundlessProver::new(config);
        assert_eq!(prover.quoted_mcycles_count(ElfType::Batch, 1_188), 1_500);
    }

    #[test]
    fn quote_batch_mcycles_rounds_up_like_old_agent() {
        assert_eq!(quote_batch_mcycles(0), 2_000);
        assert_eq!(quote_batch_mcycles(1_491), 2_000);
        assert_eq!(quote_batch_mcycles(2_000), 2_000);
        assert_eq!(quote_batch_mcycles(2_001), 3_000);
    }

    #[test]
    fn evaluated_mcycles_use_user_cycles_units() {
        assert_eq!(user_cycles_to_mcycles(1_490_550_784), 1_491);
        assert_eq!(user_cycles_to_mcycles(1_192_626_023), 1_193);
    }

    #[test]
    fn validate_offer_params_rejects_min_price_above_max_price() {
        let mut offer = sample_offer();
        offer.min_price_per_mcycle = Some("0.00000009".to_string());
        let err = validate_offer_params(&offer, 100, 2).unwrap_err();
        assert!(err.to_string().contains("min_price_per_mcycle"));
    }

    #[test]
    fn validate_offer_params_rejects_timeout_not_above_lock_timeout() {
        let mut offer = sample_offer();
        offer.lock_timeout_ms_per_mcycle = 300;
        offer.timeout_ms_per_mcycle = 300;
        let err = validate_offer_params(&offer, 100, 2).unwrap_err();
        assert!(err.to_string().contains("timeout"));
    }

    #[test]
    fn validate_offer_params_accepts_base_defaults() {
        let validated = validate_offer_params(&sample_offer(), 1_000, 2).expect("valid offer");
        assert!(validated.max_price > validated.min_price);
        assert!(validated.timeout > validated.lock_timeout);
    }

    #[test]
    fn proof_to_envelope_preserves_risc0_seal_payload() {
        let envelope = proof_to_envelope(Proof {
            proof: Some("0x1234".to_string()),
            quote: Some("{\"receipt\":true}".to_string()),
            ..Default::default()
        });

        assert_eq!(envelope.payload.payload_kind, "risc0_seal");
        assert_eq!(envelope.payload.bytes, vec![0x12, 0x34]);
        assert_eq!(envelope.verifier_artifacts.len(), 1);
        assert_eq!(envelope.verifier_artifacts[0].kind, "receipt_json");
    }
}
