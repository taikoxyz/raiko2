#![allow(missing_docs)]

pub mod aggregation;
mod config;

pub use config::{
    BatchQuoteStrategy, BoundlessConfig, BoundlessOfferParams, BoundlessPricingMode,
    DeploymentConfig, DeploymentType, OfferParamsConfig, validate_offer_spec,
};

use std::borrow::Cow;
use std::collections::HashMap;
use std::env;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use alloy_primitives::{B256, Bytes, U256, address};
use alloy_signer_local::PrivateKeySigner;
use boundless_market::{
    Client, ProofRequest, StorageUploaderConfig,
    contracts::RequestStatus,
    deployments::{BASE, Deployment, SEPOLIA},
    input::GuestEnv,
    price_oracle::{Amount, Asset},
    request_builder::OfferParams,
    storage::StorageUploaderType,
};
use raiko2_pipeline::{ProofStage, ProverBackend};
use raiko2_primitives::{AggregationGuestInput, ProverConfig};
use raiko2_primitives::{Proof, RaikoError, RaikoResult};
use raiko2_primitives_shasta::GuestInput;
use raiko2_protocol_shasta::shasta::ProofCarryData;
use risc0_ethereum_contracts_boundless::receipt::{Receipt as ContractReceipt, decode_seal};
use risc0_zkvm::{Digest, Journal, compute_image_id, local_executor};
use tokio::sync::RwLock;
use url::Url;

use crate::{
    BoundlessSubmissionProgress, BoundlessSubmissionResume, ProverProgress, ProverProgressObserver,
    encode_risc0_aggregation_seal_payload, encode_risc0_proposal_seal_payload,
    parse_shasta_aggregation_input_hash, parse_shasta_proposal_input_hash, with_shasta_extra_data,
};

const MILLION_CYCLES: u64 = 1_000_000;
const BATCH_QUOTED_MCYCLES_MIN: u32 = 2_000;
const BATCH_QUOTED_MCYCLES_STEP: u32 = 1_000;
const EXTERNAL_RETRY_ATTEMPTS: u32 = 5;
const EXTERNAL_RETRY_INITIAL_DELAY: Duration = Duration::from_secs(1);
const EXTERNAL_RETRY_MAX_DELAY: Duration = Duration::from_secs(30);
const TAIKO_MAINNET_INDEXER_URL: &str = "https://d29nqt0gudcxhl.cloudfront.net/";

async fn retry_external<T, F, Fut>(operation: &str, mut run: F) -> RaikoResult<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = RaikoResult<T>>,
{
    let mut attempt = 1_u32;
    let mut delay = EXTERNAL_RETRY_INITIAL_DELAY;

    loop {
        match run().await {
            Ok(value) => {
                if attempt > 1 {
                    tracing::info!(operation, attempt, "Boundless external operation recovered");
                }
                return Ok(value);
            }
            Err(err) if attempt < EXTERNAL_RETRY_ATTEMPTS => {
                tracing::warn!(
                    operation,
                    attempt,
                    delay_ms = delay.as_millis(),
                    error = %err,
                    "Retrying Boundless external operation"
                );
                tokio::time::sleep(delay).await;
                attempt = attempt.saturating_add(1);
                delay = delay.saturating_mul(2).min(EXTERNAL_RETRY_MAX_DELAY);
            }
            Err(err) => return Err(err),
        }
    }
}

async fn compute_boundless_image_id(elf: Vec<u8>, stage: &str) -> RaikoResult<Digest> {
    tokio::task::spawn_blocking(move || compute_image_id(&elf))
        .await
        .map_err(|err| RaikoError::Guest(format!("{stage} image id task failed: {err}")))?
        .map_err(|e| RaikoError::Guest(format!("Failed to compute {stage} image id: {e}")))
}

async fn build_boundless_aggregation_input(
    proofs: Vec<Proof>,
    expected_image_id: Digest,
) -> RaikoResult<Vec<u8>> {
    tokio::task::spawn_blocking(move || {
        crate::risc0_aggregation::build_risc0_aggregation_input_from_proofs(
            proofs,
            expected_image_id,
        )
    })
    .await
    .map_err(|err| RaikoError::Guest(format!("Boundless aggregation input task failed: {err}")))?
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn user_cycles_to_mcycles(user_cycles: u64) -> u32 {
    let mcycles = user_cycles.div_ceil(MILLION_CYCLES);
    u32::try_from(mcycles).unwrap_or(u32::MAX)
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
    pub fn get_effective_deployment(&self) -> Deployment {
        let mut deployment = match self.get_deployment_type() {
            DeploymentType::Sepolia => SEPOLIA,
            DeploymentType::Base => BASE,
            DeploymentType::Taiko => taiko_deployment(),
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
}

fn taiko_deployment() -> Deployment {
    Deployment::builder()
        .market_chain_id(167_000_u64)
        .boundless_market_address(address!("0xb3f5c7b4379052eade8c7f3fa6da37fb871da28b"))
        .verifier_router_address(address!("0x607d196b43abc5d9BE3c7Fb8e336Ca82fec18C45"))
        .set_verifier_address(address!("0x6135DC08D14EF8a44496B009e2181426628B8ebd"))
        .collateral_token_address(address!("0xC284A781072442cC1882a8Db4573990B7B49DaC4"))
        .order_stream_url(Cow::Borrowed("https://taiko-mainnet.boundless.network"))
        .indexer_url(Cow::Borrowed(TAIKO_MAINNET_INDEXER_URL))
        .deployment_block(4_819_525_u64)
        .build()
        .expect("Taiko Boundless deployment constants should be valid")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ElfType {
    Batch,
    Aggregation,
}

impl ElfType {
    const fn stage_name(self) -> &'static str {
        match self {
            Self::Batch => "batch",
            Self::Aggregation => "aggregation",
        }
    }
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

struct FreshSubmissionContext<'a> {
    client: &'a Client,
    input: &'a Bytes,
    elf: &'a [u8],
    program: &'a UploadedProgram,
    offer_spec: &'a BoundlessOfferParams,
    journal: &'a [u8],
    image_ref: &'a str,
    deployment: &'a str,
    observer: Option<&'a Arc<dyn ProverProgressObserver>>,
    quoted_mcycles_count: u32,
    evaluated_mcycles_count: u32,
}

enum BoundlessAttemptError {
    Retryable(String),
    Fatal(RaikoError),
}

impl From<RaikoError> for BoundlessAttemptError {
    fn from(value: RaikoError) -> Self {
        Self::Fatal(value)
    }
}

#[allow(clippy::too_many_arguments)]
async fn publish_boundless_progress(
    observer: Option<&Arc<dyn ProverProgressObserver>>,
    submission: &Submission,
    image_ref: &str,
    deployment: &str,
    offchain: bool,
    quoted_mcycles_count: u32,
    evaluated_mcycles_count: u32,
) {
    if let Some(observer) = observer {
        observer
            .on_progress(&ProverProgress::BoundlessSubmission(
                BoundlessSubmissionProgress {
                    provider_request_id: submission.provider_request_id.clone(),
                    remote_tx_hash: submission.remote_tx_hash.clone(),
                    expires_at: submission.expires_at,
                    image_ref: image_ref.to_string(),
                    deployment: deployment.to_string(),
                    offchain,
                    quoted_mcycles_count: Some(quoted_mcycles_count),
                    evaluated_mcycles_count: Some(evaluated_mcycles_count),
                },
            ))
            .await;
    }
}

impl TryFrom<BoundlessSubmissionResume> for Submission {
    type Error = RaikoError;

    fn try_from(value: BoundlessSubmissionResume) -> Result<Self, Self::Error> {
        let raw_id = value.provider_request_id.trim_start_matches("0x");
        let market_request_id = U256::from_str_radix(raw_id, 16).map_err(|e| {
            RaikoError::Guest(format!(
                "Invalid stored Boundless provider_request_id {}: {e}",
                value.provider_request_id
            ))
        })?;
        if market_request_id == U256::ZERO {
            return Err(RaikoError::Guest(
                "Invalid stored Boundless provider_request_id: zero".to_string(),
            ));
        }
        if value.expires_at == 0 {
            return Err(RaikoError::Guest(
                "Invalid stored Boundless submission: missing expires_at".to_string(),
            ));
        }
        Ok(Self {
            market_request_id,
            provider_request_id: value.provider_request_id,
            remote_tx_hash: value.remote_tx_hash,
            expires_at: value.expires_at,
        })
    }
}

#[derive(Debug)]
struct ValidatedOfferParams {
    max_price: Option<Amount>,
    min_price: Option<Amount>,
    max_price_cap: Option<Amount>,
    lock_collateral: Amount,
    lock_timeout: u32,
    timeout: u32,
    ramp_up_period_secs: u32,
    bidding_start: u64,
}

fn env_var(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_env_bool(name: &str, value: &str) -> RaikoResult<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "y" | "on" => Ok(true),
        "false" | "0" | "no" | "n" | "off" => Ok(false),
        _ => Err(RaikoError::InvalidRequestConfig(format!(
            "Invalid {name} boolean value, expected true/false/1/0/yes/no/on/off"
        ))),
    }
}

fn env_bool(name: &str) -> RaikoResult<Option<bool>> {
    env_var(name)
        .map(|value| parse_env_bool(name, &value))
        .transpose()
}

fn parse_env_url(name: &str, value: &str) -> RaikoResult<Url> {
    Url::parse(value)
        .map_err(|e| RaikoError::InvalidRequestConfig(format!("Invalid {name} URL: {e}")))
}

fn env_url(name: &str) -> RaikoResult<Option<Url>> {
    env_var(name)
        .map(|value| parse_env_url(name, &value))
        .transpose()
}

fn storage_uploader_config_from_env() -> RaikoResult<StorageUploaderConfig> {
    let mut config = StorageUploaderConfig::default();
    let selected = env_var("STORAGE_UPLOADER").map(|value| value.to_ascii_lowercase());
    config.storage_uploader = match selected.as_deref() {
        Some("s3") => StorageUploaderType::S3,
        Some("pinata") => StorageUploaderType::Pinata,
        Some("file") => StorageUploaderType::File,
        Some("none") => StorageUploaderType::None,
        Some(other) => {
            return Err(RaikoError::InvalidRequestConfig(format!(
                "Invalid STORAGE_UPLOADER value {other}"
            )));
        }
        None if env_var("S3_BUCKET").is_some() => StorageUploaderType::S3,
        None if env_var("PINATA_JWT").is_some() => StorageUploaderType::Pinata,
        None if env_var("FILE_PATH").is_some() => StorageUploaderType::File,
        None => StorageUploaderType::None,
    };
    config.s3_bucket = env_var("S3_BUCKET");
    config.s3_url = env_var("S3_URL");
    config.aws_access_key_id = env_var("AWS_ACCESS_KEY_ID");
    config.aws_secret_access_key = env_var("AWS_SECRET_ACCESS_KEY");
    config.aws_region = env_var("AWS_REGION");
    config.s3_presigned = env_bool("S3_PRESIGNED")?;
    config.s3_public_url = env_bool("S3_PUBLIC_URL")?;
    config.pinata_jwt = env_var("PINATA_JWT");
    config.pinata_api_url = env_url("PINATA_API_URL")?;
    config.ipfs_gateway_url = env_url("IPFS_GATEWAY_URL")?;
    config.file_path = env_var("FILE_PATH").map(PathBuf::from);
    Ok(config)
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
        let storage_config = storage_uploader_config_from_env()?;
        Client::builder()
            .with_rpc_url(rpc_url)
            .with_deployment(Some(self.deployment.clone()))
            .with_uploader_config(&storage_config)
            .await
            .map_err(|e| {
                RaikoError::InvalidRequestConfig(format!(
                    "Failed to configure boundless storage uploader: {e}"
                ))
            })?
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
        let image_id = compute_boundless_image_id(elf.to_vec(), elf_type.stage_name()).await?;

        if let Some(program) = self.programs.read().await.get(&elf_type).cloned()
            && program.image_id == image_id
            && SystemTime::now() < program.refresh_at
        {
            return Ok(program);
        }

        let url = retry_external("upload boundless program", || async {
            client.upload_program(elf).await.map_err(|e| {
                RaikoError::InvalidRequestConfig(format!("Failed to upload boundless program: {e}"))
            })
        })
        .await?;
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
        let ValidatedOfferParams {
            max_price,
            min_price,
            max_price_cap,
            lock_collateral,
            lock_timeout,
            timeout,
            ramp_up_period_secs,
            bidding_start,
        } = validate_offer_params(offer_spec, mcycles_count, self.config.block_time_sec())?;
        let input_url = retry_external("upload boundless input", || async {
            client.upload_input(guest_env_bytes).await.map_err(|e| {
                RaikoError::InvalidRequestConfig(format!("Failed to upload boundless input: {e}"))
            })
        })
        .await?;
        let mut offer_params = OfferParams::builder();
        offer_params
            .ramp_up_period(ramp_up_period_secs)
            .lock_timeout(lock_timeout)
            .timeout(timeout)
            .lock_collateral(lock_collateral)
            .bidding_start(bidding_start);
        if let Some(max_price) = max_price {
            offer_params.max_price(max_price);
        }
        if let Some(min_price) = min_price {
            offer_params.min_price(min_price);
        }
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
            .with_offer(offer_params);
        request_params = request_params
            .with_input_url(input_url)
            .expect("with_input_url is infallible for valid URLs");
        let mut request = retry_external("build boundless request", || {
            let request_params = request_params.clone();
            async move {
                Box::pin(client.build_request(request_params))
                    .await
                    .map_err(|e| {
                        RaikoError::InvalidRequestConfig(format!(
                            "Failed to build boundless request: {e:?}"
                        ))
                    })
            }
        })
        .await?;
        apply_market_max_price_cap(
            &mut request,
            max_price_cap.as_ref(),
            mcycles_count,
            offer_spec.pricing_mode,
        )?;
        Ok(request)
    }

    async fn submit_request_offchain(
        &self,
        client: &Client,
        request: &ProofRequest,
    ) -> RaikoResult<Submission> {
        let market_request_id = retry_external("submit boundless offchain request", || async {
            client
                .submit_request_offchain(request)
                .await
                .map(|(id, _)| id)
                .map_err(|e| RaikoError::Guest(format!("Failed to submit boundless request: {e}")))
        })
        .await?;
        Ok(Submission {
            market_request_id,
            provider_request_id: format!("0x{market_request_id:x}"),
            remote_tx_hash: None,
            expires_at: request.expires_at(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn submit_request_onchain(
        &self,
        client: &Client,
        request: &ProofRequest,
        observer: Option<&Arc<dyn ProverProgressObserver>>,
        image_ref: &str,
        deployment: &str,
        quoted_mcycles_count: u32,
        evaluated_mcycles_count: u32,
    ) -> RaikoResult<Submission> {
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
        let call = client
            .boundless_market
            .instance()
            .submitRequest(request.clone(), client_sig.as_bytes().into())
            .from(client.boundless_market.caller())
            .value(value);

        let mut submission = Submission {
            market_request_id: request.id,
            provider_request_id: format!("0x{:x}", request.id),
            remote_tx_hash: None,
            expires_at: request.expires_at(),
        };
        publish_boundless_progress(
            observer,
            &submission,
            image_ref,
            deployment,
            false,
            quoted_mcycles_count,
            evaluated_mcycles_count,
        )
        .await;

        match call.send().await {
            Ok(pending_tx) => {
                submission.remote_tx_hash = Some(format!("0x{:x}", pending_tx.tx_hash()));
                publish_boundless_progress(
                    observer,
                    &submission,
                    image_ref,
                    deployment,
                    false,
                    quoted_mcycles_count,
                    evaluated_mcycles_count,
                )
                .await;
            }
            Err(error) => {
                tracing::warn!(
                    provider_request_id = %submission.provider_request_id,
                    error = %error,
                    "Boundless submitRequest returned an uncertain error; polling reserved request id"
                );
            }
        }

        Ok(submission)
    }

    async fn submit_fresh_request(
        &self,
        context: FreshSubmissionContext<'_>,
    ) -> RaikoResult<Submission> {
        let (guest_env, guest_env_bytes) = Self::process_input(context.input.as_ref())?;
        let request = Box::pin(self.build_request(
            context.client,
            guest_env,
            &guest_env_bytes,
            context.elf,
            context.program,
            context.offer_spec,
            context.quoted_mcycles_count,
            context.journal.to_vec(),
        ))
        .await?;

        if self.config.offchain {
            let submission = self
                .submit_request_offchain(context.client, &request)
                .await?;
            publish_boundless_progress(
                context.observer,
                &submission,
                context.image_ref,
                context.deployment,
                true,
                context.quoted_mcycles_count,
                context.evaluated_mcycles_count,
            )
            .await;
            return Ok(submission);
        }

        self.submit_request_onchain(
            context.client,
            &request,
            context.observer,
            context.image_ref,
            context.deployment,
            context.quoted_mcycles_count,
            context.evaluated_mcycles_count,
        )
        .await
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    async fn poll_until_fulfilled(
        &self,
        client: &Client,
        submission: &Submission,
        proof_type: &'static str,
        image_id: Digest,
        block_image_id: Option<Digest>,
        expected_input_hash: B256,
        quoted_mcycles_count: u32,
        evaluated_mcycles_count: u32,
        proposal_carry_data: Option<&ProofCarryData>,
    ) -> Result<Proof, BoundlessAttemptError> {
        let poll_interval = Duration::from_millis(self.config.poll_interval_ms.max(1));
        let timeout = Duration::from_millis(self.config.timeout_ms.max(1));
        let started_at = Instant::now();
        let mut last_poll_error: Option<String> = None;
        let mut consecutive_poll_errors = 0_u32;

        loop {
            if started_at.elapsed() > timeout {
                let detail = last_poll_error
                    .as_deref()
                    .map(|error| format!("; last polling error: {error}"))
                    .unwrap_or_default();
                return Err(BoundlessAttemptError::Retryable(format!(
                    "Boundless request {} timed out before fulfillment{detail}",
                    submission.provider_request_id
                )));
            }

            let status = match client
                .boundless_market
                .get_status(submission.market_request_id, Some(submission.expires_at))
                .await
            {
                Ok(status) => {
                    consecutive_poll_errors = 0;
                    last_poll_error = None;
                    status
                }
                Err(error) => {
                    consecutive_poll_errors = consecutive_poll_errors.saturating_add(1);
                    let message = format!("Failed to read boundless status: {error}");
                    tracing::warn!(
                        provider_request_id = submission.provider_request_id,
                        consecutive_poll_errors,
                        "{message}"
                    );
                    last_poll_error = Some(message);
                    tokio::time::sleep(poll_interval).await;
                    continue;
                }
            };

            match status {
                RequestStatus::Unknown | RequestStatus::Locked => {}
                RequestStatus::Expired => {
                    return Err(BoundlessAttemptError::Retryable(format!(
                        "Boundless request {} expired before fulfillment",
                        submission.provider_request_id
                    )));
                }
                RequestStatus::Fulfilled => {
                    let fulfillment = match client
                        .boundless_market
                        .get_request_fulfillment(submission.market_request_id, None, None)
                        .await
                    {
                        Ok(fulfillment) => fulfillment,
                        Err(error) => {
                            consecutive_poll_errors = consecutive_poll_errors.saturating_add(1);
                            let message = format!("Failed to read boundless fulfillment: {error}");
                            tracing::warn!(
                                provider_request_id = submission.provider_request_id,
                                consecutive_poll_errors,
                                "{message}"
                            );
                            last_poll_error = Some(message);
                            tokio::time::sleep(poll_interval).await;
                            continue;
                        }
                    };
                    let fulfillment_data = fulfillment.data().map_err(|e| {
                        BoundlessAttemptError::Fatal(RaikoError::Guest(format!(
                            "Failed to decode boundless fulfillment payload: {e}"
                        )))
                    })?;
                    let journal = fulfillment_data.journal().ok_or_else(|| {
                        BoundlessAttemptError::Fatal(RaikoError::Guest(
                            "Boundless fulfillment is missing journal".to_string(),
                        ))
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
                        _ => parse_shasta_aggregation_input_hash(journal)?,
                    };
                    if input_hash != expected_input_hash {
                        return Err(BoundlessAttemptError::Fatal(RaikoError::Guest(
                            "Boundless fulfillment journal does not match local dry-run journal"
                                .to_string(),
                        )));
                    }
                    let stage_metadata = serde_json::json!({
                        "zkvm": "risc0",
                        "runner": "network",
                        "proof_type": proof_type,
                        "mcycles_count": quoted_mcycles_count,
                        "quoted_mcycles_count": quoted_mcycles_count,
                        "evaluated_mcycles_count": evaluated_mcycles_count,
                        "boundless": {
                            "provider_request_id": submission.provider_request_id,
                            "remote_tx_hash": submission.remote_tx_hash,
                            "expires_at": submission.expires_at,
                            "image_id": alloy_primitives::hex::encode_prefixed(image_id.as_bytes()),
                            "deployment": format!("{:?}", self.config.get_deployment_type()).to_lowercase(),
                            "offchain": self.config.offchain,
                        }
                    });
                    let extra_data = match (proof_type, proposal_carry_data) {
                        ("proposal", Some(carry)) => {
                            with_shasta_extra_data(carry, "risc0", Some(stage_metadata))?
                        }
                        _ => Some(stage_metadata),
                    };
                    let proof = match proof_type {
                        "proposal" => encode_risc0_proposal_seal_payload(
                            &seal,
                            B256::from_slice(image_id.as_bytes()),
                        ),
                        _ => encode_risc0_aggregation_seal_payload(
                            &seal,
                            B256::from_slice(
                                block_image_id
                                    .ok_or_else(|| {
                                        RaikoError::Guest(
                                            "missing block image id for aggregation proof"
                                                .to_string(),
                                        )
                                    })?
                                    .as_bytes(),
                            ),
                            B256::from_slice(image_id.as_bytes()),
                        ),
                    };
                    return Ok(Proof {
                        proof: Some(proof),
                        input: Some(input_hash),
                        quote: receipt_json,
                        uuid: Some(alloy_primitives::hex::encode_prefixed(image_id.as_bytes())),
                        kzg_proof: None,
                        extra_data,
                    });
                }
            }

            tokio::time::sleep(poll_interval).await;
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn prove_boundless(
        &self,
        elf_type: ElfType,
        offer_spec: &BoundlessOfferParams,
        proof_type: &'static str,
        input: Bytes,
        elf: &[u8],
        block_image_id: Option<Digest>,
        proposal_carry_data: Option<ProofCarryData>,
        observer: Option<Arc<dyn ProverProgressObserver>>,
    ) -> RaikoResult<Proof> {
        let client = retry_external("create boundless client", || self.create_client()).await?;
        let program = self.ensure_uploaded(&client, elf_type, elf).await?;
        // Local RISC0 dry-run can take seconds to minutes for large inputs and must not
        // occupy the async runtime threads that serve health/readiness probes.
        let (evaluated_mcycles_count, journal) =
            Self::evaluate_guest(input.to_vec(), self.config.execution_po2, elf.to_vec()).await?;
        let expected_input_hash = match proof_type {
            "proposal" => parse_shasta_proposal_input_hash(&journal)?,
            _ => parse_shasta_aggregation_input_hash(&journal)?,
        };
        let quoted_mcycles_count = self.quoted_mcycles_count(elf_type, evaluated_mcycles_count);
        let image_ref = alloy_primitives::hex::encode_prefixed(program.image_id.as_bytes());
        let deployment = format!("{:?}", self.config.get_deployment_type()).to_lowercase();

        let mut resume_submission = if let Some(observer) = observer.as_ref() {
            observer
                .load_boundless_submission()
                .await
                .map(Submission::try_from)
                .transpose()?
        } else {
            None
        };
        let mut attempt = 1_u64;

        loop {
            let submission = if let Some(submission) = resume_submission.take() {
                if submission.expires_at <= now_secs() {
                    tracing::warn!(
                        provider_request_id = %submission.provider_request_id,
                        expires_at = submission.expires_at,
                        "Stored Boundless submission is expired; submitting a new request"
                    );
                    continue;
                }
                publish_boundless_progress(
                    observer.as_ref(),
                    &submission,
                    &image_ref,
                    &deployment,
                    self.config.offchain,
                    quoted_mcycles_count,
                    evaluated_mcycles_count,
                )
                .await;
                submission
            } else {
                Box::pin(self.submit_fresh_request(FreshSubmissionContext {
                    client: &client,
                    input: &input,
                    elf,
                    program: &program,
                    offer_spec,
                    journal: &journal,
                    image_ref: &image_ref,
                    deployment: &deployment,
                    observer: observer.as_ref(),
                    quoted_mcycles_count,
                    evaluated_mcycles_count,
                }))
                .await?
            };

            tracing::info!(
                provider_request_id = %submission.provider_request_id,
                expires_at = submission.expires_at,
                attempt,
                "Using Boundless market submission"
            );

            match self
                .poll_until_fulfilled(
                    &client,
                    &submission,
                    proof_type,
                    program.image_id,
                    block_image_id,
                    expected_input_hash,
                    quoted_mcycles_count,
                    evaluated_mcycles_count,
                    proposal_carry_data.as_ref(),
                )
                .await
            {
                Ok(proof) => return Ok(proof),
                Err(BoundlessAttemptError::Retryable(reason)) => {
                    tracing::warn!(
                        provider_request_id = %submission.provider_request_id,
                        attempt,
                        reason,
                        "Boundless submission did not finish; retrying with a new market request"
                    );
                    attempt = attempt.saturating_add(1);
                    tokio::time::sleep(EXTERNAL_RETRY_INITIAL_DELAY).await;
                }
                Err(BoundlessAttemptError::Fatal(error)) => return Err(error),
            }
        }
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
        let guest_input: GuestInput = bincode::deserialize(input.as_ref())
            .map_err(|e| RaikoError::Guest(format!("Failed to deserialize input: {e}")))?;
        let elf = backend.elf(ProofStage::Proposal)?.to_vec();
        Box::pin(self.prove_boundless(
            ElfType::Batch,
            &self.config.offer_params.batch,
            "proposal",
            input,
            &elf,
            None,
            Some(guest_input.proof_carry_data),
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
        let guest_input: GuestInput = bincode::deserialize(input.as_ref())
            .map_err(|e| RaikoError::Guest(format!("Failed to deserialize input: {e}")))?;
        let elf = backend.elf(ProofStage::Proposal)?.to_vec();
        Box::pin(self.prove_boundless(
            ElfType::Batch,
            &self.config.offer_params.batch,
            "proposal",
            input,
            &elf,
            None,
            Some(guest_input.proof_carry_data),
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
        let proposal_image_id =
            compute_boundless_image_id(backend.elf(ProofStage::Proposal)?.to_vec(), "proposal")
                .await?;
        let aggregation_input =
            build_boundless_aggregation_input(input.proofs, proposal_image_id).await?;
        let aggregation_elf = backend.elf(ProofStage::Aggregation)?.to_vec();
        Box::pin(self.prove_boundless(
            ElfType::Aggregation,
            &self.config.offer_params.aggregation,
            "aggregation",
            Bytes::from(aggregation_input),
            &aggregation_elf,
            Some(proposal_image_id),
            None,
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
        let proposal_image_id =
            compute_boundless_image_id(backend.elf(ProofStage::Proposal)?.to_vec(), "proposal")
                .await?;
        let aggregation_input =
            build_boundless_aggregation_input(input.proofs, proposal_image_id).await?;
        let aggregation_elf = backend.elf(ProofStage::Aggregation)?.to_vec();
        Box::pin(self.prove_boundless(
            ElfType::Aggregation,
            &self.config.offer_params.aggregation,
            "aggregation",
            Bytes::from(aggregation_input),
            &aggregation_elf,
            Some(proposal_image_id),
            None,
            observer,
        ))
        .await
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
            ElfType::Aggregation => self.config.aggregation_quoted_mcycles,
        }
    }
}

fn parse_amount(value: &str, field: &str, asset: Asset) -> Result<Amount, String> {
    Amount::parse_with_allowed(value, &[asset], Some(asset))
        .map_err(|e| format!("Failed to parse {field} {value}: {e}"))
}

fn parse_request_amount(
    value: &str,
    field: &str,
    asset: Asset,
    multiplier: u32,
) -> RaikoResult<Amount> {
    let mut amount = parse_amount(value, field, asset).map_err(RaikoError::InvalidRequestConfig)?;
    amount.value = amount
        .value
        .checked_mul(U256::from(multiplier))
        .ok_or_else(|| {
            RaikoError::InvalidRequestConfig(format!(
                "{field} overflows when multiplied by {multiplier} mcycles"
            ))
        })?;
    Ok(amount)
}

fn capped_market_max_price(max_price: U256, max_price_cap: Option<&Amount>) -> U256 {
    max_price_cap
        .map(|cap| max_price.min(cap.value))
        .unwrap_or(max_price)
}

fn apply_market_max_price_cap(
    request: &mut ProofRequest,
    max_price_cap: Option<&Amount>,
    mcycles_count: u32,
    pricing_mode: BoundlessPricingMode,
) -> RaikoResult<()> {
    if pricing_mode != BoundlessPricingMode::Market {
        return Ok(());
    }
    let Some(max_price_cap) = max_price_cap else {
        return Ok(());
    };

    let autoprice_max_price = U256::from(request.offer.maxPrice);
    let effective_max_price = capped_market_max_price(autoprice_max_price, Some(max_price_cap));
    let was_capped = effective_max_price < autoprice_max_price;
    request.offer.maxPrice = effective_max_price;
    request.validate().map_err(|err| {
        RaikoError::InvalidRequestConfig(format!(
            "Boundless request invalid after applying market max price cap: {err}"
        ))
    })?;
    tracing::info!(
        mcycles_count,
        autoprice_max_price_wei = %autoprice_max_price,
        cap_max_price_wei = %max_price_cap.value,
        effective_max_price_wei = %effective_max_price,
        was_capped,
        "Applied Boundless market max price cap"
    );
    Ok(())
}

fn validate_offer_params(
    offer_spec: &BoundlessOfferParams,
    mcycles_count: u32,
    block_time_sec: u32,
) -> RaikoResult<ValidatedOfferParams> {
    validate_offer_spec(offer_spec).map_err(RaikoError::InvalidRequestConfig)?;
    let (max_price, min_price, max_price_cap) = match offer_spec.pricing_mode {
        BoundlessPricingMode::Manual => {
            let max_price_value = offer_spec.max_price_per_mcycle.as_deref().ok_or_else(|| {
                RaikoError::InvalidRequestConfig(
                    "max_price_per_mcycle is required when pricing_mode=manual".to_string(),
                )
            })?;
            let max_price = parse_request_amount(
                max_price_value,
                "max_price_per_mcycle",
                Asset::ETH,
                mcycles_count,
            )?;
            let min_price_value = offer_spec.min_price_per_mcycle.as_deref().unwrap_or("0");
            let min_price = parse_request_amount(
                min_price_value,
                "min_price_per_mcycle",
                Asset::ETH,
                mcycles_count,
            )?;
            (Some(max_price), Some(min_price), None)
        }
        BoundlessPricingMode::Market => {
            let max_price_cap = offer_spec
                .max_price_per_mcycle
                .as_deref()
                .map(|max_price_value| {
                    parse_request_amount(
                        max_price_value,
                        "max_price_per_mcycle",
                        Asset::ETH,
                        mcycles_count,
                    )
                })
                .transpose()?;
            (None, None, max_price_cap)
        }
    };
    let lock_timeout = offer_spec.lock_timeout_ms_per_mcycle * mcycles_count / 1000;
    let timeout = offer_spec.timeout_ms_per_mcycle * mcycles_count / 1000;
    let ramp_up_period_secs = offer_spec
        .ramp_up_period_blocks
        .saturating_mul(block_time_sec);
    if ramp_up_period_secs > lock_timeout {
        return Err(RaikoError::InvalidRequestConfig(format!(
            "ramp_up_period_blocks={} exceeds lock timeout for {} mcycles",
            offer_spec.ramp_up_period_blocks, mcycles_count
        )));
    }

    Ok(ValidatedOfferParams {
        max_price,
        min_price,
        max_price_cap,
        lock_collateral: parse_amount(&offer_spec.lock_collateral, "lock_collateral", Asset::ZKC)
            .map_err(RaikoError::InvalidRequestConfig)?,
        lock_timeout,
        timeout,
        ramp_up_period_secs,
        bidding_start: SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            + u64::from(offer_spec.ramp_up_start_sec),
    })
}

#[cfg(test)]
mod tests {
    use super::config::default_batch_offer_params;
    use super::{
        BatchQuoteStrategy, BoundlessConfig, BoundlessPricingMode, BoundlessProver,
        DeploymentConfig, DeploymentType, ElfType, capped_market_max_price, parse_env_bool,
        parse_env_url, quote_batch_mcycles, user_cycles_to_mcycles, validate_offer_params,
    };
    use alloy_primitives::{U256, address, utils::parse_ether};
    use boundless_market::price_oracle::{Amount, Asset};
    use raiko2_primitives::Proof;

    fn sample_offer() -> super::BoundlessOfferParams {
        default_batch_offer_params()
    }

    #[test]
    fn quoted_mcycles_count_matches_raiko_agent_strategy() {
        let prover = BoundlessProver::new(BoundlessConfig::default());
        assert_eq!(prover.quoted_mcycles_count(ElfType::Batch, 1_491), 2_000);
        assert_eq!(
            prover.quoted_mcycles_count(ElfType::Aggregation, 123),
            BoundlessConfig::default().aggregation_quoted_mcycles
        );
    }

    #[test]
    fn quoted_mcycles_count_can_use_evaluated_cycles_directly() {
        let config = BoundlessConfig {
            batch_quote_strategy: BatchQuoteStrategy::Evaluated,
            ..Default::default()
        };
        let prover = BoundlessProver::new(config);
        assert_eq!(prover.quoted_mcycles_count(ElfType::Batch, 1_188), 1_188);
    }

    #[test]
    fn quoted_mcycles_count_can_use_fixed_override() {
        let config = BoundlessConfig {
            batch_quoted_mcycles: Some(1_500),
            batch_quote_strategy: BatchQuoteStrategy::Evaluated,
            ..Default::default()
        };
        let prover = BoundlessProver::new(config);
        assert_eq!(prover.quoted_mcycles_count(ElfType::Batch, 1_188), 1_500);
    }

    #[test]
    fn aggregation_quoted_mcycles_can_use_fixed_override() {
        let config = BoundlessConfig {
            aggregation_quoted_mcycles: 320,
            ..Default::default()
        };
        let prover = BoundlessProver::new(config);
        assert_eq!(
            prover.quoted_mcycles_count(ElfType::Aggregation, 1_188),
            320
        );
    }

    #[test]
    fn taiko_deployment_resolves_boundless_mainnet_contracts() {
        let config = BoundlessConfig {
            deployment: Some(DeploymentConfig {
                deployment_type: Some(DeploymentType::Taiko),
                overrides: None,
            }),
            ..Default::default()
        };
        let deployment = config.get_effective_deployment();

        assert_eq!(deployment.market_chain_id, Some(167_000));
        assert_eq!(
            deployment.boundless_market_address,
            address!("0xb3f5c7b4379052eade8c7f3fa6da37fb871da28b")
        );
        assert_eq!(
            deployment.verifier_router_address,
            Some(address!("0x607d196b43abc5d9BE3c7Fb8e336Ca82fec18C45"))
        );
        assert_eq!(
            deployment.set_verifier_address,
            address!("0x6135DC08D14EF8a44496B009e2181426628B8ebd")
        );
        assert_eq!(
            deployment.collateral_token_address,
            Some(address!("0xC284A781072442cC1882a8Db4573990B7B49DaC4"))
        );
        assert_eq!(
            deployment.order_stream_url.as_deref(),
            Some("https://taiko-mainnet.boundless.network")
        );
        assert_eq!(deployment.deployment_block, Some(4_819_525));
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
        let max_price = validated.max_price.expect("manual max price");
        let min_price = validated.min_price.expect("manual min price");
        assert_eq!(max_price.asset, Asset::ETH);
        assert_eq!(min_price.asset, Asset::ETH);
        assert!(validated.max_price_cap.is_none());
        assert!(max_price.value > min_price.value);
        assert_eq!(validated.ramp_up_period_secs, 120);
        assert!(validated.timeout > validated.lock_timeout);
    }

    #[test]
    fn validate_offer_params_omits_prices_for_market_pricing() {
        let mut offer = sample_offer();
        offer.pricing_mode = BoundlessPricingMode::Market;
        offer.max_price_per_mcycle = None;
        offer.min_price_per_mcycle = None;

        let validated = validate_offer_params(&offer, 1_000, 2).expect("valid offer");

        assert!(validated.max_price.is_none());
        assert!(validated.min_price.is_none());
        assert!(validated.max_price_cap.is_none());
        assert_eq!(validated.ramp_up_period_secs, 120);
        assert!(validated.timeout > validated.lock_timeout);
    }

    #[test]
    fn validate_offer_params_preserves_market_max_price_cap() {
        let mut offer = sample_offer();
        offer.pricing_mode = BoundlessPricingMode::Market;
        offer.max_price_per_mcycle = Some("0.00000006".to_string());
        offer.min_price_per_mcycle = None;

        let validated = validate_offer_params(&offer, 1_000, 2).expect("valid offer");
        let max_price_cap = validated.max_price_cap.expect("market max price cap");

        assert!(validated.max_price.is_none());
        assert!(validated.min_price.is_none());
        assert_eq!(max_price_cap.asset, Asset::ETH);
        assert_eq!(max_price_cap.value, parse_ether("0.00006").unwrap());
    }

    #[test]
    fn capped_market_max_price_keeps_autoprice_below_cap() {
        let max_price_cap = Amount::new(U256::from(100), Asset::ETH);

        assert_eq!(
            capped_market_max_price(U256::from(80), Some(&max_price_cap)),
            U256::from(80)
        );
    }

    #[test]
    fn capped_market_max_price_clamps_autoprice_above_cap() {
        let max_price_cap = Amount::new(U256::from(100), Asset::ETH);

        assert_eq!(
            capped_market_max_price(U256::from(120), Some(&max_price_cap)),
            U256::from(100)
        );
    }

    #[test]
    fn parse_env_bool_accepts_common_operator_values() {
        for value in ["true", "1", "yes", "y", "on"] {
            assert!(parse_env_bool("S3_PRESIGNED", value).expect("true value"));
        }
        for value in ["false", "0", "no", "n", "off"] {
            assert!(!parse_env_bool("S3_PRESIGNED", value).expect("false value"));
        }
    }

    #[test]
    fn parse_env_bool_rejects_invalid_values() {
        let err = parse_env_bool("S3_PRESIGNED", "maybe").expect_err("invalid bool");
        assert!(
            err.to_string()
                .contains("Invalid S3_PRESIGNED boolean value")
        );
    }

    #[test]
    fn parse_env_url_does_not_echo_secret_url() {
        let err = parse_env_url(
            "PINATA_API_URL",
            "https://user:pass@example.com:bad/path?token=1",
        )
        .expect_err("invalid url");
        let message = err.to_string();

        assert!(message.contains("Invalid PINATA_API_URL URL"));
        assert!(!message.contains("user:pass"));
        assert!(!message.contains("token=1"));
    }

    #[test]
    fn proof_to_envelope_preserves_risc0_seal_payload() {
        let expected_input_hash = alloy_primitives::hex::encode_prefixed([0x55; 32]);
        let envelope = crate::risc0_aggregation::proof_to_envelope(Proof {
            proof: Some("0x1234".to_string()),
            input: Some(alloy_primitives::B256::from([0x55; 32])),
            quote: Some("{\"receipt\":true}".to_string()),
            extra_data: Some(serde_json::json!({
                "proof_carry_data": {
                    "chain_id": 167_013
                }
            })),
            ..Default::default()
        });

        assert_eq!(envelope.payload.payload_kind, "risc0_seal");
        assert_eq!(envelope.payload.bytes, vec![0x12, 0x34]);
        assert_eq!(envelope.verifier_artifacts.len(), 1);
        assert_eq!(envelope.verifier_artifacts[0].kind, "receipt_json");
        assert_eq!(
            envelope.public_inputs.input_hash.as_deref(),
            Some(expected_input_hash.as_str())
        );
        assert_eq!(
            envelope.carry_data,
            Some(serde_json::json!({
                "proof_carry_data": {
                    "chain_id": 167_013
                }
            }))
        );
    }
}
