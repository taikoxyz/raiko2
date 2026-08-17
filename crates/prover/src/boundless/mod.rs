#![allow(missing_docs)]

pub mod aggregation;

pub use crate::boundless_config::{
    BoundlessConfig, BoundlessOfferParams, BoundlessPricingMode, DeploymentConfig, DeploymentType,
    MIN_REBID_TIMEOUT_MS, OfferParamsConfig, QuoteSizing, TimeoutPolicy, validate_offer_spec,
};

use std::borrow::Cow;
use std::collections::HashMap;
use std::env;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use alloy_primitives::{Address, B256, Bytes, U256, address, keccak256};
use alloy_signer_local::PrivateKeySigner;
#[cfg(feature = "boundless-s3")]
use boundless_market::storage::S3StorageDownloader;
use boundless_market::{
    Client, ProofRequest, StorageUploaderConfig,
    alloy::{
        network::Ethereum,
        providers::{DynProvider, PendingTransactionBuilder, Provider},
    },
    contracts::RequestId,
    deployments::{BASE, Deployment, SEPOLIA},
    input::GuestEnv,
    price_oracle::{Amount, Asset},
    request_builder::{OfferParams, StandardRequestBuilder},
    storage::{
        FileStorageDownloader, GcsStorageDownloader, HttpDownloader, StandardUploader,
        StorageDownloader, StorageError, StorageUploaderType,
    },
};
use raiko2_pipeline::{ProofStage, ProverBackend};
use raiko2_primitives::{AggregationGuestInput, ProofType, ProverConfig};
use raiko2_primitives::{Proof, RaikoError, RaikoResult};
use raiko2_primitives_shasta::GuestInput;
use raiko2_protocol_shasta::shasta::ProofCarryData;
use raiko2_remote_poller::{
    RemotePollError, RemotePollerConfig, RemoteStatus, RemoteStatusReason, RemoteStatusSource,
    RemoteStatusTracker, RemoteSubmission, RemoteSubmissionId, RemoteSubmissionStatus,
    RemoteTerminalResult,
};
use risc0_ethereum_contracts_boundless::receipt::{Receipt as ContractReceipt, decode_seal};
use risc0_zkvm::{Digest, Journal, compute_image_id, local_executor};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use url::Url;

use crate::{
    BoundlessSubmissionProgress, BoundlessSubmissionResume, PendingProofCheckpointIdentity,
    ProverProgress, ProverProgressObserver, encode_risc0_aggregation_seal_payload,
    encode_risc0_proposal_seal_payload, ensure_shasta_proposal_input_matches_carry,
    parse_shasta_aggregation_input_hash, parse_shasta_proposal_input_hash, with_shasta_extra_data,
};

const MILLION_CYCLES: u64 = 1_000_000;
const BATCH_QUOTED_MCYCLES_MIN: u32 = 2_000;
const BATCH_QUOTED_MCYCLES_STEP: u32 = 1_000;
const AGGREGATION_QUOTED_MCYCLES_MIN: u32 = 200;
const AGGREGATION_QUOTED_MCYCLES_STEP: u32 = 100;
const EXTERNAL_RETRY_ATTEMPTS: u32 = 5;
const EXTERNAL_RETRY_INITIAL_DELAY: Duration = Duration::from_secs(1);
const EXTERNAL_RETRY_MAX_DELAY: Duration = Duration::from_secs(30);
const BOUNDLESS_RPC_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const BOUNDLESS_RPC_TOTAL_TIMEOUT: Duration = Duration::from_mins(1);
const BOUNDLESS_CHECKPOINT_TOTAL_TIMEOUT: Duration = Duration::from_secs(10);
const BOUNDLESS_SUBMIT_SEND_TIMEOUT: Duration = Duration::from_secs(30);
// Covers three confirmations on Sepolia, including Alloy's HTTP polling interval and RPC latency.
const BOUNDLESS_SUBMIT_RECEIPT_TIMEOUT: Duration = Duration::from_secs(90);
const BOUNDLESS_SUBMIT_RECEIPT_CONFIRMATIONS: u64 = 3;
const BOUNDLESS_NONCE_RECOVERY_TOTAL_TIMEOUT: Duration = Duration::from_mins(3);
const BOUNDLESS_NONCE_RECOVERY_POLL_DELAY: Duration = Duration::from_secs(1);
const BOUNDLESS_FUNDING_EXPIRY_GRACE_SECS: u64 = 60;
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

async fn retry_external_bounded<T, F, Fut>(
    operation: &str,
    total_timeout: Duration,
    run: F,
) -> RaikoResult<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = RaikoResult<T>>,
{
    tokio::time::timeout(total_timeout, retry_external(operation, run))
        .await
        .map_err(|_| {
            RaikoError::Guest(format!(
                "Boundless {operation} timed out after {} seconds",
                total_timeout.as_secs_f64()
            ))
        })?
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

type BoundlessStatusRegistry = Arc<Mutex<HashMap<RemoteSubmissionId, BoundlessSubmissionState>>>;
type BoundlessClient = Client<
    DynProvider,
    StandardUploader,
    BoundlessStorageDownloader,
    StandardRequestBuilder<DynProvider, StandardUploader, BoundlessStorageDownloader>,
    PrivateKeySigner,
>;

/// Downloader used by the Boundless requestor client.
///
/// Default builds omit Boundless S3 support entirely. Builds that explicitly enable S3 still
/// initialize its downloader only when S3 is selected, so GCS-only deployments do not start the
/// AWS credential chain or probe IMDS.
#[derive(Clone, Debug)]
struct BoundlessStorageDownloader {
    http: HttpDownloader,
    file: Option<FileStorageDownloader>,
    gcs: Option<GcsStorageDownloader>,
    #[cfg(feature = "boundless-s3")]
    s3: Option<S3StorageDownloader>,
}

impl BoundlessStorageDownloader {
    async fn from_uploader_config(config: &StorageUploaderConfig) -> Result<Self, StorageError> {
        let gcs = if config.storage_uploader == StorageUploaderType::Gcs {
            match GcsStorageDownloader::new(None).await {
                Ok(gcs) => Some(gcs),
                Err(err) => {
                    tracing::debug!(%err, "GCS downloader not available, gs:// URLs will fail");
                    None
                }
            }
        } else {
            None
        };
        #[cfg(feature = "boundless-s3")]
        let s3 = if should_initialize_s3_downloader(config) {
            Some(S3StorageDownloader::new(None).await?)
        } else {
            None
        };

        Ok(Self {
            http: HttpDownloader::default(),
            file: boundless_dev_mode_enabled().then(FileStorageDownloader::new),
            gcs,
            #[cfg(feature = "boundless-s3")]
            s3,
        })
    }
}

#[async_trait::async_trait]
impl StorageDownloader for BoundlessStorageDownloader {
    async fn download_url_with_limit(
        &self,
        url: Url,
        limit: usize,
    ) -> Result<Vec<u8>, StorageError> {
        match url.scheme() {
            "http" | "https" => self.http.download_url_with_limit(url, limit).await,
            "file" => match &self.file {
                Some(file) => file.download_url_with_limit(url, limit).await,
                None => Err(StorageError::UnsupportedScheme(
                    "file (dev mode only)".to_string(),
                )),
            },
            "gs" => match &self.gcs {
                Some(gcs) => gcs.download_url_with_limit(url, limit).await,
                None => Err(StorageError::CredentialsUnavailable {
                    scheme: "gs".to_string(),
                }),
            },
            "s3" => {
                #[cfg(feature = "boundless-s3")]
                {
                    return match &self.s3 {
                        Some(s3) => s3.download_url_with_limit(url, limit).await,
                        None => Err(StorageError::CredentialsUnavailable {
                            scheme: "s3".to_string(),
                        }),
                    };
                }
                #[cfg(not(feature = "boundless-s3"))]
                {
                    Err(StorageError::UnsupportedScheme(
                        "s3 (this build does not include the boundless-s3 feature)".to_string(),
                    ))
                }
            }
            scheme => Err(StorageError::UnsupportedScheme(scheme.to_string())),
        }
    }

    async fn download_url(&self, url: Url) -> Result<Vec<u8>, StorageError> {
        self.download_url_with_limit(url, usize::MAX).await
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BoundlessSubmissionMetadata {
    expires_at: u64,
    lock_expires_at: u64,
    submitted_at: u64,
    no_lock_deadline: u64,
    no_lock_timeout_action: BoundlessTimeoutAction,
    poll_timeout_at: Instant,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BoundlessTimeoutAction {
    Rebid,
    Abort,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BoundlessSubmissionState {
    metadata: BoundlessSubmissionMetadata,
    terminal_outcome: Option<BoundlessTerminalOutcome>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BoundlessTerminalOutcome {
    MarketExpired,
    LockExpired,
    NoLockRebidTimeout,
    NoLockAbortTimeout,
    PollTimeout { rotate_request_id: bool },
}

#[derive(Clone)]
struct BoundlessStatusSource {
    rpc_url: String,
    market_address: String,
    http: reqwest::Client,
    registry: BoundlessStatusRegistry,
}

impl BoundlessStatusSource {
    fn new(
        rpc_url: String,
        deployment: &Deployment,
        registry: BoundlessStatusRegistry,
        request_timeout: Duration,
    ) -> Self {
        Self {
            rpc_url,
            market_address: deployment.boundless_market_address.to_string(),
            http: reqwest::Client::builder()
                .timeout(request_timeout)
                .build()
                .expect("boundless status reqwest client should build"),
            registry,
        }
    }

    async fn poll_batch(
        &self,
        submissions: Vec<RemoteSubmission>,
    ) -> Result<Vec<RemoteSubmissionStatus>, RemotePollError> {
        let mut invalid_statuses = Vec::new();
        let mut pollable_submissions = Vec::new();
        for submission in submissions.iter().cloned() {
            match parse_boundless_request_id(&submission.provider_request_id) {
                Ok(request_id) => pollable_submissions.push(BoundlessPollSubmission {
                    submission,
                    request_id,
                }),
                Err(err) => invalid_statuses.push(unrecoverable_boundless_status(
                    submission.id,
                    format!("invalid boundless provider request id: {err}"),
                )),
            }
        }
        if pollable_submissions.is_empty() {
            return Ok(invalid_statuses);
        }

        let batch = self.build_batch_request(&pollable_submissions);
        let responses = match self.http.post(&self.rpc_url).json(&batch).send().await {
            Ok(response) => response,
            Err(err) => {
                return boundless_poll_error_statuses(
                    submissions,
                    format!("boundless status rpc: {err}"),
                    &self.registry,
                );
            }
        };
        let responses = match responses.error_for_status() {
            Ok(response) => response,
            Err(err) => {
                return boundless_poll_error_statuses(
                    submissions,
                    format!("boundless status rpc: {err}"),
                    &self.registry,
                );
            }
        };
        let responses = match responses.json::<Vec<JsonRpcResponse>>().await {
            Ok(responses) => responses,
            Err(err) => {
                return boundless_poll_error_statuses(
                    submissions,
                    format!("decode boundless status rpc response: {err}"),
                    &self.registry,
                );
            }
        };

        let mut by_id = responses
            .into_iter()
            .map(|response| (response.id, response))
            .collect::<HashMap<_, _>>();
        let block_result = match take_rpc_result(&mut by_id, 0) {
            Ok(result) => result,
            Err(err) => {
                return boundless_poll_error_statuses(submissions, err.to_string(), &self.registry);
            }
        };
        let block_timestamp = match parse_block_timestamp(&block_result) {
            Ok(timestamp) => timestamp,
            Err(err) => {
                return boundless_poll_error_statuses(submissions, err.to_string(), &self.registry);
            }
        };

        let mut statuses = Vec::with_capacity(submissions.len());
        statuses.extend(invalid_statuses);
        for (index, submission) in pollable_submissions.iter().enumerate() {
            let status = match self.status_from_rpc_results(
                index,
                submission,
                block_timestamp,
                &mut by_id,
            ) {
                Ok(status) => status,
                Err(err) => {
                    let error = err.to_string();
                    boundless_single_poll_error_status(
                        &submission.submission,
                        &error,
                        &self.registry,
                    )
                }
            };
            statuses.push(status);
        }
        Ok(statuses)
    }

    fn build_batch_request(&self, submissions: &[BoundlessPollSubmission]) -> Vec<JsonRpcRequest> {
        let mut batch = Vec::with_capacity(submissions.len().saturating_mul(3).saturating_add(1));
        batch.push(json_rpc_request(
            0,
            "eth_getBlockByNumber",
            vec![serde_json::json!("latest"), serde_json::json!(false)],
        ));

        for (index, submission) in submissions.iter().enumerate() {
            let base_id = rpc_base_id(index);
            let fulfilled_data =
                boundless_call_data("requestIsFulfilled(uint256)", submission.request_id);
            batch.push(eth_call_request(
                base_id,
                &self.market_address,
                &fulfilled_data,
            ));
            let locked_data =
                boundless_call_data("requestIsLocked(uint256)", submission.request_id);
            batch.push(eth_call_request(
                base_id + 1,
                &self.market_address,
                &locked_data,
            ));
            let deadline_data =
                boundless_call_data("requestDeadline(uint256)", submission.request_id);
            batch.push(eth_call_request(
                base_id + 2,
                &self.market_address,
                &deadline_data,
            ));
        }

        batch
    }

    fn status_from_rpc_results(
        &self,
        index: usize,
        poll_submission: &BoundlessPollSubmission,
        block_timestamp: u64,
        by_id: &mut HashMap<u64, JsonRpcResponse>,
    ) -> Result<RemoteSubmissionStatus, RemotePollError> {
        let submission = &poll_submission.submission;
        let Some(metadata) = boundless_submission_metadata(&self.registry, submission.id)? else {
            return Ok(unrecoverable_boundless_status(
                submission.id,
                format!(
                    "boundless status source missing metadata for request {}",
                    submission.provider_request_id
                ),
            ));
        };
        let base_id = rpc_base_id(index);
        let fulfilled_result = take_rpc_result(by_id, base_id)?;
        let locked_result = take_rpc_result(by_id, base_id + 1)?;
        let is_fulfilled = parse_bool_result(&fulfilled_result)?;
        let is_locked = parse_bool_result(&locked_result)?;
        let request_deadline = if is_locked {
            let deadline_result = take_rpc_result(by_id, base_id + 2)?;
            parse_u64_result(&deadline_result)?
        } else {
            0
        };

        let (status, terminal_outcome) = classify_boundless_status(
            submission.id,
            &submission.provider_request_id,
            &metadata,
            is_fulfilled,
            is_locked,
            request_deadline,
            block_timestamp,
        );
        if let Some(outcome) = terminal_outcome {
            record_boundless_terminal_outcome(&self.registry, submission.id, outcome)?;
        }
        Ok(status)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BoundlessPollSubmission {
    submission: RemoteSubmission,
    request_id: U256,
}

#[async_trait::async_trait]
impl RemoteStatusSource for BoundlessStatusSource {
    async fn poll(
        &self,
        _proof_type: ProofType,
        submissions: Vec<RemoteSubmission>,
    ) -> Result<Vec<RemoteSubmissionStatus>, RemotePollError> {
        self.poll_batch(submissions).await
    }
}

#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: u64,
    method: &'static str,
    params: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    id: u64,
    result: Option<serde_json::Value>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

const fn json_rpc_request(
    id: u64,
    method: &'static str,
    params: Vec<serde_json::Value>,
) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0",
        id,
        method,
        params,
    }
}

fn eth_call_request(id: u64, market_address: &str, data: &str) -> JsonRpcRequest {
    json_rpc_request(
        id,
        "eth_call",
        vec![
            serde_json::json!({
                "to": market_address,
                "data": data,
            }),
            serde_json::json!("latest"),
        ],
    )
}

fn rpc_base_id(index: usize) -> u64 {
    u64::try_from(index)
        .unwrap_or(u64::MAX / 3)
        .saturating_mul(3)
        .saturating_add(1)
}

fn parse_boundless_request_id(value: &str) -> Result<U256, String> {
    let trimmed = value.trim().trim_start_matches("0x");
    U256::from_str_radix(trimmed, 16).map_err(|err| err.to_string())
}

fn boundless_call_data(signature: &str, request_id: U256) -> String {
    let selector = keccak256(signature.as_bytes());
    let mut data = Vec::with_capacity(36);
    data.extend_from_slice(&selector.as_slice()[..4]);
    data.extend_from_slice(&request_id.to_be_bytes::<32>());
    alloy_primitives::hex::encode_prefixed(data)
}

fn take_rpc_result(
    by_id: &mut HashMap<u64, JsonRpcResponse>,
    id: u64,
) -> Result<serde_json::Value, RemotePollError> {
    let response = by_id.remove(&id).ok_or_else(|| {
        RemotePollError::Transient(format!("boundless status rpc response missing id {id}"))
    })?;
    if let Some(error) = response.error {
        return Err(RemotePollError::Transient(format!(
            "boundless status rpc id {id} error {}: {}",
            error.code, error.message
        )));
    }
    response.result.ok_or_else(|| {
        RemotePollError::Transient(format!(
            "boundless status rpc response id {id} missing result"
        ))
    })
}

fn parse_block_timestamp(value: &serde_json::Value) -> Result<u64, RemotePollError> {
    let timestamp = value
        .get("timestamp")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            RemotePollError::Transient("boundless latest block missing timestamp".to_string())
        })?;
    parse_rpc_hex_u64(timestamp)
}

fn parse_bool_result(value: &serde_json::Value) -> Result<bool, RemotePollError> {
    let word = parse_rpc_word(value)?;
    Ok(word.iter().any(|byte| *byte != 0))
}

fn parse_u64_result(value: &serde_json::Value) -> Result<u64, RemotePollError> {
    let word = parse_rpc_word(value)?;
    Ok(u64::from_be_bytes(
        word[24..32]
            .try_into()
            .expect("32-byte ABI word has trailing u64"),
    ))
}

fn parse_rpc_word(value: &serde_json::Value) -> Result<[u8; 32], RemotePollError> {
    let raw = value.as_str().ok_or_else(|| {
        RemotePollError::Transient("boundless eth_call result is not a hex string".to_string())
    })?;
    let bytes = alloy_primitives::hex::decode(raw.trim_start_matches("0x")).map_err(|err| {
        RemotePollError::Transient(format!("decode boundless eth_call result: {err}"))
    })?;
    bytes.try_into().map_err(|bytes: Vec<u8>| {
        RemotePollError::Transient(format!(
            "boundless eth_call result has {} bytes, expected 32",
            bytes.len()
        ))
    })
}

fn parse_rpc_hex_u64(value: &str) -> Result<u64, RemotePollError> {
    u64::from_str_radix(value.trim_start_matches("0x"), 16)
        .map_err(|err| RemotePollError::Transient(format!("decode rpc hex u64 {value}: {err}")))
}

fn classify_boundless_status(
    submission_id: RemoteSubmissionId,
    provider_request_id: &str,
    metadata: &BoundlessSubmissionMetadata,
    is_fulfilled: bool,
    is_locked: bool,
    request_deadline: u64,
    block_timestamp: u64,
) -> (RemoteSubmissionStatus, Option<BoundlessTerminalOutcome>) {
    let local_now = now_secs();
    let (status, reason, terminal_outcome) = if is_fulfilled {
        (RemoteStatus::Fulfilled, None, None)
    } else if block_timestamp > metadata.expires_at {
        (
            RemoteStatus::Expired,
            Some(RemoteStatusReason::new(format!(
                "Boundless request {provider_request_id} expired before fulfillment"
            ))),
            Some(BoundlessTerminalOutcome::MarketExpired),
        )
    } else if is_locked && request_deadline > 0 && block_timestamp > request_deadline {
        (
            RemoteStatus::Expired,
            Some(RemoteStatusReason::new(format!(
                "Boundless request {provider_request_id} lock deadline passed before fulfillment"
            ))),
            Some(BoundlessTerminalOutcome::LockExpired),
        )
    } else if Instant::now() >= metadata.poll_timeout_at
        && !should_defer_boundless_poll_timeout(metadata, local_now)
    {
        let rotate_request_id = local_now >= metadata.expires_at;
        (
            RemoteStatus::Failed,
            Some(RemoteStatusReason::new(format!(
                "Boundless request {provider_request_id} timed out before fulfillment"
            ))),
            Some(BoundlessTerminalOutcome::PollTimeout { rotate_request_id }),
        )
    } else if is_locked {
        (RemoteStatus::Locked, None, None)
    } else if local_now >= metadata.no_lock_deadline {
        match metadata.no_lock_timeout_action {
            BoundlessTimeoutAction::Rebid => (
                RemoteStatus::Failed,
                Some(RemoteStatusReason::new(format!(
                    "Boundless request {provider_request_id} was not locked before rebid timeout"
                ))),
                Some(BoundlessTerminalOutcome::NoLockRebidTimeout),
            ),
            BoundlessTimeoutAction::Abort => (
                RemoteStatus::Failed,
                Some(RemoteStatusReason::new(format!(
                    "Boundless request {provider_request_id} was not locked before payable window closed"
                ))),
                Some(BoundlessTerminalOutcome::NoLockAbortTimeout),
            ),
        }
    } else {
        (RemoteStatus::Pending, None, None)
    };

    (
        RemoteSubmissionStatus {
            submission_id,
            status,
            reason,
            observed_unix_secs: block_timestamp,
        },
        terminal_outcome,
    )
}

/// Whether the overall poll timeout must be deferred for the current submission.
///
/// On the final attempt (`Abort`) the request stays payable until its lock deadline, and a
/// timeout-triggered replacement could reopen the double-pay window under another request id,
/// so the overall timeout only takes effect once the payable window has closed. The deferral
/// is bounded by `expires_at` so a corrupt stored record (or one with an implausibly distant
/// lock deadline) cannot keep the poll loop open forever.
const fn should_defer_boundless_poll_timeout(
    metadata: &BoundlessSubmissionMetadata,
    block_timestamp: u64,
) -> bool {
    matches!(
        metadata.no_lock_timeout_action,
        BoundlessTimeoutAction::Abort
    ) && block_timestamp < metadata.expires_at
        && block_timestamp < metadata.no_lock_deadline
}

fn unrecoverable_boundless_status(
    submission_id: RemoteSubmissionId,
    reason: impl Into<String>,
) -> RemoteSubmissionStatus {
    RemoteSubmissionStatus {
        submission_id,
        status: RemoteStatus::Unrecoverable,
        reason: Some(RemoteStatusReason::new(reason)),
        observed_unix_secs: now_secs(),
    }
}

fn boundless_poll_error_statuses(
    submissions: Vec<RemoteSubmission>,
    error: String,
    registry: &BoundlessStatusRegistry,
) -> Result<Vec<RemoteSubmissionStatus>, RemotePollError> {
    let mut has_terminal_status = false;
    let statuses = submissions
        .into_iter()
        .map(|submission| {
            if let Err(err) = parse_boundless_request_id(&submission.provider_request_id) {
                has_terminal_status = true;
                return unrecoverable_boundless_status(
                    submission.id,
                    format!("invalid boundless provider request id: {err}"),
                );
            }
            let status = boundless_single_poll_error_status(&submission, &error, registry);
            if status.status.is_terminal() {
                has_terminal_status = true;
            }
            status
        })
        .collect::<Vec<_>>();

    if has_terminal_status {
        Ok(statuses)
    } else {
        Err(RemotePollError::Transient(error))
    }
}

fn boundless_single_poll_error_status(
    submission: &RemoteSubmission,
    error: &str,
    registry: &BoundlessStatusRegistry,
) -> RemoteSubmissionStatus {
    let local_now = now_secs();
    let metadata = match boundless_submission_metadata(registry, submission.id) {
        Ok(Some(metadata)) => metadata,
        Ok(None) => {
            return unrecoverable_boundless_status(
                submission.id,
                format!(
                    "boundless status source missing metadata for request {}",
                    submission.provider_request_id
                ),
            );
        }
        Err(err) => return err_status(submission.id, &err),
    };
    if Instant::now() >= metadata.poll_timeout_at
        && !should_defer_boundless_poll_timeout(&metadata, local_now)
    {
        let rotate_request_id = local_now >= metadata.expires_at;
        let _ = record_boundless_terminal_outcome(
            registry,
            submission.id,
            BoundlessTerminalOutcome::PollTimeout { rotate_request_id },
        );
        return RemoteSubmissionStatus {
            submission_id: submission.id,
            status: RemoteStatus::Failed,
            reason: Some(RemoteStatusReason::new(format!(
                "Boundless request {} timed out before fulfillment; last polling error: {error}",
                submission.provider_request_id
            ))),
            observed_unix_secs: local_now,
        };
    }
    RemoteSubmissionStatus {
        submission_id: submission.id,
        status: RemoteStatus::Pending,
        reason: None,
        observed_unix_secs: local_now,
    }
}

fn err_status(submission_id: RemoteSubmissionId, err: &RemotePollError) -> RemoteSubmissionStatus {
    RemoteSubmissionStatus {
        submission_id,
        status: RemoteStatus::Unrecoverable,
        reason: Some(RemoteStatusReason::new(err.to_string())),
        observed_unix_secs: now_secs(),
    }
}

fn lock_boundless_registry(
    registry: &BoundlessStatusRegistry,
) -> Result<
    std::sync::MutexGuard<'_, HashMap<RemoteSubmissionId, BoundlessSubmissionState>>,
    RemotePollError,
> {
    registry.lock().map_err(|err| {
        RemotePollError::SourceUnavailable(format!(
            "boundless status registry lock poisoned: {err}"
        ))
    })
}

fn boundless_submission_metadata(
    registry: &BoundlessStatusRegistry,
    submission_id: RemoteSubmissionId,
) -> Result<Option<BoundlessSubmissionMetadata>, RemotePollError> {
    Ok(lock_boundless_registry(registry)?
        .get(&submission_id)
        .map(|state| state.metadata.clone()))
}

fn record_boundless_terminal_outcome(
    registry: &BoundlessStatusRegistry,
    submission_id: RemoteSubmissionId,
    outcome: BoundlessTerminalOutcome,
) -> Result<(), RemotePollError> {
    if let Some(state) = lock_boundless_registry(registry)?.get_mut(&submission_id) {
        state.terminal_outcome = Some(outcome);
    }
    Ok(())
}

fn boundless_terminal_outcome(
    registry: &BoundlessStatusRegistry,
    submission_id: RemoteSubmissionId,
) -> Result<Option<BoundlessTerminalOutcome>, BoundlessAttemptError> {
    registry
        .lock()
        .map_err(|err| {
            BoundlessAttemptError::Fatal(RaikoError::Guest(format!(
                "Boundless status registry lock poisoned: {err}"
            )))
        })?
        .get(&submission_id)
        .map(|state| state.terminal_outcome)
        .ok_or_else(|| {
            BoundlessAttemptError::Fatal(RaikoError::Guest(format!(
                "Boundless status registry missing submission {submission_id}"
            )))
        })
}

struct BoundlessSubmissionGuard {
    tracker: RemoteStatusTracker,
    registry: BoundlessStatusRegistry,
    submission_id: RemoteSubmissionId,
}

impl BoundlessSubmissionGuard {
    const fn new(
        tracker: RemoteStatusTracker,
        registry: BoundlessStatusRegistry,
        submission_id: RemoteSubmissionId,
    ) -> Self {
        Self {
            tracker,
            registry,
            submission_id,
        }
    }
}

impl Drop for BoundlessSubmissionGuard {
    fn drop(&mut self) {
        if let Ok(mut registry) = self.registry.lock() {
            registry.remove(&self.submission_id);
        }
        self.tracker.untrack(self.submission_id);
    }
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

const fn should_rebid_unlocked_request(attempt: u64, max_attempts: u32) -> bool {
    attempt > 0 && attempt <= max_attempts as u64
}

/// Total market submissions allowed per proof task: the initial attempt plus `max_attempts`
/// rebids. The no-lock path already enforces this bound through [`BoundlessTimeoutAction::Abort`];
/// this check extends the same budget to the `Expired` and poll-timeout retry paths, which would
/// otherwise mint replacement requests without limit.
const fn exceeds_submission_budget(attempt: u64, max_attempts: u32) -> bool {
    attempt > max_attempts as u64 + 1
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NoLockTimeout {
    delay: Duration,
    action: BoundlessTimeoutAction,
}

fn no_lock_timeout_for_attempt(
    attempt: u64,
    rebid_timeout_ms: u64,
    max_attempts: u32,
) -> NoLockTimeout {
    let action = if should_rebid_unlocked_request(attempt, max_attempts) {
        BoundlessTimeoutAction::Rebid
    } else {
        BoundlessTimeoutAction::Abort
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
/// deadline: the market pays nothing for fulfillments past `lock_expires_at`, so that is the
/// exact end of the payable window — aborting sooner walks away from a live request the client
/// could still be charged for, and waiting longer cannot yield a paid fulfillment.
const fn no_lock_deadline(submitted_at: u64, lock_expires_at: u64, timeout: NoLockTimeout) -> u64 {
    let rebid_deadline = submitted_at.saturating_add(timeout.delay.as_secs());
    match timeout.action {
        BoundlessTimeoutAction::Rebid => rebid_deadline,
        BoundlessTimeoutAction::Abort => lock_expires_at,
    }
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct Submission {
    market_request_id: U256,
    provider_request_id: String,
    remote_tx_hash: Option<String>,
    expires_at: u64,
    // Offer lock deadline (`rampUpStart + lockTimeout`). The market pays nothing for fulfillments
    // past this time, so it bounds the payable window.
    lock_expires_at: u64,
    submitted_at: u64,
    // Floored effective price multiplier at this attempt, for progress/metadata display only.
    // Derived from `attempt` + config via `effective_price_multiplier`; never used to price offers.
    max_price_multiplier: u32,
    // Exact escalated max price this submission bid, in wei. The floored `max_price_multiplier`
    // renders the common attempt-2 (×1.5) rung as `1` — indistinguishable from an un-escalated bid —
    // so this carries the precise value for telemetry.
    max_price_wei: U256,
    // 1-based rebid attempt that produced this submission; persisted so restarts don't reset the
    // rebid budget when the price is flat (`rebid_price_step_bps == 0`).
    attempt: u64,
}

struct FreshSubmissionContext<'a> {
    client: &'a BoundlessClient,
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

/// Verification and telemetry inputs that stay constant across one proof's whole fulfillment
/// chain (status poll → fulfillment-read retry loop → fulfillment read). Built once per proof in
/// `prove_boundless` and passed by reference, mirroring [`FreshSubmissionContext`] on the submit
/// side, so the chain doesn't thread seven unchanged arguments through every hop.
struct FulfillmentContext<'a> {
    proof_type: &'static str,
    image_id: Digest,
    block_image_id: Option<Digest>,
    expected_input_hash: B256,
    quoted_mcycles_count: u32,
    evaluated_mcycles_count: u32,
    proposal_carry_data: Option<&'a ProofCarryData>,
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
    TerminalCheckpoint {
        identity: PendingProofCheckpointIdentity,
        error: RaikoError,
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
    permit: &crate::SubmissionCheckpointPermit,
    submission: &Submission,
    image_ref: &str,
    deployment: &str,
    offchain: bool,
    mcycles_count: (u32, u32),
) -> RaikoResult<()> {
    let (quoted_mcycles_count, evaluated_mcycles_count) = mcycles_count;
    let progress = ProverProgress::BoundlessSubmission(BoundlessSubmissionProgress {
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
        max_price_wei: Some(submission.max_price_wei.to_string()),
        rebid_attempt: u32::try_from(submission.attempt).unwrap_or(u32::MAX),
    });
    crate::persist_prover_progress(observer, &progress, "boundless_submission", permit).await
}

async fn terminalize_boundless_checkpoint(
    observer: Option<&Arc<dyn ProverProgressObserver>>,
    identity: &PendingProofCheckpointIdentity,
) -> RaikoResult<()> {
    let permit = crate::acquire_submission_checkpoint_permit(observer).await?;
    crate::clear_pending_proof_checkpoint(observer, identity, &permit).await
}

async fn checkpoint_boundless_tx_hash(
    observer: Option<&Arc<dyn ProverProgressObserver>>,
    submission: &Submission,
    image_ref: &str,
    deployment: &str,
    mcycles_count: (u32, u32),
) {
    let checkpoint = async {
        let tx_hash_permit = crate::acquire_submission_checkpoint_permit(observer).await?;
        publish_boundless_progress(
            observer,
            &tx_hash_permit,
            submission,
            image_ref,
            deployment,
            false,
            mcycles_count,
        )
        .await
    };
    match tokio::time::timeout(BOUNDLESS_CHECKPOINT_TOTAL_TIMEOUT, checkpoint).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::warn!(
            provider_request_id = %submission.provider_request_id,
            error = %error,
            "Boundless request id is durable but its optional transaction hash was not checkpointed"
        ),
        Err(_) => tracing::warn!(
            provider_request_id = %submission.provider_request_id,
            timeout_secs = BOUNDLESS_CHECKPOINT_TOTAL_TIMEOUT.as_secs(),
            "Timed out persisting optional Boundless transaction hash"
        ),
    }
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_offchain_after_checkpoint<F, Fut>(
    observer: Option<&Arc<dyn ProverProgressObserver>>,
    permit: crate::SubmissionCheckpointPermit,
    submission: Submission,
    image_ref: &str,
    deployment: &str,
    mcycles_count: (u32, u32),
    dispatch: F,
) -> RaikoResult<Submission>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = RaikoResult<U256>>,
{
    publish_boundless_progress(
        observer,
        &permit,
        &submission,
        image_ref,
        deployment,
        true,
        mcycles_count,
    )
    .await?;
    drop(permit);

    match dispatch().await {
        Ok(returned_id) if returned_id == submission.market_request_id => Ok(submission),
        Ok(returned_id) => Err(RaikoError::Guest(format!(
            "Boundless order stream returned request id 0x{returned_id:x}, expected checkpointed request id 0x{:x}",
            submission.market_request_id
        ))),
        Err(error) => {
            tracing::warn!(
                provider_request_id = %submission.provider_request_id,
                error = %error,
                "Boundless offchain submission returned an uncertain error; polling checkpointed request id"
            );
            Ok(submission)
        }
    }
}

impl TryFrom<BoundlessSubmissionResume> for Submission {
    type Error = RaikoError;

    fn try_from(value: BoundlessSubmissionResume) -> Result<Self, Self::Error> {
        let raw_id = value.provider_request_id.strip_prefix("0x").ok_or_else(|| {
            RaikoError::Guest(format!(
                "Invalid stored Boundless provider_request_id {}: expected canonical 0x-prefixed hexadecimal",
                value.provider_request_id
            ))
        })?;
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
        if value.provider_request_id != format!("0x{market_request_id:x}") {
            return Err(RaikoError::Guest(format!(
                "Invalid stored Boundless provider_request_id {}: non-canonical encoding",
                value.provider_request_id
            )));
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
        if value.lock_expires_at == 0 {
            return Err(RaikoError::Guest(
                "Invalid stored Boundless submission: missing lock_expires_at".to_string(),
            ));
        }
        if value.submitted_at >= value.lock_expires_at || value.lock_expires_at > value.expires_at {
            return Err(RaikoError::Guest(
                "Invalid stored Boundless submission: invalid deadline ordering".to_string(),
            ));
        }
        if value.rebid_attempt == 0 {
            return Err(RaikoError::Guest(
                "Invalid stored Boundless submission: rebid_attempt must start at one".to_string(),
            ));
        }
        if value.max_price_multiplier == 0 {
            return Err(RaikoError::Guest(
                "Invalid stored Boundless submission: max_price_multiplier must be non-zero"
                    .to_string(),
            ));
        }
        let max_price_wei = value
            .max_price_wei
            .as_deref()
            .ok_or_else(|| {
                RaikoError::Guest(
                    "Invalid stored Boundless submission: missing max_price_wei".to_string(),
                )
            })?
            .parse::<U256>()
            .map_err(|error| {
                RaikoError::Guest(format!("Invalid stored Boundless max_price_wei: {error}"))
            })?;
        if max_price_wei == U256::ZERO {
            return Err(RaikoError::Guest(
                "Invalid stored Boundless submission: max_price_wei must be non-zero".to_string(),
            ));
        }
        Ok(Self {
            market_request_id,
            provider_request_id: value.provider_request_id,
            remote_tx_hash: value.remote_tx_hash,
            expires_at: value.expires_at,
            lock_expires_at: value.lock_expires_at,
            submitted_at: value.submitted_at,
            max_price_multiplier: value.max_price_multiplier,
            max_price_wei,
            attempt: u64::from(value.rebid_attempt),
        })
    }
}

fn validate_resume_context(
    resume: &BoundlessSubmissionResume,
    image_ref: &str,
    deployment: &str,
    offchain: bool,
) -> RaikoResult<()> {
    if resume.image_ref != image_ref {
        return Err(RaikoError::Guest(format!(
            "Boundless checkpoint image {} does not match current image {image_ref}",
            resume.image_ref
        )));
    }
    if resume.deployment != deployment {
        return Err(RaikoError::Guest(format!(
            "Boundless checkpoint deployment {} does not match current deployment {deployment}",
            resume.deployment
        )));
    }
    if resume.offchain != offchain {
        return Err(RaikoError::Guest(format!(
            "Boundless checkpoint transport {} does not match current transport {offchain}",
            resume.offchain
        )));
    }
    Ok(())
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

fn boundless_dev_mode_enabled() -> bool {
    env_var("RISC0_DEV_MODE")
        .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
}

#[cfg(feature = "boundless-s3")]
fn should_initialize_s3_downloader(config: &StorageUploaderConfig) -> bool {
    config.storage_uploader == StorageUploaderType::S3
}

#[cfg(not(feature = "boundless-s3"))]
fn s3_feature_disabled_error() -> RaikoError {
    RaikoError::InvalidRequestConfig(
        "Boundless S3 storage was requested, but this build does not include the boundless-s3 feature"
            .to_string(),
    )
}

fn storage_uploader_config_from_env() -> RaikoResult<StorageUploaderConfig> {
    let mut config = StorageUploaderConfig::default();
    let selected = env_var("BOUNDLESS_STORAGE_UPLOADER")
        .or_else(|| env_var("STORAGE_UPLOADER"))
        .map(|value| value.to_ascii_lowercase());
    config.storage_uploader = match selected.as_deref() {
        Some("s3") => {
            #[cfg(feature = "boundless-s3")]
            {
                StorageUploaderType::S3
            }
            #[cfg(not(feature = "boundless-s3"))]
            {
                return Err(s3_feature_disabled_error());
            }
        }
        Some("gcs") => StorageUploaderType::Gcs,
        Some("pinata") => StorageUploaderType::Pinata,
        Some("file") => StorageUploaderType::File,
        Some("none") => StorageUploaderType::None,
        Some(other) => {
            return Err(RaikoError::InvalidRequestConfig(format!(
                "Invalid BOUNDLESS_STORAGE_UPLOADER/STORAGE_UPLOADER value {other}"
            )));
        }
        None if env_var("GCS_BUCKET").is_some() => StorageUploaderType::Gcs,
        None if env_var("PINATA_JWT").is_some() => StorageUploaderType::Pinata,
        None if env_var("FILE_PATH").is_some() => StorageUploaderType::File,
        None if env_var("S3_BUCKET").is_some() => {
            #[cfg(feature = "boundless-s3")]
            {
                StorageUploaderType::S3
            }
            #[cfg(not(feature = "boundless-s3"))]
            {
                return Err(s3_feature_disabled_error());
            }
        }
        None => StorageUploaderType::None,
    };
    #[cfg(feature = "boundless-s3")]
    {
        config.s3_bucket = env_var("S3_BUCKET");
        config.s3_url = env_var("S3_URL");
        config.aws_access_key_id = env_var("AWS_ACCESS_KEY_ID");
        config.aws_secret_access_key = env_var("AWS_SECRET_ACCESS_KEY");
        config.aws_region = env_var("AWS_REGION");
        config.s3_presigned = env_bool("S3_PRESIGNED")?;
        config.s3_public_url = env_bool("S3_PUBLIC_URL")?;
    }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RecentFundingRequest {
    max_price: U256,
    lock_expires_at: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BoundlessFundingDecision {
    reserved_count: usize,
    required_total: U256,
    attached_value: U256,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BoundlessUncertainSubmission {
    submission: Submission,
    request: ProofRequest,
    signature: Bytes,
    request_digest: B256,
    value: U256,
    nonce: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BoundlessNonceStatus {
    Mined,
    Pending,
    Available,
}

const fn classify_boundless_nonce(nonce: u64, latest: u64, pending: u64) -> BoundlessNonceStatus {
    if latest > nonce {
        BoundlessNonceStatus::Mined
    } else if pending > nonce {
        BoundlessNonceStatus::Pending
    } else {
        BoundlessNonceStatus::Available
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BoundlessReceiptOutcome {
    ConfirmedSuccess,
    ConfirmedRevert,
    Uncertain,
}

#[derive(Debug, Default)]
struct BoundlessFundingState {
    recent: HashMap<U256, HashMap<B256, RecentFundingRequest>>,
    next_nonce: Option<u64>,
    uncertain: Option<BoundlessUncertainSubmission>,
}

impl BoundlessFundingState {
    fn allocate_nonce(&mut self, latest: u64, pending: u64) -> RaikoResult<u64> {
        if let Some(uncertain) = &self.uncertain {
            return Err(RaikoError::Guest(format!(
                "Boundless account nonce {} is still uncertain",
                uncertain.nonce
            )));
        }
        let nonce = latest.max(pending).max(self.next_nonce.unwrap_or_default());
        self.next_nonce = Some(nonce.checked_add(1).ok_or_else(|| {
            RaikoError::Guest("Boundless account nonce exhausted u64".to_string())
        })?);
        Ok(nonce)
    }

    fn record_uncertain(&mut self, submission: BoundlessUncertainSubmission) -> RaikoResult<()> {
        match self.uncertain.as_ref() {
            None => {
                self.uncertain = Some(submission);
                Ok(())
            }
            Some(existing) if existing == &submission => Ok(()),
            Some(existing) => Err(RaikoError::Guest(format!(
                "Boundless account nonce {} is unresolved before nonce {} can be recorded",
                existing.nonce, submission.nonce
            ))),
        }
    }

    const fn uncertain_submission(&self) -> Option<&BoundlessUncertainSubmission> {
        self.uncertain.as_ref()
    }

    fn clear_uncertain(&mut self, nonce: u64, request_digest: B256) -> bool {
        let matches = self.uncertain.as_ref().is_some_and(|submission| {
            submission.nonce == nonce && submission.request_digest == request_digest
        });
        if matches {
            self.uncertain = None;
        }
        matches
    }

    fn reconcile_consumed_nonce(&mut self, chain_nonce: u64) -> bool {
        let consumed = self
            .uncertain
            .as_ref()
            .is_some_and(|submission| chain_nonce > submission.nonce);
        if consumed {
            self.uncertain = None;
            self.next_nonce = Some(self.next_nonce.unwrap_or_default().max(chain_nonce));
        }
        consumed
    }

    fn record_recent(
        &mut self,
        request_id: U256,
        max_price: U256,
        lock_expires_at: u64,
        request_digest: B256,
    ) {
        let recent = RecentFundingRequest {
            max_price,
            lock_expires_at,
        };
        self.recent
            .entry(request_id)
            .or_default()
            .entry(request_digest)
            .and_modify(|existing| {
                existing.max_price = existing.max_price.max(max_price);
                existing.lock_expires_at = existing.lock_expires_at.max(lock_expires_at);
            })
            .or_insert(recent);
    }

    fn reconcile_receipt(
        &mut self,
        request_id: U256,
        request_digest: B256,
        outcome: BoundlessReceiptOutcome,
    ) {
        if outcome != BoundlessReceiptOutcome::ConfirmedRevert {
            return;
        }

        let remove_request_id = self
            .recent
            .get_mut(&request_id)
            .is_some_and(|recent_by_digest| {
                recent_by_digest.remove(&request_digest);
                recent_by_digest.is_empty()
            });
        if remove_request_id {
            self.recent.remove(&request_id);
        }
    }

    fn funding_decision(
        &mut self,
        current_request_id: U256,
        current_max_price: U256,
        on_chain_balance: U256,
        now: u64,
    ) -> BoundlessFundingDecision {
        self.recent.retain(|_, recent_by_digest| {
            // The market evaluates lock expiry against block.timestamp, while `now` is local wall
            // clock. Retain a short grace so a locally fast clock cannot release funds that a
            // slightly lagging chain can still debit.
            recent_by_digest.retain(|_, recent| {
                now <= recent
                    .lock_expires_at
                    .saturating_add(BOUNDLESS_FUNDING_EXPIRY_GRACE_SECS)
            });
            !recent_by_digest.is_empty()
        });

        let mut required_by_id: HashMap<U256, U256> = HashMap::new();
        for (&request_id, recent_by_digest) in &self.recent {
            if let Some(max_price) = recent_by_digest
                .values()
                .map(|recent| recent.max_price)
                .max()
            {
                required_by_id
                    .entry(request_id)
                    .and_modify(|price| *price = (*price).max(max_price))
                    .or_insert(max_price);
            }
        }
        required_by_id
            .entry(current_request_id)
            .and_modify(|price| *price = (*price).max(current_max_price))
            .or_insert(current_max_price);

        let reserved_count = required_by_id.len();
        let required_total = required_by_id
            .into_values()
            .fold(U256::ZERO, U256::saturating_add);
        BoundlessFundingDecision {
            reserved_count,
            required_total,
            attached_value: deposit_topup(on_chain_balance, required_total),
        }
    }
}

/// Serialization point for on-chain submissions sharing one Boundless market account. The
/// submission permit orders every transaction from the shared signer, while the separately guarded
/// state retains requests sent by this process until their market lock expires.
#[derive(Clone)]
pub struct BoundlessBalanceGate {
    submission: Arc<tokio::sync::Semaphore>,
    state: Arc<tokio::sync::Mutex<BoundlessFundingState>>,
}

impl Default for BoundlessBalanceGate {
    fn default() -> Self {
        Self {
            submission: Arc::new(tokio::sync::Semaphore::new(1)),
            state: Arc::new(tokio::sync::Mutex::new(BoundlessFundingState::default())),
        }
    }
}

impl BoundlessBalanceGate {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    async fn acquire_submission(&self) -> BoundlessSubmissionPermit {
        BoundlessSubmissionPermit {
            permit: self
                .submission
                .clone()
                .acquire_owned()
                .await
                .expect("Boundless submission semaphore is never closed"),
        }
    }

    async fn lock_state(&self) -> tokio::sync::MutexGuard<'_, BoundlessFundingState> {
        self.state.lock().await
    }
}

struct BoundlessSubmissionPermit {
    permit: tokio::sync::OwnedSemaphorePermit,
}

impl BoundlessSubmissionPermit {
    async fn acquire_broadcast_permit(
        &self,
        observer: Option<&Arc<dyn ProverProgressObserver>>,
    ) -> RaikoResult<crate::SubmissionCheckpointPermit> {
        crate::acquire_submission_checkpoint_permit(observer).await
    }
}

async fn run_after_submission_permit<T, F, Fut>(permit: BoundlessSubmissionPermit, run: F) -> T
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = T>,
{
    let BoundlessSubmissionPermit { permit } = permit;
    drop(permit);
    run().await
}

#[allow(clippy::too_many_arguments)]
async fn reserve_boundless_funding_before_dispatch<T, F, Fut>(
    balance_gate: &BoundlessBalanceGate,
    request: &ProofRequest,
    submission: &Submission,
    signature: Bytes,
    request_digest: B256,
    market_balance: U256,
    latest_nonce: u64,
    pending_nonce: u64,
    now: u64,
    dispatch: F,
) -> RaikoResult<(u64, T)>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = T>,
{
    let max_price = U256::from(request.offer.maxPrice);
    let (decision, nonce) = {
        let mut state = balance_gate.lock_state().await;
        let decision = state.funding_decision(request.id, max_price, market_balance, now);
        let nonce = state.allocate_nonce(latest_nonce, pending_nonce)?;
        state.record_uncertain(BoundlessUncertainSubmission {
            submission: submission.clone(),
            request: request.clone(),
            signature,
            request_digest,
            value: decision.attached_value,
            nonce,
        })?;
        state.record_recent(
            request.id,
            max_price,
            submission.lock_expires_at,
            request_digest,
        );
        (decision, nonce)
    };
    tracing::info!(
        request_id = %request.id,
        reserved_count = decision.reserved_count,
        market_balance = %market_balance,
        required_total = %decision.required_total,
        attached_value = %decision.attached_value,
        "Prepared Boundless funding decision"
    );

    Ok((nonce, dispatch().await))
}

pub struct BoundlessProver {
    config: BoundlessConfig,
    deployment: Deployment,
    /// One market `Client` (provider + signer + storage uploader) built lazily on first proof and
    /// reused across every subsequent proof, so we don't rebuild the RPC provider and signer per
    /// proof. Lazy (rather than in `new()`) because building it is fallible and async.
    client: tokio::sync::OnceCell<BoundlessClient>,
    programs: Arc<RwLock<HashMap<ElfType, UploadedProgram>>>,
    /// Balance gate serializing on-chain funding and retaining active local reservations. Shared
    /// across every pair's prover because they fund one market account; see
    /// [`BoundlessBalanceGate`].
    balance_gate: BoundlessBalanceGate,
    status_tracker: OnceLock<RemoteStatusTracker>,
    status_registry: BoundlessStatusRegistry,
}

impl BoundlessProver {
    /// Validate process-level Boundless storage selection before the server starts workers.
    ///
    /// # Errors
    ///
    /// Returns an error when uploader environment variables are invalid or request a storage
    /// backend that was not compiled into this build.
    pub fn validate_storage_configuration() -> RaikoResult<()> {
        let config = storage_uploader_config_from_env()?;
        if config.storage_uploader == StorageUploaderType::None {
            return Err(RaikoError::InvalidRequestConfig(
                "Boundless proving requires a storage uploader".to_string(),
            ));
        }
        Ok(())
    }

    /// Build a prover with its own private balance gate. Use [`BoundlessProver::with_balance_gate`]
    /// in production so every pair funding the same market account shares one gate.
    #[must_use]
    pub fn new(config: BoundlessConfig) -> Self {
        Self::with_balance_gate(config, BoundlessBalanceGate::new())
    }

    /// Build a prover sharing `balance_gate` with every other prover funding the same market account
    /// (see [`BoundlessBalanceGate`]). Pairs share one signer/market, so the caller builds one gate
    /// at setup and clones it into each pair's prover.
    #[must_use]
    pub fn with_balance_gate(config: BoundlessConfig, balance_gate: BoundlessBalanceGate) -> Self {
        Self {
            deployment: config.get_effective_deployment(),
            config,
            client: tokio::sync::OnceCell::new(),
            programs: Arc::new(RwLock::new(HashMap::new())),
            balance_gate,
            status_tracker: OnceLock::new(),
            status_registry: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Returns the shared market `Client`, building it once on first call and reusing the same
    /// instance for every later proof. The build is wrapped in `retry_external` so a transient
    /// storage/RPC hiccup at first use retries instead of failing the proof; on success the
    /// `Client` is cached and later callers skip the build entirely.
    async fn client(&self) -> RaikoResult<&BoundlessClient> {
        // `Box::pin` the init future: building the client (storage uploader + provider + signer) is
        // a large future, and `get_or_try_init` would otherwise inline it into this frame.
        self.client
            .get_or_try_init(|| {
                Box::pin(retry_external("create boundless client", || {
                    self.create_client()
                }))
            })
            .await
    }

    async fn create_client(&self) -> RaikoResult<BoundlessClient> {
        let rpc_url = Url::parse(&self.config.rpc_url).map_err(|e| {
            RaikoError::InvalidRequestConfig(format!("Invalid boundless rpc_url: {e}"))
        })?;
        let signer: PrivateKeySigner = self.config.signer_key.parse().map_err(|e| {
            RaikoError::InvalidRequestConfig(format!("Invalid boundless signer_key: {e}"))
        })?;
        let storage_config = storage_uploader_config_from_env()?;
        let downloader = BoundlessStorageDownloader::from_uploader_config(&storage_config)
            .await
            .map_err(|e| {
                RaikoError::InvalidRequestConfig(format!(
                    "Failed to configure boundless storage downloader: {e}"
                ))
            })?;
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
            .with_downloader(downloader)
            .build()
            .await
            .map_err(|e| {
                RaikoError::InvalidRequestConfig(format!("Failed to build boundless client: {e}"))
            })
    }

    async fn funding_balance(client: &BoundlessClient, requestor: Address) -> RaikoResult<U256> {
        // This read deliberately avoids the external request-history indexer. The configured
        // market RPC must expose a monotonic `latest` view; operators using a load balancer must
        // keep lagging nodes out of rotation (see docs/operations.md).
        retry_external_bounded(
            "query boundless balance",
            BOUNDLESS_RPC_TOTAL_TIMEOUT,
            || async {
                tokio::time::timeout(
                    BOUNDLESS_RPC_REQUEST_TIMEOUT,
                    client.boundless_market.balance_of(requestor),
                )
                .await
                .map_err(|_| {
                    RaikoError::Guest("Boundless market balance query timed out".to_string())
                })?
                .map_err(|e| RaikoError::Guest(format!("Failed to query boundless balance: {e}")))
            },
        )
        .await
    }

    async fn account_nonces(client: &BoundlessClient) -> RaikoResult<(u64, u64)> {
        let provider = client.provider();
        let requestor = client.boundless_market.caller();
        retry_external_bounded(
            "query boundless account nonces",
            BOUNDLESS_RPC_TOTAL_TIMEOUT,
            || async {
                let latest = tokio::time::timeout(
                    BOUNDLESS_RPC_REQUEST_TIMEOUT,
                    provider.get_transaction_count(requestor).latest(),
                )
                .await
                .map_err(|_| {
                    RaikoError::Guest("Boundless latest nonce query timed out".to_string())
                })?
                .map_err(|error| {
                    RaikoError::Guest(format!("Failed to query Boundless latest nonce: {error}"))
                })?;
                let pending = tokio::time::timeout(
                    BOUNDLESS_RPC_REQUEST_TIMEOUT,
                    provider.get_transaction_count(requestor).pending(),
                )
                .await
                .map_err(|_| {
                    RaikoError::Guest("Boundless pending nonce query timed out".to_string())
                })?
                .map_err(|error| {
                    RaikoError::Guest(format!("Failed to query Boundless pending nonce: {error}"))
                })?;
                Ok((latest, pending))
            },
        )
        .await
    }

    fn status_tracker(&self) -> &RemoteStatusTracker {
        self.status_tracker.get_or_init(|| {
            let poll_interval = Duration::from_millis(self.config.poll_interval_ms.max(1));
            let source = Arc::new(BoundlessStatusSource::new(
                self.config.rpc_url.clone(),
                &self.deployment,
                Arc::clone(&self.status_registry),
                poll_interval,
            ));
            let mut sources: HashMap<ProofType, Arc<dyn RemoteStatusSource>> = HashMap::new();
            sources.insert(ProofType::Risc0, source);
            RemoteStatusTracker::spawn(RemotePollerConfig::new(poll_interval), sources)
        })
    }

    /// Ensure the program for `elf_type` is uploaded with a live presigned URL, returning the cached
    /// entry on a hit. `image_id` is passed in (computed once per proof) rather than recomputed here:
    /// this runs every rebid rung, and hashing the multi-MB ELF on each cache hit was pure waste.
    async fn ensure_uploaded(
        &self,
        client: &BoundlessClient,
        elf_type: ElfType,
        elf: &[u8],
        image_id: Digest,
    ) -> RaikoResult<UploadedProgram> {
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

    /// Ensure the guest input is uploaded with a live presigned URL, returning the cached URL on a
    /// hit. Building the `GuestEnv` from `input` and encoding it (both multi-MB copies) happen only
    /// in the cache-miss branch — a rebid rung that hits the cache skips the env construction, the
    /// encode, and the upload. The env is used solely to produce the uploaded bytes: the request
    /// itself carries the input by URL (`with_input_url`), and every SDK layer that would otherwise
    /// consume a `with_env(...)` value is short-circuited because raiko2 sets `request_input`,
    /// `cycles`, and `journal` explicitly, so no `GuestEnv` is threaded into `build_request`.
    async fn ensure_input_uploaded(
        &self,
        client: &BoundlessClient,
        input: &[u8],
        cache: &mut Option<UploadedInput>,
    ) -> RaikoResult<Url> {
        if let Some(input) = cache.as_ref()
            && SystemTime::now() < input.refresh_at
        {
            return Ok(input.url.clone());
        }
        let guest_env_bytes = GuestEnv::builder()
            .write_frame(input)
            .build_env()
            .encode()
            .map_err(|e| {
                RaikoError::InvalidRequestConfig(format!("Failed to encode guest environment: {e}"))
            })?;
        let url = retry_external("upload boundless input", || async {
            client.upload_input(&guest_env_bytes).await.map_err(|e| {
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
        client: &BoundlessClient,
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
        } = validate_offer_params(offer_spec, mcycles_count)?;
        // Escalate only the manual max price on resubmissions (compounding bps per rung), then
        // clamp it to the absolute ceiling threaded as `max_price_cap`; the min price keeps the
        // ramp start unchanged so an idle prover still locks cheaply.
        let max_price = max_price
            .map(|mut amount| -> RaikoResult<_> {
                let clamped = escalate_and_clamp_manual_max_price(
                    amount.value,
                    attempt,
                    self.config.rebid_price_step_bps,
                    self.config.rebid_max_attempts,
                    max_price_cap.as_ref().map(|cap| cap.value),
                )?;
                if clamped.clamped_to_ceiling {
                    tracing::warn!(
                        mcycles_count,
                        configured_max_price_wei = %amount.value,
                        ceiling_max_price_wei = %clamped.max_price,
                        "Boundless manual offer max price exceeds the absolute_max_price_per_mcycle ceiling; bidding at the ceiling"
                    );
                }
                amount.value = clamped.max_price;
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
        // No `.with_env(...)`: the input is carried by URL (`with_input_url` below), and every SDK
        // request-builder layer that would consume a `GuestEnv` is short-circuited because
        // `request_input`, `cycles`, and `journal` are all set — the storage layer skips env upload
        // when `request_input` is present, and the preflight layer early-returns on set
        // cycles+journal. `ensure_input_uploaded` already produced the uploaded bytes from the raw
        // input, so building a second env here would just be a discarded multi-MB copy per rebid.
        let mut request_params = client
            .new_request()
            .with_program(elf.to_vec())
            .with_program_url(program.url.clone())
            .expect("with_program_url is infallible for valid URLs")
            .with_groth16_proof()
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

    /// Builds the durable submission identity before dispatching a finalized request.
    ///
    /// Derives the provider id,
    /// deadlines, and exact escalated max price from `request`, and the floored display multiplier
    /// from `attempt` + config. Both dispatch paths checkpoint this exact non-zero request id before
    /// sending; `remote_tx_hash` is populated later when the on-chain transport returns it.
    fn make_submission(&self, request: &ProofRequest, attempt: u64) -> RaikoResult<Submission> {
        let market_request_id = request.id;
        if market_request_id == U256::ZERO {
            return Err(RaikoError::InvalidRequestConfig(
                "Finalized Boundless request id must be nonzero before submission".to_string(),
            ));
        }
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
            max_price_wei: U256::from(request.offer.maxPrice),
            attempt,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn submit_request_offchain(
        &self,
        client: &BoundlessClient,
        request: &ProofRequest,
        observer: Option<&Arc<dyn ProverProgressObserver>>,
        permit: crate::SubmissionCheckpointPermit,
        image_ref: &str,
        deployment: &str,
        mcycles_count: (u32, u32),
        attempt: u64,
    ) -> RaikoResult<Submission> {
        let submission = self.make_submission(request, attempt)?;
        dispatch_offchain_after_checkpoint(
            observer,
            permit,
            submission,
            image_ref,
            deployment,
            mcycles_count,
            || async {
                client
                    .submit_request_offchain(request)
                    .await
                    .map(|(id, _)| id)
                    .map_err(|error| {
                        RaikoError::Guest(format!(
                            "Boundless offchain request dispatch failed: {error}"
                        ))
                    })
            },
        )
        .await
    }

    async fn sign_onchain_request(
        client: &BoundlessClient,
        request: &ProofRequest,
    ) -> RaikoResult<(B256, Bytes)> {
        let signer = client.signer.as_ref().ok_or_else(|| {
            RaikoError::InvalidRequestConfig("Boundless signer is not configured".to_string())
        })?;
        let chain_id = retry_external_bounded(
            "query boundless chain id",
            BOUNDLESS_RPC_TOTAL_TIMEOUT,
            || async {
                tokio::time::timeout(
                    BOUNDLESS_RPC_REQUEST_TIMEOUT,
                    client.boundless_market.get_chain_id(),
                )
                .await
                .map_err(|_| RaikoError::Guest("Boundless chain id query timed out".to_string()))?
                .map_err(|error| {
                    RaikoError::Guest(format!("Failed to query boundless chain id: {error}"))
                })
            },
        )
        .await?;
        let market_addr = *client.boundless_market.instance().address();
        let request_digest = request
            .signing_hash(market_addr, chain_id)
            .map_err(|error| {
                RaikoError::Guest(format!("Failed to hash Boundless request: {error}"))
            })?;
        let signature = request
            .sign_request(signer, market_addr, chain_id)
            .await
            .map_err(|error| {
                RaikoError::Guest(format!("Failed to sign Boundless request: {error}"))
            })?;
        Ok((request_digest, signature.as_bytes().into()))
    }

    async fn broadcast_uncertain_submission(
        client: &BoundlessClient,
        uncertain: &BoundlessUncertainSubmission,
    ) -> RaikoResult<PendingTransactionBuilder<Ethereum>> {
        let call = client
            .boundless_market
            .instance()
            .submitRequest(uncertain.request.clone(), uncertain.signature.clone())
            .from(client.boundless_market.caller())
            .value(uncertain.value)
            .nonce(uncertain.nonce);

        tokio::time::timeout(BOUNDLESS_SUBMIT_SEND_TIMEOUT, call.send())
            .await
            .map_err(|_| {
                RaikoError::Guest(format!(
                    "Timed out broadcasting Boundless submitRequest at nonce {}",
                    uncertain.nonce
                ))
            })?
            .map_err(|error| {
                RaikoError::Guest(format!(
                    "Boundless submitRequest at nonce {} returned an uncertain error: {error}",
                    uncertain.nonce
                ))
            })
    }

    async fn confirm_consumed_nonce(
        &self,
        client: &BoundlessClient,
        nonce: u64,
    ) -> RaikoResult<bool> {
        let provider = client.provider();
        let confirmation_wait = async {
            let observed_head =
                tokio::time::timeout(BOUNDLESS_RPC_REQUEST_TIMEOUT, provider.get_block_number())
                    .await
                    .map_err(|_| {
                        RaikoError::Guest("Boundless confirmation head query timed out".to_string())
                    })?
                    .map_err(|error| {
                        RaikoError::Guest(format!(
                            "Failed to query Boundless confirmation head: {error}"
                        ))
                    })?;
            let target_head = observed_head
                .saturating_add(BOUNDLESS_SUBMIT_RECEIPT_CONFIRMATIONS.saturating_sub(1));

            loop {
                let head = tokio::time::timeout(
                    BOUNDLESS_RPC_REQUEST_TIMEOUT,
                    provider.get_block_number(),
                )
                .await
                .map_err(|_| {
                    RaikoError::Guest("Boundless confirmation head query timed out".to_string())
                })?
                .map_err(|error| {
                    RaikoError::Guest(format!(
                        "Failed to query Boundless confirmation head: {error}"
                    ))
                })?;
                if head >= target_head {
                    let (latest, _) = Self::account_nonces(client).await?;
                    return Ok(latest > nonce);
                }
                tokio::time::sleep(BOUNDLESS_NONCE_RECOVERY_POLL_DELAY).await;
            }
        };

        tokio::time::timeout(BOUNDLESS_SUBMIT_RECEIPT_TIMEOUT, confirmation_wait)
            .await
            .map_err(|_| {
                RaikoError::Guest(format!(
                    "Timed out confirming consumed Boundless nonce {nonce}"
                ))
            })?
    }

    async fn resolve_uncertain_submission(
        &self,
        client: &BoundlessClient,
    ) -> RaikoResult<Option<String>> {
        let uncertain = self
            .balance_gate
            .lock_state()
            .await
            .uncertain_submission()
            .cloned()
            .ok_or_else(|| {
                RaikoError::Guest(
                    "Boundless nonce recovery requested without an uncertain submission"
                        .to_string(),
                )
            })?;

        let recovery = async {
            loop {
                match Self::broadcast_uncertain_submission(client, &uncertain).await {
                    Ok(pending_tx) => {
                        let remote_tx_hash = format!("0x{:x}", pending_tx.tx_hash());
                        let mut observed_submission = uncertain.submission.clone();
                        observed_submission.remote_tx_hash = Some(remote_tx_hash.clone());
                        let outcome = self
                            .observe_onchain_receipt(
                                pending_tx,
                                &observed_submission,
                                uncertain.request.id,
                                uncertain.request_digest,
                            )
                            .await;
                        if outcome == BoundlessReceiptOutcome::Uncertain {
                            return Err(RaikoError::Guest(format!(
                                "Boundless submitRequest at nonce {} has an uncertain receipt",
                                uncertain.nonce
                            )));
                        }
                        self.balance_gate
                            .lock_state()
                            .await
                            .clear_uncertain(uncertain.nonce, uncertain.request_digest);
                        return Ok(Some(remote_tx_hash));
                    }
                    Err(error) => {
                        let (latest, pending) = Self::account_nonces(client).await?;
                        match classify_boundless_nonce(uncertain.nonce, latest, pending) {
                            BoundlessNonceStatus::Mined => {
                                if self.confirm_consumed_nonce(client, uncertain.nonce).await? {
                                    self.balance_gate
                                        .lock_state()
                                        .await
                                        .reconcile_consumed_nonce(latest);
                                    tracing::warn!(
                                        provider_request_id = %uncertain.submission.provider_request_id,
                                        nonce = uncertain.nonce,
                                        error = %error,
                                        "Recovered Boundless submission from its consumed nonce without a transaction hash"
                                    );
                                    return Ok(None);
                                }
                            }
                            BoundlessNonceStatus::Pending => tracing::warn!(
                                provider_request_id = %uncertain.submission.provider_request_id,
                                nonce = uncertain.nonce,
                                error = %error,
                                "Boundless submission is pending without an acknowledged transaction hash; retrying the same request and nonce"
                            ),
                            BoundlessNonceStatus::Available => tracing::warn!(
                                provider_request_id = %uncertain.submission.provider_request_id,
                                nonce = uncertain.nonce,
                                error = %error,
                                "Boundless submission nonce remains available; retrying the same request and nonce"
                            ),
                        }
                        tokio::time::sleep(BOUNDLESS_NONCE_RECOVERY_POLL_DELAY).await;
                    }
                }
            }
        };

        tokio::time::timeout(BOUNDLESS_NONCE_RECOVERY_TOTAL_TIMEOUT, recovery)
            .await
            .map_err(|_| {
                RaikoError::Guest(format!(
                    "Timed out recovering Boundless submitRequest at nonce {}",
                    uncertain.nonce
                ))
            })?
    }

    async fn observe_onchain_receipt(
        &self,
        pending_tx: PendingTransactionBuilder<Ethereum>,
        submission: &Submission,
        request_id: U256,
        request_digest: B256,
    ) -> BoundlessReceiptOutcome {
        let outcome = match tokio::time::timeout(
            BOUNDLESS_SUBMIT_RECEIPT_TIMEOUT,
            pending_tx
                .with_required_confirmations(BOUNDLESS_SUBMIT_RECEIPT_CONFIRMATIONS)
                .get_receipt(),
        )
        .await
        {
            Ok(Ok(receipt)) if receipt.status() => BoundlessReceiptOutcome::ConfirmedSuccess,
            Ok(Ok(_)) => {
                tracing::warn!(
                    provider_request_id = %submission.provider_request_id,
                    remote_tx_hash = ?submission.remote_tx_hash,
                    "Boundless submitRequest transaction reverted; removing its local funding reservation"
                );
                BoundlessReceiptOutcome::ConfirmedRevert
            }
            Ok(Err(error)) => {
                tracing::warn!(
                    provider_request_id = %submission.provider_request_id,
                    remote_tx_hash = ?submission.remote_tx_hash,
                    error = %error,
                    "Boundless submitRequest receipt is uncertain; retaining its local funding reservation"
                );
                BoundlessReceiptOutcome::Uncertain
            }
            Err(_) => {
                tracing::warn!(
                    provider_request_id = %submission.provider_request_id,
                    remote_tx_hash = ?submission.remote_tx_hash,
                    timeout_secs = BOUNDLESS_SUBMIT_RECEIPT_TIMEOUT.as_secs(),
                    "Timed out waiting for Boundless submitRequest receipt; retaining its local funding reservation"
                );
                BoundlessReceiptOutcome::Uncertain
            }
        };
        self.balance_gate
            .lock_state()
            .await
            .reconcile_receipt(request_id, request_digest, outcome);
        outcome
    }

    #[allow(clippy::too_many_arguments)]
    async fn submit_request_onchain(
        &self,
        client: &BoundlessClient,
        request: &ProofRequest,
        observer: Option<&Arc<dyn ProverProgressObserver>>,
        image_ref: &str,
        deployment: &str,
        quoted_mcycles_count: u32,
        evaluated_mcycles_count: u32,
        attempt: u64,
    ) -> RaikoResult<Submission> {
        // Sign the request before reserving on the balance gate: signing is independent of the
        // deposit value, so there's no reason to hold a reservation across it. The bounded chain id
        // query retries transient RPC errors. `get_chain_id` caches the value inside the SDK's
        // market service after the first fetch, and the `Client` is reused across proofs.
        let (request_digest, signature) = Self::sign_onchain_request(client, request).await?;

        let mut submission = self.make_submission(request, attempt)?;
        let checkpoint_permit = crate::acquire_submission_checkpoint_permit(observer).await?;
        publish_boundless_progress(
            observer,
            &checkpoint_permit,
            &submission,
            image_ref,
            deployment,
            false,
            (quoted_mcycles_count, evaluated_mcycles_count),
        )
        .await?;
        drop(checkpoint_permit);

        // Serialize every account operation through receipt observation so each later request sees
        // either the preceding transaction's balance effect or its conservative local reservation.
        let submission_permit = self.balance_gate.acquire_submission().await;
        if self
            .balance_gate
            .lock_state()
            .await
            .uncertain_submission()
            .is_some()
        {
            let recovery_permit = submission_permit.acquire_broadcast_permit(observer).await?;
            let recovery = self.resolve_uncertain_submission(client).await;
            drop(recovery_permit);
            recovery?;
        }
        let balance = Self::funding_balance(client, request.client_address()).await?;
        let (latest_nonce, pending_nonce) = Self::account_nonces(client).await?;
        // Recheck runtime admission after the read-only account work and before nonce reservation:
        // draining may have started while this task waited for the account permit or RPCs.
        let broadcast_permit = submission_permit.acquire_broadcast_permit(observer).await?;
        // The helper records the funding reservation and uncertain nonce before invoking the only
        // broadcast path, so cancellation at any dispatch await cannot expose an untracked send.
        let (nonce, resolution) = reserve_boundless_funding_before_dispatch(
            &self.balance_gate,
            request,
            &submission,
            signature,
            request_digest,
            balance,
            latest_nonce,
            pending_nonce,
            now_secs(),
            || self.resolve_uncertain_submission(client),
        )
        .await?;

        match resolution {
            Ok(remote_tx_hash) => submission.remote_tx_hash = remote_tx_hash,
            Err(error) => tracing::warn!(
                provider_request_id = %submission.provider_request_id,
                nonce,
                error = %error,
                "Boundless submitRequest remains uncertain; retaining its nonce and funding reservation"
            ),
        }
        drop(broadcast_permit);

        run_after_submission_permit(submission_permit, || async {
            if submission.remote_tx_hash.is_some() {
                checkpoint_boundless_tx_hash(
                    observer,
                    &submission,
                    image_ref,
                    deployment,
                    (quoted_mcycles_count, evaluated_mcycles_count),
                )
                .await;
            }
        })
        .await;

        Ok(submission)
    }

    async fn submit_fresh_request(
        &self,
        context: FreshSubmissionContext<'_>,
    ) -> RaikoResult<Submission> {
        let input_url = self
            .ensure_input_uploaded(context.client, context.input.as_ref(), context.input_cache)
            .await?;
        let request = Box::pin(self.build_request(
            context.client,
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
            let checkpoint_permit =
                crate::acquire_submission_checkpoint_permit(context.observer).await?;
            return self
                .submit_request_offchain(
                    context.client,
                    &request,
                    context.observer,
                    checkpoint_permit,
                    context.image_ref,
                    context.deployment,
                    (
                        context.quoted_mcycles_count,
                        context.evaluated_mcycles_count,
                    ),
                    context.attempt,
                )
                .await;
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

    async fn poll_until_fulfilled(
        &self,
        client: &BoundlessClient,
        submission: &Submission,
        context: &FulfillmentContext<'_>,
        no_lock_timeout: NoLockTimeout,
    ) -> Result<Proof, BoundlessAttemptError> {
        let timeout = Duration::from_millis(self.config.timeout_ms.max(1));
        let poll_timeout_at = Instant::now() + timeout;
        let submission_id = RemoteSubmissionId::new();
        self.status_registry
            .lock()
            .map_err(|err| {
                BoundlessAttemptError::Fatal(RaikoError::Guest(format!(
                    "Boundless status registry lock poisoned: {err}"
                )))
            })?
            .insert(
                submission_id,
                BoundlessSubmissionState {
                    metadata: BoundlessSubmissionMetadata {
                        expires_at: submission.expires_at,
                        lock_expires_at: submission.lock_expires_at,
                        submitted_at: submission.submitted_at,
                        no_lock_deadline: no_lock_deadline(
                            submission.submitted_at,
                            submission.lock_expires_at,
                            no_lock_timeout,
                        ),
                        no_lock_timeout_action: no_lock_timeout.action,
                        poll_timeout_at,
                    },
                    terminal_outcome: None,
                },
            );
        let status_tracker = self.status_tracker().clone();
        let _guard = BoundlessSubmissionGuard::new(
            status_tracker.clone(),
            Arc::clone(&self.status_registry),
            submission_id,
        );
        let (terminal_tx, terminal_rx) = tokio::sync::oneshot::channel();
        let remote_submission = RemoteSubmission {
            id: submission_id,
            proof_type: ProofType::Risc0,
            provider_request_id: submission.provider_request_id.clone(),
            timeout_at: None,
        };
        status_tracker
            .register(remote_submission, terminal_tx)
            .map_err(|err| {
                BoundlessAttemptError::Fatal(RaikoError::Guest(format!(
                    "Failed to register boundless status poll: {err}"
                )))
            })?;

        let terminal = terminal_rx.await.map_err(|err| {
            BoundlessAttemptError::Fatal(RaikoError::Guest(format!(
                "Boundless status poller stopped before terminal status: {err}"
            )))
        })?;

        match terminal {
            RemoteTerminalResult::Fulfilled { .. } => {
                self.fetch_boundless_fulfillment_until(client, submission, context, poll_timeout_at)
                    .await
            }
            RemoteTerminalResult::Expired { reason, .. } => Err(BoundlessAttemptError::Retryable {
                reason: reason.message,
                rotate_request_id: true,
            }),
            RemoteTerminalResult::Failed { reason, .. } => {
                let terminal_outcome =
                    boundless_terminal_outcome(&self.status_registry, submission_id)?;
                Self::boundless_failed_terminal(
                    submission,
                    no_lock_timeout,
                    reason,
                    terminal_outcome,
                )
            }
            RemoteTerminalResult::TimedOut { reason, .. } => {
                Err(BoundlessAttemptError::Retryable {
                    reason: reason.message,
                    rotate_request_id: now_secs() >= submission.expires_at,
                })
            }
            RemoteTerminalResult::Unrecoverable { reason, .. } => Err(
                BoundlessAttemptError::Fatal(RaikoError::Guest(reason.message)),
            ),
        }
    }

    fn boundless_failed_terminal(
        submission: &Submission,
        no_lock_timeout: NoLockTimeout,
        reason: RemoteStatusReason,
        terminal_outcome: Option<BoundlessTerminalOutcome>,
    ) -> Result<Proof, BoundlessAttemptError> {
        match terminal_outcome {
            Some(BoundlessTerminalOutcome::NoLockRebidTimeout) => {
                Err(BoundlessAttemptError::Retryable {
                    reason: format!(
                        "Boundless request {} was not locked within {} seconds; \
                         rebidding with higher max price under the same request id",
                        submission.provider_request_id,
                        no_lock_timeout.delay.as_secs()
                    ),
                    rotate_request_id: false,
                })
            }
            Some(BoundlessTerminalOutcome::NoLockAbortTimeout) => {
                let deadline_detail = if submission.lock_expires_at > 0 {
                    "before its payable window closed".to_string()
                } else {
                    format!("within {} seconds", no_lock_timeout.delay.as_secs())
                };
                let Some(attempt) = u32::try_from(submission.attempt)
                    .ok()
                    .and_then(std::num::NonZeroU32::new)
                else {
                    return Err(BoundlessAttemptError::Fatal(RaikoError::Guest(format!(
                        "Boundless request {} has invalid terminal checkpoint attempt {}",
                        submission.provider_request_id, submission.attempt
                    ))));
                };
                Err(BoundlessAttemptError::TerminalCheckpoint {
                    identity: PendingProofCheckpointIdentity {
                        backend: crate::NetworkProverBackend::Boundless,
                        provider_request_id: submission.provider_request_id.clone(),
                        attempt,
                    },
                    error: RaikoError::Guest(format!(
                        "Boundless request {} was not locked {deadline_detail}; \
                         exhausted boundless no-lock rebids",
                        submission.provider_request_id
                    )),
                })
            }
            Some(BoundlessTerminalOutcome::PollTimeout { rotate_request_id }) => {
                Err(BoundlessAttemptError::Retryable {
                    reason: reason.message,
                    rotate_request_id,
                })
            }
            Some(
                BoundlessTerminalOutcome::MarketExpired | BoundlessTerminalOutcome::LockExpired,
            ) => Err(BoundlessAttemptError::Retryable {
                reason: reason.message,
                rotate_request_id: true,
            }),
            None => Err(BoundlessAttemptError::Fatal(RaikoError::Guest(
                reason.message,
            ))),
        }
    }

    async fn fetch_boundless_fulfillment_until(
        &self,
        client: &BoundlessClient,
        submission: &Submission,
        context: &FulfillmentContext<'_>,
        deadline: Instant,
    ) -> Result<Proof, BoundlessAttemptError> {
        loop {
            match self
                .fetch_boundless_fulfillment(client, submission, context)
                .await
            {
                Ok(proof) => return Ok(proof),
                Err(BoundlessAttemptError::Retryable {
                    reason,
                    rotate_request_id: _,
                }) if Instant::now() < deadline => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    let delay = EXTERNAL_RETRY_INITIAL_DELAY.min(remaining);
                    tracing::warn!(
                        provider_request_id = %submission.provider_request_id,
                        reason,
                        delay_ms = delay.as_millis(),
                        "Boundless fulfillment read lagged after fulfilled status; retrying"
                    );
                    tokio::time::sleep(delay).await;
                }
                result => return result,
            }
        }
    }

    /// Stage metadata recorded on the finished proof for telemetry/debugging.
    fn boundless_stage_metadata(
        &self,
        submission: &Submission,
        context: &FulfillmentContext<'_>,
    ) -> serde_json::Value {
        serde_json::json!({
            "zkvm": "risc0",
            "runner": "network",
            "proof_type": context.proof_type,
            "quoted_mcycles_count": context.quoted_mcycles_count,
            "evaluated_mcycles_count": context.evaluated_mcycles_count,
            "boundless": {
                "provider_request_id": submission.provider_request_id,
                "remote_tx_hash": submission.remote_tx_hash,
                "expires_at": submission.expires_at,
                "lock_expires_at": submission.lock_expires_at,
                "submitted_at": submission.submitted_at,
                "max_price_multiplier": submission.max_price_multiplier,
                "max_price_wei": submission.max_price_wei.to_string(),
                "image_id": alloy_primitives::hex::encode_prefixed(context.image_id.as_bytes()),
                "deployment": format!("{:?}", self.config.get_deployment_type()).to_lowercase(),
                "offchain": self.config.offchain,
            }
        })
    }

    async fn fetch_boundless_fulfillment(
        &self,
        client: &BoundlessClient,
        submission: &Submission,
        context: &FulfillmentContext<'_>,
    ) -> Result<Proof, BoundlessAttemptError> {
        let fulfillment = retry_external("read boundless fulfillment", || async {
            client
                .boundless_market
                .get_request_fulfillment(submission.market_request_id, None, None)
                .await
                .map_err(|error| {
                    RaikoError::Guest(format!("Failed to read boundless fulfillment: {error}"))
                })
        })
        .await
        .map_err(|err| BoundlessAttemptError::Retryable {
            reason: err.to_string(),
            rotate_request_id: now_secs() >= submission.expires_at,
        })?;
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
        let receipt_json = if context.proof_type == "proposal" {
            match decode_seal(seal.clone(), context.image_id, journal.to_vec()) {
                Ok(ContractReceipt::Base(receipt)) => serde_json::to_string(&receipt).ok(),
                Ok(ContractReceipt::SetInclusion(_)) | Err(_) => None,
            }
        } else {
            None
        };
        let input_hash = match context.proof_type {
            "proposal" => parse_shasta_proposal_input_hash(journal)?,
            _ => parse_shasta_aggregation_input_hash(journal)?,
        };
        if let ("proposal", Some(carry)) = (context.proof_type, context.proposal_carry_data) {
            ensure_shasta_proposal_input_matches_carry(input_hash, carry, "boundless")?;
        }
        if input_hash != context.expected_input_hash {
            return Err(BoundlessAttemptError::Fatal(RaikoError::Guest(
                "Boundless fulfillment journal does not match local dry-run journal".to_string(),
            )));
        }
        let stage_metadata = self.boundless_stage_metadata(submission, context);
        let extra_data = match (context.proof_type, context.proposal_carry_data) {
            ("proposal", Some(carry)) => {
                with_shasta_extra_data(carry, "risc0", Some(stage_metadata))?
            }
            _ => Some(stage_metadata),
        };
        let proof = match context.proof_type {
            "proposal" => encode_risc0_proposal_seal_payload(
                &seal,
                B256::from_slice(context.image_id.as_bytes()),
            ),
            _ => encode_risc0_aggregation_seal_payload(
                &seal,
                B256::from_slice(
                    context
                        .block_image_id
                        .ok_or_else(|| {
                            RaikoError::Guest(
                                "missing block image id for aggregation proof".to_string(),
                            )
                        })?
                        .as_bytes(),
                ),
                B256::from_slice(context.image_id.as_bytes()),
            ),
        };
        Ok(Proof {
            proof: Some(proof),
            input: Some(input_hash),
            quote: receipt_json,
            uuid: Some(alloy_primitives::hex::encode_prefixed(
                context.image_id.as_bytes(),
            )),
            kzg_proof: None,
            extra_data,
        })
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
        let client = self.client().await?;
        // The image id is deterministic in `elf`, so hash the multi-MB ELF once here and reuse it
        // for every per-attempt program refresh below instead of re-hashing each rebid rung.
        let image_id = compute_boundless_image_id(elf.to_vec(), elf_type.stage_name()).await?;
        // Seed the program cache (and derive the stable image ref) up front; the per-attempt
        // refresh inside the loop below shadows this and is a cache hit unless the presigned URL
        // nears expiry.
        let seed_program = self
            .ensure_uploaded(client, elf_type, elf, image_id)
            .await?;
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
        let image_ref = alloy_primitives::hex::encode_prefixed(seed_program.image_id.as_bytes());
        let deployment = format!("{:?}", self.config.get_deployment_type()).to_lowercase();
        let fulfillment_context = FulfillmentContext {
            proof_type: elf_type.proof_type_str(),
            image_id,
            block_image_id,
            expected_input_hash,
            quoted_mcycles_count,
            evaluated_mcycles_count,
            proposal_carry_data: proposal_carry_data.as_ref(),
        };

        let mut resume_submission = if let Some(observer) = observer.as_ref() {
            observer
                .load_pending_proof_checkpoint(crate::NetworkProverBackend::Boundless)
                .await
                .map_err(|error| RaikoError::Guest(error.to_string()))?
                .map(|checkpoint| {
                    checkpoint.decode_payload_for::<BoundlessSubmissionResume>(
                        crate::NetworkProverBackend::Boundless,
                    )
                })
                .transpose()
                .map_err(|error| RaikoError::Guest(error.to_string()))?
                .map(|resume| {
                    validate_resume_context(
                        &resume,
                        &image_ref,
                        &deployment,
                        self.config.offchain,
                    )?;
                    Submission::try_from(resume)
                })
                .transpose()?
        } else {
            None
        };
        let mut attempt = 1_u64;
        let mut last_retry_reason: Option<String> = None;
        // Market request id shared by rebid rungs of this proof task. The market keys locks and
        // paid fulfillments on the id, so reusing it across rebids makes paying for more than one
        // rung impossible by construction. `None` mints a fresh id only for the first attempt or
        // after the provider has definitively terminalized the previous request.
        let mut reuse_request_id: Option<U256> = None;
        // Per-proof input-upload cache: the guest env is uploaded once and reused across rebids,
        // refreshed only when its presigned URL nears expiry.
        let mut input_cache: Option<UploadedInput> = None;

        loop {
            // Refresh the program URL per attempt (cheap cache hit unless a refresh is due) so late
            // rebids never carry an expired presigned program URL. The image id is unchanged.
            let program = self
                .ensure_uploaded(client, elf_type, elf, image_id)
                .await?;
            let submission = if let Some(mut submission) = resume_submission.take() {
                attempt = attempt.max(submission.attempt);
                submission.attempt = attempt;
                // Expired records are deliberately not short-circuited: the poll below gives
                // them one final market status read. An expired-but-fulfilled request still
                // reports Fulfilled (the SDK checks fulfillment before expiry), recovering a
                // proof that is already paid for; an expired-unfulfilled one classifies as
                // Expired and takes the normal retry arm, which escalates the attempt and
                // draws down the submission budget.
                let checkpoint_permit =
                    crate::acquire_submission_checkpoint_permit(observer.as_ref()).await?;
                publish_boundless_progress(
                    observer.as_ref(),
                    &checkpoint_permit,
                    &submission,
                    &image_ref,
                    &deployment,
                    self.config.offchain,
                    (quoted_mcycles_count, evaluated_mcycles_count),
                )
                .await?;
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
                // Once an id has been assigned, an RPC failure may have happened after the market
                // accepted the request. Keep retrying that id on the next engine attempt instead of
                // opening a second payable request under a fresh id.
                Box::pin(self.submit_fresh_request(FreshSubmissionContext {
                    client,
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
                .await?
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
                .poll_until_fulfilled(client, &submission, &fulfillment_context, no_lock_timeout)
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
                Err(BoundlessAttemptError::TerminalCheckpoint { identity, error }) => {
                    terminalize_boundless_checkpoint(observer.as_ref(), &identity).await?;
                    tracing::info!(
                        provider_request_id = %identity.provider_request_id,
                        attempt = identity.attempt.get(),
                        "Cleared exhausted Boundless submission checkpoint"
                    );
                    return Err(error);
                }
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
        config: &ProverConfig,
        backend: &B,
    ) -> RaikoResult<Proof> {
        self.prove_encoded_with_observer(input, config, backend, None)
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
        config: &ProverConfig,
        backend: &B,
    ) -> RaikoResult<Proof> {
        self.aggregate_with_observer(input, config, backend, None)
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
    const fn quoted_mcycles_count(&self, elf_type: ElfType, evaluated_mcycles_count: u32) -> u32 {
        let quote = match elf_type {
            ElfType::Batch => &self.config.batch_quote,
            ElfType::Aggregation => &self.config.aggregation_quote,
        };
        match quote {
            QuoteSizing::RaikoAgent => match elf_type {
                ElfType::Batch => quote_batch_mcycles(evaluated_mcycles_count),
                ElfType::Aggregation => quote_aggregation_mcycles(evaluated_mcycles_count),
            },
            QuoteSizing::Evaluated => evaluated_mcycles_count,
            QuoteSizing::Fixed { mcycles } => *mcycles,
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

/// Top-up needed so the on-chain balance covers all submitted-but-unlocked max-price claims.
/// Over-deposit under RPC lag or conservative local retention is safe because deposits accrue to
/// the market account.
const fn deposit_topup(on_chain_balance: U256, reserved_total: U256) -> U256 {
    reserved_total.saturating_sub(on_chain_balance)
}

/// Escalated and capped market-mode offer prices derived from the SDK's autopriced offer.
#[derive(Debug, PartialEq, Eq)]
struct MarketOfferPrices {
    max_price: U256,
    min_price: U256,
    clamped_to_cap: bool,
}

/// Escalate the autopriced max price by the rebid step (bps), then clamp it to the configured
/// per-mcycle cap.
///
/// Manual pricing escalates its configured max price in [`BoundlessProver::build_request`]; this is
/// the market-mode counterpart, applied after the SDK autoprices the offer — without it, market-mode
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
    let (max_price, clamped_to_cap) = escalate_then_clamp(
        autopriced_max,
        attempt,
        step_bps,
        max_attempts,
        max_price_cap.map(|cap| cap.value),
    )?;
    Ok(MarketOfferPrices {
        max_price,
        min_price: autopriced_min.min(max_price),
        clamped_to_cap,
    })
}

/// Shared escalate-then-clamp core for both pricing modes: compound `base` by the bps ladder for
/// `attempt`, then clamp the result to `cap` when one is configured. Returns the final price and
/// whether the cap was applied.
fn escalate_then_clamp(
    base: U256,
    attempt: u64,
    step_bps: u32,
    max_attempts: u32,
    cap: Option<U256>,
) -> RaikoResult<(U256, bool)> {
    let escalated = escalated_price(base, attempt, step_bps, max_attempts)?;
    Ok(match cap {
        Some(cap) if escalated > cap => (cap, true),
        _ => (escalated, false),
    })
}

/// Escalated and optionally ceiling-clamped manual-mode offer max price.
#[derive(Debug, PartialEq, Eq)]
struct ManualOfferMaxPrice {
    max_price: U256,
    clamped_to_ceiling: bool,
}

/// Escalate the configured manual max price by the compounding bps rebid step, then clamp it to
/// the absolute per-request ceiling when one is configured.
///
/// Manual-mode counterpart of [`escalate_and_cap_market_prices`], sharing its [`escalated_price`]
/// bps ladder: without a ceiling, rebids escalate the configured max price with no config-level
/// bound, so the worst case is derived math rather than an operator-stated budget. The ceiling
/// makes the worst case a config value. As in market mode, hitting the ceiling clamps instead of
/// failing: the offer bids at the operator's stated maximum and the no-lock machinery decides how
/// long to wait there.
fn escalate_and_clamp_manual_max_price(
    configured_max: U256,
    attempt: u64,
    step_bps: u32,
    max_attempts: u32,
    ceiling: Option<U256>,
) -> RaikoResult<ManualOfferMaxPrice> {
    let (max_price, clamped_to_ceiling) =
        escalate_then_clamp(configured_max, attempt, step_bps, max_attempts, ceiling)?;
    Ok(ManualOfferMaxPrice {
        max_price,
        clamped_to_ceiling,
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
            "Boundless market offer max price exceeds the configured per-mcycle price cap; bidding at the cap"
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

/// Market-mode price cap from whichever cap spelling is configured. `validate_offer_spec`
/// rejects setting both, so at most one of the two keys carries the cap here.
fn market_offer_price_cap(
    offer_spec: &BoundlessOfferParams,
    mcycles_count: u32,
) -> RaikoResult<Option<Amount>> {
    let (cap_field, cap_value) = match (
        offer_spec.absolute_max_price_per_mcycle.as_deref(),
        offer_spec.max_price_per_mcycle.as_deref(),
    ) {
        (Some(value), _) => ("absolute_max_price_per_mcycle", Some(value)),
        (None, value) => ("max_price_per_mcycle", value),
    };
    cap_value
        .map(|value| parse_request_amount(value, cap_field, Asset::ETH, mcycles_count))
        .transpose()
}

fn validate_offer_params(
    offer_spec: &BoundlessOfferParams,
    mcycles_count: u32,
) -> RaikoResult<ValidatedOfferParams> {
    validate_offer_spec(offer_spec).map_err(RaikoError::InvalidRequestConfig)?;
    let (max_price, min_price, max_price_cap) = match offer_spec.pricing_mode {
        BoundlessPricingMode::Manual => {
            let max_price_value = offer_spec.max_price_per_mcycle.as_deref().ok_or_else(|| {
                RaikoError::InvalidRequestConfig(
                    "max_price_per_mcycle is required when pricing_mode=manual".to_string(),
                )
            })?;
            // The base (un-escalated) max price; `build_request` escalates it per rebid rung and
            // clamps the result to the absolute ceiling threaded below as `max_price_cap`.
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
            // Absolute per-request ceiling on the bps-escalated bid (config-stated worst case),
            // threaded as the manual-mode `max_price_cap`; `build_request` clamps the escalated
            // price to it. `None` leaves escalation unbounded.
            let ceiling = offer_spec
                .absolute_max_price_per_mcycle
                .as_deref()
                .map(|ceiling_value| {
                    parse_request_amount(
                        ceiling_value,
                        "absolute_max_price_per_mcycle",
                        Asset::ETH,
                        mcycles_count,
                    )
                })
                .transpose()?;
            (Some(max_price), Some(min_price), ceiling)
        }
        BoundlessPricingMode::Market => (
            None,
            None,
            market_offer_price_cap(offer_spec, mcycles_count)?,
        ),
    };
    let (lock_timeout, timeout) = match &offer_spec.timeouts {
        TimeoutPolicy::PerMcycle {
            lock_timeout_ms_per_mcycle,
            timeout_ms_per_mcycle,
            dynamic_pricing_timeout_modifier,
        } => {
            // u64 intermediates: `*_ms_per_mcycle * mcycles_count` can exceed u32 at very large
            // mcycle counts; clamp back into u32 (a ms timeout that large is already absurd).
            let derived_lock = u32::try_from(
                u64::from(*lock_timeout_ms_per_mcycle) * u64::from(mcycles_count) / 1000,
            )
            .unwrap_or(u32::MAX);
            let derived_timeout =
                u32::try_from(u64::from(*timeout_ms_per_mcycle) * u64::from(mcycles_count) / 1000)
                    .unwrap_or(u32::MAX);
            // Scale only in market mode with a configured modifier; every other case uses the
            // per-mcycle derived timeouts unchanged.
            if offer_spec.pricing_mode == BoundlessPricingMode::Market
                && let Some(modifier) = dynamic_pricing_timeout_modifier
            {
                (
                    scale_timeout(derived_lock, *modifier, "lock_timeout")?,
                    scale_timeout(derived_timeout, *modifier, "timeout")?,
                )
            } else {
                (derived_lock, derived_timeout)
            }
        }
        TimeoutPolicy::Fixed {
            lock_timeout_secs,
            timeout_secs,
        } => (*lock_timeout_secs, *timeout_secs),
    };
    if timeout <= lock_timeout {
        return Err(RaikoError::InvalidRequestConfig(format!(
            "timeout {timeout}s must be greater than lock timeout {lock_timeout}s for {mcycles_count} mcycles"
        )));
    }
    let ramp_up_period_secs = offer_spec.ramp_up_period_sec;
    if ramp_up_period_secs > lock_timeout {
        return Err(RaikoError::InvalidRequestConfig(format!(
            "ramp_up_period_sec={} exceeds lock timeout for {} mcycles",
            offer_spec.ramp_up_period_sec, mcycles_count
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
    #[cfg(feature = "boundless-s3")]
    use super::should_initialize_s3_downloader;
    use super::{
        BOUNDLESS_SUBMIT_RECEIPT_CONFIRMATIONS, BoundlessAttemptError, BoundlessConfig,
        BoundlessFundingState, BoundlessPollSubmission, BoundlessPricingMode, BoundlessProver,
        BoundlessStatusSource, BoundlessStorageDownloader, BoundlessSubmissionMetadata,
        BoundlessSubmissionState, BoundlessTerminalOutcome, BoundlessTimeoutAction,
        DeploymentConfig, DeploymentType, ElfType, JsonRpcError, JsonRpcResponse,
        MIN_REBID_TIMEOUT_MS, NoLockTimeout, Submission, TimeoutPolicy,
        boundless_poll_error_statuses, classify_boundless_nonce, classify_boundless_status,
        dispatch_offchain_after_checkpoint, escalate_and_cap_market_prices,
        exceeds_submission_budget, no_lock_deadline, no_lock_timeout_for_attempt, now_secs,
        parse_bool_result, parse_env_bool, parse_env_url, publish_boundless_progress,
        quote_batch_mcycles, reserve_boundless_funding_before_dispatch, retry_external_bounded,
        run_after_submission_permit, should_defer_boundless_poll_timeout,
        should_rebid_unlocked_request, storage_uploader_config_from_env,
        terminalize_boundless_checkpoint, user_cycles_to_mcycles, validate_offer_params,
        validate_resume_context,
    };
    use crate::boundless_config::default_batch_offer_params;
    use alloy_primitives::{Address, B256, Bloom, Bytes, U256, address, utils::parse_ether};
    #[cfg(feature = "boundless-s3")]
    use boundless_market::storage::StorageUploaderConfig;
    use boundless_market::{
        ProofRequest, RequestId,
        alloy::{
            consensus::{Receipt, ReceiptEnvelope, ReceiptWithBloom},
            network::Ethereum,
            providers::{PendingTransactionBuilder, Provider, ProviderBuilder},
            rpc::types::{Block, Log, Transaction, TransactionReceipt},
            transports::mock::Asserter,
        },
        contracts::{Offer, Predicate, RequestInput, Requirements},
        price_oracle::{Amount, Asset},
        storage::StorageUploaderType,
    };
    use raiko2_primitives::{Proof, ProofType, RaikoError};
    use raiko2_remote_poller::{
        RemoteStatus, RemoteStatusReason, RemoteSubmission, RemoteSubmissionId,
    };
    use raiko2_runtime::RuntimeManager;
    use std::{
        collections::HashMap,
        env,
        sync::{
            Arc, Mutex, MutexGuard,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::{Duration, Instant, SystemTime},
    };
    use url::Url;

    static ENV_LOCK: Mutex<()> = Mutex::new(());
    const TEST_REBID_TIMEOUT_MS: u64 = 300_000;
    const TEST_REBID_PRICE_STEP_BPS: u32 = 5000;
    const TEST_REBID_MAX_ATTEMPTS: u32 = 4;

    struct FlakyProgressObserver {
        remaining_failures: AtomicUsize,
        calls: AtomicUsize,
    }

    struct PermanentProgressObserver {
        calls: AtomicUsize,
    }

    struct RecordingProgressObserver {
        persisted: Arc<AtomicBool>,
    }

    struct RuntimeLifecycleProgressObserver {
        runtime: Arc<RuntimeManager>,
    }

    #[derive(Default)]
    struct ClearingProgressObserver {
        cleared: Mutex<Vec<crate::PendingProofCheckpointIdentity>>,
    }

    struct PermitDropSignal(Arc<AtomicBool>);

    impl Drop for PermitDropSignal {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[async_trait::async_trait]
    impl crate::ProverProgressObserver for FlakyProgressObserver {
        async fn on_progress(
            &self,
            _progress: &crate::ProverProgress,
            _permit: &crate::SubmissionCheckpointPermit,
        ) -> Result<(), crate::ProgressPersistenceError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self
                .remaining_failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Err(crate::ProgressPersistenceError::Retryable(
                    "checkpoint unavailable".to_string(),
                ));
            }
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl crate::ProverProgressObserver for PermanentProgressObserver {
        async fn on_progress(
            &self,
            _progress: &crate::ProverProgress,
            _permit: &crate::SubmissionCheckpointPermit,
        ) -> Result<(), crate::ProgressPersistenceError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(crate::ProgressPersistenceError::Permanent(
                "runtime is draining".to_string(),
            ))
        }
    }

    #[async_trait::async_trait]
    impl crate::ProverProgressObserver for RecordingProgressObserver {
        async fn on_progress(
            &self,
            _progress: &crate::ProverProgress,
            _permit: &crate::SubmissionCheckpointPermit,
        ) -> Result<(), crate::ProgressPersistenceError> {
            self.persisted.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl crate::ProverProgressObserver for RuntimeLifecycleProgressObserver {
        async fn acquire_submission_checkpoint_permit(
            &self,
        ) -> Result<crate::SubmissionCheckpointPermit, crate::ProgressPersistenceError> {
            self.runtime
                .acquire_submission_checkpoint_permit()
                .map(crate::SubmissionCheckpointPermit::tracked)
                .map_err(|error| crate::ProgressPersistenceError::Permanent(error.to_string()))
        }

        async fn on_progress(
            &self,
            _progress: &crate::ProverProgress,
            _permit: &crate::SubmissionCheckpointPermit,
        ) -> Result<(), crate::ProgressPersistenceError> {
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl crate::ProverProgressObserver for ClearingProgressObserver {
        async fn on_progress(
            &self,
            _progress: &crate::ProverProgress,
            _permit: &crate::SubmissionCheckpointPermit,
        ) -> Result<(), crate::ProgressPersistenceError> {
            Ok(())
        }

        async fn clear_pending_proof_checkpoint(
            &self,
            identity: &crate::PendingProofCheckpointIdentity,
            _permit: &crate::SubmissionCheckpointPermit,
        ) -> Result<(), crate::ProgressPersistenceError> {
            self.cleared
                .lock()
                .expect("cleared checkpoint lock")
                .push(identity.clone());
            Ok(())
        }
    }

    fn test_submission() -> Submission {
        Submission {
            market_request_id: U256::from(1),
            provider_request_id: "0x1".to_string(),
            remote_tx_hash: None,
            expires_at: 100,
            lock_expires_at: 90,
            submitted_at: 1,
            max_price_multiplier: 1,
            max_price_wei: U256::from(1),
            attempt: 1,
        }
    }

    #[test]
    fn boundless_terminal_checkpoint_reset_carries_exact_submission_identity() {
        let mut submission = test_submission();
        submission.provider_request_id = "0xterminal".to_string();
        submission.attempt = 5;
        let result = BoundlessProver::boundless_failed_terminal(
            &submission,
            NoLockTimeout {
                delay: Duration::from_secs(30),
                action: BoundlessTimeoutAction::Abort,
            },
            RemoteStatusReason::new("not locked"),
            Some(BoundlessTerminalOutcome::NoLockAbortTimeout),
        );

        match result {
            Err(BoundlessAttemptError::TerminalCheckpoint { identity, error }) => {
                assert_eq!(identity.backend, crate::NetworkProverBackend::Boundless);
                assert_eq!(identity.provider_request_id, "0xterminal");
                assert_eq!(identity.attempt.get(), 5);
                assert!(
                    error
                        .to_string()
                        .contains("exhausted boundless no-lock rebids")
                );
            }
            _ => panic!("final no-lock abort must request checkpoint terminalization"),
        }
    }

    #[tokio::test]
    async fn boundless_terminal_checkpoint_reset_clears_before_returning_failure() {
        let observer = Arc::new(ClearingProgressObserver::default());
        let observer_dyn: Arc<dyn crate::ProverProgressObserver> = observer.clone();
        let identity = crate::PendingProofCheckpointIdentity {
            backend: crate::NetworkProverBackend::Boundless,
            provider_request_id: "0xterminal".to_string(),
            attempt: std::num::NonZeroU32::new(5).expect("non-zero attempt"),
        };

        terminalize_boundless_checkpoint(Some(&observer_dyn), &identity)
            .await
            .expect("terminal checkpoint clears");

        assert_eq!(
            *observer.cleared.lock().expect("cleared checkpoint lock"),
            vec![identity]
        );
    }

    fn test_proof_request() -> ProofRequest {
        ProofRequest::new(
            RequestId::new(address!("1000000000000000000000000000000000000001"), 1),
            Requirements::new(Predicate::prefix_match(
                risc0_zkvm::Digest::default(),
                Bytes::new(),
            )),
            "https://example.invalid/program",
            RequestInput::inline(Bytes::new()),
            Offer {
                minPrice: U256::from(1),
                maxPrice: U256::from(2),
                rampUpStart: 1,
                timeout: 10,
                rampUpPeriod: 1,
                lockCollateral: U256::ZERO,
                lockTimeout: 5,
            },
        )
    }

    #[test]
    fn boundless_nonce_allocation_uses_chain_and_local_high_water() {
        let mut state = BoundlessFundingState::default();

        assert_eq!(state.allocate_nonce(5, 7).expect("first nonce"), 7);
        assert_eq!(state.allocate_nonce(5, 7).expect("second nonce"), 8);
        assert_eq!(state.allocate_nonce(11, 9).expect("latest nonce"), 11);
    }

    #[test]
    fn boundless_nonce_uncertainty_blocks_fresh_allocation_until_consumed() {
        let mut state = BoundlessFundingState::default();
        let uncertain = super::BoundlessUncertainSubmission {
            submission: test_submission(),
            request: test_proof_request(),
            signature: Bytes::from_static(b"fixture_signature"),
            request_digest: B256::repeat_byte(0x11),
            value: U256::from(7),
            nonce: 4,
        };

        state
            .record_uncertain(uncertain.clone())
            .expect("record uncertain submission");
        assert!(state.allocate_nonce(4, 4).is_err());
        assert_eq!(
            state.uncertain_submission().expect("uncertain submission"),
            &uncertain
        );

        assert!(!state.reconcile_consumed_nonce(4));
        assert!(state.reconcile_consumed_nonce(5));
        assert!(state.uncertain_submission().is_none());
        assert_eq!(state.allocate_nonce(5, 5).expect("next nonce"), 5);
    }

    #[test]
    fn boundless_nonce_clear_requires_matching_identity() {
        let mut state = BoundlessFundingState::default();
        let uncertain = super::BoundlessUncertainSubmission {
            submission: test_submission(),
            request: test_proof_request(),
            signature: Bytes::from_static(b"fixture_signature"),
            request_digest: B256::repeat_byte(0x11),
            value: U256::from(7),
            nonce: 4,
        };
        state
            .record_uncertain(uncertain)
            .expect("record uncertain submission");

        assert!(!state.clear_uncertain(4, B256::repeat_byte(0x22)));
        assert!(state.uncertain_submission().is_some());
        assert!(state.clear_uncertain(4, B256::repeat_byte(0x11)));
        assert!(state.uncertain_submission().is_none());
    }

    #[test]
    fn boundless_nonce_recovery_distinguishes_mined_pending_and_available() {
        assert_eq!(
            classify_boundless_nonce(7, 8, 8),
            super::BoundlessNonceStatus::Mined
        );
        assert_eq!(
            classify_boundless_nonce(7, 7, 8),
            super::BoundlessNonceStatus::Pending
        );
        assert_eq!(
            classify_boundless_nonce(7, 7, 7),
            super::BoundlessNonceStatus::Available
        );
    }

    #[test]
    fn boundless_receipt_wait_requires_three_confirmations() {
        assert_eq!(BOUNDLESS_SUBMIT_RECEIPT_CONFIRMATIONS, 3);
    }

    #[tokio::test]
    async fn boundless_receipt_budget_allows_three_sepolia_confirmations() {
        let tx_hash = B256::repeat_byte(0x11);
        let block_hash = B256::repeat_byte(0x22);
        let receipt = TransactionReceipt {
            inner: ReceiptEnvelope::Legacy(ReceiptWithBloom {
                receipt: Receipt {
                    status: true.into(),
                    cumulative_gas_used: 21_000,
                    logs: Vec::<Log>::new(),
                },
                logs_bloom: Bloom::default(),
            }),
            transaction_hash: tx_hash,
            transaction_index: Some(0),
            block_hash: Some(block_hash),
            block_number: Some(100),
            gas_used: 21_000,
            effective_gas_price: 1,
            blob_gas_used: None,
            blob_gas_price: None,
            from: Address::default(),
            to: Some(Address::default()),
            contract_address: None,
        };
        let asserter = Asserter::new();
        asserter.push_success(&Some(receipt.clone()));
        asserter.push_success(&100_u64);
        for number in 99..=100 {
            let mut block = Block::<Transaction>::default();
            block.header.inner.number = number;
            asserter.push_success(&Some(block));
        }
        asserter.push_success(&101_u64);
        let mut block = Block::<Transaction>::default();
        block.header.inner.number = 101;
        asserter.push_success(&Some(block));
        asserter.push_success(&102_u64);
        let mut block = Block::<Transaction>::default();
        block.header.inner.number = 102;
        asserter.push_success(&Some(block));
        asserter.push_success(&Some(receipt));

        let provider = ProviderBuilder::<_, _, Ethereum>::default().connect_mocked_client(asserter);
        provider.client().set_poll_interval(Duration::from_secs(6));
        let pending = PendingTransactionBuilder::new(provider.root().clone(), tx_hash);
        let prover = BoundlessProver::new(BoundlessConfig::default());

        let outcome = prover
            .observe_onchain_receipt(
                pending,
                &test_submission(),
                U256::from(1),
                B256::repeat_byte(0x33),
            )
            .await;

        assert_eq!(outcome, super::BoundlessReceiptOutcome::ConfirmedSuccess);
    }

    #[tokio::test]
    async fn boundless_checkpoint_retries_the_accepted_submission_in_place() {
        let observer = Arc::new(FlakyProgressObserver {
            remaining_failures: AtomicUsize::new(2),
            calls: AtomicUsize::new(0),
        });
        let progress_observer: Arc<dyn crate::ProverProgressObserver> = observer.clone();
        let permit = crate::SubmissionCheckpointPermit::untracked();
        let submission = Submission {
            market_request_id: U256::from(1),
            provider_request_id: "0xaccepted".to_string(),
            remote_tx_hash: None,
            expires_at: 100,
            lock_expires_at: 90,
            submitted_at: 1,
            max_price_multiplier: 1,
            max_price_wei: U256::from(1),
            attempt: 1,
        };

        publish_boundless_progress(
            Some(&progress_observer),
            &permit,
            &submission,
            "image",
            "deployment",
            true,
            (1, 1),
        )
        .await
        .expect("checkpoint eventually persists");

        assert_eq!(observer.calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn boundless_retry_total_timeout_stops_pending_operation() {
        let started = Instant::now();
        let error = retry_external_bounded(
            "pending fixture operation",
            Duration::from_millis(10),
            || async { std::future::pending::<Result<(), RaikoError>>().await },
        )
        .await
        .expect_err("outer timeout must stop a pending attempt");

        assert!(error.to_string().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn boundless_checkpoint_stops_on_permanent_runtime_fence() {
        let observer = Arc::new(PermanentProgressObserver {
            calls: AtomicUsize::new(0),
        });
        let progress_observer: Arc<dyn crate::ProverProgressObserver> = observer.clone();
        let permit = crate::SubmissionCheckpointPermit::untracked();
        let submission = Submission {
            market_request_id: U256::from(1),
            provider_request_id: "0xaccepted".to_string(),
            remote_tx_hash: None,
            expires_at: 100,
            lock_expires_at: 90,
            submitted_at: 1,
            max_price_multiplier: 1,
            max_price_wei: U256::from(1),
            attempt: 1,
        };

        let error = publish_boundless_progress(
            Some(&progress_observer),
            &permit,
            &submission,
            "image",
            "deployment",
            true,
            (1, 1),
        )
        .await
        .expect_err("permanent runtime fence must stop checkpoint persistence");

        assert!(error.to_string().contains("runtime is draining"));
        assert_eq!(observer.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn offchain_dispatch_starts_only_after_checkpoint_is_durable() {
        let persisted = Arc::new(AtomicBool::new(false));
        let observer: Arc<dyn crate::ProverProgressObserver> =
            Arc::new(RecordingProgressObserver {
                persisted: Arc::clone(&persisted),
            });
        let dispatched = Arc::new(AtomicUsize::new(0));
        let dispatched_for_call = Arc::clone(&dispatched);
        let persisted_for_call = Arc::clone(&persisted);
        let permit_released = Arc::new(AtomicBool::new(false));
        let permit = crate::SubmissionCheckpointPermit::tracked(PermitDropSignal(Arc::clone(
            &permit_released,
        )));
        let permit_released_for_call = Arc::clone(&permit_released);

        let submission = dispatch_offchain_after_checkpoint(
            Some(&observer),
            permit,
            test_submission(),
            "image",
            "deployment",
            (1, 1),
            move || async move {
                assert!(persisted_for_call.load(Ordering::SeqCst));
                assert!(permit_released_for_call.load(Ordering::SeqCst));
                dispatched_for_call.fetch_add(1, Ordering::SeqCst);
                Ok(U256::from(1))
            },
        )
        .await
        .expect("checkpointed offchain request should dispatch");

        assert_eq!(submission.market_request_id, U256::from(1));
        assert_eq!(dispatched.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn permanent_checkpoint_failure_prevents_offchain_dispatch() {
        let observer: Arc<dyn crate::ProverProgressObserver> =
            Arc::new(PermanentProgressObserver {
                calls: AtomicUsize::new(0),
            });
        let dispatched = Arc::new(AtomicUsize::new(0));
        let dispatched_for_call = Arc::clone(&dispatched);
        let permit = crate::SubmissionCheckpointPermit::untracked();

        let error = dispatch_offchain_after_checkpoint(
            Some(&observer),
            permit,
            test_submission(),
            "image",
            "deployment",
            (1, 1),
            move || async move {
                dispatched_for_call.fetch_add(1, Ordering::SeqCst);
                Ok(U256::from(1))
            },
        )
        .await
        .expect_err("a fenced checkpoint must prevent provider dispatch");

        assert!(error.to_string().contains("runtime is draining"));
        assert_eq!(dispatched.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn uncertain_offchain_dispatch_polls_the_checkpointed_request_once() {
        let dispatched = Arc::new(AtomicUsize::new(0));
        let dispatched_for_call = Arc::clone(&dispatched);
        let permit = crate::SubmissionCheckpointPermit::untracked();

        let submission = dispatch_offchain_after_checkpoint(
            None,
            permit,
            test_submission(),
            "image",
            "deployment",
            (1, 1),
            move || async move {
                dispatched_for_call.fetch_add(1, Ordering::SeqCst);
                Err(RaikoError::Guest("uncertain transport failure".to_string()))
            },
        )
        .await
        .expect("an uncertain response must retain the pre-dispatch request id");

        assert_eq!(submission.market_request_id, U256::from(1));
        assert_eq!(dispatched.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn offchain_dispatch_rejects_a_provider_id_mismatch() {
        let permit = crate::SubmissionCheckpointPermit::untracked();

        let error = dispatch_offchain_after_checkpoint(
            None,
            permit,
            test_submission(),
            "image",
            "deployment",
            (1, 1),
            || async { Ok(U256::from(2)) },
        )
        .await
        .expect_err("provider must confirm the checkpointed request id");

        assert!(
            error
                .to_string()
                .contains("expected checkpointed request id")
        );
    }

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
        "GOOGLE_APPLICATION_CREDENTIALS",
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
    fn aggregation_quote_matches_raiko_agent_strategy() {
        let prover = BoundlessProver::new(BoundlessConfig::default());
        assert_eq!(prover.quoted_mcycles_count(ElfType::Aggregation, 0), 200);
        assert_eq!(prover.quoted_mcycles_count(ElfType::Aggregation, 123), 200);
        assert_eq!(prover.quoted_mcycles_count(ElfType::Aggregation, 200), 200);
        assert_eq!(prover.quoted_mcycles_count(ElfType::Aggregation, 201), 300);
    }

    #[test]
    fn quoted_mcycles_count_dispatches_on_quote_sizing() {
        use super::QuoteSizing;
        // RaikoAgent rounds (batch floor 2000, step 1000).
        let prover = BoundlessProver::new(BoundlessConfig::default());
        assert_eq!(prover.quoted_mcycles_count(ElfType::Batch, 1_491), 2_000);

        // Evaluated passes through.
        let cfg = BoundlessConfig {
            batch_quote: QuoteSizing::Evaluated,
            ..Default::default()
        };
        assert_eq!(
            BoundlessProver::new(cfg).quoted_mcycles_count(ElfType::Batch, 1_188),
            1_188
        );

        // Fixed pins the value.
        let cfg = BoundlessConfig {
            batch_quote: QuoteSizing::Fixed { mcycles: 1_500 },
            ..Default::default()
        };
        assert_eq!(
            BoundlessProver::new(cfg).quoted_mcycles_count(ElfType::Batch, 1_188),
            1_500
        );

        // Aggregation Fixed pins independently.
        let cfg = BoundlessConfig {
            aggregation_quote: QuoteSizing::Fixed { mcycles: 320 },
            ..Default::default()
        };
        assert_eq!(
            BoundlessProver::new(cfg).quoted_mcycles_count(ElfType::Aggregation, 1_188),
            320
        );
    }

    #[test]
    fn boundless_poll_errors_return_terminal_status_after_poll_timeout() {
        let now = now_secs();
        let submission = RemoteSubmission {
            id: RemoteSubmissionId::new(),
            proof_type: ProofType::Risc0,
            provider_request_id: "0x1".to_string(),
            timeout_at: None,
        };
        let registry = Arc::new(Mutex::new(HashMap::from([(
            submission.id,
            BoundlessSubmissionState {
                metadata: BoundlessSubmissionMetadata {
                    expires_at: now.saturating_add(300),
                    lock_expires_at: now.saturating_add(300),
                    submitted_at: now.saturating_sub(30),
                    no_lock_deadline: now.saturating_add(60),
                    no_lock_timeout_action: BoundlessTimeoutAction::Rebid,
                    poll_timeout_at: Instant::now().checked_sub(Duration::from_secs(1)).unwrap(),
                },
                terminal_outcome: None,
            },
        )])));

        let statuses = boundless_poll_error_statuses(
            vec![submission],
            "missing rpc id".to_string(),
            &registry,
        )
        .expect("poll timeout status");

        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].status, RemoteStatus::Failed);
        assert!(matches!(
            registry
                .lock()
                .expect("registry")
                .values()
                .next()
                .and_then(|state| state.terminal_outcome),
            Some(BoundlessTerminalOutcome::PollTimeout {
                rotate_request_id: false
            })
        ));
    }

    #[test]
    fn boundless_poll_errors_terminalize_invalid_id_without_blocking_valid_submission() {
        let now = now_secs();
        let invalid = RemoteSubmission {
            id: RemoteSubmissionId::new(),
            proof_type: ProofType::Risc0,
            provider_request_id: "not-a-request-id".to_string(),
            timeout_at: None,
        };
        let valid = RemoteSubmission {
            id: RemoteSubmissionId::new(),
            proof_type: ProofType::Risc0,
            provider_request_id: "0x1".to_string(),
            timeout_at: None,
        };
        let registry = Arc::new(Mutex::new(HashMap::from([(
            valid.id,
            BoundlessSubmissionState {
                metadata: BoundlessSubmissionMetadata {
                    expires_at: now.saturating_add(300),
                    lock_expires_at: now.saturating_add(300),
                    submitted_at: now.saturating_sub(30),
                    no_lock_deadline: now.saturating_add(60),
                    no_lock_timeout_action: BoundlessTimeoutAction::Rebid,
                    poll_timeout_at: Instant::now() + Duration::from_secs(60),
                },
                terminal_outcome: None,
            },
        )])));

        let statuses = boundless_poll_error_statuses(
            vec![invalid.clone(), valid.clone()],
            "rpc unavailable".to_string(),
            &registry,
        )
        .expect("invalid id status should be returned with valid pending status");

        assert_eq!(statuses.len(), 2);
        assert_eq!(
            statuses
                .iter()
                .find(|status| status.submission_id == invalid.id)
                .expect("invalid status")
                .status,
            RemoteStatus::Unrecoverable
        );
        assert_eq!(
            statuses
                .iter()
                .find(|status| status.submission_id == valid.id)
                .expect("valid status")
                .status,
            RemoteStatus::Pending
        );
    }

    fn rpc_word(value: u64) -> serde_json::Value {
        serde_json::json!(format!("0x{value:064x}"))
    }

    fn rpc_result(id: u64, result: serde_json::Value) -> JsonRpcResponse {
        JsonRpcResponse {
            id,
            result: Some(result),
            error: None,
        }
    }

    fn rpc_error(id: u64, message: &str) -> JsonRpcResponse {
        JsonRpcResponse {
            id,
            result: None,
            error: Some(JsonRpcError {
                code: 3,
                message: message.to_string(),
            }),
        }
    }

    fn boundless_submission_state(
        now: u64,
        no_lock_timeout_action: BoundlessTimeoutAction,
        no_lock_deadline_delta: i64,
        poll_timeout_at: Instant,
    ) -> BoundlessSubmissionState {
        BoundlessSubmissionState {
            metadata: BoundlessSubmissionMetadata {
                expires_at: now.saturating_add(300),
                lock_expires_at: now.saturating_add(120),
                submitted_at: now.saturating_sub(30),
                no_lock_deadline: if no_lock_deadline_delta.is_negative() {
                    now.saturating_sub(no_lock_deadline_delta.unsigned_abs())
                } else {
                    now.saturating_add(no_lock_deadline_delta.unsigned_abs())
                },
                no_lock_timeout_action,
                poll_timeout_at,
            },
            terminal_outcome: None,
        }
    }

    #[test]
    fn boundless_deadline_revert_for_unlocked_request_does_not_poison_batch() {
        let now = now_secs();
        let unlocked = RemoteSubmission {
            id: RemoteSubmissionId::new(),
            proof_type: ProofType::Risc0,
            provider_request_id: "0x1".to_string(),
            timeout_at: None,
        };
        let fulfilled = RemoteSubmission {
            id: RemoteSubmissionId::new(),
            proof_type: ProofType::Risc0,
            provider_request_id: "0x2".to_string(),
            timeout_at: None,
        };
        let registry = Arc::new(Mutex::new(HashMap::from([
            (
                unlocked.id,
                boundless_submission_state(
                    now,
                    BoundlessTimeoutAction::Rebid,
                    60,
                    Instant::now() + Duration::from_secs(60),
                ),
            ),
            (
                fulfilled.id,
                boundless_submission_state(
                    now,
                    BoundlessTimeoutAction::Rebid,
                    60,
                    Instant::now() + Duration::from_secs(60),
                ),
            ),
        ])));
        let source = BoundlessStatusSource {
            rpc_url: "http://localhost".to_string(),
            market_address: "0x0000000000000000000000000000000000000000".to_string(),
            http: reqwest::Client::new(),
            registry,
        };
        let mut by_id = HashMap::from([
            (1, rpc_result(1, rpc_word(0))),
            (2, rpc_result(2, rpc_word(0))),
            (3, rpc_error(3, "RequestIsNotLocked")),
            (4, rpc_result(4, rpc_word(1))),
            (5, rpc_result(5, rpc_word(0))),
            (6, rpc_error(6, "RequestIsNotLocked")),
        ]);

        let unlocked_status = source
            .status_from_rpc_results(
                0,
                &BoundlessPollSubmission {
                    submission: unlocked.clone(),
                    request_id: U256::from(1),
                },
                now,
                &mut by_id,
            )
            .expect("unlocked status");
        let fulfilled_status = source
            .status_from_rpc_results(
                1,
                &BoundlessPollSubmission {
                    submission: fulfilled.clone(),
                    request_id: U256::from(2),
                },
                now,
                &mut by_id,
            )
            .expect("fulfilled status");

        assert_eq!(unlocked_status.status, RemoteStatus::Pending);
        assert_eq!(fulfilled_status.status, RemoteStatus::Fulfilled);
    }

    #[test]
    fn classify_boundless_status_covers_timeout_actions_and_rotate_boundary() {
        let now = now_secs();
        let submission_id = RemoteSubmissionId::new();
        let rebid_state = boundless_submission_state(
            now,
            BoundlessTimeoutAction::Rebid,
            -1,
            Instant::now() + Duration::from_secs(60),
        );
        let abort_state = boundless_submission_state(
            now,
            BoundlessTimeoutAction::Abort,
            -1,
            Instant::now() + Duration::from_secs(60),
        );

        let (status, outcome) = classify_boundless_status(
            submission_id,
            "0x1",
            &rebid_state.metadata,
            false,
            false,
            0,
            now,
        );
        assert_eq!(status.status, RemoteStatus::Failed);
        assert_eq!(outcome, Some(BoundlessTerminalOutcome::NoLockRebidTimeout));

        let (status, outcome) = classify_boundless_status(
            submission_id,
            "0x1",
            &abort_state.metadata,
            false,
            false,
            0,
            now,
        );
        assert_eq!(status.status, RemoteStatus::Failed);
        assert_eq!(outcome, Some(BoundlessTerminalOutcome::NoLockAbortTimeout));

        let mut locked_state = boundless_submission_state(
            now,
            BoundlessTimeoutAction::Rebid,
            60,
            Instant::now() + Duration::from_secs(60),
        );
        locked_state.metadata.expires_at = now.saturating_add(300);
        let (status, outcome) = classify_boundless_status(
            submission_id,
            "0x1",
            &locked_state.metadata,
            false,
            true,
            now.saturating_sub(1),
            now,
        );
        assert_eq!(status.status, RemoteStatus::Expired);
        assert_eq!(outcome, Some(BoundlessTerminalOutcome::LockExpired));

        let mut poll_timeout_state = boundless_submission_state(
            now,
            BoundlessTimeoutAction::Rebid,
            60,
            Instant::now().checked_sub(Duration::from_secs(1)).unwrap(),
        );
        poll_timeout_state.metadata.expires_at = now.saturating_sub(1);
        let (status, outcome) = classify_boundless_status(
            submission_id,
            "0x1",
            &poll_timeout_state.metadata,
            false,
            false,
            0,
            now.saturating_sub(1),
        );
        assert_eq!(status.status, RemoteStatus::Failed);
        assert_eq!(
            outcome,
            Some(BoundlessTerminalOutcome::PollTimeout {
                rotate_request_id: true
            })
        );
    }

    #[test]
    fn parse_bool_result_treats_any_nonzero_word_as_true() {
        let encoded =
            serde_json::json!("0x0000000000000000000000000000000000000000000000000000000000000002");

        assert!(parse_bool_result(&encoded).expect("bool result"));
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

    #[test]
    fn resumed_submission_carries_lock_deadline() {
        let resume = crate::BoundlessSubmissionResume {
            provider_request_id: "0x1".to_string(),
            remote_tx_hash: None,
            image_ref: "0ximage".to_string(),
            deployment: "base".to_string(),
            offchain: false,
            expires_at: 2_000,
            lock_expires_at: 1_500,
            submitted_at: 1_000,
            max_price_multiplier: 1,
            max_price_wei: Some("1".to_string()),
            rebid_attempt: 1,
        };
        let submission = super::Submission::try_from(resume.clone()).expect("valid resume record");
        assert_eq!(submission.lock_expires_at, 1_500);

        let mut invalid_deadlines = resume.clone();
        invalid_deadlines.submitted_at = invalid_deadlines.lock_expires_at;
        let deadline_error = super::Submission::try_from(invalid_deadlines)
            .expect_err("invalid deadline ordering must fail closed");
        assert!(deadline_error.to_string().contains("deadline ordering"));

        let mut zero_price = resume.clone();
        zero_price.max_price_wei = Some("0".to_string());
        let price_error =
            super::Submission::try_from(zero_price).expect_err("zero max price must fail closed");
        assert!(
            price_error
                .to_string()
                .contains("max_price_wei must be non-zero")
        );

        let mut noncanonical_id = resume.clone();
        noncanonical_id.provider_request_id = "0x01".to_string();
        let id_error = super::Submission::try_from(noncanonical_id)
            .expect_err("non-canonical request identity must fail closed");
        assert!(id_error.to_string().contains("non-canonical encoding"));

        let mut zero_multiplier = resume;
        zero_multiplier.max_price_multiplier = 0;
        let multiplier_error = super::Submission::try_from(zero_multiplier)
            .expect_err("zero max price multiplier must fail closed");
        assert!(
            multiplier_error
                .to_string()
                .contains("max_price_multiplier must be non-zero")
        );

        let error = serde_json::from_value::<crate::BoundlessSubmissionResume>(serde_json::json!({
            "provider_request_id": "0x1",
            "remote_tx_hash": null,
            "image_ref": "0ximage",
            "deployment": "base",
            "offchain": false,
            "expires_at": 2_000,
            "submitted_at": 1_000,
            "max_price_multiplier": 1,
        }))
        .expect_err("checkpoint without required fields must be rejected");
        assert!(error.to_string().contains("lock_expires_at"));
    }

    #[test]
    fn resumed_submission_rejects_a_different_provider_context() {
        let resume = crate::BoundlessSubmissionResume {
            provider_request_id: "0x1".to_string(),
            remote_tx_hash: None,
            image_ref: "0ximage".to_string(),
            deployment: "base".to_string(),
            offchain: false,
            expires_at: 2_000,
            lock_expires_at: 1_500,
            submitted_at: 1_000,
            max_price_multiplier: 1,
            max_price_wei: Some("1".to_string()),
            rebid_attempt: 1,
        };

        let image_error = validate_resume_context(&resume, "0xother", "base", false)
            .expect_err("a different guest image must not resume a paid request");
        assert!(
            image_error
                .to_string()
                .contains("does not match current image")
        );

        let deployment_error = validate_resume_context(&resume, "0ximage", "sepolia", false)
            .expect_err("a different market must not reuse the checkpointed request id");
        assert!(
            deployment_error
                .to_string()
                .contains("does not match current deployment")
        );

        let transport_error = validate_resume_context(&resume, "0ximage", "base", true)
            .expect_err("a different transport must not reuse the checkpointed request id");
        assert!(
            transport_error
                .to_string()
                .contains("does not match current transport")
        );
    }

    #[test]
    fn no_lock_deadline_uses_submission_wall_clock() {
        let timeout =
            no_lock_timeout_for_attempt(1, TEST_REBID_TIMEOUT_MS, TEST_REBID_MAX_ATTEMPTS);

        // Rebid attempts abandon the request after the configured rebid delay; the offer's lock
        // deadline (here far in the future) must not stretch that window.
        assert_eq!(no_lock_deadline(1_000, 10_000, timeout), 1_300);
    }

    #[test]
    fn final_attempt_waits_for_the_offer_lock_deadline() {
        // Attempt 5 exhausts the rebid budget (action = Abort). The request stays payable until
        // the offer's lock deadline, so the final attempt must wait it out rather than walking
        // away from a live request after only the rebid delay.
        let timeout =
            no_lock_timeout_for_attempt(5, TEST_REBID_TIMEOUT_MS, TEST_REBID_MAX_ATTEMPTS);
        assert_eq!(timeout.action, BoundlessTimeoutAction::Abort);

        assert_eq!(no_lock_deadline(1_000, 10_000, timeout), 10_000);
    }

    #[test]
    fn poll_timeout_defers_while_final_attempt_is_payable() {
        // submitted_at = 1_000, lock deadline = 10_000, request expiry = 20_000. Asserts run
        // against the production predicate on real metadata (deadline derived via
        // `no_lock_deadline`), so they cannot drift from the shipped deferral logic.
        let abort = no_lock_timeout_for_attempt(5, TEST_REBID_TIMEOUT_MS, TEST_REBID_MAX_ATTEMPTS);
        assert_eq!(abort.action, BoundlessTimeoutAction::Abort);
        let metadata = |lock_expires_at: u64, expires_at: u64, timeout: super::NoLockTimeout| {
            BoundlessSubmissionMetadata {
                expires_at,
                lock_expires_at,
                submitted_at: 1_000,
                no_lock_deadline: no_lock_deadline(1_000, lock_expires_at, timeout),
                no_lock_timeout_action: timeout.action,
                poll_timeout_at: Instant::now(),
            }
        };

        // Within the payable window the overall poll timeout must not fire: a timeout-triggered
        // replacement could reopen the double-pay window under another request id.
        assert!(should_defer_boundless_poll_timeout(
            &metadata(10_000, 20_000, abort),
            9_999
        ));
        // Once the lock deadline passes, the timeout may fire again.
        assert!(!should_defer_boundless_poll_timeout(
            &metadata(10_000, 20_000, abort),
            10_000
        ));
        // The deferral is bounded by the request expiry even when a (corrupt) record claims a
        // later lock deadline, so the poll loop cannot be pinned open forever.
        assert!(!should_defer_boundless_poll_timeout(
            &metadata(u64::MAX, 20_000, abort),
            20_000
        ));

        // Rebid attempts never defer the overall timeout.
        let rebid = no_lock_timeout_for_attempt(1, TEST_REBID_TIMEOUT_MS, TEST_REBID_MAX_ATTEMPTS);
        assert_eq!(rebid.action, BoundlessTimeoutAction::Rebid);
        assert!(!should_defer_boundless_poll_timeout(
            &metadata(10_000, 20_000, rebid),
            9_999
        ));
    }

    #[test]
    fn no_lock_timeout_uses_configured_rebid_delay() {
        let timeout = no_lock_timeout_for_attempt(1, 900_000, TEST_REBID_MAX_ATTEMPTS);

        assert_eq!(timeout.delay, Duration::from_millis(900_000));
        assert_eq!(timeout.action, BoundlessTimeoutAction::Rebid);
    }

    #[test]
    fn no_lock_timeout_clamps_invalid_rebid_delay_to_minimum() {
        for rebid_timeout_ms in [0, 999] {
            let timeout = no_lock_timeout_for_attempt(1, rebid_timeout_ms, TEST_REBID_MAX_ATTEMPTS);

            assert_eq!(timeout.delay, Duration::from_millis(MIN_REBID_TIMEOUT_MS));
            assert_eq!(timeout.action, BoundlessTimeoutAction::Rebid);
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
            BoundlessTimeoutAction::Rebid
        );
        assert_eq!(
            no_lock_timeout_for_attempt(4, TEST_REBID_TIMEOUT_MS, TEST_REBID_MAX_ATTEMPTS).action,
            BoundlessTimeoutAction::Rebid
        );
        assert_eq!(
            no_lock_timeout_for_attempt(5, TEST_REBID_TIMEOUT_MS, TEST_REBID_MAX_ATTEMPTS).action,
            BoundlessTimeoutAction::Abort
        );
        assert_eq!(
            no_lock_timeout_for_attempt(6, TEST_REBID_TIMEOUT_MS, TEST_REBID_MAX_ATTEMPTS).action,
            BoundlessTimeoutAction::Abort
        );
    }

    #[test]
    fn validate_offer_params_rejects_min_price_above_max_price() {
        let mut offer = sample_offer();
        offer.min_price_per_mcycle = Some("0.000001".to_string());
        let err = validate_offer_params(&offer, 100).unwrap_err();
        assert!(err.to_string().contains("min_price_per_mcycle"));
    }

    #[test]
    fn validate_offer_params_rejects_timeout_not_above_lock_timeout() {
        let mut offer = sample_offer();
        offer.timeouts = TimeoutPolicy::PerMcycle {
            lock_timeout_ms_per_mcycle: 300,
            timeout_ms_per_mcycle: 300,
            dynamic_pricing_timeout_modifier: None,
        };
        let err = validate_offer_params(&offer, 100).unwrap_err();
        assert!(err.to_string().contains("timeout"));
    }

    #[test]
    fn validate_offer_params_accepts_base_defaults() {
        let validated = validate_offer_params(&sample_offer(), 1_000).expect("valid offer");
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
        let validated = validate_offer_params(&sample_offer(), 1_000).expect("valid offer");
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

        let validated = validate_offer_params(&offer, 1_000).expect("valid offer");

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

        let validated = validate_offer_params(&offer, 1_000).expect("valid offer");
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
        offer.timeouts = TimeoutPolicy::PerMcycle {
            lock_timeout_ms_per_mcycle: 300,
            timeout_ms_per_mcycle: 900,
            dynamic_pricing_timeout_modifier: Some(2.0),
        };

        let validated = validate_offer_params(&offer, 1_000).expect("valid market offer");

        assert_eq!(validated.lock_timeout, 600);
        assert_eq!(validated.timeout, 1800);
        assert!(validated.timeout > validated.lock_timeout);
    }

    #[test]
    fn validate_offer_params_uses_fixed_timeout_policy() {
        let mut offer = sample_offer();
        offer.timeouts = TimeoutPolicy::Fixed {
            lock_timeout_secs: 600,
            timeout_secs: 3600,
        };

        let small = validate_offer_params(&offer, 100).expect("valid small offer");
        let large = validate_offer_params(&offer, 5_000).expect("valid large offer");

        assert_eq!(small.lock_timeout, 600);
        assert_eq!(small.timeout, 3600);
        assert_eq!(large.lock_timeout, 600);
        assert_eq!(large.timeout, 3600);
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
    fn deposit_topup_covers_all_in_flight_claims() {
        use alloy_primitives::U256;
        // Balance already covers all reserved claims -> no deposit.
        assert_eq!(
            super::deposit_topup(U256::from(30u64), U256::from(20u64)),
            U256::ZERO
        );
        // Two 10-wei claims, 15 on chain -> top up the 5 shortfall (the bug case: each alone looked covered).
        assert_eq!(
            super::deposit_topup(U256::from(15u64), U256::from(20u64)),
            U256::from(5u64)
        );
        // Empty account, one 10-wei claim -> deposit 10.
        assert_eq!(
            super::deposit_topup(U256::ZERO, U256::from(10u64)),
            U256::from(10u64)
        );
    }

    fn test_digest(value: u64) -> B256 {
        format!("0x{value:064x}").parse().expect("test digest")
    }

    #[tokio::test]
    async fn boundless_submission_gate_serializes_callers() {
        let gate = super::BoundlessBalanceGate::new();
        let first = gate.acquire_submission().await;

        assert!(
            tokio::time::timeout(Duration::from_millis(20), gate.acquire_submission())
                .await
                .is_err()
        );

        drop(first);
        let second = tokio::time::timeout(Duration::from_secs(1), gate.acquire_submission())
            .await
            .expect("second caller acquires the released submission permit");
        drop(second);
    }

    #[tokio::test]
    async fn boundless_account_wait_rechecks_the_runtime_lifecycle_before_broadcast() {
        let runtime = Arc::new(
            RuntimeManager::new_memory("test".to_string(), "boundless-lifecycle".to_string())
                .expect("runtime"),
        );
        let observer: Arc<dyn crate::ProverProgressObserver> =
            Arc::new(RuntimeLifecycleProgressObserver {
                runtime: Arc::clone(&runtime),
            });
        let gate = super::BoundlessBalanceGate::new();
        let blocker = gate.acquire_submission().await;
        let mut waiting = tokio::spawn({
            let gate = gate.clone();
            let observer = Arc::clone(&observer);
            async move {
                let account_permit = gate.acquire_submission().await;
                account_permit
                    .acquire_broadcast_permit(Some(&observer))
                    .await
            }
        });

        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut waiting)
                .await
                .is_err(),
            "the candidate must still be waiting for the account permit"
        );
        runtime.start_draining();
        drop(blocker);

        let result = tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .expect("candidate finishes after account release")
            .expect("candidate task");
        let Err(error) = result else {
            panic!("draining must reject lifecycle admission before broadcast");
        };
        assert!(error.to_string().contains("runtime is draining"));
    }

    #[tokio::test]
    async fn boundless_checkpoint_does_not_hold_account_permit() {
        let gate = super::BoundlessBalanceGate::new();
        let permit = gate.acquire_submission().await;

        let acquired = run_after_submission_permit(permit, || async {
            tokio::time::timeout(Duration::from_secs(1), gate.acquire_submission())
                .await
                .is_ok()
        })
        .await;

        assert!(acquired);
    }

    #[test]
    fn boundless_receipt_revert_removes_only_matching_reservation() {
        let request_id = U256::from(1u64);
        let reverted_digest = test_digest(11);
        let rebid_digest = test_digest(12);
        let mut state = super::BoundlessFundingState::default();
        state.record_recent(request_id, U256::from(100u64), 200, reverted_digest);
        state.record_recent(request_id, U256::from(120u64), 200, rebid_digest);

        state.reconcile_receipt(
            request_id,
            reverted_digest,
            super::BoundlessReceiptOutcome::ConfirmedRevert,
        );

        let remaining = state
            .recent
            .get(&request_id)
            .expect("rebid remains reserved");
        assert!(!remaining.contains_key(&reverted_digest));
        assert!(remaining.contains_key(&rebid_digest));
    }

    #[test]
    fn boundless_receipt_success_retains_reservation() {
        let request_id = U256::from(1u64);
        let request_digest = test_digest(11);
        let mut state = super::BoundlessFundingState::default();
        state.record_recent(request_id, U256::from(100u64), 200, request_digest);

        state.reconcile_receipt(
            request_id,
            request_digest,
            super::BoundlessReceiptOutcome::ConfirmedSuccess,
        );

        assert!(state.recent[&request_id].contains_key(&request_digest));
    }

    #[test]
    fn boundless_receipt_uncertain_retains_reservation() {
        let request_id = U256::from(1u64);
        let request_digest = test_digest(11);
        let mut state = super::BoundlessFundingState::default();
        state.record_recent(request_id, U256::from(100u64), 200, request_digest);

        state.reconcile_receipt(
            request_id,
            request_digest,
            super::BoundlessReceiptOutcome::Uncertain,
        );

        assert!(state.recent[&request_id].contains_key(&request_digest));
    }

    #[tokio::test]
    async fn boundless_cancelled_broadcast_retains_uncertain_and_funding_reservation() {
        let gate = super::BoundlessBalanceGate::new();
        let request = test_proof_request();
        let request_id = request.id;
        let request_digest = test_digest(11);
        let submission = test_submission();
        let gate_during_broadcast = gate.clone();
        let dispatch_entered = Arc::new(AtomicBool::new(false));
        let dispatch_entered_from_closure = Arc::clone(&dispatch_entered);
        let cancelled = tokio::time::timeout(
            Duration::from_millis(20),
            reserve_boundless_funding_before_dispatch(
                &gate,
                &request,
                &submission,
                Bytes::from_static(b"fixture_signature"),
                request_digest,
                U256::ZERO,
                1,
                1,
                10,
                || async move {
                    dispatch_entered_from_closure.store(true, Ordering::SeqCst);
                    let state = gate_during_broadcast.lock_state().await;
                    assert!(state.uncertain_submission().is_some());
                    assert!(state.recent[&request_id].contains_key(&request_digest));
                    drop(state);
                    std::future::pending::<()>().await;
                },
            ),
        )
        .await;

        assert!(
            cancelled.is_err(),
            "the simulated broadcast must be cancelled"
        );
        assert!(
            dispatch_entered.load(Ordering::SeqCst),
            "dispatch must start before the timeout cancels it"
        );
        let state = gate.lock_state().await;
        assert!(state.uncertain_submission().is_some());
        assert!(state.recent[&request_id].contains_key(&request_digest));
    }

    #[test]
    fn boundless_funding_state_sums_max_price_per_active_request_id() {
        let mut state = super::BoundlessFundingState::default();
        state.record_recent(U256::from(1u64), U256::from(100u64), 200, test_digest(11));
        state.record_recent(U256::from(1u64), U256::from(150u64), 200, test_digest(12));
        state.record_recent(U256::from(2u64), U256::from(200u64), 200, test_digest(21));

        let decision =
            state.funding_decision(U256::from(3u64), U256::from(70u64), U256::from(120u64), 100);

        assert_eq!(decision.reserved_count, 3);
        assert_eq!(decision.required_total, U256::from(420u64));
        assert_eq!(decision.attached_value, U256::from(300u64));
    }

    #[test]
    fn boundless_funding_state_current_rebid_uses_the_request_id_maximum() {
        let mut state = super::BoundlessFundingState::default();
        state.record_recent(U256::from(1u64), U256::from(100u64), 200, test_digest(11));
        state.record_recent(U256::from(1u64), U256::from(150u64), 200, test_digest(12));

        let decision =
            state.funding_decision(U256::from(1u64), U256::from(120u64), U256::from(50u64), 100);

        assert_eq!(decision.reserved_count, 1);
        assert_eq!(decision.required_total, U256::from(150u64));
        assert_eq!(decision.attached_value, U256::from(100u64));
    }

    #[test]
    fn boundless_funding_state_prunes_reservations_after_chain_time_grace() {
        let mut state = super::BoundlessFundingState::default();
        state.record_recent(U256::from(1u64), U256::from(100u64), 100, test_digest(11));
        state.record_recent(U256::from(2u64), U256::from(50u64), 101, test_digest(21));

        let at_boundary =
            state.funding_decision(U256::from(3u64), U256::from(70u64), U256::ZERO, 160);

        assert!(state.recent.contains_key(&U256::from(1u64)));
        assert!(state.recent.contains_key(&U256::from(2u64)));
        assert_eq!(at_boundary.reserved_count, 3);
        assert_eq!(at_boundary.required_total, U256::from(220u64));

        let after_boundary =
            state.funding_decision(U256::from(3u64), U256::from(70u64), U256::ZERO, 161);

        assert!(!state.recent.contains_key(&U256::from(1u64)));
        assert!(state.recent.contains_key(&U256::from(2u64)));
        assert_eq!(after_boundary.reserved_count, 2);
        assert_eq!(after_boundary.required_total, U256::from(120u64));
        assert_eq!(after_boundary.attached_value, U256::from(120u64));
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
    fn manual_price_escalates_without_ceiling() {
        // 10_000 bps = +100%/rung (x2); attempt 3 -> 2 rungs -> x4.
        let result =
            super::escalate_and_clamp_manual_max_price(U256::from(100), 3, 10_000, 4, None)
                .expect("escalated price");
        assert_eq!(result.max_price, U256::from(400));
        assert!(!result.clamped_to_ceiling);
    }

    #[test]
    fn manual_price_clamps_escalation_to_ceiling() {
        let result = super::escalate_and_clamp_manual_max_price(
            U256::from(100),
            3,
            10_000,
            4,
            Some(U256::from(250)),
        )
        .expect("escalated price");
        assert_eq!(result.max_price, U256::from(250));
        assert!(result.clamped_to_ceiling);
    }

    #[test]
    fn manual_price_at_ceiling_is_not_clamped() {
        // Attempt 1 escalates by zero rungs, so the base bids unchanged and equals the ceiling.
        let result = super::escalate_and_clamp_manual_max_price(
            U256::from(100),
            1,
            10_000,
            4,
            Some(U256::from(100)),
        )
        .expect("escalated price");
        assert_eq!(result.max_price, U256::from(100));
        assert!(!result.clamped_to_ceiling);
    }

    #[test]
    fn manual_price_escalation_rejects_overflow() {
        let err = super::escalate_and_clamp_manual_max_price(
            U256::MAX,
            2,
            10_000,
            4,
            Some(U256::from(1)),
        )
        .expect_err("overflowing escalation");
        assert!(err.to_string().contains("overflows"));
    }

    #[test]
    fn validate_offer_params_threads_manual_absolute_ceiling_to_cap() {
        let mut offer = sample_offer();
        offer.absolute_max_price_per_mcycle = Some("0.0000015".to_string());

        let validated = validate_offer_params(&offer, 1_000).expect("valid offer");

        // Manual mode returns the base max price plus the ceiling as `max_price_cap`; the clamp of
        // the bps-escalated bid to it happens in `build_request`.
        assert!(validated.max_price.is_some());
        let cap = validated.max_price_cap.expect("manual ceiling cap");
        assert_eq!(cap.value, parse_ether("0.0015").unwrap());
    }

    #[test]
    fn validate_offer_params_maps_market_absolute_ceiling_to_cap() {
        let mut offer = sample_offer();
        offer.pricing_mode = BoundlessPricingMode::Market;
        offer.max_price_per_mcycle = None;
        offer.min_price_per_mcycle = None;
        offer.absolute_max_price_per_mcycle = Some("0.00000006".to_string());

        let validated = validate_offer_params(&offer, 1_000).expect("valid offer");
        let max_price_cap = validated.max_price_cap.expect("market max price cap");

        assert!(validated.max_price.is_none());
        assert!(validated.min_price.is_none());
        assert_eq!(max_price_cap.asset, Asset::ETH);
        assert_eq!(max_price_cap.value, parse_ether("0.00006").unwrap());
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
    fn storage_validation_rejects_missing_boundless_uploader() {
        let _guard = StorageEnvGuard::new(&[]);

        let error = BoundlessProver::validate_storage_configuration()
            .expect_err("Boundless requires a storage uploader");

        assert!(error.to_string().contains("storage uploader"), "{error}");
    }

    #[test]
    fn storage_uploader_config_prefers_implicit_gcs_over_stale_s3_bucket() {
        let _guard = StorageEnvGuard::new(&[
            ("GCS_BUCKET", "raiko-boundless"),
            ("S3_BUCKET", "stale-s3-bucket"),
        ]);

        let config = storage_uploader_config_from_env().expect("GCS storage config");

        assert_eq!(config.storage_uploader, StorageUploaderType::Gcs);
        assert_eq!(config.gcs_bucket.as_deref(), Some("raiko-boundless"));
    }

    #[cfg(not(feature = "boundless-s3"))]
    #[test]
    fn storage_uploader_config_rejects_implicit_s3_without_compiled_support() {
        let _guard = StorageEnvGuard::new(&[("S3_BUCKET", "stale-s3-bucket")]);

        let error = BoundlessProver::validate_storage_configuration()
            .expect_err("implicit S3 must require the optional boundless-s3 feature");

        assert!(error.to_string().contains("boundless-s3"), "{error}");
    }

    #[cfg(not(feature = "boundless-s3"))]
    #[test]
    fn storage_uploader_config_rejects_s3_without_compiled_support() {
        let _guard = StorageEnvGuard::new(&[
            ("BOUNDLESS_STORAGE_UPLOADER", "s3"),
            ("S3_BUCKET", "raiko-boundless"),
            ("S3_URL", "http://127.0.0.1:9000"),
            ("AWS_ACCESS_KEY_ID", "access-key"),
            ("AWS_SECRET_ACCESS_KEY", "secret-key"),
            ("AWS_REGION", "us-east-1"),
            ("S3_PRESIGNED", "true"),
            ("S3_PUBLIC_URL", "false"),
        ]);

        let error = BoundlessProver::validate_storage_configuration()
            .expect_err("S3 must require the optional boundless-s3 feature");

        assert!(error.to_string().contains("boundless-s3"), "{error}");
    }

    #[cfg(feature = "boundless-s3")]
    #[test]
    fn storage_uploader_config_keeps_s3_available_when_compiled() {
        let _guard = StorageEnvGuard::new(&[
            ("BOUNDLESS_STORAGE_UPLOADER", "s3"),
            ("S3_BUCKET", "raiko-boundless"),
            ("S3_URL", "http://127.0.0.1:9000"),
            ("AWS_ACCESS_KEY_ID", "access-key"),
            ("AWS_SECRET_ACCESS_KEY", "secret-key"),
            ("AWS_REGION", "us-east-1"),
            ("S3_PRESIGNED", "true"),
            ("S3_PUBLIC_URL", "false"),
        ]);

        let config = storage_uploader_config_from_env().expect("storage config");

        assert_eq!(config.storage_uploader, StorageUploaderType::S3);
        assert_eq!(config.s3_bucket.as_deref(), Some("raiko-boundless"));
        assert_eq!(config.s3_url.as_deref(), Some("http://127.0.0.1:9000"));
        assert_eq!(config.aws_access_key_id.as_deref(), Some("access-key"));
        assert_eq!(config.aws_secret_access_key.as_deref(), Some("secret-key"));
        assert_eq!(config.aws_region.as_deref(), Some("us-east-1"));
        assert_eq!(config.s3_presigned, Some(true));
        assert_eq!(config.s3_public_url, Some(false));
    }

    #[cfg(feature = "boundless-s3")]
    #[test]
    fn storage_uploader_config_selects_implicit_s3_when_compiled() {
        let _guard = StorageEnvGuard::new(&[("S3_BUCKET", "raiko-boundless")]);

        let config = storage_uploader_config_from_env().expect("implicit S3 storage config");

        assert_eq!(config.storage_uploader, StorageUploaderType::S3);
        assert_eq!(config.s3_bucket.as_deref(), Some("raiko-boundless"));
    }

    #[cfg(feature = "boundless-s3")]
    #[tokio::test]
    async fn storage_downloader_skips_s3_when_s3_is_unconfigured() {
        let downloader =
            BoundlessStorageDownloader::from_uploader_config(&StorageUploaderConfig::default())
                .await
                .expect("unconfigured storage downloader");

        assert!(downloader.s3.is_none());
    }

    #[tokio::test]
    async fn storage_downloader_allows_gcs_uploader_without_adc() {
        let _guard = StorageEnvGuard::new(&[
            ("BOUNDLESS_STORAGE_UPLOADER", "gcs"),
            (
                "GOOGLE_APPLICATION_CREDENTIALS",
                "raiko2-test-missing-gcs-credentials.json",
            ),
        ]);
        let config = storage_uploader_config_from_env().expect("GCS storage config");

        let downloader = BoundlessStorageDownloader::from_uploader_config(&config)
            .await
            .expect("GCS uploader must not require downloader ADC");

        assert!(downloader.gcs.is_none());
    }

    #[cfg(feature = "boundless-s3")]
    #[test]
    fn storage_downloader_only_initializes_s3_for_s3_uploads() {
        let mut config = StorageUploaderConfig::default();
        assert!(!should_initialize_s3_downloader(&config));

        for storage_uploader in [
            StorageUploaderType::Gcs,
            StorageUploaderType::Pinata,
            StorageUploaderType::File,
        ] {
            config.storage_uploader = storage_uploader;
            assert!(!should_initialize_s3_downloader(&config));
        }

        config.storage_uploader = StorageUploaderType::S3;
        assert!(should_initialize_s3_downloader(&config));
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
        let before = SystemTime::now();
        let refresh = super::presigned_refresh_at(&url);
        let after = SystemTime::now();
        // Refreshes 120s before the 3600s expiry, bracketed by the clock reads around
        // presigned_refresh_at so it stays deterministic under scheduler jitter.
        assert!(refresh >= before + Duration::from_secs(3600 - 120));
        assert!(refresh <= after + Duration::from_secs(3600 - 120));
    }
}
