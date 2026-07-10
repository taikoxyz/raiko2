//! SP1 zkVM Prover for Raiko V2
//!
//! This module provides the SP1 prover implementation for generating
//! zero-knowledge proofs of Taiko block execution.

mod types;

pub use crate::sp1_config::{
    ExecutionMode, ProverMode, RecursionMode, Sp1Config, Sp1ConfigError, Sp1ConfigOverrides,
    Sp1FulfillmentStrategy, Sp1NetworkMetadata, Sp1NetworkMode, Sp1NetworkSubmissionProgress,
    Sp1RemoteVerifyConfig, Sp1RequestContext, Sp1SystemConfig,
};
pub use types::{Sp1ExecutionMetadata, Sp1Response};

use alloy::{providers::ProviderBuilder, sol};
use alloy_primitives::{Address, B256, Bytes};
use raiko2_guest_common::aggregate_shasta_zk_with_verifier;
use raiko2_pipeline::{ProofStage, ProverBackend};
use raiko2_primitives::{
    AggregationGuestInput, Proof, ProofType, ProverConfig, RaikoError, RaikoResult,
};
use raiko2_primitives_shasta::{
    GuestInput, ShastaZkAggregationGuestInput, instance::sp1_contract_block_program_id,
};
use raiko2_remote_poller::{
    RemotePollError, RemotePollerConfig, RemoteStatus, RemoteStatusReason, RemoteStatusSource,
    RemoteStatusTracker, RemoteSubmission, RemoteSubmissionId, RemoteSubmissionStatus,
    RemoteTerminalResult,
};
use serde::Deserialize;
use sp1_sdk::{
    HashableKey, NetworkProver, ProveRequest as _, Prover as _, SP1Proof, SP1ProofMode,
    SP1ProofWithPublicValues, SP1ProvingKey, SP1Stdin, SP1VerifyingKey,
    blocking::{
        CpuProver as BlockingCpuProver, MockProver as BlockingMockProver, ProveRequest as _,
        Prover as BlockingProver, ProverClient as BlockingProverClient,
    },
    network::{
        FulfillmentStrategy, NetworkMode as Sp1SdkNetworkMode,
        proto::types::{ExecutionStatus, FulfillmentStatus},
        signer::NetworkSigner,
    },
};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};
use tokio::{task::JoinSet, time::timeout};
use tracing::info;
use url::Url;

use crate::{
    GuestInputCodec, ProverProgress, ProverProgressObserver, build_shasta_aggregation_input,
    ensure_shasta_proposal_input_matches_carry, parse_shasta_aggregation_input_hash,
    parse_shasta_proposal_input_hash, with_shasta_extra_data,
};

const SP1_NETWORK_WAIT_RETRY_DELAY: Duration = Duration::from_secs(15);
const SP1_NETWORK_REQUEST_RETRY_DELAY: Duration = Duration::from_secs(15);
const SP1_STATUS_POLL_CONCURRENCY: usize = 4;
const SP1_MAINNET_RPC_URL: &str = "https://rpc.mainnet.succinct.xyz";
const SP1_RESERVED_RPC_URL: &str = "https://rpc.production.succinct.xyz";

type Sp1StatusRegistry = Arc<Mutex<HashMap<RemoteSubmissionId, Sp1SubmissionState>>>;

#[derive(Clone, Debug, Eq, PartialEq)]
struct Sp1SubmissionMetadata {
    auction_timeout_at: Option<Instant>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Sp1SubmissionState {
    metadata: Sp1SubmissionMetadata,
    terminal_outcome: Option<Sp1TerminalOutcome>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Sp1TerminalOutcome {
    Unfulfillable,
    TimedOut,
    AuctionTimedOut,
    Unexecutable,
}

#[derive(Clone)]
struct Sp1StatusSource {
    client: NetworkProver,
    registry: Sp1StatusRegistry,
}

#[async_trait::async_trait]
impl RemoteStatusSource for Sp1StatusSource {
    async fn poll(
        &self,
        _proof_type: ProofType,
        submissions: Vec<RemoteSubmission>,
    ) -> Result<Vec<RemoteSubmissionStatus>, RemotePollError> {
        let submission_count = submissions.len();
        let mut statuses = Vec::with_capacity(submissions.len());
        let mut submissions = submissions.into_iter();
        let mut tasks = JoinSet::new();

        for _ in 0..SP1_STATUS_POLL_CONCURRENCY.min(submission_count) {
            if let Some(submission) = submissions.next() {
                self.spawn_status_task(&mut tasks, submission);
            }
        }

        while let Some(result) = tasks.join_next().await {
            let (submission_id, status_result) = result.map_err(|err| {
                RemotePollError::SourceUnavailable(format!("sp1 status poll task failed: {err}"))
            })?;
            match status_result {
                Ok(status) => statuses.push(status),
                Err(RemotePollError::Transient(error)) => {
                    statuses.push(sp1_transient_poll_status(submission_id, error));
                }
                Err(err) => return Err(err),
            }

            if let Some(submission) = submissions.next() {
                self.spawn_status_task(&mut tasks, submission);
            }
        }
        Ok(statuses)
    }
}

impl Sp1StatusSource {
    fn spawn_status_task(
        &self,
        tasks: &mut JoinSet<(
            RemoteSubmissionId,
            Result<RemoteSubmissionStatus, RemotePollError>,
        )>,
        submission: RemoteSubmission,
    ) {
        let source = self.clone();
        let submission_id = submission.id;
        tasks.spawn(async move {
            (
                submission_id,
                source.status_for_submission(submission).await,
            )
        });
    }

    #[allow(clippy::too_many_lines)]
    async fn status_for_submission(
        &self,
        submission: RemoteSubmission,
    ) -> Result<RemoteSubmissionStatus, RemotePollError> {
        let Some(metadata) = sp1_submission_metadata(&self.registry, submission.id)? else {
            return Ok(unrecoverable_sp1_status(
                submission.id,
                format!(
                    "sp1 status source missing metadata for request {}",
                    submission.provider_request_id
                ),
            ));
        };
        let request_id = match B256::from_str(&submission.provider_request_id) {
            Ok(request_id) => request_id,
            Err(err) => {
                return Ok(unrecoverable_sp1_status(
                    submission.id,
                    format!("invalid sp1 request id: {err}"),
                ));
            }
        };

        let (status, maybe_proof) = self
            .client
            .get_proof_status(request_id)
            .await
            .map_err(|err| RemotePollError::Transient(format!("sp1 status rpc: {err}")))?;
        let fulfillment_status = decode_fulfillment_status(status.fulfillment_status());
        let execution_status = decode_execution_status(status.execution_status());

        if fulfillment_status == FulfillmentStatus::Requested
            && metadata
                .auction_timeout_at
                .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.client
                .cancel_request(request_id)
                .await
                .map_err(|err| RemotePollError::Transient(format!("sp1 cancel request: {err}")))?;
            record_sp1_terminal_outcome(
                &self.registry,
                submission.id,
                Sp1TerminalOutcome::AuctionTimedOut,
            )?;
            return Ok(sp1_status(
                submission.id,
                RemoteStatus::Failed,
                Some(RemoteStatusReason::new(
                    "SP1 network request auction timeout elapsed",
                )),
            ));
        }

        let now = sp1_now_secs();
        let (remote_status, reason, terminal_outcome) =
            if fulfillment_status == FulfillmentStatus::Fulfilled && maybe_proof.is_some() {
                (RemoteStatus::Fulfilled, None, None)
            } else if execution_status == ExecutionStatus::Unexecutable {
                (
                    RemoteStatus::Unrecoverable,
                    Some(RemoteStatusReason::new(
                        "SP1 network request is unexecutable",
                    )),
                    Some(Sp1TerminalOutcome::Unexecutable),
                )
            } else if fulfillment_status == FulfillmentStatus::Unfulfillable {
                (
                    RemoteStatus::Failed,
                    Some(RemoteStatusReason::new(
                        "SP1 network request is unfulfillable",
                    )),
                    Some(Sp1TerminalOutcome::Unfulfillable),
                )
            } else if now > status.deadline() {
                (
                    RemoteStatus::Failed,
                    Some(RemoteStatusReason::new("SP1 network request timed out")),
                    Some(Sp1TerminalOutcome::TimedOut),
                )
            } else if fulfillment_status == FulfillmentStatus::Assigned {
                (RemoteStatus::Locked, None, None)
            } else {
                (RemoteStatus::Pending, None, None)
            };
        if let Some(outcome) = terminal_outcome {
            record_sp1_terminal_outcome(&self.registry, submission.id, outcome)?;
        }

        Ok(RemoteSubmissionStatus {
            submission_id: submission.id,
            status: remote_status,
            reason,
            observed_unix_secs: now,
            context: None,
        })
    }
}

fn decode_fulfillment_status(value: i32) -> FulfillmentStatus {
    FulfillmentStatus::try_from(value).unwrap_or(FulfillmentStatus::UnspecifiedFulfillmentStatus)
}

fn decode_execution_status(value: i32) -> ExecutionStatus {
    ExecutionStatus::try_from(value).unwrap_or(ExecutionStatus::UnspecifiedExecutionStatus)
}

#[allow(clippy::too_many_arguments)]
fn sp1_status(
    submission_id: RemoteSubmissionId,
    status: RemoteStatus,
    reason: Option<RemoteStatusReason>,
) -> RemoteSubmissionStatus {
    RemoteSubmissionStatus {
        submission_id,
        status,
        reason,
        observed_unix_secs: sp1_now_secs(),
        context: None,
    }
}

fn unrecoverable_sp1_status(
    submission_id: RemoteSubmissionId,
    reason: impl Into<String>,
) -> RemoteSubmissionStatus {
    RemoteSubmissionStatus {
        submission_id,
        status: RemoteStatus::Unrecoverable,
        reason: Some(RemoteStatusReason::new(reason)),
        observed_unix_secs: sp1_now_secs(),
        context: None,
    }
}

fn sp1_transient_poll_status(
    submission_id: RemoteSubmissionId,
    error: impl Into<String>,
) -> RemoteSubmissionStatus {
    let error = error.into();
    tracing::warn!(
        submission_id = %submission_id,
        error = %error,
        "SP1 status poll failed for one submission; keeping it active"
    );
    RemoteSubmissionStatus {
        submission_id,
        status: RemoteStatus::Pending,
        reason: None,
        observed_unix_secs: sp1_now_secs(),
        context: None,
    }
}

fn lock_sp1_registry(
    registry: &Sp1StatusRegistry,
) -> Result<
    std::sync::MutexGuard<'_, HashMap<RemoteSubmissionId, Sp1SubmissionState>>,
    RemotePollError,
> {
    registry.lock().map_err(|err| {
        RemotePollError::SourceUnavailable(format!("sp1 status registry lock poisoned: {err}"))
    })
}

fn sp1_submission_metadata(
    registry: &Sp1StatusRegistry,
    submission_id: RemoteSubmissionId,
) -> Result<Option<Sp1SubmissionMetadata>, RemotePollError> {
    Ok(lock_sp1_registry(registry)?
        .get(&submission_id)
        .map(|state| state.metadata.clone()))
}

fn record_sp1_terminal_outcome(
    registry: &Sp1StatusRegistry,
    submission_id: RemoteSubmissionId,
    outcome: Sp1TerminalOutcome,
) -> Result<(), RemotePollError> {
    if let Some(state) = lock_sp1_registry(registry)?.get_mut(&submission_id) {
        state.terminal_outcome = Some(outcome);
    }
    Ok(())
}

fn sp1_terminal_outcome(
    registry: &Sp1StatusRegistry,
    submission_id: RemoteSubmissionId,
) -> RaikoResult<Option<Sp1TerminalOutcome>> {
    registry
        .lock()
        .map_err(|err| RaikoError::Guest(format!("SP1 status registry lock poisoned: {err}")))?
        .get(&submission_id)
        .map(|state| state.terminal_outcome)
        .ok_or_else(|| {
            RaikoError::Guest(format!(
                "SP1 status registry missing submission {submission_id}"
            ))
        })
}

struct Sp1SubmissionGuard {
    tracker: RemoteStatusTracker,
    registry: Sp1StatusRegistry,
    submission_id: RemoteSubmissionId,
}

impl Sp1SubmissionGuard {
    const fn new(
        tracker: RemoteStatusTracker,
        registry: Sp1StatusRegistry,
        submission_id: RemoteSubmissionId,
    ) -> Self {
        Self {
            tracker,
            registry,
            submission_id,
        }
    }
}

impl Drop for Sp1SubmissionGuard {
    fn drop(&mut self) {
        if let Ok(mut registry) = self.registry.lock() {
            registry.remove(&self.submission_id);
        }
        self.tracker.untrack(self.submission_id);
    }
}

fn sp1_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

sol!(
    #[sol(rpc)]
    #[allow(dead_code)]
    contract ISP1Verifier {
        function verifyProof(
            bytes32 programVKey,
            bytes publicValues,
            bytes proofBytes
        ) external view;
    }
);

impl From<RecursionMode> for SP1ProofMode {
    fn from(value: RecursionMode) -> Self {
        match value {
            RecursionMode::Core => Self::Core,
            RecursionMode::Compressed => Self::Compressed,
            RecursionMode::Plonk => Self::Plonk,
        }
    }
}

impl From<Sp1FulfillmentStrategy> for FulfillmentStrategy {
    fn from(value: Sp1FulfillmentStrategy) -> Self {
        match value {
            Sp1FulfillmentStrategy::Reserved => Self::Reserved,
            Sp1FulfillmentStrategy::Hosted => Self::Hosted,
            Sp1FulfillmentStrategy::Auction => Self::Auction,
        }
    }
}

/// SP1 Prover for Shasta proposal proofs.
#[derive(Clone)]
pub struct Sp1Prover {
    config: Sp1Config,
    setup_cache: Arc<Sp1SetupCache>,
    network_status_trackers: Arc<Mutex<HashMap<Sp1StatusTrackerKey, RemoteStatusTracker>>>,
    network_status_registry: Sp1StatusRegistry,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Sp1StatusTrackerKey {
    network_mode: Sp1NetworkMode,
    rpc_url: String,
}

struct Sp1SetupCache {
    proposal: OnceLock<Arc<Sp1ProgramSetup>>,
    aggregation: OnceLock<Arc<Sp1ProgramSetup>>,
}

#[derive(Clone)]
struct Sp1ProgramSetup {
    pk: SP1ProvingKey,
    vk: SP1VerifyingKey,
}

#[must_use]
/// Returns a stable JSON representation of an SP1 verifying key.
///
/// # Panics
///
/// Panics if the verifying key cannot be serialized to JSON.
pub fn sp1_vk_uuid(vk: &SP1VerifyingKey) -> String {
    serde_json::to_string(vk).expect("SP1 verifying key should serialize")
}

#[must_use]
pub fn sp1_vk_digest(vk: &SP1VerifyingKey) -> String {
    alloy_primitives::hex::encode_prefixed(vk.hash_bytes())
}

/// Parses either an SP1 verifying key JSON string or a 32-byte hex image id.
///
/// SP1 verifying key JSON is kept in SP1's native `hash_u32` word form. Raw 32-byte
/// image ids are encoded into little-endian `u32` words so RISC0/Boundless aggregation
/// paths can round-trip them back to their original bytes.
///
/// # Errors
///
/// Returns an error when the input is neither valid SP1 verifying key JSON nor a 32-byte hex
/// payload.
pub fn sp1_image_id_words_from_uuid(raw: &str) -> Result<[u32; 8], String> {
    if let Ok(vk) = serde_json::from_str::<SP1VerifyingKey>(raw) {
        return Ok(vk.hash_u32());
    }

    let bytes =
        alloy_primitives::hex::decode(raw).map_err(|err| format!("invalid hex uuid: {err}"))?;
    if bytes.len() != 32 {
        return Err(format!(
            "expected 32 bytes or SP1 verifying key JSON, got {}",
            bytes.len()
        ));
    }

    let mut words = [0u32; 8];
    for (index, chunk) in bytes.chunks_exact(4).enumerate() {
        let mut word = [0u8; 4];
        word.copy_from_slice(chunk);
        words[index] = u32::from_le_bytes(word);
    }
    Ok(words)
}

impl Sp1Prover {
    /// Create a new SP1 prover with the given configuration.
    ///
    /// Call `preload_setup` before using this prover to prove. Server code should prefer
    /// `new_with_backend` so SP1 setup happens during startup instead of the async proving path.
    #[must_use]
    pub fn new(config: Sp1Config) -> Self {
        Self {
            config,
            setup_cache: Arc::new(Sp1SetupCache::new()),
            network_status_trackers: Arc::new(Mutex::new(HashMap::new())),
            network_status_registry: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Create a new SP1 prover and eagerly prepare proving/verifying keys for the configured ELF
    /// backend.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend cannot provide the configured proposal or aggregation ELF.
    pub fn new_with_backend<B>(config: Sp1Config, backend: &B) -> RaikoResult<Self>
    where
        B: ProverBackend,
    {
        let prover = Self::new(config);
        prover.preload_setup(backend)?;
        Ok(prover)
    }

    /// Eagerly prepare proving/verifying keys for both SP1 proof stages.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend cannot provide either ELF.
    pub fn preload_setup<B>(&self, backend: &B) -> RaikoResult<()>
    where
        B: ProverBackend,
    {
        self.preload_setup_for_stage(backend, ProofStage::Proposal)?;
        self.preload_setup_for_stage(backend, ProofStage::Aggregation)?;
        Ok(())
    }

    fn resolve_config_for_request(
        &self,
        config: &ProverConfig,
        fallback_context: Sp1RequestContext,
    ) -> RaikoResult<Sp1Config> {
        let overrides = match config.get("sp1") {
            Some(value) => Sp1ConfigOverrides::deserialize(value).map_err(|e| {
                RaikoError::InvalidRequestConfig(format!("Failed to parse 'sp1' prover args: {e}"))
            })?,
            None => Sp1ConfigOverrides::default(),
        };
        let system = match config.get("sp1_system") {
            Some(value) => Some(Sp1SystemConfig::deserialize(value).map_err(|e| {
                RaikoError::InvalidRequestConfig(format!(
                    "Failed to parse internal 'sp1_system' config: {e}"
                ))
            })?),
            None => None,
        };
        let effective = self
            .config
            .resolve_request_config(Some(&overrides), fallback_context)
            .map_err(|err| RaikoError::InvalidRequestConfig(err.to_string()))?;
        Ok(system
            .as_ref()
            .map_or(effective.clone(), |system| system.applied_to(&effective)))
    }

    fn network_status_tracker(
        &self,
        config: &Sp1Config,
        client: &NetworkProver,
    ) -> RaikoResult<RemoteStatusTracker> {
        let key = Sp1StatusTrackerKey {
            network_mode: config.network_mode,
            rpc_url: sp1_network_rpc_url(config)?,
        };
        let mut trackers = self.network_status_trackers.lock().map_err(|err| {
            RaikoError::Guest(format!(
                "SP1 network status tracker cache lock poisoned: {err}"
            ))
        })?;
        if let Some(tracker) = trackers.get(&key) {
            return Ok(tracker.clone());
        }
        let tracker = {
            let source: Arc<dyn RemoteStatusSource> = Arc::new(Sp1StatusSource {
                client: client.clone(),
                registry: Arc::clone(&self.network_status_registry),
            });
            let mut sources = HashMap::new();
            sources.insert(ProofType::Sp1, source);
            RemoteStatusTracker::spawn(
                RemotePollerConfig::new(SP1_NETWORK_WAIT_RETRY_DELAY),
                sources,
            )
        };
        trackers.insert(key, tracker.clone());
        Ok(tracker)
    }

    fn preload_setup_for_stage<B>(
        &self,
        backend: &B,
        stage: ProofStage,
    ) -> RaikoResult<Arc<Sp1ProgramSetup>>
    where
        B: ProverBackend,
    {
        let cell = self.setup_cache.cell(stage);
        if let Some(setup) = cell.get() {
            return Ok(Arc::clone(setup));
        }

        let elf = backend.elf(stage)?;
        let vk: SP1VerifyingKey = bincode::deserialize(backend.sp1_vk(stage)?).map_err(|err| {
            RaikoError::Guest(format!(
                "Failed to load SP1 {} verifying key: {err}",
                sp1_stage_name(stage)
            ))
        })?;
        let pk = SP1ProvingKey::new(vk.clone(), elf.into());
        let setup = Arc::new(Sp1ProgramSetup { pk, vk });

        match cell.set(Arc::clone(&setup)) {
            Ok(()) => {
                tracing::info!(
                    stage = sp1_stage_name(stage),
                    vkey_hash = %sp1_vk_digest(&setup.vk),
                    "Loaded SP1 program setup"
                );
                Ok(setup)
            }
            Err(setup) => Ok(cell.get().cloned().unwrap_or(setup)),
        }
    }

    fn setup_for_stage<B>(
        &self,
        backend: &B,
        stage: ProofStage,
    ) -> RaikoResult<Arc<Sp1ProgramSetup>>
    where
        B: ProverBackend,
    {
        self.preload_setup_for_stage(backend, stage)
    }
}

impl Sp1SetupCache {
    const fn new() -> Self {
        Self {
            proposal: OnceLock::new(),
            aggregation: OnceLock::new(),
        }
    }

    const fn cell(&self, stage: ProofStage) -> &OnceLock<Arc<Sp1ProgramSetup>> {
        match stage {
            ProofStage::Proposal => &self.proposal,
            ProofStage::Aggregation => &self.aggregation,
        }
    }
}

const fn sp1_stage_name(stage: ProofStage) -> &'static str {
    match stage {
        ProofStage::Proposal => "proposal",
        ProofStage::Aggregation => "aggregation",
    }
}

impl GuestInputCodec<GuestInput> for Sp1Prover {
    fn encode(&self, input: &GuestInput, _config: &ProverConfig) -> RaikoResult<Bytes> {
        let bytes = bincode::serialize(input)
            .map_err(|e| RaikoError::Guest(format!("Failed to serialize input: {e}")))?;
        Ok(Bytes::from(bytes))
    }
}

#[async_trait::async_trait]
impl<B> crate::Prover<B> for Sp1Prover
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
        config: &ProverConfig,
        backend: &B,
    ) -> RaikoResult<Proof> {
        self.prove_encoded_with_observer(input, config, backend, None)
            .await
    }

    async fn prove_encoded_with_observer(
        &self,
        input: Bytes,
        config: &ProverConfig,
        backend: &B,
        observer: Option<std::sync::Arc<dyn ProverProgressObserver>>,
    ) -> RaikoResult<Proof> {
        let effective_config = self.resolve_config_for_request(
            config,
            Sp1RequestContext::ProposalBatch { aggregate: false },
        )?;
        info!(mode = %effective_config.mode.as_str(), "Starting SP1 proposal run...");

        let guest_input: GuestInput = bincode::deserialize(input.as_ref())
            .map_err(|e| RaikoError::Guest(format!("Failed to deserialize input: {e}")))?;

        // Use the proposal ELF selected by the configured backend.
        match effective_config.mode {
            ExecutionMode::Execute => {
                if effective_config.prover == ProverMode::Network {
                    return Err(RaikoError::InvalidRequestConfig(
                        "sp1.mode=execute does not support sp1.prover=network".to_string(),
                    ));
                }
                execute_proposal_with_local_client(
                    effective_config.prover,
                    backend.elf(ProofStage::Proposal)?.to_vec(),
                    guest_input,
                )
                .await
            }
            ExecutionMode::Prove => {
                let proof_mode: SP1ProofMode = effective_config.recursion.into();
                let setup = self.setup_for_stage(backend, ProofStage::Proposal)?;
                match effective_config.prover {
                    ProverMode::Mock | ProverMode::Local => {
                        prove_proposal_with_local_client(
                            effective_config.prover,
                            setup,
                            guest_input,
                            proof_mode,
                            effective_config.verify,
                        )
                        .await
                    }
                    ProverMode::Network => {
                        let mut stdin = SP1Stdin::new();
                        stdin.write(&guest_input);
                        let client = build_network_prover(&effective_config).await?;
                        let status_tracker =
                            self.network_status_tracker(&effective_config, &client)?;
                        prove_proposal_with_network_client(
                            &client,
                            &status_tracker,
                            &self.network_status_registry,
                            setup.as_ref(),
                            stdin,
                            proof_mode,
                            &effective_config,
                            &guest_input,
                            observer,
                        )
                        .await
                    }
                }
            }
        }
    }

    async fn aggregate(
        &self,
        input: AggregationGuestInput,
        config: &ProverConfig,
        backend: &B,
    ) -> RaikoResult<Proof> {
        let effective_config =
            self.resolve_config_for_request(config, Sp1RequestContext::Aggregation)?;
        info!(
            "Starting SP1 aggregation proof generation with {} proofs...",
            input.proofs.len()
        );

        let aggregation_input = build_shasta_aggregation_input(&input.proofs)?;

        // Get the proposal prover's verifying key for proof verification.
        // The proposal proofs were generated with the proposal ELF.
        let proposal_setup = self.setup_for_stage(backend, ProofStage::Proposal)?;
        let aggregation_setup = self.setup_for_stage(backend, ProofStage::Aggregation)?;
        let proof_mode: SP1ProofMode = effective_config.recursion.into();

        match effective_config.prover {
            ProverMode::Mock | ProverMode::Local => {
                aggregate_with_local_client(
                    effective_config.prover,
                    proposal_setup,
                    aggregation_setup,
                    input,
                    aggregation_input,
                    proof_mode,
                    effective_config.verify,
                )
                .await
            }
            ProverMode::Network => {
                // The guest reads ShastaZkAggregationGuestInput via sp1_zkvm::io::read().
                let mut stdin = SP1Stdin::new();
                stdin.write(&aggregation_input);
                let expected_input_hash = expected_sp1_aggregation_input_hash(&aggregation_input)?;
                let client = build_network_prover(&effective_config).await?;
                let status_tracker = self.network_status_tracker(&effective_config, &client)?;
                aggregate_with_network_client(
                    &client,
                    &status_tracker,
                    &self.network_status_registry,
                    proposal_setup.as_ref(),
                    aggregation_setup.as_ref(),
                    &input,
                    stdin,
                    &effective_config,
                    expected_input_hash,
                    None,
                )
                .await
            }
        }
    }

    async fn aggregate_with_observer(
        &self,
        input: AggregationGuestInput,
        config: &ProverConfig,
        backend: &B,
        observer: Option<std::sync::Arc<dyn ProverProgressObserver>>,
    ) -> RaikoResult<Proof> {
        let effective_config =
            self.resolve_config_for_request(config, Sp1RequestContext::Aggregation)?;
        info!(
            "Starting SP1 aggregation proof generation with {} proofs...",
            input.proofs.len()
        );

        let aggregation_input = build_shasta_aggregation_input(&input.proofs)?;
        let proposal_setup = self.setup_for_stage(backend, ProofStage::Proposal)?;
        let aggregation_setup = self.setup_for_stage(backend, ProofStage::Aggregation)?;
        let proof_mode: SP1ProofMode = effective_config.recursion.into();

        match effective_config.prover {
            ProverMode::Mock | ProverMode::Local => {
                aggregate_with_local_client(
                    effective_config.prover,
                    proposal_setup,
                    aggregation_setup,
                    input,
                    aggregation_input,
                    proof_mode,
                    effective_config.verify,
                )
                .await
            }
            ProverMode::Network => {
                let mut stdin = SP1Stdin::new();
                stdin.write(&aggregation_input);
                let expected_input_hash = expected_sp1_aggregation_input_hash(&aggregation_input)?;
                let client = build_network_prover(&effective_config).await?;
                let status_tracker = self.network_status_tracker(&effective_config, &client)?;
                aggregate_with_network_client(
                    &client,
                    &status_tracker,
                    &self.network_status_registry,
                    proposal_setup.as_ref(),
                    aggregation_setup.as_ref(),
                    &input,
                    stdin,
                    &effective_config,
                    expected_input_hash,
                    observer,
                )
                .await
            }
        }
    }
}

#[derive(Clone)]
struct NetworkProofRequestResult {
    request_id: String,
    proof: SP1ProofWithPublicValues,
}

fn remote_verifier_program_vkey(vk: &SP1VerifyingKey) -> B256 {
    B256::from_slice(&vk.bytes32_raw())
}

const fn sp1_proof_mode_name(proof: &SP1ProofWithPublicValues) -> &'static str {
    match &proof.proof {
        SP1Proof::Core(_) => "core",
        SP1Proof::Compressed(_) => "compressed",
        SP1Proof::Plonk(_) => "plonk",
        SP1Proof::Groth16(_) => "groth16",
    }
}

fn encode_sp1_onchain_payload(segments: &[String], proof_bytes: &[u8]) -> String {
    let mut payload = String::from("0x");
    for segment in segments {
        payload.push_str(segment.strip_prefix("0x").unwrap_or(segment));
    }
    payload.push_str(&alloy_primitives::hex::encode(proof_bytes));
    payload
}

#[must_use]
pub fn encode_sp1_proposal_proof_payload(
    proof: &SP1ProofWithPublicValues,
    vk: &SP1VerifyingKey,
) -> Option<String> {
    let proof_bytes = remote_verifier_proof_bytes(proof)?;
    Some(encode_sp1_onchain_payload(&[vk.bytes32()], &proof_bytes))
}

#[must_use]
pub fn encode_sp1_aggregation_proof_payload(
    proof: &SP1ProofWithPublicValues,
    aggregation_vk: &SP1VerifyingKey,
    block_vk: &SP1VerifyingKey,
) -> Option<String> {
    let proof_bytes = remote_verifier_proof_bytes(proof)?;
    Some(encode_sp1_onchain_payload(
        &[
            aggregation_vk.bytes32(),
            alloy_primitives::hex::encode_prefixed(block_vk.hash_bytes()),
        ],
        &proof_bytes,
    ))
}

/// # Errors
///
/// Returns an error when the proof does not contain a valid SP1 quote or legacy encoded proof
/// payload that can be deserialized for aggregation.
pub fn load_sp1_subproof_for_aggregation(proof: &Proof) -> RaikoResult<SP1Proof> {
    if let Some(quote) = proof.quote.as_deref() {
        return serde_json::from_str::<SP1Proof>(quote)
            .map_err(|e| RaikoError::Guest(format!("Failed to deserialize SP1 proof quote: {e}")));
    }

    if let Some(proof_hex) = proof.proof.as_deref() {
        let proof_bytes = alloy_primitives::hex::decode(proof_hex)
            .map_err(|e| RaikoError::Guest(format!("Failed to decode proof hex: {e}")))?;
        let legacy: SP1ProofWithPublicValues = bincode::deserialize(&proof_bytes).map_err(|e| {
            RaikoError::Guest(format!(
                "Failed to deserialize legacy SP1ProofWithPublicValues: {e}"
            ))
        })?;
        return Ok(legacy.proof);
    }

    Err(RaikoError::Guest(
        "Aggregation requires SP1 quote or legacy proof bytes".to_string(),
    ))
}

// Old `raiko` only ran remote contract verification when the proof could be encoded into
// onchain verifier bytes. Shasta proposal proofs used for aggregation are `Compressed`, so they
// must skip this path.
fn remote_verifier_proof_bytes(proof: &SP1ProofWithPublicValues) -> Option<Vec<u8>> {
    match &proof.proof {
        SP1Proof::Plonk(_) | SP1Proof::Groth16(_) => {
            let proof_bytes = proof.bytes();
            (!proof_bytes.is_empty()).then_some(proof_bytes)
        }
        SP1Proof::Core(_) | SP1Proof::Compressed(_) => None,
    }
}

async fn verify_network_proposal_proof(
    config: &Sp1Config,
    proof: &SP1ProofWithPublicValues,
    vk: &SP1VerifyingKey,
) -> RaikoResult<()> {
    let Some(proof_bytes) = remote_verifier_proof_bytes(proof) else {
        tracing::info!(
            proof_mode = sp1_proof_mode_name(proof),
            "Skipping SP1 proposal remote verification for non-onchain proof mode"
        );
        return Ok(());
    };
    let input_hash = parse_shasta_proposal_input_hash(proof.public_values.as_slice())?;
    verify_sp1_remote_contract(
        config,
        vk,
        input_hash.as_slice().to_vec(),
        proof_bytes,
        "proposal",
    )
    .await
}

async fn verify_network_aggregation_proof(
    config: &Sp1Config,
    proof: &SP1ProofWithPublicValues,
    vk: &SP1VerifyingKey,
) -> RaikoResult<()> {
    let Some(proof_bytes) = remote_verifier_proof_bytes(proof) else {
        tracing::info!(
            proof_mode = sp1_proof_mode_name(proof),
            "Skipping SP1 aggregation remote verification for non-onchain proof mode"
        );
        return Ok(());
    };
    verify_sp1_remote_contract(
        config,
        vk,
        proof.public_values.as_slice().to_vec(),
        proof_bytes,
        "aggregation",
    )
    .await
}

async fn verify_sp1_remote_contract(
    config: &Sp1Config,
    vk: &SP1VerifyingKey,
    public_values: Vec<u8>,
    proof_bytes: Vec<u8>,
    stage: &str,
) -> RaikoResult<()> {
    let remote_verify = config.remote_verify.as_ref().ok_or_else(|| {
        RaikoError::InvalidRequestConfig(
            "sp1.prover=network with sp1.verify=true requires internal remote verifier configuration"
                .to_string(),
        )
    })?;
    let rpc_url = Url::parse(&remote_verify.rpc_url).map_err(|e| {
        RaikoError::InvalidRequestConfig(format!(
            "invalid SP1 remote verifier rpc_url '{}': {e}",
            remote_verify.rpc_url
        ))
    })?;
    let verifier_address = Address::from_str(&remote_verify.verifier_address).map_err(|e| {
        RaikoError::InvalidRequestConfig(format!(
            "invalid SP1 remote verifier address '{}': {e}",
            remote_verify.verifier_address
        ))
    })?;
    let provider = ProviderBuilder::new().connect_http(rpc_url);
    let verifier = ISP1Verifier::new(verifier_address, provider);
    let verify_timeout = Duration::from_secs(config.timeout_secs);
    let verify_started = std::time::Instant::now();

    tracing::info!(
        stage,
        rpc_url = %remote_verify.rpc_url,
        verifier_address = %remote_verify.verifier_address,
        timeout_secs = config.timeout_secs,
        "Starting SP1 remote verification"
    );

    timeout(
        verify_timeout,
        verifier
            .verifyProof(
                remote_verifier_program_vkey(vk),
                Bytes::from(public_values),
                Bytes::from(proof_bytes),
            )
            .call(),
    )
    .await
    .map_err(|_| {
        tracing::error!(
            stage,
            elapsed_ms = verify_started.elapsed().as_millis(),
            "Timed out while remotely verifying SP1 proof"
        );
        RaikoError::Guest(format!(
            "SP1 {stage} remote verification timed out after {}s",
            config.timeout_secs
        ))
    })?
    .map_err(|e| {
        tracing::error!("Failed to remotely verify SP1 {stage} proof: {:?}", e);
        RaikoError::Guest(format!("SP1 {stage} remote verification failed: {e}"))
    })?;

    tracing::info!(
        stage,
        elapsed_ms = verify_started.elapsed().as_millis(),
        "SP1 remote verification succeeded"
    );

    Ok(())
}

fn blocking_sp1_join_error(operation: &str, err: &tokio::task::JoinError) -> RaikoError {
    RaikoError::Guest(format!("SP1 {operation} task join failed: {err}"))
}

enum LocalSp1Client {
    Mock(BlockingMockProver),
    Cpu(BlockingCpuProver),
}

impl LocalSp1Client {
    fn new(prover_mode: ProverMode) -> Self {
        match prover_mode {
            ProverMode::Mock => Self::Mock(BlockingProverClient::builder().mock().build()),
            ProverMode::Local => Self::Cpu(BlockingProverClient::builder().cpu().build()),
            ProverMode::Network => unreachable!("network prover is handled by the network path"),
        }
    }

    fn execute(
        &self,
        elf: Vec<u8>,
        stdin: SP1Stdin,
    ) -> Result<(sp1_sdk::SP1PublicValues, sp1_sdk::ExecutionReport), String> {
        match self {
            Self::Mock(client) => client
                .execute(elf.into(), stdin)
                .run()
                .map_err(|err| err.to_string()),
            Self::Cpu(client) => client
                .execute(elf.into(), stdin)
                .run()
                .map_err(|err| err.to_string()),
        }
    }

    fn prove(
        &self,
        pk: &SP1ProvingKey,
        stdin: SP1Stdin,
        proof_mode: SP1ProofMode,
    ) -> Result<SP1ProofWithPublicValues, String> {
        match self {
            Self::Mock(client) => client
                .prove(pk, stdin)
                .mode(proof_mode)
                .run()
                .map_err(|err| err.to_string()),
            Self::Cpu(client) => client
                .prove(pk, stdin)
                .mode(proof_mode)
                .run()
                .map_err(|err| err.to_string()),
        }
    }

    fn verify(&self, proof: &SP1ProofWithPublicValues, vk: &SP1VerifyingKey) -> Result<(), String> {
        match self {
            Self::Mock(client) => client
                .verify(proof, vk, None)
                .map_err(|err| err.to_string()),
            Self::Cpu(client) => client
                .verify(proof, vk, None)
                .map_err(|err| err.to_string()),
        }
    }
}

async fn execute_proposal_with_local_client(
    prover_mode: ProverMode,
    elf: Vec<u8>,
    guest_input: GuestInput,
) -> RaikoResult<Proof> {
    tokio::task::spawn_blocking(move || {
        let mut stdin = SP1Stdin::new();
        stdin.write(&guest_input);

        let client = LocalSp1Client::new(prover_mode);
        let (public_values, execution_report) = client.execute(elf, stdin).map_err(|e| {
            tracing::error!("Failed to execute SP1 proposal: {:?}", e);
            RaikoError::Guest(format!("SP1 proposal execute failed: {e}"))
        })?;
        let public_values_raw = public_values.raw();
        let input_hash = parse_shasta_proposal_input_hash(public_values.as_slice())?;
        ensure_shasta_proposal_input_matches_carry(
            input_hash,
            &guest_input.proof_carry_data,
            "sp1",
        )?;
        let metadata = serde_json::to_value(Sp1ExecutionMetadata::from_execution_report(
            public_values_raw,
            &execution_report,
        ))
        .map_err(|e| {
            RaikoError::Guest(format!("Failed to serialize SP1 execution metadata: {e}"))
        })?;

        Ok(Sp1Response {
            proof: None,
            vkey_hash: None,
            input: input_hash,
            sp1_proof: None,
            vkey: None,
            extra_data: with_shasta_extra_data(
                &guest_input.proof_carry_data,
                "sp1",
                Some(metadata),
            )?,
        }
        .into())
    })
    .await
    .map_err(|err| blocking_sp1_join_error("proposal execute", &err))?
}

async fn prove_proposal_with_local_client(
    prover_mode: ProverMode,
    setup: Arc<Sp1ProgramSetup>,
    guest_input: GuestInput,
    proof_mode: SP1ProofMode,
    verify: bool,
) -> RaikoResult<Proof> {
    tokio::task::spawn_blocking(move || {
        let mut stdin = SP1Stdin::new();
        stdin.write(&guest_input);

        let client = LocalSp1Client::new(prover_mode);
        prove_proposal_with_client(
            &client,
            setup.as_ref(),
            stdin,
            proof_mode,
            verify,
            &guest_input,
        )
    })
    .await
    .map_err(|err| blocking_sp1_join_error("proposal proof", &err))?
}

fn prove_proposal_with_client(
    client: &LocalSp1Client,
    setup: &Sp1ProgramSetup,
    stdin: SP1Stdin,
    proof_mode: SP1ProofMode,
    verify: bool,
    guest_input: &GuestInput,
) -> RaikoResult<Proof> {
    let proof = client.prove(&setup.pk, stdin, proof_mode).map_err(|e| {
        tracing::error!("Failed to generate SP1 proposal proof: {:?}", e);
        RaikoError::Guest(format!("SP1 proposal proof generation failed: {e}"))
    })?;

    if verify {
        client.verify(&proof, &setup.vk).map_err(|e| {
            tracing::error!("Failed to verify SP1 proposal proof: {:?}", e);
            RaikoError::Guest(format!("SP1 proposal proof verification failed: {e}"))
        })?;
    }

    let public_values = proof.public_values.as_slice();
    let input_hash = parse_shasta_proposal_input_hash(public_values)?;
    ensure_shasta_proposal_input_matches_carry(input_hash, &guest_input.proof_carry_data, "sp1")?;

    Ok(Sp1Response {
        proof: encode_sp1_proposal_proof_payload(&proof, &setup.vk),
        vkey_hash: Some(sp1_vk_digest(&setup.vk)),
        input: input_hash,
        sp1_proof: Some(proof),
        vkey: Some(setup.vk.clone()),
        extra_data: with_shasta_extra_data(&guest_input.proof_carry_data, "sp1", None)?,
    }
    .into())
}

#[allow(clippy::too_many_arguments)]
async fn prove_proposal_with_network_client(
    client: &NetworkProver,
    status_tracker: &RemoteStatusTracker,
    status_registry: &Sp1StatusRegistry,
    setup: &Sp1ProgramSetup,
    stdin: SP1Stdin,
    proof_mode: SP1ProofMode,
    config: &Sp1Config,
    guest_input: &GuestInput,
    observer: Option<std::sync::Arc<dyn ProverProgressObserver>>,
) -> RaikoResult<Proof> {
    let request = request_network_proof(
        client,
        status_tracker,
        status_registry,
        &setup.pk,
        stdin,
        proof_mode,
        config,
        observer,
        "proposal",
    )
    .await?;

    if config.verify {
        verify_network_proposal_proof(config, &request.proof, &setup.vk).await?;
    }

    let public_values = request.proof.public_values.as_slice();
    let input_hash = parse_shasta_proposal_input_hash(public_values)?;
    ensure_shasta_proposal_input_matches_carry(input_hash, &guest_input.proof_carry_data, "sp1")?;
    let base_extra_data = with_shasta_extra_data(&guest_input.proof_carry_data, "sp1", None)?;
    let network_metadata =
        serde_json::to_value(Sp1NetworkMetadata::from_config(request.request_id, config))
            .map_err(|e| RaikoError::Guest(format!("Failed to serialize SP1 metadata: {e}")))?;

    Ok(Sp1Response {
        proof: encode_sp1_proposal_proof_payload(&request.proof, &setup.vk),
        vkey_hash: Some(sp1_vk_digest(&setup.vk)),
        input: input_hash,
        sp1_proof: Some(request.proof),
        vkey: Some(setup.vk.clone()),
        extra_data: insert_sp1_metadata(
            base_extra_data,
            serde_json::json!({ "network": network_metadata }),
        )?,
    }
    .into())
}

async fn aggregate_with_local_client(
    prover_mode: ProverMode,
    proposal_setup: Arc<Sp1ProgramSetup>,
    aggregation_setup: Arc<Sp1ProgramSetup>,
    input: AggregationGuestInput,
    aggregation_input: ShastaZkAggregationGuestInput,
    proof_mode: SP1ProofMode,
    verify: bool,
) -> RaikoResult<Proof> {
    tokio::task::spawn_blocking(move || {
        let mut stdin = SP1Stdin::new();
        stdin.write(&aggregation_input);

        let client = LocalSp1Client::new(prover_mode);
        aggregate_with_client(
            &client,
            proposal_setup.as_ref(),
            aggregation_setup.as_ref(),
            &input,
            stdin,
            proof_mode,
            verify,
        )
    })
    .await
    .map_err(|err| blocking_sp1_join_error("aggregation proof", &err))?
}

fn aggregate_with_client(
    client: &LocalSp1Client,
    proposal_setup: &Sp1ProgramSetup,
    aggregation_setup: &Sp1ProgramSetup,
    input: &AggregationGuestInput,
    mut stdin: SP1Stdin,
    proof_mode: SP1ProofMode,
    verify: bool,
) -> RaikoResult<Proof> {
    for proof in &input.proofs {
        match load_sp1_subproof_for_aggregation(proof)? {
            SP1Proof::Compressed(reduce_proof) => {
                stdin.write_proof(*reduce_proof, proposal_setup.vk.vk.clone());
            }
            _ => {
                return Err(RaikoError::Guest(
                    "Aggregation requires Compressed proofs".to_string(),
                ));
            }
        }
    }

    let proof = client
        .prove(&aggregation_setup.pk, stdin, proof_mode)
        .map_err(|e| {
            tracing::error!("Failed to generate SP1 aggregation proof: {:?}", e);
            RaikoError::Guest(format!("SP1 aggregation proof generation failed: {e}"))
        })?;

    if verify {
        client.verify(&proof, &aggregation_setup.vk).map_err(|e| {
            tracing::error!("Failed to verify SP1 aggregation proof: {:?}", e);
            RaikoError::Guest(format!("SP1 aggregation proof verification failed: {e}"))
        })?;
    }

    let public_values = proof.public_values.as_slice();
    let agg_input_hash = parse_shasta_aggregation_input_hash(public_values)?;

    Ok(Sp1Response {
        proof: encode_sp1_aggregation_proof_payload(
            &proof,
            &aggregation_setup.vk,
            &proposal_setup.vk,
        ),
        vkey_hash: Some(sp1_vk_digest(&aggregation_setup.vk)),
        input: agg_input_hash,
        sp1_proof: Some(proof),
        vkey: Some(aggregation_setup.vk.clone()),
        extra_data: None,
    }
    .into())
}

fn ensure_sp1_network_aggregation_input_hash_matches(
    expected: B256,
    actual: B256,
) -> RaikoResult<()> {
    if expected != actual {
        return Err(RaikoError::Guest(format!(
            "SP1 network aggregation public values mismatch: expected {expected}, got {actual}"
        )));
    }
    Ok(())
}

fn expected_sp1_aggregation_input_hash(
    aggregation_input: &ShastaZkAggregationGuestInput,
) -> RaikoResult<B256> {
    aggregate_shasta_zk_with_verifier(
        aggregation_input,
        sp1_contract_block_program_id(&aggregation_input.image_id),
        |_, _| Ok(()),
    )
    .map_err(|e| {
        RaikoError::Guest(format!(
            "failed to compute expected SP1 aggregation input hash: {e}"
        ))
    })
}

#[allow(clippy::too_many_arguments)]
async fn aggregate_with_network_client(
    client: &NetworkProver,
    status_tracker: &RemoteStatusTracker,
    status_registry: &Sp1StatusRegistry,
    proposal_setup: &Sp1ProgramSetup,
    aggregation_setup: &Sp1ProgramSetup,
    input: &AggregationGuestInput,
    mut stdin: SP1Stdin,
    config: &Sp1Config,
    expected_input_hash: B256,
    observer: Option<std::sync::Arc<dyn ProverProgressObserver>>,
) -> RaikoResult<Proof> {
    for proof in &input.proofs {
        match load_sp1_subproof_for_aggregation(proof)? {
            SP1Proof::Compressed(reduce_proof) => {
                stdin.write_proof(*reduce_proof, proposal_setup.vk.vk.clone());
            }
            _ => {
                return Err(RaikoError::Guest(
                    "Aggregation requires Compressed proofs".to_string(),
                ));
            }
        }
    }

    let proof_mode: SP1ProofMode = config.recursion.into();
    let request = request_network_proof(
        client,
        status_tracker,
        status_registry,
        &aggregation_setup.pk,
        stdin,
        proof_mode,
        config,
        observer,
        "aggregation",
    )
    .await?;

    if config.verify {
        verify_network_aggregation_proof(config, &request.proof, &aggregation_setup.vk).await?;
    }

    let public_values = request.proof.public_values.as_slice();
    let agg_input_hash = parse_shasta_aggregation_input_hash(public_values)?;
    ensure_sp1_network_aggregation_input_hash_matches(expected_input_hash, agg_input_hash)?;
    let network_metadata =
        serde_json::to_value(Sp1NetworkMetadata::from_config(request.request_id, config))
            .map_err(|e| RaikoError::Guest(format!("Failed to serialize SP1 metadata: {e}")))?;

    Ok(Sp1Response {
        proof: encode_sp1_aggregation_proof_payload(
            &request.proof,
            &aggregation_setup.vk,
            &proposal_setup.vk,
        ),
        vkey_hash: Some(sp1_vk_digest(&aggregation_setup.vk)),
        input: agg_input_hash,
        sp1_proof: Some(request.proof),
        vkey: Some(aggregation_setup.vk.clone()),
        extra_data: insert_sp1_metadata(None, serde_json::json!({ "network": network_metadata }))?,
    }
    .into())
}

async fn build_network_prover(config: &Sp1Config) -> RaikoResult<NetworkProver> {
    let private_key = std::env::var("NETWORK_PRIVATE_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            RaikoError::InvalidRequestConfig(
                "NETWORK_PRIVATE_KEY must be set for sp1.prover=network".to_string(),
            )
        })?;
    let signer = NetworkSigner::local(&private_key).map_err(|err| {
        RaikoError::InvalidRequestConfig(format!(
            "NETWORK_PRIVATE_KEY is not a valid SP1 network signer: {err}"
        ))
    })?;
    let rpc_url = sp1_network_rpc_url(config)?;
    Ok(NetworkProver::new(signer, &rpc_url, sp1_sdk_network_mode(config.network_mode)).await)
}

fn sp1_network_rpc_url(config: &Sp1Config) -> RaikoResult<String> {
    let env_rpc_url = std::env::var("NETWORK_RPC_URL").ok();
    resolve_sp1_network_rpc_url(config, env_rpc_url.as_deref())
}

fn resolve_sp1_network_rpc_url(
    config: &Sp1Config,
    env_rpc_url: Option<&str>,
) -> RaikoResult<String> {
    let rpc_url = config.rpc_url.clone().unwrap_or_else(|| {
        env_rpc_url
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| default_sp1_network_rpc_url(config.network_mode))
            .to_string()
    });
    if rpc_url != rpc_url.trim() {
        return Err(RaikoError::InvalidRequestConfig(
            "SP1 network RPC URL must not include leading or trailing whitespace".to_string(),
        ));
    }
    Url::parse(&rpc_url).map_err(|err| {
        RaikoError::InvalidRequestConfig(format!("SP1 network RPC URL is invalid: {err}"))
    })?;
    Ok(rpc_url)
}

const fn default_sp1_network_rpc_url(mode: Sp1NetworkMode) -> &'static str {
    match mode {
        Sp1NetworkMode::Mainnet => SP1_MAINNET_RPC_URL,
        Sp1NetworkMode::Reserved => SP1_RESERVED_RPC_URL,
    }
}

const fn sp1_sdk_network_mode(mode: Sp1NetworkMode) -> Sp1SdkNetworkMode {
    match mode {
        Sp1NetworkMode::Mainnet => Sp1SdkNetworkMode::Mainnet,
        Sp1NetworkMode::Reserved => Sp1SdkNetworkMode::Reserved,
    }
}

const fn sp1_network_submission_progress(
    provider_request_id: String,
    config: &Sp1Config,
) -> Sp1NetworkSubmissionProgress {
    Sp1NetworkSubmissionProgress {
        provider_request_id,
        network_mode: config.network_mode,
        fulfillment_strategy: config.fulfillment_strategy,
        skip_simulation: config.skip_simulation,
        cycle_limit: config.cycle_limit,
        timeout_secs: config.timeout_secs,
        max_price_per_pgu: config.max_price_per_pgu,
        auction_timeout_secs: config.auction_timeout_secs,
    }
}

async fn notify_sp1_network_submission(
    observer: Option<&Arc<dyn ProverProgressObserver>>,
    provider_request_id: String,
    config: &Sp1Config,
) {
    if let Some(observer) = observer {
        observer
            .on_progress(&ProverProgress::Sp1NetworkSubmission(
                sp1_network_submission_progress(provider_request_id, config),
            ))
            .await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn request_network_proof(
    client: &NetworkProver,
    status_tracker: &RemoteStatusTracker,
    status_registry: &Sp1StatusRegistry,
    pk: &SP1ProvingKey,
    stdin: SP1Stdin,
    proof_mode: SP1ProofMode,
    config: &Sp1Config,
    observer: Option<std::sync::Arc<dyn ProverProgressObserver>>,
    stage: &str,
) -> RaikoResult<NetworkProofRequestResult> {
    let timeout = Duration::from_secs(config.timeout_secs);
    let auction_timeout = (config.network_mode == Sp1NetworkMode::Mainnet)
        .then(|| config.auction_timeout_secs.map(Duration::from_secs))
        .flatten();
    let mut stored_request_id = if let Some(observer) = observer.as_ref() {
        observer.load_sp1_network_request_id().await
    } else {
        None
    };
    let mut request_attempt = 1_u64;

    loop {
        let request_id_string = if let Some(request_id_string) = stored_request_id.take() {
            notify_sp1_network_submission(observer.as_ref(), request_id_string.clone(), config)
                .await;

            tracing::info!(
                stage,
                request_id = %request_id_string,
                timeout_secs = config.timeout_secs,
                "Resuming SP1 network proof wait"
            );
            request_id_string
        } else {
            tracing::info!(
                stage,
                proof_mode = ?proof_mode,
                network_mode = %config.network_mode.as_str(),
                fulfillment_strategy = %config.fulfillment_strategy.as_str(),
                skip_simulation = config.skip_simulation,
                cycle_limit = config.cycle_limit,
                timeout_secs = config.timeout_secs,
                max_price_per_pgu = ?config.max_price_per_pgu,
                auction_timeout_secs = ?config.auction_timeout_secs,
                request_attempt,
                "Submitting SP1 network proof request"
            );

            let mut request = client
                .prove(pk, stdin.clone())
                .mode(proof_mode)
                .strategy(config.fulfillment_strategy.into())
                .skip_simulation(config.skip_simulation)
                .cycle_limit(config.cycle_limit)
                .timeout(timeout);
            if let Some(max_price_per_pgu) = config.max_price_per_pgu {
                request = request.max_price_per_pgu(max_price_per_pgu);
            }

            let request_id = request.request().await.map_err(|e| {
                tracing::error!("Failed to request SP1 {stage} proof: {:?}", e);
                RaikoError::Guest(format!("SP1 {stage} proof request failed: {e}"))
            })?;

            let request_id_string = request_id.to_string();
            notify_sp1_network_submission(observer.as_ref(), request_id_string.clone(), config)
                .await;

            tracing::info!(
                stage,
                request_id = %request_id_string,
                timeout_secs = config.timeout_secs,
                request_attempt,
                "Waiting for SP1 network proof"
            );
            request_id_string
        };

        let request_id = B256::from_str(&request_id_string).map_err(|e| {
            RaikoError::Guest(format!("Invalid stored SP1 {stage} request id: {e}"))
        })?;
        match wait_sp1_network_proof(
            client,
            status_tracker,
            status_registry,
            request_id,
            timeout,
            auction_timeout,
            stage,
            &request_id_string,
        )
        .await?
        {
            Sp1NetworkWaitOutcome::Fulfilled(proof) => {
                return Ok(NetworkProofRequestResult {
                    request_id: request_id_string,
                    proof: *proof,
                });
            }
            Sp1NetworkWaitOutcome::RetryRequest(reason) => {
                tracing::warn!(
                    stage,
                    request_id = %request_id_string,
                    request_attempt,
                    reason,
                    "SP1 network proof request reached a retryable terminal state; submitting a new request"
                );
                request_attempt = request_attempt.saturating_add(1);
                tokio::time::sleep(SP1_NETWORK_REQUEST_RETRY_DELAY).await;
            }
        }
    }
}

enum Sp1NetworkWaitOutcome {
    Fulfilled(Box<SP1ProofWithPublicValues>),
    RetryRequest(String),
}

#[allow(clippy::too_many_arguments)]
async fn wait_sp1_network_proof(
    client: &NetworkProver,
    status_tracker: &RemoteStatusTracker,
    status_registry: &Sp1StatusRegistry,
    request_id: B256,
    timeout: Duration,
    auction_timeout: Option<Duration>,
    stage: &str,
    request_id_string: &str,
) -> RaikoResult<Sp1NetworkWaitOutcome> {
    let wait_started = Instant::now();
    let submission_id = RemoteSubmissionId::new();
    status_registry
        .lock()
        .map_err(|err| RaikoError::Guest(format!("SP1 status registry lock poisoned: {err}")))?
        .insert(
            submission_id,
            Sp1SubmissionState {
                metadata: Sp1SubmissionMetadata {
                    auction_timeout_at: auction_timeout.map(|timeout| Instant::now() + timeout),
                },
                terminal_outcome: None,
            },
        );
    let _guard = Sp1SubmissionGuard::new(
        status_tracker.clone(),
        Arc::clone(status_registry),
        submission_id,
    );
    let (terminal_tx, terminal_rx) = tokio::sync::oneshot::channel();
    let remote_submission = RemoteSubmission {
        id: submission_id,
        proof_type: ProofType::Sp1,
        provider_request_id: request_id_string.to_string(),
        timeout_at: Some(Instant::now() + timeout),
    };
    status_tracker
        .register(remote_submission, terminal_tx)
        .map_err(|err| {
            RaikoError::Guest(format!("Failed to register SP1 network status poll: {err}"))
        })?;

    let terminal = terminal_rx.await.map_err(|err| {
        RaikoError::Guest(format!(
            "SP1 network status poller stopped before terminal status: {err}"
        ))
    })?;

    match terminal {
        RemoteTerminalResult::Fulfilled { .. } => {
            let proof = fetch_sp1_network_proof_until(
                client,
                request_id,
                stage,
                request_id_string,
                wait_started + timeout,
            )
            .await?;
            tracing::info!(
                stage,
                request_id = %request_id_string,
                proof_mode = sp1_proof_mode_name(&proof),
                elapsed_ms = wait_started.elapsed().as_millis(),
                "SP1 network proof received"
            );
            Ok(Sp1NetworkWaitOutcome::Fulfilled(Box::new(proof)))
        }
        RemoteTerminalResult::Failed { reason, .. } => {
            match sp1_terminal_outcome(status_registry, submission_id)? {
                Some(
                    Sp1TerminalOutcome::Unfulfillable
                    | Sp1TerminalOutcome::TimedOut
                    | Sp1TerminalOutcome::AuctionTimedOut,
                ) => Ok(Sp1NetworkWaitOutcome::RetryRequest(reason.message)),
                Some(Sp1TerminalOutcome::Unexecutable) | None => Err(RaikoError::Guest(format!(
                    "SP1 {stage} network proof failed: {}",
                    reason.message
                ))),
            }
        }
        RemoteTerminalResult::TimedOut { reason, .. }
        | RemoteTerminalResult::Expired { reason, .. } => {
            Ok(Sp1NetworkWaitOutcome::RetryRequest(reason.message))
        }
        RemoteTerminalResult::Unrecoverable { reason, .. } => Err(RaikoError::Guest(format!(
            "SP1 {stage} network proof failed: {}",
            reason.message
        ))),
    }
}

async fn fetch_sp1_network_proof_until(
    client: &NetworkProver,
    request_id: B256,
    stage: &str,
    request_id_string: &str,
    deadline: Instant,
) -> RaikoResult<SP1ProofWithPublicValues> {
    loop {
        match fetch_sp1_network_proof(client, request_id, stage, request_id_string).await {
            Ok(proof) => return Ok(proof),
            Err(err) if Instant::now() < deadline => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                let delay = SP1_NETWORK_WAIT_RETRY_DELAY.min(remaining);
                tracing::warn!(
                    stage,
                    request_id = %request_id_string,
                    error = %err,
                    delay_ms = delay.as_millis(),
                    "SP1 fulfilled proof fetch failed transiently; retrying"
                );
                tokio::time::sleep(delay).await;
            }
            Err(err) => return Err(err),
        }
    }
}

async fn fetch_sp1_network_proof(
    client: &NetworkProver,
    request_id: B256,
    stage: &str,
    request_id_string: &str,
) -> RaikoResult<SP1ProofWithPublicValues> {
    let (_status, maybe_proof) = client.get_proof_status(request_id).await.map_err(|err| {
        RaikoError::Guest(format!(
            "Failed to fetch fulfilled SP1 {stage} proof {request_id_string}: {err}"
        ))
    })?;
    maybe_proof.ok_or_else(|| {
        RaikoError::Guest(format!(
            "SP1 {stage} proof {request_id_string} is fulfilled but proof payload is missing"
        ))
    })
}

fn insert_sp1_metadata(
    extra_data: Option<serde_json::Value>,
    metadata: serde_json::Value,
) -> RaikoResult<Option<serde_json::Value>> {
    let mut extra_data = extra_data.unwrap_or_else(|| serde_json::json!({}));
    let Some(root) = extra_data.as_object_mut() else {
        return Err(RaikoError::Guest(
            "SP1 extra_data root must be a JSON object".to_string(),
        ));
    };
    root.insert("sp1".to_string(), metadata);
    Ok(Some(extra_data))
}

#[cfg(test)]
mod tests {
    use super::{
        decode_execution_status, decode_fulfillment_status, default_sp1_network_rpc_url,
        encode_sp1_onchain_payload, load_sp1_subproof_for_aggregation,
        remote_verifier_program_vkey, remote_verifier_proof_bytes, resolve_sp1_network_rpc_url,
        sp1_sdk_network_mode, sp1_transient_poll_status, sp1_vk_uuid,
    };
    use alloy_primitives::B256;
    use raiko2_guests::{Sp1ShastaGuestElves, load_sp1_shasta_guest_elves};
    use raiko2_pipeline::ProofStage;
    use raiko2_pipeline::forks::shasta::sp1_shasta_backend_from_elves;
    use raiko2_primitives::Proof;
    use raiko2_primitives_shasta::instance::{sp1_contract_block_program_id, words_to_bytes_le};
    use raiko2_remote_poller::{RemoteStatus, RemoteSubmissionId};
    use sp1_sdk::{
        HashableKey, ProvingKey as _, SP1ProofMode, SP1ProofWithPublicValues, SP1ProvingKey,
        SP1VerifyingKey,
        blocking::{Prover as _, ProverClient},
        network::NetworkMode as Sp1SdkNetworkMode,
        network::proto::types::{ExecutionStatus, FulfillmentStatus},
    };
    use std::str::FromStr;

    fn sp1_test_elves() -> Sp1ShastaGuestElves {
        load_sp1_shasta_guest_elves().expect("load SP1 Shasta guest ELFs")
    }

    #[test]
    fn sp1_transient_poll_status_keeps_submission_active() {
        let submission_id = RemoteSubmissionId::new();

        let status = sp1_transient_poll_status(submission_id, "rpc 429");

        assert_eq!(status.submission_id, submission_id);
        assert_eq!(status.status, RemoteStatus::Pending);
        assert!(status.reason.is_none());
    }

    #[test]
    fn sp1_unknown_status_values_decode_to_unspecified() {
        assert_eq!(
            decode_fulfillment_status(i32::MAX),
            FulfillmentStatus::UnspecifiedFulfillmentStatus
        );
        assert_eq!(
            decode_execution_status(i32::MAX),
            ExecutionStatus::UnspecifiedExecutionStatus
        );
    }

    fn setup_sp1_pk(client: &sp1_sdk::blocking::MockProver, elf: &[u8]) -> SP1ProvingKey {
        client.setup(elf.into()).expect("setup SP1 test ELF")
    }

    fn legacy_raiko_sp1_aggregation_payload(
        aggregation_vk: &SP1VerifyingKey,
        block_vk: &SP1VerifyingKey,
        proof_bytes: impl AsRef<[u8]>,
    ) -> String {
        format!(
            "{}{}{}",
            aggregation_vk.bytes32(),
            alloy_primitives::hex::encode(block_vk.hash_bytes()),
            alloy_primitives::hex::encode(proof_bytes)
        )
    }

    #[test]
    fn sp1_new_does_not_preload_setup_cache() {
        let prover = super::Sp1Prover::new(super::Sp1Config::default());

        assert!(
            prover
                .setup_cache
                .cell(ProofStage::Proposal)
                .get()
                .is_none()
        );
        assert!(
            prover
                .setup_cache
                .cell(ProofStage::Aggregation)
                .get()
                .is_none()
        );
    }

    #[test]
    fn sp1_new_with_backend_preloads_setup_cache() {
        let backend = sp1_shasta_backend_from_elves(sp1_test_elves());
        let prover = super::Sp1Prover::new_with_backend(super::Sp1Config::default(), &backend)
            .expect("preload SP1 setup");

        assert!(
            prover
                .setup_cache
                .cell(ProofStage::Proposal)
                .get()
                .is_some()
        );
        assert!(
            prover
                .setup_cache
                .cell(ProofStage::Aggregation)
                .get()
                .is_some()
        );
    }

    #[test]
    fn sp1_network_mode_maps_to_sdk_mode_explicitly() {
        assert_eq!(
            sp1_sdk_network_mode(super::Sp1NetworkMode::Mainnet),
            Sp1SdkNetworkMode::Mainnet
        );
        assert_eq!(
            sp1_sdk_network_mode(super::Sp1NetworkMode::Reserved),
            Sp1SdkNetworkMode::Reserved
        );
    }

    #[test]
    fn default_sp1_network_rpc_url_follows_configured_mode() {
        assert_eq!(
            default_sp1_network_rpc_url(super::Sp1NetworkMode::Mainnet),
            super::SP1_MAINNET_RPC_URL
        );
        assert_eq!(
            default_sp1_network_rpc_url(super::Sp1NetworkMode::Reserved),
            super::SP1_RESERVED_RPC_URL
        );
    }

    #[test]
    fn sp1_network_rpc_url_trims_and_validates_env_override() {
        let config = super::Sp1Config::default();

        let rpc_url = resolve_sp1_network_rpc_url(&config, Some(" https://example.invalid/rpc "))
            .expect("valid env rpc url");

        assert_eq!(rpc_url, "https://example.invalid/rpc");
    }

    #[test]
    fn sp1_network_rpc_url_prefers_config_over_env() {
        let config = super::Sp1Config {
            rpc_url: Some("https://config.example.invalid/rpc".to_string()),
            ..super::Sp1Config::default()
        };

        let rpc_url = resolve_sp1_network_rpc_url(&config, Some("https://env.example.invalid/rpc"))
            .expect("valid config rpc url");

        assert_eq!(rpc_url, "https://config.example.invalid/rpc");
    }

    #[test]
    fn sp1_network_rpc_url_rejects_untrimmed_config_override() {
        let config = super::Sp1Config {
            rpc_url: Some(" https://config.example.invalid/rpc ".to_string()),
            ..super::Sp1Config::default()
        };

        let err = resolve_sp1_network_rpc_url(&config, Some("https://env.example.invalid/rpc"))
            .expect_err("untrimmed config rpc url");

        assert!(
            err.to_string()
                .contains("SP1 network RPC URL must not include leading or trailing whitespace")
        );
    }

    #[test]
    fn sp1_network_rpc_url_rejects_invalid_env_override() {
        let config = super::Sp1Config::default();

        let err = resolve_sp1_network_rpc_url(&config, Some("not a url"))
            .expect_err("invalid env rpc url");

        assert!(err.to_string().contains("SP1 network RPC URL is invalid"));
    }

    #[test]
    fn remote_verifier_program_vkey_matches_contract_bytes32_encoding() {
        let client = ProverClient::builder().mock().build();
        let elves = sp1_test_elves();
        let pk = setup_sp1_pk(&client, elves.proposal.as_ref());
        let vk = pk.verifying_key().clone();

        let expected = B256::from_str(&vk.bytes32()).expect("valid bytes32 encoding");
        let old_buggy = B256::from_slice(&vk.hash_bytes());

        assert_eq!(remote_verifier_program_vkey(&vk), expected);
        assert_ne!(remote_verifier_program_vkey(&vk), old_buggy);
    }

    #[test]
    fn raw_image_id_hex_roundtrips_through_guest_word_encoding() {
        let expected = B256::from([0x5a; 32]);
        let words = super::sp1_image_id_words_from_uuid(&expected.to_string()).expect("image id");

        assert_eq!(B256::from(words_to_bytes_le(&words)), expected);
    }

    #[test]
    fn sp1_vkey_words_match_contract_block_program_encoding() {
        let client = ProverClient::builder().mock().build();
        let elves = sp1_test_elves();
        let pk = setup_sp1_pk(&client, elves.proposal.as_ref());
        let vk = pk.verifying_key().clone();
        let words = super::sp1_image_id_words_from_uuid(&sp1_vk_uuid(&vk)).expect("image id");

        assert_eq!(words, vk.hash_u32());
        assert_eq!(
            sp1_contract_block_program_id(&words),
            B256::from_slice(&vk.hash_bytes())
        );
        assert_ne!(
            B256::from(words_to_bytes_le(&words)),
            B256::from_slice(&vk.hash_bytes())
        );
    }

    #[test]
    fn sp1_aggregation_public_input_matches_contract_payload_program_id() {
        let client = ProverClient::builder().mock().build();
        let elves = sp1_test_elves();
        let block_vk = setup_sp1_pk(&client, elves.proposal.as_ref())
            .verifying_key()
            .clone();
        let aggregation_vk = setup_sp1_pk(&client, elves.aggregation.as_ref())
            .verifying_key()
            .clone();
        let words = super::sp1_image_id_words_from_uuid(&sp1_vk_uuid(&block_vk)).expect("image id");
        let proof_bytes = [0xaa, 0xbb, 0xcc];

        let payload = legacy_raiko_sp1_aggregation_payload(&aggregation_vk, &block_vk, proof_bytes);
        let payload_bytes = alloy_primitives::hex::decode(payload.strip_prefix("0x").unwrap())
            .expect("payload hex");
        let block_program_from_contract_payload = B256::from_slice(&payload_bytes[32..64]);

        assert_eq!(
            block_program_from_contract_payload,
            sp1_contract_block_program_id(&words)
        );
        assert_ne!(
            block_program_from_contract_payload,
            B256::from(words_to_bytes_le(&words))
        );
    }

    #[test]
    fn remote_verifier_proof_bytes_matches_legacy_raiko_semantics() {
        let client = ProverClient::builder().mock().build();
        let elves = sp1_test_elves();
        let pk = setup_sp1_pk(&client, elves.proposal.as_ref());

        let core = SP1ProofWithPublicValues::create_mock_proof(
            pk.verifying_key(),
            sp1_sdk::SP1PublicValues::new(),
            SP1ProofMode::Core,
            sp1_sdk::SP1_CIRCUIT_VERSION,
        );
        let compressed = SP1ProofWithPublicValues::create_mock_proof(
            pk.verifying_key(),
            sp1_sdk::SP1PublicValues::new(),
            SP1ProofMode::Compressed,
            sp1_sdk::SP1_CIRCUIT_VERSION,
        );
        let plonk = SP1ProofWithPublicValues::create_mock_proof(
            pk.verifying_key(),
            sp1_sdk::SP1PublicValues::new(),
            SP1ProofMode::Plonk,
            sp1_sdk::SP1_CIRCUIT_VERSION,
        );

        assert!(remote_verifier_proof_bytes(&core).is_none());
        assert!(remote_verifier_proof_bytes(&compressed).is_none());
        assert!(remote_verifier_proof_bytes(&plonk).is_none());
    }

    #[test]
    fn sp1_aggregation_payload_layout_matches_legacy_raiko() {
        let client = ProverClient::builder().mock().build();
        let elves = sp1_test_elves();
        let block_vk = setup_sp1_pk(&client, elves.proposal.as_ref())
            .verifying_key()
            .clone();
        let aggregation_vk = setup_sp1_pk(&client, elves.aggregation.as_ref())
            .verifying_key()
            .clone();
        let proof_bytes = [0xaa, 0xbb, 0xcc];

        let payload = encode_sp1_onchain_payload(
            &[
                aggregation_vk.bytes32(),
                alloy_primitives::hex::encode_prefixed(block_vk.hash_bytes()),
            ],
            &proof_bytes,
        );

        assert_eq!(
            payload,
            legacy_raiko_sp1_aggregation_payload(&aggregation_vk, &block_vk, proof_bytes)
        );
    }

    #[test]
    fn load_sp1_subproof_for_aggregation_prefers_quote() {
        let client = ProverClient::builder().mock().build();
        let elves = sp1_test_elves();
        let pk = setup_sp1_pk(&client, elves.proposal.as_ref());
        let compressed = SP1ProofWithPublicValues::create_mock_proof(
            pk.verifying_key(),
            sp1_sdk::SP1PublicValues::new(),
            SP1ProofMode::Compressed,
            sp1_sdk::SP1_CIRCUIT_VERSION,
        );
        let proof = Proof {
            quote: Some(serde_json::to_string(&compressed.proof).expect("serialize quote")),
            proof: Some("0xdeadbeef".to_string()),
            ..Proof::default()
        };

        let loaded = load_sp1_subproof_for_aggregation(&proof).expect("load subproof");
        assert!(matches!(loaded, sp1_sdk::SP1Proof::Compressed(_)));
    }

    #[test]
    fn load_sp1_subproof_for_aggregation_accepts_legacy_bincode_payload() {
        let client = ProverClient::builder().mock().build();
        let elves = sp1_test_elves();
        let pk = setup_sp1_pk(&client, elves.proposal.as_ref());
        let compressed = SP1ProofWithPublicValues::create_mock_proof(
            pk.verifying_key(),
            sp1_sdk::SP1PublicValues::new(),
            SP1ProofMode::Compressed,
            sp1_sdk::SP1_CIRCUIT_VERSION,
        );
        let encoded = bincode::serialize(&compressed).expect("serialize legacy payload");
        let proof = Proof {
            proof: Some(alloy_primitives::hex::encode_prefixed(encoded)),
            ..Proof::default()
        };

        let loaded = load_sp1_subproof_for_aggregation(&proof).expect("load legacy subproof");
        assert!(matches!(loaded, sp1_sdk::SP1Proof::Compressed(_)));
    }

    #[test]
    fn expected_sp1_aggregation_input_hash_binds_image_id() {
        use raiko2_primitives_shasta::ShastaZkAggregationGuestInput;
        use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
        use raiko2_protocol_shasta::shasta::ProofCarryData;

        let carry = ProofCarryData {
            chain_id: 1,
            ..ProofCarryData::default()
        };
        let make = |image_id: [u32; 8]| ShastaZkAggregationGuestInput {
            image_id,
            block_inputs: vec![hash_shasta_subproof_input(&carry)],
            proof_carry_data_vec: vec![carry.clone()],
            prover_address: alloy_primitives::Address::ZERO,
        };

        let input = make([1, 2, 3, 4, 5, 6, 7, 8]);
        let hash = super::expected_sp1_aggregation_input_hash(&input).expect("hash");
        let other = make([8, 7, 6, 5, 4, 3, 2, 1]);

        assert_eq!(
            hash,
            super::expected_sp1_aggregation_input_hash(&input).expect("hash")
        );
        assert_ne!(
            hash,
            super::expected_sp1_aggregation_input_hash(&other).expect("hash")
        );
        assert!(super::ensure_sp1_network_aggregation_input_hash_matches(hash, hash).is_ok());
        assert!(
            super::ensure_sp1_network_aggregation_input_hash_matches(hash, B256::repeat_byte(0xff))
                .is_err()
        );
    }
}
