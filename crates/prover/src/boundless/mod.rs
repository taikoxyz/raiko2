#![allow(missing_docs)]

pub mod aggregation;

pub use crate::boundless_config::{
    BatchQuoteStrategy, BoundlessConfig, BoundlessOfferParams, BoundlessPricingMode,
    DeploymentConfig, DeploymentType, MIN_REBID_TIMEOUT_MS, OfferParamsConfig, validate_offer_spec,
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
    contracts::{RequestId, RequestStatus},
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
    ensure_shasta_proposal_input_matches_carry, parse_shasta_aggregation_input_hash,
    parse_shasta_proposal_input_hash, with_shasta_extra_data,
};

const MILLION_CYCLES: u64 = 1_000_000;
const BATCH_QUOTED_MCYCLES_MIN: u32 = 2_000;
const BATCH_QUOTED_MCYCLES_STEP: u32 = 1_000;
const AGGREGATION_QUOTED_MCYCLES_MIN: u32 = 200;
const AGGREGATION_QUOTED_MCYCLES_STEP: u32 = 100;
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

/// Number of rebid rungs applied at 1-based `attempt`, capped at `max_attempts`.
fn escalation_rungs(attempt: u64, max_attempts: u32) -> u64 {
    attempt.saturating_sub(1).min(u64::from(max_attempts))
}

/// Compound `base` by `step_bps` basis points per rebid rung, applied iteratively in U256 so the
/// magnitude stays bounded. `step_bps == 0` is a flat (no-escalation) ladder.
fn escalated_price(
    base: U256,
    attempt: u64,
    step_bps: u32,
    max_attempts: u32,
) -> RaikoResult<U256> {
    let rungs = escalation_rungs(attempt, max_attempts);
    let numer = U256::from(10_000u64 + u64::from(step_bps));
    let denom = U256::from(10_000u64);
    let mut price = base;
    for _ in 0..rungs {
        price = price
            .checked_mul(numer)
            .ok_or_else(|| {
                RaikoError::InvalidRequestConfig(format!(
                    "Boundless offer price {base} wei overflows escalating {step_bps} bps over {rungs} rebid rungs"
                ))
            })?
            / denom;
    }
    Ok(price)
}

/// Floored effective multiplier at `attempt`, for progress/metadata display only. Never used to
/// price an offer — pricing goes through [`escalated_price`] on the real base.
fn effective_price_multiplier(attempt: u64, step_bps: u32, max_attempts: u32) -> u32 {
    const SCALE: u64 = 1_000_000;
    let scaled = escalated_price(U256::from(SCALE), attempt, step_bps, max_attempts)
        .unwrap_or(U256::from(SCALE));
    u32::try_from(scaled / U256::from(SCALE)).unwrap_or(u32::MAX)
}

/// Attempt number to resume a stored submission at. Records carry the real 1-based attempt;
/// legacy records written before the field existed (`attempt == 0`) fall back to 1.
fn resume_attempt(submission: &Submission) -> u64 {
    submission.attempt.max(1)
}

const fn should_rebid_unlocked_request(attempt: u64, max_attempts: u32) -> bool {
    attempt > 0 && attempt <= max_attempts as u64
}

/// Total market submissions allowed per proof task: the initial attempt plus `max_attempts`
/// rebids. The no-lock path already enforces this bound through [`NoLockTimeoutAction::Abort`];
/// this check extends the same budget to the `Expired` and poll-timeout retry paths, which would
/// otherwise mint replacement requests without limit.
const fn exceeds_submission_budget(attempt: u64, max_attempts: u32) -> bool {
    attempt > max_attempts as u64 + 1
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NoLockTimeoutAction {
    Rebid,
    Abort,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NoLockTimeout {
    delay: Duration,
    action: NoLockTimeoutAction,
}

fn no_lock_timeout_for_attempt(
    attempt: u64,
    rebid_timeout_ms: u64,
    max_attempts: u32,
) -> NoLockTimeout {
    let action = if should_rebid_unlocked_request(attempt, max_attempts) {
        NoLockTimeoutAction::Rebid
    } else {
        NoLockTimeoutAction::Abort
    };
    NoLockTimeout {
        delay: Duration::from_millis(rebid_timeout_ms.max(MIN_REBID_TIMEOUT_MS)),
        action,
    }
}

/// Wall-clock deadline after which an unlocked submission is given up on.
///
/// Rebid attempts use the configured rebid delay: the request is abandoned early in favor of a
/// higher-priced resubmission. The final attempt (`Abort`) instead waits out the offer's lock
/// deadline when known: the market pays nothing for fulfillments past `lock_expires_at`, so that
/// is the exact end of the payable window — aborting sooner walks away from a live request the
/// client could still be charged for, and waiting longer cannot yield a paid fulfillment.
const fn no_lock_deadline(submitted_at: u64, lock_expires_at: u64, timeout: NoLockTimeout) -> u64 {
    let rebid_deadline = submitted_at.saturating_add(timeout.delay.as_secs());
    match timeout.action {
        NoLockTimeoutAction::Rebid => rebid_deadline,
        // `lock_expires_at == 0` means a legacy resume record; keep the old rebid-delay behavior.
        NoLockTimeoutAction::Abort => {
            if lock_expires_at > 0 {
                lock_expires_at
            } else {
                rebid_deadline
            }
        }
    }
}

const fn no_lock_deadline_elapsed(
    submitted_at: u64,
    lock_expires_at: u64,
    timeout: NoLockTimeout,
    now_secs: u64,
) -> bool {
    now_secs >= no_lock_deadline(submitted_at, lock_expires_at, timeout)
}

/// Whether the overall poll timeout must be deferred for the current submission.
///
/// On the final attempt (`Abort`) the request stays payable until its lock deadline, and a
/// timeout-triggered resubmission can reopen the double-pay window through the fresh-id
/// fallback, so the overall timeout only takes effect once the payable window has closed.
/// The deferral is bounded by `expires_at` so a corrupt stored record (or one with an
/// implausibly distant lock deadline) cannot keep the poll loop open forever.
const fn defer_poll_timeout_while_payable(
    submitted_at: u64,
    lock_expires_at: u64,
    expires_at: u64,
    timeout: NoLockTimeout,
    now_secs: u64,
) -> bool {
    matches!(timeout.action, NoLockTimeoutAction::Abort)
        && now_secs < expires_at
        && !no_lock_deadline_elapsed(submitted_at, lock_expires_at, timeout, now_secs)
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

const fn quote_aggregation_mcycles(evaluated_mcycles: u32) -> u32 {
    let rounded = if evaluated_mcycles == 0 {
        0
    } else {
        evaluated_mcycles.div_ceil(AGGREGATION_QUOTED_MCYCLES_STEP)
            * AGGREGATION_QUOTED_MCYCLES_STEP
    };
    if rounded < AGGREGATION_QUOTED_MCYCLES_MIN {
        AGGREGATION_QUOTED_MCYCLES_MIN
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

    /// Journal/seal/metadata discriminator: `ElfType::Batch` proves a "proposal", `Aggregation` an "aggregation".
    const fn proof_type_str(self) -> &'static str {
        match self {
            Self::Batch => "proposal",
            Self::Aggregation => "aggregation",
        }
    }

    const fn is_proposal(self) -> bool {
        matches!(self, Self::Batch)
    }
}

#[derive(Clone, Debug)]
struct UploadedProgram {
    image_id: Digest,
    url: Url,
    refresh_at: SystemTime,
}

#[derive(Clone, Debug)]
struct UploadedInput {
    url: Url,
    refresh_at: SystemTime,
}

/// Refresh deadline for a presigned upload URL: `X-Amz-Expires` seconds out (default 3600),
/// pulled back by 120s of headroom and floored at 60s so we re-upload before the URL dies.
fn presigned_refresh_at(url: &Url) -> SystemTime {
    let expires_secs = url
        .query_pairs()
        .find(|(key, _)| key.eq_ignore_ascii_case("X-Amz-Expires"))
        .and_then(|(_, value)| value.parse::<u64>().ok())
        .unwrap_or(3600);
    SystemTime::now() + Duration::from_secs(expires_secs.saturating_sub(120).max(60))
}

#[derive(Clone, Debug)]
struct Submission {
    market_request_id: U256,
    provider_request_id: String,
    remote_tx_hash: Option<String>,
    expires_at: u64,
    // Offer lock deadline (`rampUpStart + lockTimeout`). The market pays nothing for fulfillments
    // past this time, so it bounds the payable window; `0` when resumed from a legacy record.
    lock_expires_at: u64,
    submitted_at: u64,
    // Floored effective price multiplier at this attempt, for progress/metadata display only.
    // Derived from `attempt` + config via `effective_price_multiplier`; never used to price offers.
    max_price_multiplier: u32,
    // 1-based rebid attempt that produced this submission; persisted so restarts don't reset the
    // rebid budget when the price is flat (`rebid_price_step_bps == 0`).
    attempt: u64,
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
    attempt: u64,
    // Per-proof input-upload cache. Uploaded once (first attempt) and reused across rebids; a fresh
    // presigned URL is minted only when the cached one nears expiry (see `ensure_input_uploaded`).
    input_cache: &'a mut Option<UploadedInput>,
    // Market request id to reuse for this submission. Set on rebids so every rung of one proof
    // task shares an id: the market keys locks and paid fulfillments on the id, which makes
    // paying more than one rung impossible by construction. `None` mints a fresh id.
    reuse_request_id: Option<U256>,
}

enum BoundlessAttemptError {
    Retryable {
        reason: String,
        // Whether the next attempt must mint a fresh market request id. Rebid rungs of one proof
        // task share an id so the market can pay for at most one of them; rotation is only needed
        // once that id is dead (request expired, or a locked rung's deadline passed — the id can
        // never be locked again).
        rotate_request_id: bool,
    },
    Fatal(RaikoError),
}

impl From<RaikoError> for BoundlessAttemptError {
    fn from(value: RaikoError) -> Self {
        Self::Fatal(value)
    }
}

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
                    lock_expires_at: submission.lock_expires_at,
                    image_ref: image_ref.to_string(),
                    deployment: deployment.to_string(),
                    offchain,
                    quoted_mcycles_count: Some(quoted_mcycles_count),
                    evaluated_mcycles_count: Some(evaluated_mcycles_count),
                    submitted_at: submission.submitted_at,
                    max_price_multiplier: submission.max_price_multiplier,
                    rebid_attempt: u32::try_from(submission.attempt).unwrap_or(u32::MAX),
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
        if value.submitted_at == 0 {
            return Err(RaikoError::Guest(
                "Invalid stored Boundless submission: missing submitted_at".to_string(),
            ));
        }
        Ok(Self {
            market_request_id,
            provider_request_id: value.provider_request_id,
            remote_tx_hash: value.remote_tx_hash,
            expires_at: value.expires_at,
            lock_expires_at: value.lock_expires_at,
            submitted_at: value.submitted_at,
            max_price_multiplier: value.max_price_multiplier.max(1),
            attempt: u64::from(value.rebid_attempt),
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
    let selected = env_var("BOUNDLESS_STORAGE_UPLOADER")
        .or_else(|| env_var("STORAGE_UPLOADER"))
        .map(|value| value.to_ascii_lowercase());
    config.storage_uploader = match selected.as_deref() {
        Some("s3") => StorageUploaderType::S3,
        Some("gcs") => StorageUploaderType::Gcs,
        Some("pinata") => StorageUploaderType::Pinata,
        Some("file") => StorageUploaderType::File,
        Some("none") => StorageUploaderType::None,
        Some(other) => {
            return Err(RaikoError::InvalidRequestConfig(format!(
                "Invalid BOUNDLESS_STORAGE_UPLOADER/STORAGE_UPLOADER value {other}"
            )));
        }
        None if env_var("S3_BUCKET").is_some() => StorageUploaderType::S3,
        None if env_var("GCS_BUCKET").is_some() => StorageUploaderType::Gcs,
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
    config.gcs_bucket = env_var("GCS_BUCKET");
    config.gcs_url = env_var("GCS_URL");
    config.gcs_credentials_json = env_var("GCS_CREDENTIALS_JSON");
    let gcs_public_url = env_bool("GCS_PUBLIC_URL")?;
    config.gcs_public_url = if config.storage_uploader == StorageUploaderType::Gcs {
        Some(gcs_public_url.unwrap_or(false))
    } else {
        gcs_public_url
    };
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
        let refresh_at = presigned_refresh_at(&url);
        let program = UploadedProgram {
            image_id,
            url,
            refresh_at,
        };
        self.programs
            .write()
            .await
            .insert(elf_type, program.clone());
        Ok(program)
    }

    async fn ensure_input_uploaded(
        &self,
        client: &Client,
        guest_env_bytes: &[u8],
        cache: &mut Option<UploadedInput>,
    ) -> RaikoResult<Url> {
        if let Some(input) = cache.as_ref()
            && SystemTime::now() < input.refresh_at
        {
            return Ok(input.url.clone());
        }
        let url = retry_external("upload boundless input", || async {
            client.upload_input(guest_env_bytes).await.map_err(|e| {
                RaikoError::InvalidRequestConfig(format!("Failed to upload boundless input: {e}"))
            })
        })
        .await?;
        let refresh_at = presigned_refresh_at(&url);
        *cache = Some(UploadedInput {
            url: url.clone(),
            refresh_at,
        });
        Ok(url)
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
        elf: &[u8],
        program: &UploadedProgram,
        offer_spec: &BoundlessOfferParams,
        mcycles_count: u32,
        journal: Vec<u8>,
        attempt: u64,
        input_url: Url,
        reuse_request_id: Option<U256>,
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
        // Escalate only the manual max-price cap on resubmissions; the min price keeps the ramp
        // start unchanged so an idle prover still locks cheaply.
        let max_price = max_price
            .map(|mut amount| -> RaikoResult<_> {
                amount.value = escalated_price(
                    amount.value,
                    attempt,
                    self.config.rebid_price_step_bps,
                    self.config.rebid_max_attempts,
                )?;
                Ok(amount)
            })
            .transpose()?;
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
        if let Some(reuse_request_id) = reuse_request_id {
            let request_id = RequestId::try_from(reuse_request_id).map_err(|e| {
                RaikoError::Guest(format!(
                    "Invalid boundless request id 0x{reuse_request_id:x} to reuse: {e}"
                ))
            })?;
            request_params = request_params.with_request_id(request_id);
        }
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
        apply_market_offer_pricing(
            &mut request,
            offer_spec.pricing_mode,
            attempt,
            self.config.rebid_price_step_bps,
            self.config.rebid_max_attempts,
            max_price_cap.as_ref(),
            mcycles_count,
        )?;
        Ok(request)
    }

    async fn submit_request_offchain(
        &self,
        client: &Client,
        request: &ProofRequest,
        attempt: u64,
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
            lock_expires_at: request.lock_expires_at(),
            submitted_at: now_secs(),
            max_price_multiplier: effective_price_multiplier(
                attempt,
                self.config.rebid_price_step_bps,
                self.config.rebid_max_attempts,
            ),
            attempt,
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
        attempt: u64,
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
            lock_expires_at: request.lock_expires_at(),
            submitted_at: now_secs(),
            max_price_multiplier: effective_price_multiplier(
                attempt,
                self.config.rebid_price_step_bps,
                self.config.rebid_max_attempts,
            ),
            attempt,
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
        let input_url = self
            .ensure_input_uploaded(context.client, &guest_env_bytes, context.input_cache)
            .await?;
        let request = Box::pin(self.build_request(
            context.client,
            guest_env,
            context.elf,
            context.program,
            context.offer_spec,
            context.quoted_mcycles_count,
            context.journal.to_vec(),
            context.attempt,
            input_url,
            context.reuse_request_id,
        ))
        .await?;

        if self.config.offchain {
            let submission = self
                .submit_request_offchain(context.client, &request, context.attempt)
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
            context.attempt,
        )
        .await
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    async fn poll_until_fulfilled(
        &self,
        client: &Client,
        submission: &Submission,
        elf_type: ElfType,
        image_id: Digest,
        block_image_id: Option<Digest>,
        expected_input_hash: B256,
        quoted_mcycles_count: u32,
        evaluated_mcycles_count: u32,
        proposal_carry_data: Option<&ProofCarryData>,
        no_lock_timeout: NoLockTimeout,
    ) -> Result<Proof, BoundlessAttemptError> {
        let poll_interval = Duration::from_millis(self.config.poll_interval_ms.max(1));
        let timeout = Duration::from_millis(self.config.timeout_ms.max(1));
        let started_at = Instant::now();
        let mut last_poll_error: Option<String> = None;
        let mut consecutive_poll_errors = 0_u32;

        loop {
            let now = now_secs();
            if started_at.elapsed() > timeout
                && !defer_poll_timeout_while_payable(
                    submission.submitted_at,
                    submission.lock_expires_at,
                    submission.expires_at,
                    no_lock_timeout,
                    now,
                )
            {
                let detail = last_poll_error
                    .as_deref()
                    .map(|error| format!("; last polling error: {error}"))
                    .unwrap_or_default();
                // Before the request expiry the rungs may still be payable, so keep the shared
                // id: a replacement rung under the same id cannot be double-paid. Past the
                // expiry every rung is past its lock deadline (nothing on this id is payable),
                // and the id may even carry a dead lock that the failing status reads could not
                // surface, so treat this like the Expired branch and rotate.
                return Err(BoundlessAttemptError::Retryable {
                    reason: format!(
                        "Boundless request {} timed out before fulfillment{detail}",
                        submission.provider_request_id
                    ),
                    rotate_request_id: now >= submission.expires_at,
                });
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
                RequestStatus::Unknown => {
                    if no_lock_deadline_elapsed(
                        submission.submitted_at,
                        submission.lock_expires_at,
                        no_lock_timeout,
                        now_secs(),
                    ) {
                        return match no_lock_timeout.action {
                            // Escalate under the same market request id: the market pays for at
                            // most one request per id, so the abandoned cheaper rung can never be
                            // paid in addition to its replacement.
                            NoLockTimeoutAction::Rebid => Err(BoundlessAttemptError::Retryable {
                                reason: format!(
                                    "Boundless request {} was not locked within {} seconds; \
                                     rebidding with higher max price under the same request id",
                                    submission.provider_request_id,
                                    no_lock_timeout.delay.as_secs()
                                ),
                                rotate_request_id: false,
                            }),
                            NoLockTimeoutAction::Abort => {
                                // Legacy resume records (lock_expires_at == 0) abort on the
                                // rebid-delay fallback, not the offer's lock deadline; report
                                // whichever deadline actually elapsed.
                                let deadline_detail = if submission.lock_expires_at > 0 {
                                    "before its payable window closed".to_string()
                                } else {
                                    format!("within {} seconds", no_lock_timeout.delay.as_secs())
                                };
                                Err(BoundlessAttemptError::Fatal(RaikoError::Guest(format!(
                                    "Boundless request {} was not locked {deadline_detail}; \
                                     exhausted boundless no-lock rebids",
                                    submission.provider_request_id
                                ))))
                            }
                        };
                    }
                }
                RequestStatus::Locked => {}
                RequestStatus::Expired => {
                    // Expired covers both a fully timed-out request and a locked rung whose
                    // deadline passed. Either way the id is dead for paid fulfillment (a lock is
                    // never re-grantable), so the next attempt must mint a fresh id.
                    return Err(BoundlessAttemptError::Retryable {
                        reason: format!(
                            "Boundless request {} expired before fulfillment",
                            submission.provider_request_id
                        ),
                        rotate_request_id: true,
                    });
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
                    let receipt_json = if elf_type.is_proposal() {
                        match decode_seal(seal.clone(), image_id, journal.to_vec()) {
                            Ok(ContractReceipt::Base(receipt)) => {
                                serde_json::to_string(&receipt).ok()
                            }
                            Ok(ContractReceipt::SetInclusion(_)) | Err(_) => None,
                        }
                    } else {
                        None
                    };
                    let input_hash = if elf_type.is_proposal() {
                        parse_shasta_proposal_input_hash(journal)?
                    } else {
                        parse_shasta_aggregation_input_hash(journal)?
                    };
                    if let (true, Some(carry)) = (elf_type.is_proposal(), proposal_carry_data) {
                        ensure_shasta_proposal_input_matches_carry(input_hash, carry, "boundless")?;
                    }
                    if input_hash != expected_input_hash {
                        return Err(BoundlessAttemptError::Fatal(RaikoError::Guest(
                            "Boundless fulfillment journal does not match local dry-run journal"
                                .to_string(),
                        )));
                    }
                    let stage_metadata = serde_json::json!({
                                    "zkvm": "risc0",
                                    "runner": "network",
                                    "proof_type": elf_type.proof_type_str(),
                                    "mcycles_count": quoted_mcycles_count,
                                    "quoted_mcycles_count": quoted_mcycles_count,
                                    "evaluated_mcycles_count": evaluated_mcycles_count,
                        "boundless": {
                            "provider_request_id": submission.provider_request_id,
                            "remote_tx_hash": submission.remote_tx_hash,
                            "expires_at": submission.expires_at,
                            "lock_expires_at": submission.lock_expires_at,
                            "submitted_at": submission.submitted_at,
                            "max_price_multiplier": submission.max_price_multiplier,
                            "image_id": alloy_primitives::hex::encode_prefixed(image_id.as_bytes()),
                            "deployment": format!("{:?}", self.config.get_deployment_type()).to_lowercase(),
                            "offchain": self.config.offchain,
                        }
                    });
                    let extra_data = match (elf_type.is_proposal(), proposal_carry_data) {
                        (true, Some(carry)) => {
                            with_shasta_extra_data(carry, "risc0", Some(stage_metadata))?
                        }
                        _ => Some(stage_metadata),
                    };
                    let proof = if elf_type.is_proposal() {
                        encode_risc0_proposal_seal_payload(
                            &seal,
                            B256::from_slice(image_id.as_bytes()),
                        )
                    } else {
                        encode_risc0_aggregation_seal_payload(
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
                        )
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

    #[allow(
        clippy::large_futures,
        clippy::too_many_arguments,
        clippy::too_many_lines
    )]
    async fn prove_boundless(
        &self,
        elf_type: ElfType,
        offer_spec: &BoundlessOfferParams,
        input: Bytes,
        elf: &[u8],
        block_image_id: Option<Digest>,
        proposal_carry_data: Option<ProofCarryData>,
        observer: Option<Arc<dyn ProverProgressObserver>>,
    ) -> RaikoResult<Proof> {
        let client = retry_external("create boundless client", || self.create_client()).await?;
        // Seed the program cache (and derive the stable image ref) up front; the per-attempt
        // refresh below is a cache hit unless the presigned URL nears expiry.
        let program = self.ensure_uploaded(&client, elf_type, elf).await?;
        // Local RISC0 dry-run can take seconds to minutes for large inputs and must not
        // occupy the async runtime threads that serve health/readiness probes.
        let (evaluated_mcycles_count, journal) =
            Self::evaluate_guest(input.to_vec(), self.config.execution_po2, elf.to_vec()).await?;
        let expected_input_hash = if elf_type.is_proposal() {
            parse_shasta_proposal_input_hash(&journal)?
        } else {
            parse_shasta_aggregation_input_hash(&journal)?
        };
        let quoted_mcycles_count = self.quoted_mcycles_count(elf_type, evaluated_mcycles_count);
        // The image id is deterministic in `elf`, so it stays stable across program URL refreshes.
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
        let mut last_retry_reason: Option<String> = None;
        // Market request id shared by rebid rungs of this proof task. The market keys locks and
        // paid fulfillments on the id, so reusing it across rebids makes paying for more than one
        // rung impossible by construction. `None` mints a fresh id (first attempt, or after the
        // previous id became unusable).
        let mut reuse_request_id: Option<U256> = None;
        // Per-proof input-upload cache: the guest env is uploaded once and reused across rebids,
        // refreshed only when its presigned URL nears expiry.
        let mut input_cache: Option<UploadedInput> = None;

        loop {
            // Refresh the program URL per attempt (cheap cache hit unless a refresh is due) so late
            // rebids never carry an expired presigned program URL. The image id is unchanged.
            let program = self.ensure_uploaded(&client, elf_type, elf).await?;
            let submission = if let Some(submission) = resume_submission.take() {
                attempt = attempt.max(resume_attempt(&submission));
                // Expired records are deliberately not short-circuited: the poll below gives
                // them one final market status read. An expired-but-fulfilled request still
                // reports Fulfilled (the SDK checks fulfillment before expiry), recovering a
                // proof that is already paid for; an expired-unfulfilled one classifies as
                // Expired and takes the normal retry arm, which escalates the attempt and
                // draws down the submission budget.
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
                if exceeds_submission_budget(attempt, self.config.rebid_max_attempts) {
                    let detail = last_retry_reason
                        .as_deref()
                        .map(|reason| format!("; last attempt: {reason}"))
                        .unwrap_or_default();
                    return Err(RaikoError::Guest(format!(
                        "Exhausted Boundless submission budget of {} attempts \
                         (rebid_max_attempts = {}){detail}",
                        u64::from(self.config.rebid_max_attempts) + 1,
                        self.config.rebid_max_attempts,
                    )));
                }
                // `input_cache` is threaded by `&mut` so the reused-id attempt and the fresh-id
                // fallback below share a single per-proof input upload. The context is built inline
                // (rather than via a closure) because it borrows `&mut input_cache`, whose lifetime
                // a closure returning the borrow cannot name.
                let first = Box::pin(self.submit_fresh_request(FreshSubmissionContext {
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
                    attempt,
                    input_cache: &mut input_cache,
                    reuse_request_id,
                }))
                .await;
                match first {
                    Ok(submission) => submission,
                    Err(error) => {
                        // Id reuse is best-effort: an order stream or RPC may refuse a same-id
                        // resubmission. Fall back to a fresh id (the pre-reuse behavior, which
                        // re-opens the double-pay window for this rung) instead of failing the
                        // proof.
                        let Some(previous_request_id) = reuse_request_id.take() else {
                            return Err(error);
                        };
                        tracing::warn!(
                            previous_request_id = format!("0x{previous_request_id:x}"),
                            error = %error,
                            "Boundless rebid under the reused request id failed; retrying once with a fresh id"
                        );
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
                            attempt,
                            input_cache: &mut input_cache,
                            reuse_request_id: None,
                        }))
                        .await?
                    }
                }
            };

            tracing::info!(
                provider_request_id = %submission.provider_request_id,
                expires_at = submission.expires_at,
                attempt,
                max_price_multiplier = submission.max_price_multiplier,
                "Using Boundless market submission"
            );
            let no_lock_timeout = no_lock_timeout_for_attempt(
                attempt,
                self.config.rebid_timeout_ms,
                self.config.rebid_max_attempts,
            );

            match self
                .poll_until_fulfilled(
                    &client,
                    &submission,
                    elf_type,
                    program.image_id,
                    block_image_id,
                    expected_input_hash,
                    quoted_mcycles_count,
                    evaluated_mcycles_count,
                    proposal_carry_data.as_ref(),
                    no_lock_timeout,
                )
                .await
            {
                Ok(proof) => return Ok(proof),
                Err(BoundlessAttemptError::Retryable {
                    reason,
                    rotate_request_id,
                }) => {
                    tracing::warn!(
                        provider_request_id = %submission.provider_request_id,
                        attempt,
                        reason,
                        rotate_request_id,
                        "Boundless submission did not finish; retrying"
                    );
                    reuse_request_id = if rotate_request_id {
                        None
                    } else {
                        Some(submission.market_request_id)
                    };
                    last_retry_reason = Some(reason);
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
    fn quoted_mcycles_count(&self, elf_type: ElfType, evaluated_mcycles_count: u32) -> u32 {
        match elf_type {
            ElfType::Batch => {
                if let Some(batch_quoted_mcycles) = self.config.batch_quoted_mcycles {
                    batch_quoted_mcycles
                } else {
                    match self.config.batch_quote_strategy.clone() {
                        BatchQuoteStrategy::RaikoAgent => {
                            quote_batch_mcycles(evaluated_mcycles_count)
                        }
                        BatchQuoteStrategy::Evaluated => evaluated_mcycles_count,
                    }
                }
            }
            ElfType::Aggregation => {
                if let Some(aggregation_quoted_mcycles) = self.config.aggregation_quoted_mcycles {
                    aggregation_quoted_mcycles
                } else {
                    match self.config.aggregation_quote_strategy.clone() {
                        BatchQuoteStrategy::RaikoAgent => {
                            quote_aggregation_mcycles(evaluated_mcycles_count)
                        }
                        BatchQuoteStrategy::Evaluated => evaluated_mcycles_count,
                    }
                }
            }
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

/// Escalated and capped market-mode offer prices derived from the SDK's autopriced offer.
#[derive(Debug, PartialEq, Eq)]
struct MarketOfferPrices {
    max_price: U256,
    min_price: U256,
    clamped_to_cap: bool,
}

/// Escalate the autopriced max price by the rebid multiplier, then clamp it to the configured
/// per-mcycle cap.
///
/// Manual pricing escalates its configured max price in [`validate_offer_params`]; this is the
/// market-mode counterpart, applied after the SDK autoprices the offer — without it, market-mode
/// rebids would just resubmit at whatever the SDK quotes again, repeating an offer the market
/// already declined to lock. Only the max price escalates; the min price keeps the ramp start
/// unchanged so an idle prover still locks cheaply, and is lowered only when the clamped max
/// falls below it so the offer stays well-formed. Clamping (rather than rejecting) keeps an
/// autoprice spike from turning into a proving outage: the offer bids at the operator's ceiling,
/// and the no-lock rebid machinery handles a ceiling the market will not clear.
fn escalate_and_cap_market_prices(
    autopriced_max: U256,
    autopriced_min: U256,
    attempt: u64,
    step_bps: u32,
    max_attempts: u32,
    max_price_cap: Option<&Amount>,
) -> RaikoResult<MarketOfferPrices> {
    let escalated_max = escalated_price(autopriced_max, attempt, step_bps, max_attempts)?;
    let (max_price, clamped_to_cap) = match max_price_cap {
        Some(cap) if escalated_max > cap.value => (cap.value, true),
        _ => (escalated_max, false),
    };
    Ok(MarketOfferPrices {
        max_price,
        min_price: autopriced_min.min(max_price),
        clamped_to_cap,
    })
}

fn apply_market_offer_pricing(
    request: &mut ProofRequest,
    pricing_mode: BoundlessPricingMode,
    attempt: u64,
    step_bps: u32,
    max_attempts: u32,
    max_price_cap: Option<&Amount>,
    mcycles_count: u32,
) -> RaikoResult<()> {
    if pricing_mode != BoundlessPricingMode::Market {
        return Ok(());
    }
    let autopriced_max = request.offer.maxPrice;
    let prices = escalate_and_cap_market_prices(
        autopriced_max,
        request.offer.minPrice,
        attempt,
        step_bps,
        max_attempts,
        max_price_cap,
    )?;
    if prices.clamped_to_cap {
        tracing::warn!(
            mcycles_count,
            autopriced_max_price_wei = %autopriced_max,
            capped_max_price_wei = %prices.max_price,
            "Boundless market offer max price exceeds the max_price_per_mcycle cap; bidding at the cap"
        );
    } else if escalation_rungs(attempt, max_attempts) > 0 {
        tracing::info!(
            mcycles_count,
            autopriced_max_price_wei = %autopriced_max,
            escalated_max_price_wei = %prices.max_price,
            "Escalated Boundless market offer max price for rebid"
        );
    }
    request.offer.maxPrice = prices.max_price;
    request.offer.minPrice = prices.min_price;
    Ok(())
}

fn apply_dynamic_pricing_timeout_modifier(
    offer_spec: &BoundlessOfferParams,
    timeout: u32,
    field: &str,
) -> RaikoResult<u32> {
    if offer_spec.pricing_mode != BoundlessPricingMode::Market {
        return Ok(timeout);
    }
    let Some(modifier) = offer_spec.dynamic_pricing_timeout_modifier else {
        return Ok(timeout);
    };

    let modified_timeout = scale_timeout(timeout, modifier, field)?;

    tracing::debug!(
        modifier,
        timeout,
        modified_timeout,
        field,
        "Applied Boundless dynamic-pricing timeout modifier"
    );
    Ok(modified_timeout)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn scale_timeout(value: u32, modifier: f64, field: &str) -> RaikoResult<u32> {
    if !modifier.is_finite() || modifier < 1.0 {
        return Err(RaikoError::InvalidRequestConfig(
            "dynamic_pricing_timeout_modifier must be a finite number greater than or equal to 1.0"
                .to_string(),
        ));
    }
    let scaled_timeout = (f64::from(value) * modifier).ceil();
    if scaled_timeout > f64::from(u32::MAX) {
        return Err(RaikoError::InvalidRequestConfig(format!(
            "dynamic_pricing_timeout_modifier overflows {field}"
        )));
    }
    Ok(scaled_timeout as u32)
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
            // The base (un-escalated) max price; `build_request` escalates it per rebid rung.
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
    let derived_lock_timeout = offer_spec.lock_timeout_ms_per_mcycle * mcycles_count / 1000;
    let derived_timeout = offer_spec.timeout_ms_per_mcycle * mcycles_count / 1000;
    let lock_timeout = match offer_spec.lock_timeout_secs {
        Some(lock_timeout) => lock_timeout,
        None => apply_dynamic_pricing_timeout_modifier(
            offer_spec,
            derived_lock_timeout,
            "lock_timeout",
        )?,
    };
    let timeout = match offer_spec.timeout_secs {
        Some(timeout) => timeout,
        None => apply_dynamic_pricing_timeout_modifier(offer_spec, derived_timeout, "timeout")?,
    };
    if timeout <= lock_timeout {
        return Err(RaikoError::InvalidRequestConfig(format!(
            "timeout {timeout}s must be greater than lock timeout {lock_timeout}s for {mcycles_count} mcycles"
        )));
    }
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
    use super::{
        BatchQuoteStrategy, BoundlessConfig, BoundlessPricingMode, BoundlessProver,
        DeploymentConfig, DeploymentType, ElfType, MIN_REBID_TIMEOUT_MS, NoLockTimeoutAction,
        defer_poll_timeout_while_payable, escalate_and_cap_market_prices,
        exceeds_submission_budget, no_lock_deadline_elapsed, no_lock_timeout_for_attempt,
        parse_env_bool, parse_env_url, quote_batch_mcycles, should_rebid_unlocked_request,
        storage_uploader_config_from_env, user_cycles_to_mcycles, validate_offer_params,
    };
    use crate::boundless_config::default_batch_offer_params;
    use alloy_primitives::{U256, address, utils::parse_ether};
    use boundless_market::{
        price_oracle::{Amount, Asset},
        storage::StorageUploaderType,
    };
    use raiko2_primitives::Proof;
    use std::{
        env,
        sync::{Mutex, MutexGuard},
        time::{Duration, SystemTime},
    };
    use url::Url;

    static ENV_LOCK: Mutex<()> = Mutex::new(());
    const TEST_REBID_TIMEOUT_MS: u64 = 300_000;
    const TEST_REBID_PRICE_STEP_BPS: u32 = 5000;
    const TEST_REBID_MAX_ATTEMPTS: u32 = 4;

    const STORAGE_ENV_VARS: &[&str] = &[
        "BOUNDLESS_STORAGE_UPLOADER",
        "STORAGE_UPLOADER",
        "S3_BUCKET",
        "S3_URL",
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "AWS_REGION",
        "S3_PRESIGNED",
        "S3_PUBLIC_URL",
        "GCS_BUCKET",
        "GCS_URL",
        "GCS_CREDENTIALS_JSON",
        "GCS_PUBLIC_URL",
        "PINATA_JWT",
        "PINATA_API_URL",
        "IPFS_GATEWAY_URL",
        "FILE_PATH",
    ];

    #[allow(unsafe_code)]
    fn set_test_env_var(name: &str, value: &str) {
        // SAFETY: StorageEnvGuard serializes all env mutation in these tests.
        unsafe {
            env::set_var(name, value);
        }
    }

    #[allow(unsafe_code)]
    fn remove_test_env_var(name: &str) {
        // SAFETY: StorageEnvGuard serializes all env mutation in these tests.
        unsafe {
            env::remove_var(name);
        }
    }

    struct StorageEnvGuard {
        originals: Vec<(&'static str, Option<String>)>,
        _lock: MutexGuard<'static, ()>,
    }

    impl StorageEnvGuard {
        fn new(vars: &[(&'static str, &str)]) -> Self {
            let lock = ENV_LOCK.lock().expect("storage env lock");
            let originals = STORAGE_ENV_VARS
                .iter()
                .map(|name| (*name, env::var(name).ok()))
                .collect();

            for name in STORAGE_ENV_VARS {
                remove_test_env_var(name);
            }
            for (name, value) in vars {
                set_test_env_var(name, value);
            }

            Self {
                originals,
                _lock: lock,
            }
        }
    }

    impl Drop for StorageEnvGuard {
        fn drop(&mut self) {
            for (name, original) in &self.originals {
                if let Some(value) = original {
                    set_test_env_var(name, value);
                } else {
                    remove_test_env_var(name);
                }
            }
        }
    }

    fn sample_offer() -> super::BoundlessOfferParams {
        default_batch_offer_params()
    }

    #[test]
    fn elf_type_maps_to_proof_type_and_stage() {
        assert_eq!(ElfType::Batch.proof_type_str(), "proposal");
        assert_eq!(ElfType::Aggregation.proof_type_str(), "aggregation");
        assert_eq!(ElfType::Batch.stage_name(), "batch");
        assert_eq!(ElfType::Aggregation.stage_name(), "aggregation");
        assert!(ElfType::Batch.is_proposal());
        assert!(!ElfType::Aggregation.is_proposal());
    }

    #[test]
    fn quoted_mcycles_count_matches_raiko_agent_strategy() {
        let prover = BoundlessProver::new(BoundlessConfig::default());
        assert_eq!(prover.quoted_mcycles_count(ElfType::Batch, 1_491), 2_000);
    }

    #[test]
    fn aggregation_quoted_mcycles_matches_raiko_agent_strategy() {
        let prover = BoundlessProver::new(BoundlessConfig::default());
        assert_eq!(prover.quoted_mcycles_count(ElfType::Aggregation, 0), 200);
        assert_eq!(prover.quoted_mcycles_count(ElfType::Aggregation, 123), 200);
        assert_eq!(prover.quoted_mcycles_count(ElfType::Aggregation, 200), 200);
        assert_eq!(prover.quoted_mcycles_count(ElfType::Aggregation, 201), 300);
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
    fn aggregation_quoted_mcycles_can_use_evaluated_cycles_directly() {
        let config = BoundlessConfig {
            aggregation_quote_strategy: BatchQuoteStrategy::Evaluated,
            ..Default::default()
        };
        let prover = BoundlessProver::new(config);
        assert_eq!(
            prover.quoted_mcycles_count(ElfType::Aggregation, 1_188),
            1_188
        );
    }

    #[test]
    fn aggregation_quoted_mcycles_can_use_fixed_override() {
        let config = BoundlessConfig {
            aggregation_quoted_mcycles: Some(320),
            aggregation_quote_strategy: BatchQuoteStrategy::Evaluated,
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
    fn escalated_price_compounds_by_step_per_rung() {
        let base = U256::from(100u64);
        let bps = TEST_REBID_PRICE_STEP_BPS; // +50%
        let max = TEST_REBID_MAX_ATTEMPTS; // 4
        assert_eq!(
            super::escalated_price(base, 1, bps, max).unwrap(),
            U256::from(100u64)
        );
        assert_eq!(
            super::escalated_price(base, 2, bps, max).unwrap(),
            U256::from(150u64)
        );
        assert_eq!(
            super::escalated_price(base, 3, bps, max).unwrap(),
            U256::from(225u64)
        );
        assert_eq!(
            super::escalated_price(base, 4, bps, max).unwrap(),
            U256::from(337u64)
        ); // 337.5 truncated
        assert_eq!(
            super::escalated_price(base, 5, bps, max).unwrap(),
            U256::from(505u64)
        ); // 505.5 truncated
        // Capped at max_attempts rungs.
        assert_eq!(
            super::escalated_price(base, 6, bps, max).unwrap(),
            super::escalated_price(base, 5, bps, max).unwrap()
        );
    }

    #[test]
    fn escalated_price_zero_step_is_flat() {
        let base = U256::from(1000u64);
        assert_eq!(super::escalated_price(base, 5, 0, 4).unwrap(), base);
    }

    #[test]
    fn effective_price_multiplier_floors_the_ratio() {
        let bps = TEST_REBID_PRICE_STEP_BPS;
        let max = TEST_REBID_MAX_ATTEMPTS;
        assert_eq!(super::effective_price_multiplier(1, bps, max), 1);
        assert_eq!(super::effective_price_multiplier(2, bps, max), 1); // 1.5 -> 1
        assert_eq!(super::effective_price_multiplier(3, bps, max), 2); // 2.25 -> 2
        assert_eq!(super::effective_price_multiplier(4, bps, max), 3); // 3.375 -> 3
        assert_eq!(super::effective_price_multiplier(5, bps, max), 5); // 5.06 -> 5
    }

    fn test_submission(max_price_multiplier: u32, attempt: u64) -> super::Submission {
        super::Submission {
            market_request_id: U256::from(1u64),
            provider_request_id: "0x1".to_string(),
            remote_tx_hash: None,
            expires_at: 1,
            lock_expires_at: 1,
            submitted_at: 1,
            max_price_multiplier,
            attempt,
        }
    }

    #[test]
    fn resume_attempt_uses_persisted_attempt() {
        // The persisted attempt is the sole source of truth.
        assert_eq!(super::resume_attempt(&test_submission(4, 3)), 3);
        // A legacy record without a persisted attempt (0) falls back to 1.
        assert_eq!(super::resume_attempt(&test_submission(1, 0)), 1);
    }

    #[test]
    fn resumed_submission_carries_lock_deadline() {
        let resume = crate::BoundlessSubmissionResume {
            provider_request_id: "0x1".to_string(),
            remote_tx_hash: None,
            expires_at: 2_000,
            lock_expires_at: 1_500,
            submitted_at: 1_000,
            max_price_multiplier: 1,
            rebid_attempt: 1,
        };
        let submission = super::Submission::try_from(resume).expect("valid resume record");
        assert_eq!(submission.lock_expires_at, 1_500);

        // Legacy records deserialize with lock_expires_at == 0 and must stay accepted.
        let legacy: crate::BoundlessSubmissionResume = serde_json::from_value(serde_json::json!({
            "provider_request_id": "0x1",
            "remote_tx_hash": null,
            "expires_at": 2_000,
            "submitted_at": 1_000,
            "max_price_multiplier": 1,
        }))
        .expect("legacy record without lock_expires_at");
        assert_eq!(legacy.lock_expires_at, 0);
        let submission = super::Submission::try_from(legacy).expect("valid legacy record");
        assert_eq!(submission.lock_expires_at, 0);
    }

    #[test]
    fn no_lock_deadline_uses_submission_wall_clock() {
        let timeout =
            no_lock_timeout_for_attempt(1, TEST_REBID_TIMEOUT_MS, TEST_REBID_MAX_ATTEMPTS);

        // Rebid attempts abandon the request after the configured rebid delay; the offer's lock
        // deadline (here far in the future) must not stretch that window.
        assert!(!no_lock_deadline_elapsed(1_000, 10_000, timeout, 1_299));
        assert!(no_lock_deadline_elapsed(1_000, 10_000, timeout, 1_300));
        assert!(no_lock_deadline_elapsed(1_000, 10_000, timeout, 1_600));
    }

    #[test]
    fn final_attempt_waits_for_the_offer_lock_deadline() {
        // Attempt 5 exhausts the rebid budget (action = Abort). The request stays payable until
        // the offer's lock deadline, so the final attempt must wait it out rather than walking
        // away from a live request after only the rebid delay.
        let timeout =
            no_lock_timeout_for_attempt(5, TEST_REBID_TIMEOUT_MS, TEST_REBID_MAX_ATTEMPTS);
        assert_eq!(timeout.action, NoLockTimeoutAction::Abort);

        assert!(!no_lock_deadline_elapsed(1_000, 10_000, timeout, 1_300));
        assert!(!no_lock_deadline_elapsed(1_000, 10_000, timeout, 9_999));
        assert!(no_lock_deadline_elapsed(1_000, 10_000, timeout, 10_000));
    }

    #[test]
    fn final_attempt_falls_back_to_rebid_delay_without_lock_deadline() {
        // Legacy resume records predate the lock_expires_at field (stored as 0); keep the old
        // rebid-delay behavior for them.
        let timeout =
            no_lock_timeout_for_attempt(5, TEST_REBID_TIMEOUT_MS, TEST_REBID_MAX_ATTEMPTS);
        assert_eq!(timeout.action, NoLockTimeoutAction::Abort);

        assert!(!no_lock_deadline_elapsed(1_000, 0, timeout, 1_299));
        assert!(no_lock_deadline_elapsed(1_000, 0, timeout, 1_300));
    }

    #[test]
    fn poll_timeout_defers_while_final_attempt_is_payable() {
        // submitted_at = 1_000, lock deadline = 10_000, request expiry = 20_000.
        let abort = no_lock_timeout_for_attempt(5, TEST_REBID_TIMEOUT_MS, TEST_REBID_MAX_ATTEMPTS);
        assert_eq!(abort.action, NoLockTimeoutAction::Abort);

        // Within the payable window the overall poll timeout must not fire: a timeout-triggered
        // resubmission could reopen the double-pay window through the fresh-id fallback.
        assert!(defer_poll_timeout_while_payable(
            1_000, 10_000, 20_000, abort, 9_999
        ));
        // Once the lock deadline passes, the timeout may fire again.
        assert!(!defer_poll_timeout_while_payable(
            1_000, 10_000, 20_000, abort, 10_000
        ));
        // The deferral is bounded by the request expiry even when a (corrupt) record claims a
        // later lock deadline, so the poll loop cannot be pinned open forever.
        assert!(!defer_poll_timeout_while_payable(
            1_000,
            u64::MAX,
            20_000,
            abort,
            20_000
        ));

        // Rebid attempts never defer the overall timeout.
        let rebid = no_lock_timeout_for_attempt(1, TEST_REBID_TIMEOUT_MS, TEST_REBID_MAX_ATTEMPTS);
        assert_eq!(rebid.action, NoLockTimeoutAction::Rebid);
        assert!(!defer_poll_timeout_while_payable(
            1_000, 10_000, 20_000, rebid, 9_999
        ));

        // Legacy records (lock_expires_at == 0) defer only for the rebid-delay window, matching
        // their abort deadline.
        assert!(defer_poll_timeout_while_payable(
            1_000, 0, 20_000, abort, 1_299
        ));
        assert!(!defer_poll_timeout_while_payable(
            1_000, 0, 20_000, abort, 1_300
        ));
    }

    #[test]
    fn no_lock_timeout_uses_configured_rebid_delay() {
        let timeout = no_lock_timeout_for_attempt(1, 900_000, TEST_REBID_MAX_ATTEMPTS);

        assert_eq!(timeout.delay, Duration::from_millis(900_000));
        assert_eq!(timeout.action, NoLockTimeoutAction::Rebid);
    }

    #[test]
    fn no_lock_timeout_clamps_invalid_rebid_delay_to_minimum() {
        for rebid_timeout_ms in [0, 999] {
            let timeout = no_lock_timeout_for_attempt(1, rebid_timeout_ms, TEST_REBID_MAX_ATTEMPTS);

            assert_eq!(timeout.delay, Duration::from_millis(MIN_REBID_TIMEOUT_MS));
            assert_eq!(timeout.action, NoLockTimeoutAction::Rebid);
        }
    }

    #[test]
    fn no_lock_rebid_stops_after_four_higher_price_requests() {
        assert!(!should_rebid_unlocked_request(0, TEST_REBID_MAX_ATTEMPTS));
        assert!(should_rebid_unlocked_request(1, TEST_REBID_MAX_ATTEMPTS));
        assert!(should_rebid_unlocked_request(2, TEST_REBID_MAX_ATTEMPTS));
        assert!(should_rebid_unlocked_request(3, TEST_REBID_MAX_ATTEMPTS));
        assert!(should_rebid_unlocked_request(4, TEST_REBID_MAX_ATTEMPTS));
        assert!(!should_rebid_unlocked_request(5, TEST_REBID_MAX_ATTEMPTS));
    }

    #[test]
    fn no_lock_rebid_uses_configured_attempt_cap() {
        assert!(should_rebid_unlocked_request(1, 2));
        assert!(should_rebid_unlocked_request(2, 2));
        assert!(!should_rebid_unlocked_request(3, 2));
    }

    #[test]
    fn no_lock_timeout_aborts_after_final_rebid_attempt() {
        assert_eq!(
            no_lock_timeout_for_attempt(1, TEST_REBID_TIMEOUT_MS, TEST_REBID_MAX_ATTEMPTS).action,
            NoLockTimeoutAction::Rebid
        );
        assert_eq!(
            no_lock_timeout_for_attempt(4, TEST_REBID_TIMEOUT_MS, TEST_REBID_MAX_ATTEMPTS).action,
            NoLockTimeoutAction::Rebid
        );
        assert_eq!(
            no_lock_timeout_for_attempt(5, TEST_REBID_TIMEOUT_MS, TEST_REBID_MAX_ATTEMPTS).action,
            NoLockTimeoutAction::Abort
        );
        assert_eq!(
            no_lock_timeout_for_attempt(6, TEST_REBID_TIMEOUT_MS, TEST_REBID_MAX_ATTEMPTS).action,
            NoLockTimeoutAction::Abort
        );
    }

    #[test]
    fn validate_offer_params_rejects_min_price_above_max_price() {
        let mut offer = sample_offer();
        offer.min_price_per_mcycle = Some("0.000001".to_string());
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
    fn validate_offer_params_returns_unescalated_manual_max_price() {
        // Escalation moved to `build_request`; `validate_offer_params` returns the base cap so the
        // per-rung compounding in `escalated_price` is applied once, on the real base.
        let validated = validate_offer_params(&sample_offer(), 1_000, 2).expect("valid offer");
        let base_max = validated.max_price.expect("manual max price");
        let base_min = validated.min_price.expect("manual min price");

        // A fresh first attempt escalates by zero rungs, so the offer bids the base cap unchanged.
        assert_eq!(
            super::escalated_price(
                base_max.value,
                1,
                TEST_REBID_PRICE_STEP_BPS,
                TEST_REBID_MAX_ATTEMPTS
            )
            .expect("escalate base"),
            base_max.value
        );
        // Three rungs at +50% compound the base cap to 3.375x (truncated per rung), and the min
        // price is never escalated (only the max cap moves on rebids).
        let escalated_max = super::escalated_price(
            base_max.value,
            4,
            TEST_REBID_PRICE_STEP_BPS,
            TEST_REBID_MAX_ATTEMPTS,
        )
        .expect("escalate base");
        assert!(escalated_max > base_max.value);
        assert!(base_min.value < base_max.value);
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
    fn validate_offer_params_applies_market_timeout_modifier() {
        let mut offer = sample_offer();
        offer.pricing_mode = BoundlessPricingMode::Market;
        offer.max_price_per_mcycle = None;
        offer.min_price_per_mcycle = None;
        offer.dynamic_pricing_timeout_modifier = Some(2.0);

        let validated = validate_offer_params(&offer, 1_000, 2).expect("valid market offer");

        assert_eq!(validated.lock_timeout, 600);
        assert_eq!(validated.timeout, 1800);
        assert!(validated.timeout > validated.lock_timeout);
    }

    #[test]
    fn validate_offer_params_uses_fixed_timeout_overrides() {
        let mut offer = sample_offer();
        offer.lock_timeout_secs = Some(600);
        offer.timeout_secs = Some(3600);

        let small = validate_offer_params(&offer, 100, 2).expect("valid small offer");
        let large = validate_offer_params(&offer, 5_000, 2).expect("valid large offer");

        assert_eq!(small.lock_timeout, 600);
        assert_eq!(small.timeout, 3600);
        assert_eq!(large.lock_timeout, 600);
        assert_eq!(large.timeout, 3600);
    }

    #[test]
    fn validate_offer_params_rejects_fixed_timeout_below_derived_lock_timeout() {
        let mut offer = sample_offer();
        // The derived lock timeout for 5000 mcycles is 1500s, above the fixed timeout.
        offer.timeout_secs = Some(900);
        let err = validate_offer_params(&offer, 5_000, 2).unwrap_err();
        assert!(err.to_string().contains("timeout"));
    }

    #[test]
    fn validate_offer_params_ramp_up_check_uses_fixed_lock_timeout() {
        let mut offer = sample_offer();
        // 60 blocks at 2s block time is a 120s ramp-up, while the derived lock
        // timeout for 100 mcycles is only 30s; the fixed override lifts it.
        assert!(validate_offer_params(&offer, 100, 2).is_err());

        offer.lock_timeout_secs = Some(600);
        offer.timeout_secs = Some(3600);
        let validated = validate_offer_params(&offer, 100, 2).expect("valid offer");
        assert_eq!(validated.lock_timeout, 600);
        assert_eq!(validated.ramp_up_period_secs, 120);
    }

    #[test]
    fn validate_offer_params_market_timeout_modifier_does_not_scale_fixed_overrides() {
        let mut offer = sample_offer();
        offer.pricing_mode = BoundlessPricingMode::Market;
        offer.max_price_per_mcycle = None;
        offer.min_price_per_mcycle = None;
        offer.dynamic_pricing_timeout_modifier = Some(2.0);
        offer.lock_timeout_secs = Some(600);
        offer.timeout_secs = Some(3600);

        let validated = validate_offer_params(&offer, 1_000, 2).expect("valid market offer");

        assert_eq!(validated.lock_timeout, 600);
        assert_eq!(validated.timeout, 3600);
    }

    #[test]
    fn validate_offer_params_skips_timeout_modifier_overflow_for_fixed_overrides() {
        let mut offer = sample_offer();
        offer.pricing_mode = BoundlessPricingMode::Market;
        offer.max_price_per_mcycle = None;
        offer.min_price_per_mcycle = None;
        offer.dynamic_pricing_timeout_modifier = Some(f64::from(u32::MAX));
        offer.lock_timeout_secs = Some(600);
        offer.timeout_secs = Some(3600);

        let validated = validate_offer_params(&offer, 1_000, 2).expect("valid market offer");

        assert_eq!(validated.lock_timeout, 600);
        assert_eq!(validated.timeout, 3600);
    }

    #[test]
    fn market_prices_escalate_only_the_max_price() {
        // One rung at +300% escalates the max 100 -> 400; the min price is untouched.
        let prices =
            escalate_and_cap_market_prices(U256::from(100), U256::from(10), 2, 30_000, 4, None)
                .expect("escalated prices");

        assert_eq!(prices.max_price, U256::from(400));
        assert_eq!(prices.min_price, U256::from(10));
        assert!(!prices.clamped_to_cap);
    }

    #[test]
    fn market_prices_pass_through_without_escalation_or_cap() {
        // A fresh first attempt (0 rungs) and a flat 0-bps ladder both leave the max unchanged.
        for (attempt, step_bps) in [(1u64, TEST_REBID_PRICE_STEP_BPS), (5, 0)] {
            let prices = escalate_and_cap_market_prices(
                U256::from(100),
                U256::from(10),
                attempt,
                step_bps,
                TEST_REBID_MAX_ATTEMPTS,
                None,
            )
            .expect("flat prices");

            assert_eq!(prices.max_price, U256::from(100));
            assert_eq!(prices.min_price, U256::from(10));
            assert!(!prices.clamped_to_cap);
        }
    }

    #[test]
    fn market_prices_accept_offers_at_or_below_cap() {
        let max_price_cap = Amount::new(U256::from(1_000), Asset::ETH);
        // One rung at +100% escalates the max 100 -> 200, which stays under the 1000 cap.
        let prices = escalate_and_cap_market_prices(
            U256::from(100),
            U256::from(10),
            2,
            10_000,
            4,
            Some(&max_price_cap),
        )
        .expect("uncapped prices");

        assert_eq!(prices.max_price, U256::from(200));
        assert_eq!(prices.min_price, U256::from(10));
        assert!(!prices.clamped_to_cap);
    }

    #[test]
    fn market_prices_clamp_escalated_max_to_cap() {
        let max_price_cap = Amount::new(U256::from(150), Asset::ETH);
        // One rung at +300% would reach 400, but the 150 cap clamps it.
        let prices = escalate_and_cap_market_prices(
            U256::from(100),
            U256::from(10),
            2,
            30_000,
            4,
            Some(&max_price_cap),
        )
        .expect("capped prices");

        assert_eq!(prices.max_price, U256::from(150));
        assert_eq!(prices.min_price, U256::from(10));
        assert!(prices.clamped_to_cap);
    }

    #[test]
    fn market_prices_lower_min_price_when_cap_undercuts_it() {
        let max_price_cap = Amount::new(U256::from(5), Asset::ETH);
        // No escalation (fresh attempt), but the 5 cap sits below the autopriced max and min.
        let prices = escalate_and_cap_market_prices(
            U256::from(100),
            U256::from(10),
            1,
            TEST_REBID_PRICE_STEP_BPS,
            4,
            Some(&max_price_cap),
        )
        .expect("capped prices");

        assert_eq!(prices.max_price, U256::from(5));
        assert_eq!(prices.min_price, U256::from(5));
        assert!(prices.clamped_to_cap);
    }

    #[test]
    fn market_price_escalation_rejects_overflow() {
        // One rung at +100% on U256::MAX overflows the checked multiply.
        let err = escalate_and_cap_market_prices(U256::MAX, U256::from(10), 2, 10_000, 4, None)
            .expect_err("overflowing escalation");
        assert!(err.to_string().contains("overflows"));
    }

    #[test]
    fn submission_budget_bounds_every_retry_path() {
        // Initial attempt plus rebid_max_attempts rebids, matching the no-lock Abort bound.
        assert!(!exceeds_submission_budget(1, TEST_REBID_MAX_ATTEMPTS));
        assert!(!exceeds_submission_budget(5, TEST_REBID_MAX_ATTEMPTS));
        assert!(exceeds_submission_budget(6, TEST_REBID_MAX_ATTEMPTS));

        // Zero configured rebids allow exactly the initial submission.
        assert!(!exceeds_submission_budget(1, 0));
        assert!(exceeds_submission_budget(2, 0));
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
    fn storage_uploader_config_selects_gcs_from_boundless_env() {
        let _guard = StorageEnvGuard::new(&[
            ("BOUNDLESS_STORAGE_UPLOADER", "gcs"),
            ("GCS_BUCKET", "raiko-boundless"),
            ("GCS_URL", "http://127.0.0.1:4443"),
            ("GCS_CREDENTIALS_JSON", "{}"),
            ("GCS_PUBLIC_URL", "false"),
        ]);

        let config = storage_uploader_config_from_env().expect("storage config");

        assert_eq!(config.storage_uploader, StorageUploaderType::Gcs);
        assert_eq!(config.gcs_bucket.as_deref(), Some("raiko-boundless"));
        assert_eq!(config.gcs_url.as_deref(), Some("http://127.0.0.1:4443"));
        assert_eq!(config.gcs_credentials_json.as_deref(), Some("{}"));
        assert_eq!(config.gcs_public_url, Some(false));
    }

    #[test]
    fn storage_uploader_config_auto_selects_gcs_from_bucket() {
        let _guard = StorageEnvGuard::new(&[("GCS_BUCKET", "raiko-boundless")]);

        let config = storage_uploader_config_from_env().expect("storage config");

        assert_eq!(config.storage_uploader, StorageUploaderType::Gcs);
        assert_eq!(config.gcs_bucket.as_deref(), Some("raiko-boundless"));
        assert_eq!(config.gcs_public_url, Some(false));
    }

    #[test]
    fn storage_uploader_config_prefers_boundless_storage_env() {
        let _guard = StorageEnvGuard::new(&[
            ("BOUNDLESS_STORAGE_UPLOADER", "gcs"),
            ("STORAGE_UPLOADER", "s3"),
            ("S3_BUCKET", "s3-bucket"),
            ("GCS_BUCKET", "gcs-bucket"),
        ]);

        let config = storage_uploader_config_from_env().expect("storage config");

        assert_eq!(config.storage_uploader, StorageUploaderType::Gcs);
        assert_eq!(config.gcs_bucket.as_deref(), Some("gcs-bucket"));
        assert_eq!(config.gcs_public_url, Some(false));
    }

    #[test]
    fn storage_uploader_config_still_accepts_legacy_storage_env() {
        let _guard = StorageEnvGuard::new(&[
            ("STORAGE_UPLOADER", "gcs"),
            ("GCS_BUCKET", "raiko-boundless"),
            ("GCS_PUBLIC_URL", "true"),
        ]);

        let config = storage_uploader_config_from_env().expect("storage config");

        assert_eq!(config.storage_uploader, StorageUploaderType::Gcs);
        assert_eq!(config.gcs_public_url, Some(true));
    }

    #[test]
    fn storage_uploader_config_rejects_invalid_boundless_storage_env() {
        let _guard = StorageEnvGuard::new(&[("BOUNDLESS_STORAGE_UPLOADER", "ftp")]);

        let err = storage_uploader_config_from_env().expect_err("invalid storage uploader");

        assert!(
            err.to_string()
                .contains("Invalid BOUNDLESS_STORAGE_UPLOADER/STORAGE_UPLOADER value ftp")
        );
    }

    #[test]
    fn storage_uploader_config_rejects_invalid_gcs_public_url() {
        let _guard = StorageEnvGuard::new(&[
            ("BOUNDLESS_STORAGE_UPLOADER", "gcs"),
            ("GCS_BUCKET", "raiko-boundless"),
            ("GCS_PUBLIC_URL", "maybe"),
        ]);

        let err = storage_uploader_config_from_env().expect_err("invalid GCS_PUBLIC_URL");

        assert!(
            err.to_string()
                .contains("Invalid GCS_PUBLIC_URL boolean value")
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

    #[test]
    fn presigned_refresh_at_leaves_headroom_before_expiry() {
        let url = Url::parse("https://s3.example/obj?X-Amz-Expires=3600").unwrap();
        let refresh = super::presigned_refresh_at(&url);
        // Refreshes at least 120s before the 3600s expiry.
        assert!(refresh <= SystemTime::now() + Duration::from_secs(3600 - 120));
        assert!(refresh >= SystemTime::now() + Duration::from_secs(3600 - 121));
    }
}
