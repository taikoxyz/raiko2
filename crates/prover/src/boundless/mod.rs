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
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use alloy_primitives::{B256, Bytes, U256, address, keccak256};
use alloy_signer_local::PrivateKeySigner;
use boundless_market::{
    Client, ProofRequest, StorageUploaderConfig,
    contracts::RequestId,
    deployments::{BASE, Deployment, SEPOLIA},
    input::GuestEnv,
    price_oracle::{Amount, Asset},
    request_builder::OfferParams,
    storage::StorageUploaderType,
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

type BoundlessStatusRegistry = Arc<Mutex<HashMap<RemoteSubmissionId, BoundlessSubmissionState>>>;

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
            poll_submission.request_id,
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

#[allow(clippy::too_many_arguments)]
fn classify_boundless_status(
    submission_id: RemoteSubmissionId,
    provider_request_id: &str,
    request_id: U256,
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

    let _ = (request_id, is_fulfilled, is_locked, request_deadline);
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

fn retry_price_multiplier(attempt: u64, price_multiplier: u32, max_attempts: u32) -> u32 {
    let rebids = attempt.saturating_sub(1).min(u64::from(max_attempts));
    let price_multiplier = price_multiplier.max(1);
    let mut multiplier = 1_u32;
    for _ in 0..rebids {
        multiplier = multiplier.saturating_mul(price_multiplier);
    }
    multiplier
}

fn attempt_for_price_multiplier(multiplier: u32, price_multiplier: u32, max_attempts: u32) -> u64 {
    let multiplier = multiplier.max(1);
    if price_multiplier <= 1 {
        return 1;
    }
    let mut attempt = 1_u64;
    while retry_price_multiplier(attempt, price_multiplier, max_attempts) < multiplier
        && attempt < u64::from(max_attempts).saturating_add(1)
    {
        attempt = attempt.saturating_add(1);
    }
    attempt
}

/// Attempt number to resume at for a stored submission. Submissions written after this field was
/// added carry the real attempt; legacy records (`attempt == 0`) fall back to inferring it from the
/// escalated price, which is only recoverable when the price actually escalates
/// (`price_multiplier > 1`). Persisting the attempt is what keeps a flat-price (`== 1`) rebid budget
/// bounded across restarts instead of resetting to 1 every time.
fn resume_attempt(submission: &Submission, price_multiplier: u32, max_attempts: u32) -> u64 {
    if submission.attempt > 0 {
        submission.attempt
    } else {
        attempt_for_price_multiplier(
            submission.max_price_multiplier,
            price_multiplier,
            max_attempts,
        )
    }
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

#[cfg(test)]
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
#[cfg(test)]
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
    // Offer lock deadline (`rampUpStart + lockTimeout`). The market pays nothing for fulfillments
    // past this time, so it bounds the payable window; `0` when resumed from a legacy record.
    lock_expires_at: u64,
    submitted_at: u64,
    max_price_multiplier: u32,
    // 1-based rebid attempt that produced this submission; persisted so restarts don't reset the
    // rebid budget when the price is flat (`rebid_price_multiplier == 1`).
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
    max_price_multiplier: u32,
    attempt: u64,
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

#[allow(clippy::too_many_arguments)]
async fn publish_boundless_progress(
    observer: Option<&Arc<dyn ProverProgressObserver>>,
    submission: &Submission,
    image_ref: &str,
    deployment: &str,
    offchain: bool,
    quoted_mcycles_count: u32,
    evaluated_mcycles_count: u32,
    max_price_multiplier: u32,
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
                    max_price_multiplier,
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
    status_tracker: OnceLock<RemoteStatusTracker>,
    status_registry: BoundlessStatusRegistry,
}

impl BoundlessProver {
    #[must_use]
    pub fn new(config: BoundlessConfig) -> Self {
        Self {
            deployment: config.get_effective_deployment(),
            config,
            programs: Arc::new(RwLock::new(HashMap::new())),
            status_tracker: OnceLock::new(),
            status_registry: Arc::new(Mutex::new(HashMap::new())),
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
        max_price_multiplier: u32,
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
        } = validate_offer_params(
            offer_spec,
            mcycles_count,
            self.config.block_time_sec(),
            max_price_multiplier,
        )?;
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
            max_price_multiplier,
            max_price_cap.as_ref(),
            mcycles_count,
        )?;
        Ok(request)
    }

    async fn submit_request_offchain(
        &self,
        client: &Client,
        request: &ProofRequest,
        max_price_multiplier: u32,
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
            max_price_multiplier,
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
        max_price_multiplier: u32,
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
            max_price_multiplier,
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
            max_price_multiplier,
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
                    max_price_multiplier,
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
            context.max_price_multiplier,
            context.reuse_request_id,
        ))
        .await?;

        if self.config.offchain {
            let submission = self
                .submit_request_offchain(
                    context.client,
                    &request,
                    context.max_price_multiplier,
                    context.attempt,
                )
                .await?;
            publish_boundless_progress(
                context.observer,
                &submission,
                context.image_ref,
                context.deployment,
                true,
                context.quoted_mcycles_count,
                context.evaluated_mcycles_count,
                context.max_price_multiplier,
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
            context.max_price_multiplier,
            context.attempt,
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
                        no_lock_timeout_action: match no_lock_timeout.action {
                            NoLockTimeoutAction::Rebid => BoundlessTimeoutAction::Rebid,
                            NoLockTimeoutAction::Abort => BoundlessTimeoutAction::Abort,
                        },
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
                self.fetch_boundless_fulfillment_until(
                    client,
                    submission,
                    proof_type,
                    image_id,
                    block_image_id,
                    expected_input_hash,
                    quoted_mcycles_count,
                    evaluated_mcycles_count,
                    proposal_carry_data,
                    poll_timeout_at,
                )
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
                Err(BoundlessAttemptError::Fatal(RaikoError::Guest(format!(
                    "Boundless request {} was not locked {deadline_detail}; \
                     exhausted boundless no-lock rebids",
                    submission.provider_request_id
                ))))
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

    #[allow(clippy::too_many_arguments)]
    async fn fetch_boundless_fulfillment_until(
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
        deadline: Instant,
    ) -> Result<Proof, BoundlessAttemptError> {
        loop {
            match self
                .fetch_boundless_fulfillment(
                    client,
                    submission,
                    proof_type,
                    image_id,
                    block_image_id,
                    expected_input_hash,
                    quoted_mcycles_count,
                    evaluated_mcycles_count,
                    proposal_carry_data,
                )
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

    #[allow(clippy::too_many_arguments)]
    async fn fetch_boundless_fulfillment(
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
        let receipt_json = if proof_type == "proposal" {
            match decode_seal(seal.clone(), image_id, journal.to_vec()) {
                Ok(ContractReceipt::Base(receipt)) => serde_json::to_string(&receipt).ok(),
                Ok(ContractReceipt::SetInclusion(_)) | Err(_) => None,
            }
        } else {
            None
        };
        let input_hash = match proof_type {
            "proposal" => parse_shasta_proposal_input_hash(journal)?,
            _ => parse_shasta_aggregation_input_hash(journal)?,
        };
        if let ("proposal", Some(carry)) = (proof_type, proposal_carry_data) {
            ensure_shasta_proposal_input_matches_carry(input_hash, carry, "boundless")?;
        }
        if input_hash != expected_input_hash {
            return Err(BoundlessAttemptError::Fatal(RaikoError::Guest(
                "Boundless fulfillment journal does not match local dry-run journal".to_string(),
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
                "lock_expires_at": submission.lock_expires_at,
                "submitted_at": submission.submitted_at,
                "max_price_multiplier": submission.max_price_multiplier,
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
            "proposal" => {
                encode_risc0_proposal_seal_payload(&seal, B256::from_slice(image_id.as_bytes()))
            }
            _ => encode_risc0_aggregation_seal_payload(
                &seal,
                B256::from_slice(
                    block_image_id
                        .ok_or_else(|| {
                            RaikoError::Guest(
                                "missing block image id for aggregation proof".to_string(),
                            )
                        })?
                        .as_bytes(),
                ),
                B256::from_slice(image_id.as_bytes()),
            ),
        };
        Ok(Proof {
            proof: Some(proof),
            input: Some(input_hash),
            quote: receipt_json,
            uuid: Some(alloy_primitives::hex::encode_prefixed(image_id.as_bytes())),
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
        let mut last_retry_reason: Option<String> = None;
        // Market request id shared by rebid rungs of this proof task. The market keys locks and
        // paid fulfillments on the id, so reusing it across rebids makes paying for more than one
        // rung impossible by construction. `None` mints a fresh id (first attempt, or after the
        // previous id became unusable).
        let mut reuse_request_id: Option<U256> = None;

        loop {
            let submission = if let Some(submission) = resume_submission.take() {
                attempt = attempt.max(resume_attempt(
                    &submission,
                    self.config.rebid_price_multiplier,
                    self.config.rebid_max_attempts,
                ));
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
                    submission.max_price_multiplier,
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
                let max_price_multiplier = retry_price_multiplier(
                    attempt,
                    self.config.rebid_price_multiplier,
                    self.config.rebid_max_attempts,
                );
                let submit = |reuse_request_id: Option<U256>| {
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
                        max_price_multiplier,
                        attempt,
                        reuse_request_id,
                    }))
                };
                match submit(reuse_request_id).await {
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
                        submit(None).await?
                    }
                }
            };
            attempt = attempt.max(attempt_for_price_multiplier(
                submission.max_price_multiplier,
                self.config.rebid_price_multiplier,
                self.config.rebid_max_attempts,
            ));

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
                    proof_type,
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
    max_price_multiplier: u32,
    max_price_cap: Option<&Amount>,
) -> RaikoResult<MarketOfferPrices> {
    let escalated_max = autopriced_max
        .checked_mul(U256::from(max_price_multiplier.max(1)))
        .ok_or_else(|| {
            RaikoError::InvalidRequestConfig(format!(
                "Boundless market max price {autopriced_max} wei overflows when multiplied by \
                 retry multiplier {max_price_multiplier}"
            ))
        })?;
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
    max_price_multiplier: u32,
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
        max_price_multiplier,
        max_price_cap,
    )?;
    if prices.clamped_to_cap {
        tracing::warn!(
            mcycles_count,
            autopriced_max_price_wei = %autopriced_max,
            max_price_multiplier,
            capped_max_price_wei = %prices.max_price,
            "Boundless market offer max price exceeds the max_price_per_mcycle cap; bidding at the cap"
        );
    } else if max_price_multiplier > 1 {
        tracing::info!(
            mcycles_count,
            autopriced_max_price_wei = %autopriced_max,
            max_price_multiplier,
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
    max_price_multiplier: u32,
) -> RaikoResult<ValidatedOfferParams> {
    validate_offer_spec(offer_spec).map_err(RaikoError::InvalidRequestConfig)?;
    let (max_price, min_price, max_price_cap) = match offer_spec.pricing_mode {
        BoundlessPricingMode::Manual => {
            let max_price_value = offer_spec.max_price_per_mcycle.as_deref().ok_or_else(|| {
                RaikoError::InvalidRequestConfig(
                    "max_price_per_mcycle is required when pricing_mode=manual".to_string(),
                )
            })?;
            let mut max_price = parse_request_amount(
                max_price_value,
                "max_price_per_mcycle",
                Asset::ETH,
                mcycles_count,
            )?;
            // Escalate only the cap on resubmissions; the min price keeps the
            // ramp start unchanged so an idle prover still locks cheaply.
            max_price.value = max_price
                .value
                .checked_mul(U256::from(max_price_multiplier.max(1)))
                .ok_or_else(|| {
                    RaikoError::InvalidRequestConfig(format!(
                        "max_price_per_mcycle overflows when multiplied by retry multiplier {max_price_multiplier}"
                    ))
                })?;
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
        BatchQuoteStrategy, BoundlessConfig, BoundlessPollSubmission, BoundlessPricingMode,
        BoundlessProver, BoundlessStatusSource, BoundlessSubmissionMetadata,
        BoundlessSubmissionState, BoundlessTerminalOutcome, BoundlessTimeoutAction,
        DeploymentConfig, DeploymentType, ElfType, JsonRpcError, JsonRpcResponse,
        MIN_REBID_TIMEOUT_MS, NoLockTimeoutAction, attempt_for_price_multiplier,
        boundless_poll_error_statuses, classify_boundless_status, defer_poll_timeout_while_payable,
        escalate_and_cap_market_prices, exceeds_submission_budget, no_lock_deadline_elapsed,
        no_lock_timeout_for_attempt, now_secs, parse_bool_result, parse_env_bool, parse_env_url,
        quote_batch_mcycles, retry_price_multiplier, should_rebid_unlocked_request,
        storage_uploader_config_from_env, user_cycles_to_mcycles, validate_offer_params,
    };
    use crate::boundless_config::default_batch_offer_params;
    use alloy_primitives::{U256, address, utils::parse_ether};
    use boundless_market::{
        price_oracle::{Amount, Asset},
        storage::StorageUploaderType,
    };
    use raiko2_primitives::Proof;
    use raiko2_primitives::ProofType;
    use raiko2_remote_poller::{RemoteStatus, RemoteSubmission, RemoteSubmissionId};
    use std::{
        collections::HashMap,
        env,
        sync::{Arc, Mutex, MutexGuard},
        time::{Duration, Instant},
    };

    static ENV_LOCK: Mutex<()> = Mutex::new(());
    const TEST_REBID_TIMEOUT_MS: u64 = 300_000;
    const TEST_REBID_PRICE_MULTIPLIER: u32 = 2;
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
                    poll_timeout_at: Instant::now() - Duration::from_secs(1),
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
        let request_id = U256::from(1);
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
            request_id,
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
            request_id,
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
            request_id,
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
            Instant::now() - Duration::from_secs(1),
        );
        poll_timeout_state.metadata.expires_at = now.saturating_sub(1);
        let (status, outcome) = classify_boundless_status(
            submission_id,
            "0x1",
            request_id,
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
    fn retry_price_multiplier_allows_four_rebid_attempts() {
        assert_eq!(
            retry_price_multiplier(0, TEST_REBID_PRICE_MULTIPLIER, TEST_REBID_MAX_ATTEMPTS),
            1
        );
        assert_eq!(
            retry_price_multiplier(1, TEST_REBID_PRICE_MULTIPLIER, TEST_REBID_MAX_ATTEMPTS),
            1
        );
        assert_eq!(
            retry_price_multiplier(2, TEST_REBID_PRICE_MULTIPLIER, TEST_REBID_MAX_ATTEMPTS),
            2
        );
        assert_eq!(
            retry_price_multiplier(3, TEST_REBID_PRICE_MULTIPLIER, TEST_REBID_MAX_ATTEMPTS),
            4
        );
        assert_eq!(
            retry_price_multiplier(4, TEST_REBID_PRICE_MULTIPLIER, TEST_REBID_MAX_ATTEMPTS),
            8
        );
        assert_eq!(
            retry_price_multiplier(5, TEST_REBID_PRICE_MULTIPLIER, TEST_REBID_MAX_ATTEMPTS),
            16
        );
        assert_eq!(
            retry_price_multiplier(6, TEST_REBID_PRICE_MULTIPLIER, TEST_REBID_MAX_ATTEMPTS),
            16
        );
    }

    #[test]
    fn retry_price_multiplier_uses_configured_multiplier_and_attempts() {
        assert_eq!(retry_price_multiplier(1, 3, 2), 1);
        assert_eq!(retry_price_multiplier(2, 3, 2), 3);
        assert_eq!(retry_price_multiplier(3, 3, 2), 9);
        assert_eq!(retry_price_multiplier(4, 3, 2), 9);
    }

    #[test]
    fn attempt_for_price_multiplier_restores_rebid_state() {
        assert_eq!(
            attempt_for_price_multiplier(0, TEST_REBID_PRICE_MULTIPLIER, TEST_REBID_MAX_ATTEMPTS),
            1
        );
        assert_eq!(
            attempt_for_price_multiplier(1, TEST_REBID_PRICE_MULTIPLIER, TEST_REBID_MAX_ATTEMPTS),
            1
        );
        assert_eq!(
            attempt_for_price_multiplier(2, TEST_REBID_PRICE_MULTIPLIER, TEST_REBID_MAX_ATTEMPTS),
            2
        );
        assert_eq!(
            attempt_for_price_multiplier(4, TEST_REBID_PRICE_MULTIPLIER, TEST_REBID_MAX_ATTEMPTS),
            3
        );
        assert_eq!(
            attempt_for_price_multiplier(8, TEST_REBID_PRICE_MULTIPLIER, TEST_REBID_MAX_ATTEMPTS),
            4
        );
        assert_eq!(
            attempt_for_price_multiplier(16, TEST_REBID_PRICE_MULTIPLIER, TEST_REBID_MAX_ATTEMPTS),
            5
        );
        assert_eq!(
            attempt_for_price_multiplier(32, TEST_REBID_PRICE_MULTIPLIER, TEST_REBID_MAX_ATTEMPTS),
            5
        );
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
    fn resume_attempt_prefers_persisted_attempt() {
        // A persisted attempt is restored verbatim, even for flat-price rebids
        // (rebid_price_multiplier == 1) where the price cannot encode the attempt.
        let submission = test_submission(1, 3);
        assert_eq!(
            super::resume_attempt(&submission, 1, TEST_REBID_MAX_ATTEMPTS),
            3
        );
        assert_eq!(
            super::resume_attempt(
                &submission,
                TEST_REBID_PRICE_MULTIPLIER,
                TEST_REBID_MAX_ATTEMPTS
            ),
            3
        );
    }

    #[test]
    fn resume_attempt_falls_back_to_price_for_legacy_records() {
        // Legacy records predate the attempt field (attempt == 0): recover it from the escalated
        // price when the price actually escalates...
        let escalated = test_submission(4, 0);
        assert_eq!(
            super::resume_attempt(
                &escalated,
                TEST_REBID_PRICE_MULTIPLIER,
                TEST_REBID_MAX_ATTEMPTS
            ),
            attempt_for_price_multiplier(4, TEST_REBID_PRICE_MULTIPLIER, TEST_REBID_MAX_ATTEMPTS)
        );
        // ...but a flat-price legacy record is unrecoverable and falls back to 1 (known limitation
        // that only affects records written before this field existed).
        let flat = test_submission(1, 0);
        assert_eq!(super::resume_attempt(&flat, 1, TEST_REBID_MAX_ATTEMPTS), 1);
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
        let err = validate_offer_params(&offer, 100, 2, 1).unwrap_err();
        assert!(err.to_string().contains("min_price_per_mcycle"));
    }

    #[test]
    fn validate_offer_params_rejects_timeout_not_above_lock_timeout() {
        let mut offer = sample_offer();
        offer.lock_timeout_ms_per_mcycle = 300;
        offer.timeout_ms_per_mcycle = 300;
        let err = validate_offer_params(&offer, 100, 2, 1).unwrap_err();
        assert!(err.to_string().contains("timeout"));
    }

    #[test]
    fn validate_offer_params_accepts_base_defaults() {
        let validated = validate_offer_params(&sample_offer(), 1_000, 2, 1).expect("valid offer");
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
    fn validate_offer_params_escalates_only_max_price() {
        let base = validate_offer_params(&sample_offer(), 1_000, 2, 1).expect("valid offer");
        let escalated = validate_offer_params(&sample_offer(), 1_000, 2, 4).expect("valid offer");

        let base_max = base.max_price.expect("manual max price");
        let escalated_max = escalated.max_price.expect("manual max price");
        assert_eq!(escalated_max.value, base_max.value * U256::from(4));

        let base_min = base.min_price.expect("manual min price");
        let escalated_min = escalated.min_price.expect("manual min price");
        assert_eq!(escalated_min.value, base_min.value);
    }

    #[test]
    fn retry_price_multiplier_doubles_per_attempt_with_cap() {
        assert_eq!(
            super::retry_price_multiplier(1, TEST_REBID_PRICE_MULTIPLIER, TEST_REBID_MAX_ATTEMPTS),
            1
        );
        assert_eq!(
            super::retry_price_multiplier(2, TEST_REBID_PRICE_MULTIPLIER, TEST_REBID_MAX_ATTEMPTS),
            2
        );
        assert_eq!(
            super::retry_price_multiplier(3, TEST_REBID_PRICE_MULTIPLIER, TEST_REBID_MAX_ATTEMPTS),
            4
        );
        assert_eq!(
            super::retry_price_multiplier(4, TEST_REBID_PRICE_MULTIPLIER, TEST_REBID_MAX_ATTEMPTS),
            8
        );
        assert_eq!(
            super::retry_price_multiplier(5, TEST_REBID_PRICE_MULTIPLIER, TEST_REBID_MAX_ATTEMPTS),
            16
        );
        assert_eq!(
            super::retry_price_multiplier(
                100,
                TEST_REBID_PRICE_MULTIPLIER,
                TEST_REBID_MAX_ATTEMPTS
            ),
            16
        );
    }

    #[test]
    fn validate_offer_params_omits_prices_for_market_pricing() {
        let mut offer = sample_offer();
        offer.pricing_mode = BoundlessPricingMode::Market;
        offer.max_price_per_mcycle = None;
        offer.min_price_per_mcycle = None;

        let validated = validate_offer_params(&offer, 1_000, 2, 1).expect("valid offer");

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

        let validated = validate_offer_params(&offer, 1_000, 2, 1).expect("valid offer");
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

        let validated = validate_offer_params(&offer, 1_000, 2, 1).expect("valid market offer");

        assert_eq!(validated.lock_timeout, 600);
        assert_eq!(validated.timeout, 1800);
        assert!(validated.timeout > validated.lock_timeout);
    }

    #[test]
    fn validate_offer_params_uses_fixed_timeout_overrides() {
        let mut offer = sample_offer();
        offer.lock_timeout_secs = Some(600);
        offer.timeout_secs = Some(3600);

        let small = validate_offer_params(&offer, 100, 2, 1).expect("valid small offer");
        let large = validate_offer_params(&offer, 5_000, 2, 1).expect("valid large offer");

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
        let err = validate_offer_params(&offer, 5_000, 2, 1).unwrap_err();
        assert!(err.to_string().contains("timeout"));
    }

    #[test]
    fn validate_offer_params_ramp_up_check_uses_fixed_lock_timeout() {
        let mut offer = sample_offer();
        // 60 blocks at 2s block time is a 120s ramp-up, while the derived lock
        // timeout for 100 mcycles is only 30s; the fixed override lifts it.
        assert!(validate_offer_params(&offer, 100, 2, 1).is_err());

        offer.lock_timeout_secs = Some(600);
        offer.timeout_secs = Some(3600);
        let validated = validate_offer_params(&offer, 100, 2, 1).expect("valid offer");
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

        let validated = validate_offer_params(&offer, 1_000, 2, 1).expect("valid market offer");

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

        let validated = validate_offer_params(&offer, 1_000, 2, 1).expect("valid market offer");

        assert_eq!(validated.lock_timeout, 600);
        assert_eq!(validated.timeout, 3600);
    }

    #[test]
    fn market_prices_escalate_only_the_max_price() {
        let prices = escalate_and_cap_market_prices(U256::from(100), U256::from(10), 4, None)
            .expect("escalated prices");

        assert_eq!(prices.max_price, U256::from(400));
        assert_eq!(prices.min_price, U256::from(10));
        assert!(!prices.clamped_to_cap);
    }

    #[test]
    fn market_prices_pass_through_without_escalation_or_cap() {
        for multiplier in [0, 1] {
            let prices =
                escalate_and_cap_market_prices(U256::from(100), U256::from(10), multiplier, None)
                    .expect("flat prices");

            assert_eq!(prices.max_price, U256::from(100));
            assert_eq!(prices.min_price, U256::from(10));
            assert!(!prices.clamped_to_cap);
        }
    }

    #[test]
    fn market_prices_accept_offers_at_or_below_cap() {
        let max_price_cap = Amount::new(U256::from(1_000), Asset::ETH);
        let prices = escalate_and_cap_market_prices(
            U256::from(100),
            U256::from(10),
            2,
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
        let prices = escalate_and_cap_market_prices(
            U256::from(100),
            U256::from(10),
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
        let prices = escalate_and_cap_market_prices(
            U256::from(100),
            U256::from(10),
            1,
            Some(&max_price_cap),
        )
        .expect("capped prices");

        assert_eq!(prices.max_price, U256::from(5));
        assert_eq!(prices.min_price, U256::from(5));
        assert!(prices.clamped_to_cap);
    }

    #[test]
    fn market_price_escalation_rejects_overflow() {
        let err = escalate_and_cap_market_prices(U256::MAX, U256::from(10), 2, None)
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
}
