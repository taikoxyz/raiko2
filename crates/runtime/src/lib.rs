#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![expect(
    clippy::missing_errors_doc,
    reason = "the runtime crate currently permits undocumented internal-facing public APIs"
)]

mod artifact_store;
mod publication;

pub use artifact_store::{
    GcsProofArtifactStore, MemoryProofArtifactStore, ProofArtifactConflict,
    ProofArtifactDeleteResult, ProofArtifactDescriptor, ProofArtifactKey, ProofArtifactObject,
    ProofArtifactPrefix, ProofArtifactPutResult, ProofArtifactStore, RuntimeStateObject,
    RuntimeStateWriteResult, validate_scope_component,
};
pub use publication::ProofArtifactPublicationInvalidated;

use anyhow::{Context, Result};
use raiko2_pipeline::{PipelineKey, PipelineRoute};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, RwLock, watch};

#[derive(Debug)]
pub struct PendingProofPublicationRemoved {
    proof_ref: String,
}

impl std::fmt::Display for PendingProofPublicationRemoved {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "pending proof publication {} has been removed",
            self.proof_ref
        )
    }
}

impl std::error::Error for PendingProofPublicationRemoved {}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct RuntimeState {
    tasks: HashMap<String, RuntimeTaskRecord>,
    artifacts: HashMap<String, ProofArtifactRecord>,
    pending_publications: HashMap<String, PendingProofPublicationRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PendingProofPublicationRecord {
    content_hash: String,
    owner_incarnations: Vec<uuid::Uuid>,
}

#[derive(Debug, Clone)]
pub struct PendingProofPublication {
    pub bytes: Vec<u8>,
    pub owner_incarnations: Vec<uuid::Uuid>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum StateCoherence {
    Coherent,
    Recoverable,
    Violated,
}

impl StateCoherence {
    fn load(value: &AtomicU8) -> Self {
        match value.load(Ordering::Acquire) {
            0 => Self::Coherent,
            1 => Self::Recoverable,
            _ => Self::Violated,
        }
    }
}

#[derive(Debug)]
struct SubmissionCheckpointAdmission {
    state: StdMutex<SubmissionCheckpointAdmissionState>,
    active: watch::Sender<usize>,
}

#[derive(Debug)]
struct SubmissionCheckpointAdmissionState {
    accepting: bool,
}

impl Default for SubmissionCheckpointAdmission {
    fn default() -> Self {
        let (active, _) = watch::channel(0);
        Self {
            state: StdMutex::new(SubmissionCheckpointAdmissionState { accepting: true }),
            active,
        }
    }
}

impl SubmissionCheckpointAdmission {
    fn acquire(self: &Arc<Self>) -> Result<RuntimeSubmissionCheckpointPermit> {
        let state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("submission checkpoint admission lock poisoned"))?;
        anyhow::ensure!(state.accepting, "submission checkpoint admission is closed");
        self.active
            .send_modify(|active| *active = active.saturating_add(1));
        drop(state);
        Ok(RuntimeSubmissionCheckpointPermit {
            admission: Arc::clone(self),
        })
    }

    fn close(&self) -> Result<()> {
        self.state
            .lock()
            .map_err(|_| anyhow::anyhow!("submission checkpoint admission lock poisoned"))?
            .accepting = false;
        Ok(())
    }

    async fn wait_until_drained(&self) {
        let mut active = self.active.subscribe();
        while *active.borrow_and_update() != 0 {
            if active.changed().await.is_err() {
                break;
            }
        }
    }
}

/// Runtime-owned permit spanning provider acceptance through durable checkpoint persistence.
#[derive(Debug)]
pub struct RuntimeSubmissionCheckpointPermit {
    admission: Arc<SubmissionCheckpointAdmission>,
}

impl Drop for RuntimeSubmissionCheckpointPermit {
    fn drop(&mut self) {
        let Ok(state) = self.admission.state.lock() else {
            return;
        };
        self.admission
            .active
            .send_modify(|active| *active = active.saturating_sub(1));
        drop(state);
    }
}

#[derive(Debug)]
pub struct RuntimeManager {
    store: Arc<dyn ProofArtifactStore>,
    state: RwLock<RuntimeState>,
    generation: StdMutex<Option<i64>>,
    lifecycle_commit: StdMutex<()>,
    mutation: Mutex<()>,
    pending_publication_mutation: Mutex<()>,
    lifecycle_operation: Arc<Mutex<()>>,
    lifecycle_gate: Arc<RwLock<()>>,
    submission_checkpoints: Arc<SubmissionCheckpointAdmission>,
    active: AtomicBool,
    draining: AtomicBool,
    state_coherence: AtomicU8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpiredTaskCursor {
    pub updated_at: i64,
    pub task_id: String,
}

#[derive(Debug, Clone)]
pub enum TaskRegistrationOutcome {
    Created(RuntimeTaskRecord),
    Existing(RuntimeTaskRecord),
}

#[derive(Debug, Clone)]
pub struct ProofArtifactRegistration {
    pub network_pair: String,
    pub proof_ref: String,
    pub pipeline_key: PipelineKey,
    pub route: PipelineRoute,
    pub proof_uri: String,
    pub content_hash: String,
    pub generation: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ProofArtifactPrecondition {
    pub network_pair: String,
    pub proof_ref: String,
    pub pipeline_key: PipelineKey,
    pub route: PipelineRoute,
    pub descriptor: ProofArtifactDescriptor,
}

/// Process-local guard serializing lifecycle operations that span runtime and queue state.
#[derive(Debug)]
pub struct RuntimeLifecycleOperationGuard {
    _guard: tokio::sync::OwnedMutexGuard<()>,
    _lifecycle: tokio::sync::OwnedRwLockReadGuard<()>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofArtifactInvalidationResult {
    Invalidated(ProofArtifactDeleteResult),
    BlockedByLiveTask,
    MissingOrChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProofArtifactInvalidationAdmission {
    Marked,
    BlockedByLiveTask,
    MissingOrChanged,
}

impl ProofArtifactRegistration {
    #[must_use]
    pub fn descriptor(&self) -> ProofArtifactDescriptor {
        ProofArtifactDescriptor {
            proof_uri: self.proof_uri.clone(),
            content_hash: self.content_hash.clone(),
            generation: self.generation,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProofArtifactLifecycle {
    Pending,
    Active,
    Invalidated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProofArtifactRecord {
    pub environment: String,
    pub network_pair: String,
    pub proof_ref: String,
    pub pipeline_key: PipelineKey,
    pub route: PipelineRoute,
    pub proof_uri: String,
    pub content_hash: String,
    pub generation: Option<i64>,
    pub lifecycle: ProofArtifactLifecycle,
    pub invalidated_at: Option<i64>,
    pub updated_at: i64,
}

impl ProofArtifactRecord {
    #[must_use]
    pub fn descriptor(&self) -> ProofArtifactDescriptor {
        ProofArtifactDescriptor {
            proof_uri: self.proof_uri.clone(),
            content_hash: self.content_hash.clone(),
            generation: self.generation,
        }
    }
}

impl RuntimeManager {
    pub async fn get_tasks_by_ref(&self, task_ref: &str) -> Vec<RuntimeTaskRecord> {
        let mut records = self
            .state
            .read()
            .await
            .tasks
            .values()
            .filter(|record| task_references(record, task_ref))
            .cloned()
            .collect::<Vec<_>>();
        records.sort_by(|left, right| left.task_id.cmp(&right.task_id));
        records
    }

    /// Atomically updates every runtime root that references `task_ref` in one runtime-state CAS.
    ///
    /// The callback may be invoked more than once after a recoverable transport failure, so it must
    /// derive its result exclusively from the supplied records and must not perform side effects.
    pub async fn update_tasks_by_ref<T, F>(&self, task_ref: &str, update: F) -> Result<T>
    where
        F: Fn(&mut Vec<RuntimeTaskRecord>) -> Result<T>,
    {
        let _pending_publication = self.pending_publication_mutation.lock().await;
        let task_ref = task_ref.to_string();
        self.mutate(move |state| {
            let mut records = state
                .tasks
                .values()
                .filter(|record| task_references(record, &task_ref))
                .cloned()
                .collect::<Vec<_>>();
            records.sort_by(|left, right| left.task_id.cmp(&right.task_id));
            let output = update(&mut records)?;
            for record in records {
                state.tasks.insert(record.task_id.clone(), record);
            }
            Ok(output)
        })
        .await
    }

    /// Persists a provider request identifier after the global shutdown fence has closed.
    ///
    /// Callers must hold a `RuntimeSubmissionCheckpointPermit` acquired before draining started.
    #[doc(hidden)]
    pub async fn checkpoint_tasks_by_ref<T, F>(
        &self,
        permit: &RuntimeSubmissionCheckpointPermit,
        task_ref: &str,
        update: F,
    ) -> Result<T>
    where
        F: Fn(&mut Vec<RuntimeTaskRecord>) -> Result<T>,
    {
        let _pending_publication = self.pending_publication_mutation.lock().await;
        let task_ref = task_ref.to_string();
        self.mutate_checkpoint(permit, move |state| {
            let mut records = state
                .tasks
                .values()
                .filter(|record| task_references(record, &task_ref))
                .cloned()
                .collect::<Vec<_>>();
            records.sort_by(|left, right| left.task_id.cmp(&right.task_id));
            let output = update(&mut records)?;
            for record in records {
                state.tasks.insert(record.task_id.clone(), record);
            }
            Ok(output)
        })
        .await
    }

    pub fn new(test_root: impl AsRef<std::path::Path>) -> Result<Self> {
        let namespace = format!(
            "test-{}",
            artifact_store::content_hash(test_root.as_ref().to_string_lossy().as_bytes())
        );
        let store = Arc::new(MemoryProofArtifactStore::new(
            "local".to_string(),
            namespace,
        )?);
        Self::with_store(store)
    }

    pub fn new_memory(environment: String, namespace: String) -> Result<Self> {
        let store = Arc::new(MemoryProofArtifactStore::new(environment, namespace)?);
        Self::with_store(store)
    }

    pub fn with_store(store: Arc<dyn ProofArtifactStore>) -> Result<Self> {
        Ok(Self {
            store,
            state: RwLock::new(RuntimeState::default()),
            generation: StdMutex::new(None),
            lifecycle_commit: StdMutex::new(()),
            mutation: Mutex::new(()),
            pending_publication_mutation: Mutex::new(()),
            lifecycle_operation: Arc::new(Mutex::new(())),
            lifecycle_gate: Arc::new(RwLock::new(())),
            submission_checkpoints: Arc::new(SubmissionCheckpointAdmission::default()),
            active: AtomicBool::new(true),
            draining: AtomicBool::new(false),
            state_coherence: AtomicU8::new(StateCoherence::Coherent as u8),
        })
    }

    /// Starts one process-local lifecycle operation spanning runtime, queue, and artifact work.
    pub async fn acquire_lifecycle_operation(&self) -> Result<RuntimeLifecycleOperationGuard> {
        self.ensure_active()?;
        let operation = Arc::clone(&self.lifecycle_operation).lock_owned().await;
        let lifecycle = Arc::clone(&self.lifecycle_gate).read_owned().await;
        self.ensure_active()?;
        Ok(RuntimeLifecycleOperationGuard {
            _guard: operation,
            _lifecycle: lifecycle,
        })
    }

    #[doc(hidden)]
    pub fn new_with_artifact_store(
        _test_identity: PathBuf,
        store: Arc<dyn ProofArtifactStore>,
    ) -> Result<Self> {
        Self::with_store(store)
    }

    pub async fn begin_draining(&self) {
        self.start_draining();
        let _lifecycle = self.lifecycle_gate.write().await;
        self.submission_checkpoints.wait_until_drained().await;
        self.deactivate();
    }

    /// Closes provider-submission admission, waits until `deadline` for accepted checkpoints, then
    /// makes the runtime inactive regardless of whether the deadline elapsed.
    ///
    /// Returns `true` when every accepted checkpoint completed before the deadline.
    pub async fn begin_draining_with_deadline(&self, deadline: Instant) -> bool {
        self.start_draining();
        let drained = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
            let _lifecycle = self.lifecycle_gate.write().await;
            self.submission_checkpoints.wait_until_drained().await;
        })
        .await
        .is_ok();
        self.deactivate();
        drained
    }

    /// Engages the namespace-wide shutdown fence while allowing already accepted provider
    /// submissions to persist their request identifiers.
    pub fn start_draining(&self) {
        self.draining.store(true, Ordering::Release);
        if self.submission_checkpoints.close().is_err() {
            self.deactivate();
        }
    }

    fn deactivate(&self) {
        let _commit = self
            .lifecycle_commit
            .lock()
            .expect("runtime lifecycle commit lock poisoned");
        self.active.store(false, Ordering::Release);
    }

    /// Reserves one remote-provider submission until its request id is durably checkpointed.
    pub fn acquire_submission_checkpoint_permit(
        &self,
    ) -> Result<RuntimeSubmissionCheckpointPermit> {
        self.ensure_active()?;
        let permit = self.submission_checkpoints.acquire()?;
        if let Err(error) = self.ensure_active() {
            drop(permit);
            return Err(error);
        }
        Ok(permit)
    }

    #[must_use]
    pub fn accepts_mutations(&self) -> bool {
        self.active.load(Ordering::Acquire)
            && !self.draining.load(Ordering::Acquire)
            && StateCoherence::load(&self.state_coherence) == StateCoherence::Coherent
    }

    #[must_use]
    pub fn mutation_failure_is_retryable(&self) -> bool {
        self.active.load(Ordering::Acquire)
            && !self.draining.load(Ordering::Acquire)
            && StateCoherence::load(&self.state_coherence) != StateCoherence::Violated
    }

    #[must_use]
    pub fn checkpoint_failure_is_retryable(
        &self,
        permit: &RuntimeSubmissionCheckpointPermit,
    ) -> bool {
        self.active.load(Ordering::Acquire)
            && Arc::ptr_eq(&self.submission_checkpoints, &permit.admission)
            && StateCoherence::load(&self.state_coherence) != StateCoherence::Violated
    }

    #[must_use]
    pub fn is_lifecycle_active(&self) -> bool {
        self.active.load(Ordering::Acquire) && !self.draining.load(Ordering::Acquire)
    }

    /// Verifies that the single-instance runtime is active and its authoritative store is readable.
    pub async fn check_readiness(&self) -> Result<()> {
        self.ensure_active_lifecycle()?;
        let _mutation = self.mutation.lock().await;
        self.ensure_active_lifecycle()?;
        let stored = self
            .store
            .load_runtime_state()
            .await
            .context("authoritative runtime store is unavailable")?;
        match StateCoherence::load(&self.state_coherence) {
            StateCoherence::Coherent => {}
            StateCoherence::Recoverable => self.install_runtime_state_object(stored).await?,
            StateCoherence::Violated => {
                anyhow::bail!("runtime state generation invariant was violated")
            }
        }
        self.ensure_active()
    }

    fn ensure_active_lifecycle(&self) -> Result<()> {
        anyhow::ensure!(
            self.active.load(Ordering::Acquire) && !self.draining.load(Ordering::Acquire),
            "runtime is draining"
        );
        Ok(())
    }

    fn ensure_checkpoint_write_allowed(
        &self,
        permit: &RuntimeSubmissionCheckpointPermit,
    ) -> Result<()> {
        anyhow::ensure!(
            self.active.load(Ordering::Acquire),
            "runtime checkpoint deadline elapsed"
        );
        anyhow::ensure!(
            StateCoherence::load(&self.state_coherence) != StateCoherence::Violated,
            "runtime state generation invariant was violated"
        );
        anyhow::ensure!(
            Arc::ptr_eq(&self.submission_checkpoints, &permit.admission),
            "submission checkpoint permit belongs to another runtime"
        );
        Ok(())
    }

    fn ensure_active(&self) -> Result<()> {
        self.ensure_active_lifecycle()?;
        anyhow::ensure!(
            StateCoherence::load(&self.state_coherence) == StateCoherence::Coherent,
            "runtime state generation is not coherent with the authoritative store"
        );
        Ok(())
    }

    pub async fn initialize(&self) -> Result<()> {
        let _mutation = self.mutation.lock().await;
        self.reload_authoritative_state().await
    }

    #[must_use]
    pub fn environment(&self) -> &str {
        self.store.environment()
    }

    #[must_use]
    pub fn namespace(&self) -> &str {
        self.store.namespace()
    }

    #[must_use]
    pub fn backend_name(&self) -> &'static str {
        self.store.backend_name()
    }

    async fn mutate<T>(&self, update: impl Fn(&mut RuntimeState) -> Result<T>) -> Result<T> {
        self.mutate_authoritative(update, None).await
    }

    async fn mutate_checkpoint<T>(
        &self,
        permit: &RuntimeSubmissionCheckpointPermit,
        update: impl Fn(&mut RuntimeState) -> Result<T>,
    ) -> Result<T> {
        self.mutate_authoritative(update, Some(permit)).await
    }

    fn ensure_authoritative_write_allowed(
        &self,
        checkpoint_permit: Option<&RuntimeSubmissionCheckpointPermit>,
    ) -> Result<()> {
        if let Some(permit) = checkpoint_permit {
            self.ensure_checkpoint_write_allowed(permit)
        } else {
            self.ensure_active()
        }
    }

    fn mark_state_coherence_recoverable(&self) {
        self.state_coherence
            .store(StateCoherence::Recoverable as u8, Ordering::Release);
    }

    fn ensure_in_flight_write_allowed(
        &self,
        checkpoint_permit: Option<&RuntimeSubmissionCheckpointPermit>,
        context: &'static str,
    ) -> Result<()> {
        if let Err(error) = self.ensure_authoritative_write_allowed(checkpoint_permit) {
            self.mark_state_coherence_recoverable();
            return Err(error).context(context);
        }
        Ok(())
    }

    async fn mutate_authoritative<T>(
        &self,
        update: impl Fn(&mut RuntimeState) -> Result<T>,
        checkpoint_permit: Option<&RuntimeSubmissionCheckpointPermit>,
    ) -> Result<T> {
        const MAX_ATTEMPTS: usize = 3;

        self.ensure_authoritative_write_allowed(checkpoint_permit)?;
        let _lifecycle = if checkpoint_permit.is_some() {
            None
        } else {
            Some(
                self.lifecycle_gate
                    .try_read()
                    .context("runtime is draining")?,
            )
        };
        self.ensure_authoritative_write_allowed(checkpoint_permit)?;
        let _mutation = self.mutation.lock().await;
        self.reload_checkpoint_state_if_needed(checkpoint_permit)
            .await?;
        let mut last_error = None;
        for attempt in 1..=MAX_ATTEMPTS {
            self.ensure_authoritative_write_allowed(checkpoint_permit)?;
            let current = self.state.read().await.clone();
            let current_bytes =
                serde_json::to_vec(&current).context("encode runtime store state")?;
            let expected_generation = self.current_generation()?;
            let mut next = current.clone();
            let output = update(&mut next)?;
            let next_bytes = serde_json::to_vec(&next).context("encode runtime store state")?;

            let write_result = self
                .store
                .store_runtime_state(&next_bytes, expected_generation)
                .await;
            self.ensure_in_flight_write_allowed(
                checkpoint_permit,
                "runtime lifecycle closed while authoritative state write was in flight",
            )?;
            match write_result {
                Ok(RuntimeStateWriteResult::Stored { generation }) => {
                    self.install_checkpointed_runtime_state(next, generation, checkpoint_permit)
                        .await?;
                    return Ok(output);
                }
                Ok(RuntimeStateWriteResult::Conflict(_)) => {
                    self.state_coherence
                        .store(StateCoherence::Violated as u8, Ordering::Release);
                    anyhow::bail!(
                        "runtime state generation changed during mutation; refusing foreign state"
                    );
                }
                Err(write_error) => {
                    let observed = match self.store.load_runtime_state().await {
                        Ok(observed) => observed,
                        Err(read_error) => {
                            self.mark_state_coherence_recoverable();
                            return Err(write_error).context(format!(
                                "runtime state write outcome is unknown and read-back failed: {read_error:#}"
                            ));
                        }
                    };
                    self.ensure_in_flight_write_allowed(
                        checkpoint_permit,
                        "runtime lifecycle closed during authoritative state read-back",
                    )?;
                    if let Some(observed) = observed.as_ref()
                        && observed.bytes == next_bytes
                    {
                        self.install_checkpointed_runtime_state(
                            next,
                            observed.generation,
                            checkpoint_permit,
                        )
                        .await?;
                        return Ok(output);
                    }
                    let remote_matches_current = match observed.as_ref() {
                        Some(observed) => {
                            observed.bytes == current_bytes
                                && observed.generation == expected_generation
                        }
                        None => expected_generation.is_none(),
                    };
                    if remote_matches_current {
                        last_error =
                            Some(write_error.context("runtime state write failed before commit"));
                    } else {
                        self.state_coherence
                            .store(StateCoherence::Violated as u8, Ordering::Release);
                        return Err(write_error).context(
                            "runtime state write outcome conflicted with authoritative read-back",
                        );
                    }
                }
            }

            if attempt < MAX_ATTEMPTS {
                tokio::task::yield_now().await;
            }
        }
        self.mark_state_coherence_recoverable();
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("runtime state mutation failed")))
    }

    async fn reload_checkpoint_state_if_needed(
        &self,
        checkpoint_permit: Option<&RuntimeSubmissionCheckpointPermit>,
    ) -> Result<()> {
        if checkpoint_permit.is_some()
            && StateCoherence::load(&self.state_coherence) == StateCoherence::Recoverable
        {
            self.reload_authoritative_state()
                .await
                .context("reload authoritative runtime state for checkpoint retry")?;
        }
        Ok(())
    }

    fn current_generation(&self) -> Result<Option<i64>> {
        self.generation
            .lock()
            .map(|generation| *generation)
            .map_err(|_| anyhow::anyhow!("runtime generation lock poisoned"))
    }

    async fn install_runtime_state(
        &self,
        state: RuntimeState,
        generation: Option<i64>,
    ) -> Result<()> {
        let mut local_state = self.state.write().await;
        let _commit = self
            .lifecycle_commit
            .lock()
            .map_err(|_| anyhow::anyhow!("runtime lifecycle commit lock poisoned"))?;
        anyhow::ensure!(
            self.active.load(Ordering::Acquire),
            "runtime became inactive before authoritative state was installed locally"
        );
        *local_state = state;
        *self
            .generation
            .lock()
            .map_err(|_| anyhow::anyhow!("runtime generation lock poisoned"))? = generation;
        self.state_coherence
            .store(StateCoherence::Coherent as u8, Ordering::Release);
        Ok(())
    }

    async fn install_checkpointed_runtime_state(
        &self,
        state: RuntimeState,
        generation: Option<i64>,
        checkpoint_permit: Option<&RuntimeSubmissionCheckpointPermit>,
    ) -> Result<()> {
        let mut local_state = self.state.write().await;
        let _commit = self
            .lifecycle_commit
            .lock()
            .map_err(|_| anyhow::anyhow!("runtime lifecycle commit lock poisoned"))?;
        if let Err(error) = self.ensure_authoritative_write_allowed(checkpoint_permit) {
            self.mark_state_coherence_recoverable();
            return Err(error).context(
                "runtime lifecycle closed before authoritative state was installed locally",
            );
        }
        *local_state = state;
        *self
            .generation
            .lock()
            .map_err(|_| anyhow::anyhow!("runtime generation lock poisoned"))? = generation;
        self.state_coherence
            .store(StateCoherence::Coherent as u8, Ordering::Release);
        Ok(())
    }

    async fn install_runtime_state_object(&self, stored: Option<RuntimeStateObject>) -> Result<()> {
        match stored {
            Some(stored) => {
                let state = serde_json::from_slice(&stored.bytes)
                    .context("decode authoritative runtime store state")?;
                self.install_runtime_state(state, stored.generation).await
            }
            None => {
                self.install_runtime_state(RuntimeState::default(), None)
                    .await
            }
        }
    }

    async fn reload_authoritative_state(&self) -> Result<()> {
        let stored = self.store.load_runtime_state().await?;
        self.install_runtime_state_object(stored).await
    }

    fn fence_external_mutation(&self) -> Result<tokio::sync::RwLockReadGuard<'_, ()>> {
        self.ensure_active()?;
        let lifecycle = self
            .lifecycle_gate
            .try_read()
            .context("runtime is draining")?;
        self.ensure_active()?;
        Ok(lifecycle)
    }

    fn artifact_key(
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> ProofArtifactKey {
        ProofArtifactKey {
            network_pair: network_pair.to_string(),
            pipeline_key,
            route,
            proof_ref: proof_ref.to_string(),
        }
    }

    pub async fn publish_proof_artifact_bytes(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        bytes: &[u8],
    ) -> Result<ProofArtifactPutResult> {
        let _pending_publication = self.pending_publication_mutation.lock().await;
        self.publish_proof_artifact_bytes_locked(
            network_pair,
            pipeline_key,
            route,
            proof_ref,
            bytes,
        )
        .await
    }

    pub(crate) async fn publish_proof_artifact_bytes_locked(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        bytes: &[u8],
    ) -> Result<ProofArtifactPutResult> {
        let lifecycle = self.fence_external_mutation()?;
        let key = Self::artifact_key(network_pair, pipeline_key, route, proof_ref);
        let publication = self.store.put_if_absent(&key, bytes).await?;
        drop(lifecycle);
        let _lifecycle = self
            .fence_external_mutation()
            .context("global runtime fence changed during artifact publication")?;
        Ok(publication)
    }

    pub async fn publish_active_proof_artifact_bytes(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        bytes: &[u8],
    ) -> Result<ProofArtifactPutResult> {
        let _pending_publication = self.pending_publication_mutation.lock().await;
        let publication = self
            .publish_proof_artifact_bytes_locked(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
                bytes,
            )
            .await?;
        let artifact = publication
            .try_object()
            .context("active proof artifact conflict references missing content")?;
        self.upsert_proof_artifact_locked(ProofArtifactRegistration {
            network_pair: network_pair.to_string(),
            proof_ref: proof_ref.to_string(),
            pipeline_key,
            route,
            proof_uri: artifact.proof_uri.clone(),
            content_hash: artifact.content_hash.clone(),
            generation: artifact.generation,
        })
        .await?;
        Ok(publication)
    }

    pub async fn read_proof_artifact_bytes(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<Option<ProofArtifactObject>> {
        self.store
            .get(&Self::artifact_key(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
            ))
            .await
    }

    pub async fn read_proof_artifact_prefix(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        max_bytes: usize,
    ) -> Result<Option<ProofArtifactPrefix>> {
        self.store
            .get_prefix(
                &Self::artifact_key(network_pair, pipeline_key, route, proof_ref),
                max_bytes,
            )
            .await
    }

    pub async fn delete_proof_artifact(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        generation: Option<i64>,
        expected_content_hash: &str,
    ) -> Result<ProofArtifactDeleteResult> {
        let _lifecycle = self.fence_external_mutation()?;
        self.store
            .delete(
                &Self::artifact_key(network_pair, pipeline_key, route, proof_ref),
                generation,
                expected_content_hash,
            )
            .await
    }

    pub const fn ensure_layout(&self) -> Result<()> {
        Ok(())
    }

    pub async fn register_task(&self, registration: TaskRegistration) -> Result<RuntimeTaskRecord> {
        let record = build_task_record(&registration)?;
        self.upsert_task(&record).await?;
        Ok(record)
    }

    pub async fn register_task_with_artifact_preconditions(
        &self,
        registration: TaskRegistration,
        artifact_preconditions: &[ProofArtifactPrecondition],
    ) -> Result<RuntimeTaskRecord> {
        let _pending_publication = self.pending_publication_mutation.lock().await;
        let record = build_task_record(&registration)?;
        let artifact_preconditions = artifact_preconditions.to_vec();
        self.mutate(move |state| {
            ensure_artifact_preconditions(state, &artifact_preconditions)?;
            ensure_task_fingerprint_available(state, &record)?;
            state.tasks.insert(record.task_id.clone(), record.clone());
            Ok(record.clone())
        })
        .await
    }

    pub async fn register_task_if_absent(
        &self,
        registration: TaskRegistration,
    ) -> Result<TaskRegistrationOutcome> {
        self.register_task_if_absent_with_artifact_preconditions(registration, &[])
            .await
    }

    pub async fn register_task_if_absent_with_artifact_preconditions(
        &self,
        registration: TaskRegistration,
        artifact_preconditions: &[ProofArtifactPrecondition],
    ) -> Result<TaskRegistrationOutcome> {
        let _pending_publication = self.pending_publication_mutation.lock().await;
        let fingerprint = registration
            .request_fingerprint
            .as_deref()
            .context("request_fingerprint is required for idempotent registration")?
            .to_string();
        let record = build_task_record(&registration)?;
        let artifact_preconditions = artifact_preconditions.to_vec();
        self.mutate(move |state| {
            if let Some(existing) = state.tasks.get(&record.task_id).cloned().or_else(|| {
                state
                    .tasks
                    .values()
                    .find(|task| task.request_fingerprint.as_deref() == Some(&fingerprint))
                    .cloned()
            }) {
                return Ok(TaskRegistrationOutcome::Existing(existing));
            }
            ensure_artifact_preconditions(state, &artifact_preconditions)?;
            state.tasks.insert(record.task_id.clone(), record.clone());
            Ok(TaskRegistrationOutcome::Created(record.clone()))
        })
        .await
    }

    pub async fn upsert_task(&self, record: &RuntimeTaskRecord) -> Result<()> {
        let _pending_publication = self.pending_publication_mutation.lock().await;
        let record = record.clone();
        self.mutate(move |state| {
            ensure_task_fingerprint_available(state, &record)?;
            state.tasks.insert(record.task_id.clone(), record.clone());
            Ok(())
        })
        .await
    }

    pub async fn update_task_metadata(
        &self,
        task_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<bool> {
        let task_id = task_id.to_string();
        let metadata = metadata.clone();
        self.mutate(move |state| {
            let Some(task) = state.tasks.get_mut(&task_id) else {
                return Ok(false);
            };
            task.metadata = metadata.clone();
            task.updated_at = now_ts();
            Ok(true)
        })
        .await
    }

    pub async fn update_task_metadata_integer(
        &self,
        task_id: &str,
        json_path: &str,
        value: i64,
    ) -> Result<bool> {
        let task_id = task_id.to_string();
        let field = json_path
            .strip_prefix("$.")
            .context("metadata path must start with '$.'")?
            .split('.')
            .map(str::to_string)
            .collect::<Vec<_>>();
        self.mutate(move |state| {
            let Some(task) = state.tasks.get_mut(&task_id) else {
                return Ok(false);
            };
            set_json_integer(&mut task.metadata, &field, value)?;
            task.updated_at = now_ts();
            Ok(true)
        })
        .await
    }

    pub async fn get_task(&self, task_id: &str) -> Result<Option<RuntimeTaskRecord>> {
        Ok(self.state.read().await.tasks.get(task_id).cloned())
    }

    pub async fn find_task_by_request_fingerprint(
        &self,
        fingerprint: &str,
    ) -> Result<Option<RuntimeTaskRecord>> {
        Ok(self
            .state
            .read()
            .await
            .tasks
            .values()
            .filter(|task| task.request_fingerprint.as_deref() == Some(fingerprint))
            .max_by_key(|task| (task.updated_at, std::cmp::Reverse(task.task_id.clone())))
            .cloned())
    }

    pub async fn find_task_by_engine_task_id(
        &self,
        engine_task_id: &str,
    ) -> Result<Option<RuntimeTaskRecord>> {
        self.find_task_by_task_ref(engine_task_id).await
    }

    pub async fn find_task_by_task_ref(&self, task_ref: &str) -> Result<Option<RuntimeTaskRecord>> {
        Ok(self
            .find_tasks_by_task_ref(task_ref)
            .await?
            .into_iter()
            .next())
    }

    pub async fn find_tasks_by_engine_task_id(
        &self,
        engine_task_id: &str,
    ) -> Result<Vec<RuntimeTaskRecord>> {
        self.find_tasks_by_task_ref(engine_task_id).await
    }

    pub async fn find_tasks_by_task_ref(&self, task_ref: &str) -> Result<Vec<RuntimeTaskRecord>> {
        let mut tasks = self
            .state
            .read()
            .await
            .tasks
            .values()
            .filter(|task| {
                task.task_id == task_ref
                    || task.proof_ids.iter().any(|id| id == task_ref)
                    || task
                        .metadata
                        .get("aggregate_task_id")
                        .and_then(serde_json::Value::as_str)
                        == Some(task_ref)
            })
            .cloned()
            .collect::<Vec<_>>();
        tasks.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.task_id.cmp(&right.task_id))
        });
        Ok(tasks)
    }

    pub async fn sync_status(
        &self,
        task_id: &str,
        runner_status: RunnerStatus,
        error: Option<String>,
        proof_uri: Option<String>,
    ) -> Result<()> {
        let task_id = task_id.to_string();
        self.mutate(move |state| {
            if let Some(task) = state.tasks.get_mut(&task_id) {
                if task.runner_status.is_terminal() {
                    return Ok(());
                }
                task.runner_status = runner_status;
                task.error.clone_from(&error);
                if proof_uri.is_some() {
                    task.proof_uri.clone_from(&proof_uri);
                }
                task.updated_at = now_ts();
            }
            Ok(())
        })
        .await
    }

    pub async fn complete_nonterminal_task(&self, task_id: &str, proof_uri: &str) -> Result<bool> {
        let task_id = task_id.to_string();
        let proof_uri = proof_uri.to_string();
        self.mutate(move |state| {
            let Some(task) = state.tasks.get_mut(&task_id) else {
                return Ok(false);
            };
            if task.runner_status.is_terminal() {
                return Ok(false);
            }
            task.runner_status = RunnerStatus::Completed;
            task.proof_uri = Some(proof_uri.clone());
            task.error = None;
            task.updated_at = now_ts();
            Ok(true)
        })
        .await
    }

    pub async fn reopen_task_for_recovery(
        &self,
        task_id: &str,
        expected_status: RunnerStatus,
    ) -> Result<bool> {
        let _pending_publication = self.pending_publication_mutation.lock().await;
        let task_id = task_id.to_string();
        self.mutate(move |state| {
            let Some(task) = state.tasks.get_mut(&task_id) else {
                return Ok(false);
            };
            if task.runner_status != expected_status {
                return Ok(false);
            }
            task.runner_status = RunnerStatus::Allocated;
            task.error = None;
            task.updated_at = now_ts();
            Ok(true)
        })
        .await
    }

    pub async fn cancel_nonterminal_task(
        &self,
        task_id: &str,
        error: Option<String>,
    ) -> Result<bool> {
        self.cancel_nonterminal_task_matching(task_id, error, None)
            .await
    }

    pub async fn cancel_nonterminal_task_if_stale(
        &self,
        task_id: &str,
        updated_at_or_before: i64,
        error: Option<String>,
    ) -> Result<bool> {
        self.cancel_nonterminal_task_matching(task_id, error, Some(updated_at_or_before))
            .await
    }

    pub async fn cancel_nonterminal_task_if_incarnation_and_stale(
        &self,
        task_id: &str,
        expected_incarnation: uuid::Uuid,
        updated_at_or_before: i64,
        error: Option<String>,
    ) -> Result<bool> {
        let task_id = task_id.to_string();
        self.mutate(move |state| {
            let Some(task) = state.tasks.get_mut(&task_id) else {
                return Ok(false);
            };
            if task.incarnation_id != expected_incarnation
                || task.runner_status.is_terminal()
                || task.updated_at > updated_at_or_before
            {
                return Ok(false);
            }
            task.runner_status = RunnerStatus::Cancelled;
            task.error.clone_from(&error);
            task.updated_at = now_ts();
            Ok(true)
        })
        .await
    }

    async fn cancel_nonterminal_task_matching(
        &self,
        task_id: &str,
        error: Option<String>,
        updated_at_or_before: Option<i64>,
    ) -> Result<bool> {
        let task_id = task_id.to_string();
        self.mutate(move |state| {
            let Some(task) = state.tasks.get_mut(&task_id) else {
                return Ok(false);
            };
            if task.runner_status.is_terminal()
                || updated_at_or_before.is_some_and(|cutoff| task.updated_at > cutoff)
            {
                return Ok(false);
            }
            task.runner_status = RunnerStatus::Cancelled;
            task.error.clone_from(&error);
            task.updated_at = now_ts();
            Ok(true)
        })
        .await
    }

    pub async fn upsert_proof_artifact(
        &self,
        registration: ProofArtifactRegistration,
    ) -> Result<()> {
        let _pending_publication = self.pending_publication_mutation.lock().await;
        self.upsert_proof_artifact_locked(registration).await
    }

    async fn upsert_proof_artifact_locked(
        &self,
        registration: ProofArtifactRegistration,
    ) -> Result<()> {
        self.upsert_proof_artifact_with_lifecycle(registration, ProofArtifactLifecycle::Active)
            .await
    }

    pub(crate) async fn register_pending_proof_artifact(
        &self,
        registration: ProofArtifactRegistration,
    ) -> Result<ProofArtifactLifecycle> {
        let network_pair = registration.network_pair.clone();
        let pipeline_key = registration.pipeline_key;
        let route = registration.route;
        let proof_ref = registration.proof_ref.clone();
        let (key, record) =
            self.proof_artifact_record(registration, ProofArtifactLifecycle::Pending);
        let replaced_invalidated_descriptor = self
            .state
            .read()
            .await
            .artifacts
            .get(&key)
            .filter(|existing| {
                existing.lifecycle == ProofArtifactLifecycle::Invalidated
                    && existing.descriptor() != record.descriptor()
            })
            .map(ProofArtifactRecord::descriptor);
        let may_replace_invalidated = if replaced_invalidated_descriptor.is_some() {
            self.proof_artifact_descriptor_is_current(
                &record.network_pair,
                record.pipeline_key,
                record.route,
                &record.proof_ref,
                &record.descriptor(),
            )
            .await?
        } else {
            false
        };
        self.mutate(move |state| {
            if let Some(existing) = state.artifacts.get(&key) {
                if existing.descriptor() != record.descriptor() {
                    let replacement_is_current = may_replace_invalidated
                        && existing.lifecycle == ProofArtifactLifecycle::Invalidated
                        && replaced_invalidated_descriptor
                            .as_ref()
                            .is_some_and(|expected| existing.descriptor() == *expected);
                    if !replacement_is_current {
                        return Ok(ProofArtifactLifecycle::Invalidated);
                    }
                }
                if existing.lifecycle == ProofArtifactLifecycle::Invalidated
                    && existing.descriptor() == record.descriptor()
                {
                    return Ok(ProofArtifactLifecycle::Invalidated);
                }
            }
            let has_owner =
                artifact_task_records(state, &network_pair, pipeline_key, route, &proof_ref)?
                    .iter()
                    .any(|task| {
                        matches!(
                            task.runner_status,
                            RunnerStatus::Allocated
                                | RunnerStatus::Running
                                | RunnerStatus::Completed
                        )
                    });
            let mut next = record.clone();
            next.lifecycle = if has_owner {
                ProofArtifactLifecycle::Pending
            } else {
                ProofArtifactLifecycle::Invalidated
            };
            next.invalidated_at = (!has_owner).then(now_ts);
            state.artifacts.insert(key.clone(), next.clone());
            Ok(next.lifecycle)
        })
        .await
    }

    pub(crate) async fn register_invalidated_proof_artifact(
        &self,
        registration: ProofArtifactRegistration,
    ) -> Result<()> {
        let (key, record) =
            self.proof_artifact_record(registration, ProofArtifactLifecycle::Invalidated);
        self.mutate(move |state| {
            match state.artifacts.get_mut(&key) {
                Some(existing) if existing.descriptor() != record.descriptor() => {}
                Some(existing) => {
                    existing.lifecycle = ProofArtifactLifecycle::Invalidated;
                    existing.invalidated_at.get_or_insert_with(now_ts);
                    existing.updated_at = now_ts();
                }
                None => {
                    state.artifacts.insert(key.clone(), record.clone());
                }
            }
            Ok(())
        })
        .await
    }

    async fn upsert_proof_artifact_with_lifecycle(
        &self,
        registration: ProofArtifactRegistration,
        lifecycle: ProofArtifactLifecycle,
    ) -> Result<()> {
        let (key, record) = self.proof_artifact_record(registration, lifecycle);
        self.mutate(move |state| {
            if let Some(existing) = state.artifacts.get(&key) {
                anyhow::ensure!(
                    existing.descriptor() == record.descriptor(),
                    "proof artifact lifecycle descriptor conflict"
                );
                if existing.lifecycle == ProofArtifactLifecycle::Invalidated {
                    return Ok(());
                }
            }
            state.artifacts.insert(key.clone(), record.clone());
            Ok(())
        })
        .await
    }

    pub async fn reconcile_proof_artifact_registration(
        &self,
        registration: ProofArtifactRegistration,
    ) -> Result<bool> {
        let descriptor = registration.descriptor();
        if !self
            .proof_artifact_descriptor_is_current(
                &registration.network_pair,
                registration.pipeline_key,
                registration.route,
                &registration.proof_ref,
                &descriptor,
            )
            .await?
        {
            return Ok(false);
        }

        let key = artifact_record_key(
            &registration.network_pair,
            registration.pipeline_key,
            registration.route,
            &registration.proof_ref,
        );
        Ok(self
            .state
            .read()
            .await
            .artifacts
            .get(&key)
            .is_some_and(|existing| {
                existing.descriptor() == descriptor
                    && existing.lifecycle == ProofArtifactLifecycle::Active
            }))
    }

    fn proof_artifact_record(
        &self,
        registration: ProofArtifactRegistration,
        lifecycle: ProofArtifactLifecycle,
    ) -> (String, ProofArtifactRecord) {
        let key = artifact_record_key(
            &registration.network_pair,
            registration.pipeline_key,
            registration.route,
            &registration.proof_ref,
        );
        let invalidated_at = (lifecycle == ProofArtifactLifecycle::Invalidated).then(now_ts);
        let record = ProofArtifactRecord {
            environment: self.environment().to_string(),
            network_pair: registration.network_pair,
            proof_ref: registration.proof_ref,
            pipeline_key: registration.pipeline_key,
            route: registration.route,
            proof_uri: registration.proof_uri,
            content_hash: registration.content_hash,
            generation: registration.generation,
            lifecycle,
            invalidated_at,
            updated_at: now_ts(),
        };
        (key, record)
    }

    pub async fn get_proof_artifact(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<Option<ProofArtifactRecord>> {
        Ok(self
            .get_proof_artifact_including_invalidated(network_pair, pipeline_key, route, proof_ref)
            .await?
            .filter(|record| record.lifecycle == ProofArtifactLifecycle::Active))
    }

    /// Atomically updates all matching roots and activates their exact pending artifact.
    /// Returning `None` from `update` leaves both roots and artifact unchanged.
    pub async fn activate_proof_artifact_with_tasks<T, F>(
        &self,
        task_ref: &str,
        registration: ProofArtifactRegistration,
        owner_incarnations: &[uuid::Uuid],
        update: F,
    ) -> Result<Option<T>>
    where
        F: Fn(&mut Vec<RuntimeTaskRecord>) -> Result<Option<T>>,
    {
        let _pending_publication = self.pending_publication_mutation.lock().await;
        let task_ref = task_ref.to_string();
        let network_pair = registration.network_pair.clone();
        let pipeline_key = registration.pipeline_key;
        let route = registration.route;
        let descriptor = registration.descriptor();
        let key = artifact_record_key(
            &registration.network_pair,
            registration.pipeline_key,
            registration.route,
            &registration.proof_ref,
        );
        let owner_incarnations = owner_incarnations.to_vec();
        self.mutate(move |state| {
            let pending = state
                .pending_publications
                .get(&key)
                .context("pending proof publication ownership is missing")?;
            anyhow::ensure!(
                pending.content_hash == descriptor.content_hash
                    && pending.owner_incarnations == owner_incarnations,
                "pending proof publication ownership changed before activation"
            );
            let Some(artifact) = state.artifacts.get(&key) else {
                anyhow::bail!("proof artifact lifecycle registration is missing");
            };
            anyhow::ensure!(
                artifact.descriptor() == descriptor,
                "proof artifact lifecycle descriptor changed before activation"
            );
            anyhow::ensure!(
                artifact.lifecycle != ProofArtifactLifecycle::Invalidated,
                "proof artifact lifecycle was invalidated before activation"
            );

            let mut records =
                artifact_task_records(state, &network_pair, pipeline_key, route, &task_ref)?;
            records.sort_by(|left, right| left.task_id.cmp(&right.task_id));
            let Some(output) = update(&mut records)? else {
                return Ok(None);
            };
            for record in records {
                state.tasks.insert(record.task_id.clone(), record);
            }
            let artifact = state
                .artifacts
                .get_mut(&key)
                .context("proof artifact lifecycle registration disappeared")?;
            artifact.lifecycle = ProofArtifactLifecycle::Active;
            artifact.invalidated_at = None;
            artifact.updated_at = now_ts();
            state.pending_publications.remove(&key);
            Ok(Some(output))
        })
        .await
    }

    /// Atomically updates all matching roots and optionally invalidates their current artifact.
    pub async fn update_tasks_and_invalidate_artifact<T, F>(
        &self,
        task_ref: &str,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        update: F,
    ) -> Result<(T, bool, Option<ProofArtifactDescriptor>)>
    where
        F: Fn(&mut Vec<RuntimeTaskRecord>) -> Result<(T, bool)>,
    {
        let _pending_publication = self.pending_publication_mutation.lock().await;
        let task_ref = task_ref.to_string();
        let network_pair = network_pair.to_string();
        let key = artifact_record_key(&network_pair, pipeline_key, route, &task_ref);
        self.mutate(move |state| {
            let mut records =
                artifact_task_records(state, &network_pair, pipeline_key, route, &task_ref)?;
            records.sort_by(|left, right| left.task_id.cmp(&right.task_id));
            let (output, requested_invalidation) = update(&mut records)?;
            for record in records {
                state.tasks.insert(record.task_id.clone(), record);
            }
            let live_incarnations =
                artifact_task_records(state, &network_pair, pipeline_key, route, &task_ref)?
                    .into_iter()
                    .filter(|record| {
                        !matches!(
                            record.runner_status,
                            RunnerStatus::Failed | RunnerStatus::Cancelled
                        )
                    })
                    .map(|record| record.incarnation_id)
                    .collect::<Vec<_>>();
            if let Some(pending) = state.pending_publications.get_mut(&key) {
                pending
                    .owner_incarnations
                    .retain(|owner| live_incarnations.contains(owner));
                if pending.owner_incarnations.is_empty() {
                    state.pending_publications.remove(&key);
                }
            }
            let invalidate = requested_invalidation && live_incarnations.is_empty();
            let descriptor = if invalidate {
                state.pending_publications.remove(&key);
                state.artifacts.get_mut(&key).map(|artifact| {
                    if artifact.lifecycle == ProofArtifactLifecycle::Invalidated {
                        return artifact.descriptor();
                    }
                    artifact.lifecycle = ProofArtifactLifecycle::Invalidated;
                    artifact.invalidated_at.get_or_insert_with(now_ts);
                    artifact.updated_at = now_ts();
                    artifact.descriptor()
                })
            } else {
                None
            };
            Ok((output, invalidate, descriptor))
        })
        .await
    }

    pub async fn get_proof_artifact_including_invalidated(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<Option<ProofArtifactRecord>> {
        let key = artifact_record_key(network_pair, pipeline_key, route, proof_ref);
        Ok(self.state.read().await.artifacts.get(&key).cloned())
    }

    pub async fn list_proof_artifacts(&self) -> Result<Vec<ProofArtifactRecord>> {
        let mut artifacts = self
            .state
            .read()
            .await
            .artifacts
            .values()
            .cloned()
            .collect::<Vec<_>>();
        artifacts.sort_by_key(|record| {
            (
                record.updated_at,
                record.network_pair.clone(),
                record.proof_ref.clone(),
            )
        });
        Ok(artifacts)
    }

    pub async fn remove_proof_artifact_if_descriptor(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        descriptor: &ProofArtifactDescriptor,
    ) -> Result<bool> {
        let key = artifact_record_key(network_pair, pipeline_key, route, proof_ref);
        let expected = descriptor.clone();
        self.mutate(move |state| {
            if state
                .artifacts
                .get(&key)
                .is_some_and(|record| record.descriptor() == expected)
            {
                state.artifacts.remove(&key);
                Ok(true)
            } else {
                Ok(false)
            }
        })
        .await
    }

    pub async fn mark_proof_artifact_invalidated(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        content_hash: &str,
    ) -> Result<Option<ProofArtifactDeleteResult>> {
        let Some(record) = self
            .get_proof_artifact_including_invalidated(network_pair, pipeline_key, route, proof_ref)
            .await?
        else {
            return Ok(None);
        };
        if record.content_hash != content_hash {
            return Ok(None);
        }
        self.mark_proof_artifact_descriptor_invalidated(
            network_pair,
            pipeline_key,
            route,
            proof_ref,
            &record.descriptor(),
        )
        .await
    }

    pub async fn mark_proof_artifact_descriptor_invalidated(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        descriptor: &ProofArtifactDescriptor,
    ) -> Result<Option<ProofArtifactDeleteResult>> {
        let key = artifact_record_key(network_pair, pipeline_key, route, proof_ref);
        let expected_descriptor = descriptor.clone();
        let invalidated_generation = self
            .mutate(move |state| {
                let Some(record) = state.artifacts.get_mut(&key) else {
                    return Ok(None);
                };
                if record.descriptor() != expected_descriptor {
                    return Ok(None);
                }
                record.lifecycle = ProofArtifactLifecycle::Invalidated;
                record.invalidated_at.get_or_insert_with(now_ts);
                record.updated_at = now_ts();
                Ok(Some(record.generation))
            })
            .await?;
        let Some(generation) = invalidated_generation else {
            return Ok(None);
        };
        self.finalize_proof_artifact_invalidation(
            network_pair,
            pipeline_key,
            route,
            proof_ref,
            generation,
            &descriptor.content_hash,
        )
        .await
        .map(Some)
    }

    pub async fn invalidate_proof_artifact_descriptor_if_unowned(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        descriptor: &ProofArtifactDescriptor,
    ) -> Result<ProofArtifactInvalidationResult> {
        let _pending_publication = self.pending_publication_mutation.lock().await;
        let key = artifact_record_key(network_pair, pipeline_key, route, proof_ref);
        let expected_descriptor = descriptor.clone();
        let network_pair_owned = network_pair.to_string();
        let proof_ref_owned = proof_ref.to_string();
        let marked = self
            .mutate(move |state| {
                if !state
                    .artifacts
                    .get(&key)
                    .is_some_and(|record| record.descriptor() == expected_descriptor)
                {
                    return Ok(ProofArtifactInvalidationAdmission::MissingOrChanged);
                }
                let has_live_task = artifact_task_records(
                    state,
                    &network_pair_owned,
                    pipeline_key,
                    route,
                    &proof_ref_owned,
                )?
                .iter()
                .any(|task| {
                    !matches!(
                        task.runner_status,
                        RunnerStatus::Failed | RunnerStatus::Cancelled
                    )
                });
                if has_live_task {
                    return Ok(ProofArtifactInvalidationAdmission::BlockedByLiveTask);
                }
                let record = state
                    .artifacts
                    .get_mut(&key)
                    .context("proof artifact disappeared during invalidation")?;
                record.lifecycle = ProofArtifactLifecycle::Invalidated;
                record.invalidated_at.get_or_insert_with(now_ts);
                record.updated_at = now_ts();
                Ok(ProofArtifactInvalidationAdmission::Marked)
            })
            .await?;
        match marked {
            ProofArtifactInvalidationAdmission::BlockedByLiveTask => {
                return Ok(ProofArtifactInvalidationResult::BlockedByLiveTask);
            }
            ProofArtifactInvalidationAdmission::MissingOrChanged => {
                return Ok(ProofArtifactInvalidationResult::MissingOrChanged);
            }
            ProofArtifactInvalidationAdmission::Marked => {}
        }

        let delete_result = self
            .finalize_proof_artifact_invalidation(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
                descriptor.generation,
                &descriptor.content_hash,
            )
            .await?;
        self.remove_proof_artifact_if_descriptor(
            network_pair,
            pipeline_key,
            route,
            proof_ref,
            descriptor,
        )
        .await?;
        Ok(ProofArtifactInvalidationResult::Invalidated(delete_result))
    }

    pub async fn finalize_proof_artifact_invalidation(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        generation: Option<i64>,
        content_hash: &str,
    ) -> Result<ProofArtifactDeleteResult> {
        let object_key = Self::artifact_key(network_pair, pipeline_key, route, proof_ref);
        let _lifecycle = self.fence_external_mutation()?;
        self.store
            .mark_invalidated(&object_key, generation, content_hash)
            .await?;
        self.store
            .delete(&object_key, generation, content_hash)
            .await
    }

    /// Completes external tombstone/delete work for invalidations committed before a crash.
    pub async fn reconcile_invalidated_proof_artifacts(&self) -> Result<usize> {
        let invalidated = self
            .state
            .read()
            .await
            .artifacts
            .values()
            .filter(|record| record.lifecycle == ProofArtifactLifecycle::Invalidated)
            .cloned()
            .collect::<Vec<_>>();
        for record in &invalidated {
            self.finalize_proof_artifact_invalidation(
                &record.network_pair,
                record.pipeline_key,
                record.route,
                &record.proof_ref,
                record.generation,
                &record.content_hash,
            )
            .await?;
            if self
                .get_recoverable_pending_proof_publication(
                    &record.network_pair,
                    record.pipeline_key,
                    record.route,
                    &record.proof_ref,
                )
                .await?
                .is_none()
            {
                self.remove_pending_proof_publication_if_unowned(
                    &record.network_pair,
                    record.pipeline_key,
                    record.route,
                    &record.proof_ref,
                )
                .await?;
            }
        }
        Ok(invalidated.len())
    }

    pub async fn proof_artifact_is_invalidated(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        content_hash: &str,
    ) -> Result<bool> {
        let local_record = self
            .get_proof_artifact_including_invalidated(network_pair, pipeline_key, route, proof_ref)
            .await?;
        let Some(record) = local_record.filter(|record| record.content_hash == content_hash) else {
            return Ok(false);
        };
        self.proof_artifact_descriptor_is_invalidated(
            network_pair,
            pipeline_key,
            route,
            proof_ref,
            &record.descriptor(),
        )
        .await
    }

    pub async fn proof_artifact_descriptor_is_invalidated(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        descriptor: &ProofArtifactDescriptor,
    ) -> Result<bool> {
        let local_record = self
            .get_proof_artifact_including_invalidated(network_pair, pipeline_key, route, proof_ref)
            .await?;
        if local_record.as_ref().is_some_and(|record| {
            record.descriptor() == *descriptor
                && record.lifecycle == ProofArtifactLifecycle::Invalidated
        }) {
            return Ok(true);
        }
        let object_key = Self::artifact_key(network_pair, pipeline_key, route, proof_ref);
        self.store
            .is_invalidated(&object_key, descriptor.generation, &descriptor.content_hash)
            .await
    }

    pub async fn proof_artifact_descriptor_is_current(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        descriptor: &ProofArtifactDescriptor,
    ) -> Result<bool> {
        Ok(self
            .store
            .get_descriptor(&Self::artifact_key(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
            ))
            .await?
            .is_some_and(|current| current == *descriptor))
    }

    pub async fn upsert_pending_proof_publication(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        proof_bytes: &[u8],
    ) -> Result<()> {
        let _pending_publication = self.pending_publication_mutation.lock().await;
        match self
            .put_pending_proof_publication_bytes(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
                proof_bytes,
            )
            .await?
        {
            ProofArtifactPutResult::Created(_) | ProofArtifactPutResult::AlreadyExists(_) => Ok(()),
            ProofArtifactPutResult::Conflict(_) => {
                anyhow::bail!("different pending proof already exists")
            }
        }
    }

    async fn put_pending_proof_publication_bytes(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        proof_bytes: &[u8],
    ) -> Result<ProofArtifactPutResult> {
        let _lifecycle = self.fence_external_mutation()?;
        let key = Self::pending_artifact_key(network_pair, pipeline_key, route, proof_ref);
        self.store.put_if_absent(&key, proof_bytes).await
    }

    pub async fn checkpoint_pending_proof_publication(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        owner_incarnations: &[uuid::Uuid],
        proof_bytes: &[u8],
    ) -> Result<bool> {
        anyhow::ensure!(
            !owner_incarnations.is_empty(),
            "pending proof publication requires an owner incarnation"
        );
        let _pending_publication = self.pending_publication_mutation.lock().await;
        let mut publication = self
            .put_pending_proof_publication_bytes(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
                proof_bytes,
            )
            .await?;
        if matches!(publication, ProofArtifactPutResult::Conflict(_)) {
            anyhow::ensure!(
                self.remove_pending_proof_publication_if_unowned_locked(
                    network_pair,
                    pipeline_key,
                    route,
                    proof_ref,
                )
                .await?,
                "different pending proof is owned by another task incarnation"
            );
            publication = self
                .put_pending_proof_publication_bytes(
                    network_pair,
                    pipeline_key,
                    route,
                    proof_ref,
                    proof_bytes,
                )
                .await?;
        }
        anyhow::ensure!(
            !matches!(publication, ProofArtifactPutResult::Conflict(_)),
            "different pending proof still exists after orphan cleanup"
        );
        let key = artifact_record_key(network_pair, pipeline_key, route, proof_ref);
        let proof_ref_owned = proof_ref.to_string();
        let owner_incarnations = owner_incarnations.to_vec();
        let hash = artifact_store::content_hash(proof_bytes);
        let checkpointed = self
            .mutate(move |state| {
                let has_active_owner = state.tasks.values().any(|record| {
                    task_references(record, &proof_ref_owned)
                        && !matches!(
                            record.runner_status,
                            RunnerStatus::Failed | RunnerStatus::Cancelled
                        )
                        && owner_incarnations.contains(&record.incarnation_id)
                });
                if !has_active_owner {
                    return Ok(false);
                }
                state.pending_publications.insert(
                    key.clone(),
                    PendingProofPublicationRecord {
                        content_hash: hash.clone(),
                        owner_incarnations: owner_incarnations.clone(),
                    },
                );
                Ok(true)
            })
            .await?;
        if !checkpointed {
            self.remove_pending_proof_publication_if_unowned_locked(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
            )
            .await?;
        }
        Ok(checkpointed)
    }

    pub async fn get_pending_proof_publication(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<Option<Vec<u8>>> {
        let key = Self::pending_artifact_key(network_pair, pipeline_key, route, proof_ref);
        Ok(self.store.get(&key).await?.map(|object| object.bytes))
    }

    pub async fn get_recoverable_pending_proof_publication(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<Option<PendingProofPublication>> {
        let state_key = artifact_record_key(network_pair, pipeline_key, route, proof_ref);
        let proof_ref_owned = proof_ref.to_string();
        let state = self.state.read().await;
        let Some(record) = state
            .pending_publications
            .get(&state_key)
            .filter(|pending| {
                state.tasks.values().any(|task| {
                    task_references(task, &proof_ref_owned)
                        && !matches!(
                            task.runner_status,
                            RunnerStatus::Failed | RunnerStatus::Cancelled
                        )
                        && pending.owner_incarnations.contains(&task.incarnation_id)
                })
            })
            .cloned()
        else {
            return Ok(None);
        };
        drop(state);
        let Some(bytes) = self
            .get_pending_proof_publication(network_pair, pipeline_key, route, proof_ref)
            .await?
        else {
            return Ok(None);
        };
        if artifact_store::content_hash(&bytes) != record.content_hash {
            anyhow::bail!("pending proof publication content hash mismatch");
        }
        Ok(Some(PendingProofPublication {
            bytes,
            owner_incarnations: record.owner_incarnations,
        }))
    }

    pub async fn remove_pending_proof_publication(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<bool> {
        let _pending_publication = self.pending_publication_mutation.lock().await;
        let state_key = artifact_record_key(network_pair, pipeline_key, route, proof_ref);
        self.mutate(move |state| {
            state.pending_publications.remove(&state_key);
            Ok(())
        })
        .await?;
        self.remove_local_pending(network_pair, pipeline_key, route, proof_ref)
            .await
    }

    pub async fn remove_pending_proof_publication_if_unowned(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<bool> {
        let _pending_publication = self.pending_publication_mutation.lock().await;
        self.remove_pending_proof_publication_if_unowned_locked(
            network_pair,
            pipeline_key,
            route,
            proof_ref,
        )
        .await
    }

    async fn remove_pending_proof_publication_if_unowned_locked(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<bool> {
        let state_key = artifact_record_key(network_pair, pipeline_key, route, proof_ref);
        let proof_ref_owned = proof_ref.to_string();
        let unowned = self
            .mutate(move |state| {
                let live_owners =
                    state
                        .pending_publications
                        .get(&state_key)
                        .is_some_and(|pending| {
                            pending.owner_incarnations.iter().any(|owner| {
                                state.tasks.values().any(|task| {
                                    task.incarnation_id == *owner
                                        && task_references(task, &proof_ref_owned)
                                        && !matches!(
                                            task.runner_status,
                                            RunnerStatus::Failed | RunnerStatus::Cancelled
                                        )
                                })
                            })
                        });
                if live_owners {
                    return Ok(false);
                }
                state.pending_publications.remove(&state_key);
                Ok(true)
            })
            .await?;
        if !unowned {
            return Ok(false);
        }
        self.remove_local_pending(network_pair, pipeline_key, route, proof_ref)
            .await
    }

    pub async fn release_pending_proof_publication_owner(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        owner_incarnation: uuid::Uuid,
    ) -> Result<bool> {
        let _pending_publication = self.pending_publication_mutation.lock().await;
        let state_key = artifact_record_key(network_pair, pipeline_key, route, proof_ref);
        self.mutate(move |state| {
            if let Some(pending) = state.pending_publications.get_mut(&state_key) {
                pending
                    .owner_incarnations
                    .retain(|owner| *owner != owner_incarnation);
                if pending.owner_incarnations.is_empty() {
                    state.pending_publications.remove(&state_key);
                }
            }
            Ok(())
        })
        .await?;
        self.remove_pending_proof_publication_if_unowned_locked(
            network_pair,
            pipeline_key,
            route,
            proof_ref,
        )
        .await
    }

    async fn remove_local_pending(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<bool> {
        let key = Self::pending_artifact_key(network_pair, pipeline_key, route, proof_ref);
        let Some(object) = self.store.get(&key).await? else {
            return Ok(false);
        };
        let _lifecycle = self.fence_external_mutation()?;
        self.store
            .delete(&key, object.generation, &object.content_hash)
            .await?;
        Ok(true)
    }

    /// Invalidates the publication only if the authoritative state still has no live owner.
    /// Returns `false` when a live owner prevents invalidation.
    pub async fn invalidate_pending_proof_publication(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<bool> {
        let _pending_publication = self.pending_publication_mutation.lock().await;
        let object_key = Self::artifact_key(network_pair, pipeline_key, route, proof_ref);
        let canonical = self.store.get(&object_key).await?;
        let state_key = artifact_record_key(network_pair, pipeline_key, route, proof_ref);
        let network_pair_owned = network_pair.to_string();
        let proof_ref_owned = proof_ref.to_string();
        let canonical_record = canonical.as_ref().map(|object| {
            self.proof_artifact_record(
                ProofArtifactRegistration {
                    network_pair: network_pair.to_string(),
                    proof_ref: proof_ref.to_string(),
                    pipeline_key,
                    route,
                    proof_uri: object.proof_uri.clone(),
                    content_hash: object.content_hash.clone(),
                    generation: object.generation,
                },
                ProofArtifactLifecycle::Invalidated,
            )
            .1
        });
        let invalidated = self
            .mutate(move |state| {
                let has_live_owner = artifact_task_records(
                    state,
                    &network_pair_owned,
                    pipeline_key,
                    route,
                    &proof_ref_owned,
                )?
                .iter()
                .any(|task| {
                    !matches!(
                        task.runner_status,
                        RunnerStatus::Failed | RunnerStatus::Cancelled
                    )
                });
                if has_live_owner {
                    return Ok(false);
                }
                state.pending_publications.remove(&state_key);
                if let Some(canonical_record) = canonical_record.as_ref() {
                    match state.artifacts.get_mut(&state_key) {
                        None => {
                            state
                                .artifacts
                                .insert(state_key.clone(), canonical_record.clone());
                        }
                        Some(record) if record.descriptor() == canonical_record.descriptor() => {
                            record.lifecycle = ProofArtifactLifecycle::Invalidated;
                            record.invalidated_at.get_or_insert_with(now_ts);
                            record.updated_at = now_ts();
                        }
                        Some(_) => {}
                    }
                }
                Ok(true)
            })
            .await?;
        if !invalidated {
            return Ok(false);
        }

        if let Some(object) = canonical.as_ref() {
            self.finalize_proof_artifact_invalidation(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
                object.generation,
                &object.content_hash,
            )
            .await?;
        }
        let pending_key = Self::pending_artifact_key(network_pair, pipeline_key, route, proof_ref);
        let pending = self.store.get(&pending_key).await?;
        if let Some(pending) = pending.as_ref() {
            let _lifecycle = self.fence_external_mutation()?;
            self.store
                .delete(&pending_key, pending.generation, &pending.content_hash)
                .await?;
        }
        Ok(true)
    }

    fn pending_artifact_key(
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> ProofArtifactKey {
        Self::artifact_key(
            network_pair,
            pipeline_key,
            route,
            &format!("__pending__:{proof_ref}"),
        )
    }

    pub async fn list_tasks(&self) -> Result<Vec<RuntimeTaskRecord>> {
        let mut tasks = self
            .state
            .read()
            .await
            .tasks
            .values()
            .cloned()
            .collect::<Vec<_>>();
        tasks.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.task_id.cmp(&right.task_id))
        });
        Ok(tasks)
    }

    pub async fn list_expired_terminal_tasks(
        &self,
        now: i64,
        ttl_secs: u64,
        after: Option<&ExpiredTaskCursor>,
        limit: usize,
    ) -> Result<Vec<RuntimeTaskRecord>> {
        self.list_tasks_matching(now, ttl_secs, after, limit, true)
            .await
    }

    pub async fn list_stale_nonterminal_tasks(
        &self,
        now: i64,
        ttl_secs: u64,
        after: Option<&ExpiredTaskCursor>,
        limit: usize,
    ) -> Result<Vec<RuntimeTaskRecord>> {
        self.list_tasks_matching(now, ttl_secs, after, limit, false)
            .await
    }

    async fn list_tasks_matching(
        &self,
        now: i64,
        ttl_secs: u64,
        after: Option<&ExpiredTaskCursor>,
        limit: usize,
        terminal: bool,
    ) -> Result<Vec<RuntimeTaskRecord>> {
        if ttl_secs == 0 || limit == 0 {
            return Ok(Vec::new());
        }
        let cutoff = now.saturating_sub(i64::try_from(ttl_secs).unwrap_or(i64::MAX));
        let mut tasks = self
            .state
            .read()
            .await
            .tasks
            .values()
            .filter(|task| {
                task.runner_status.is_terminal() == terminal
                    && task.updated_at <= cutoff
                    && after.is_none_or(|cursor| {
                        (task.updated_at, task.task_id.as_str())
                            > (cursor.updated_at, cursor.task_id.as_str())
                    })
            })
            .cloned()
            .collect::<Vec<_>>();
        tasks.sort_by(|left, right| {
            left.updated_at
                .cmp(&right.updated_at)
                .then_with(|| left.task_id.cmp(&right.task_id))
        });
        tasks.truncate(limit);
        Ok(tasks)
    }

    pub async fn remove_task(&self, task_id: &str) -> Result<bool> {
        self.remove_task_matching_incarnation(task_id, None).await
    }

    pub async fn remove_task_if_incarnation(
        &self,
        task_id: &str,
        expected_incarnation: uuid::Uuid,
    ) -> Result<bool> {
        self.remove_task_matching_incarnation(task_id, Some(expected_incarnation))
            .await
    }

    async fn remove_task_matching_incarnation(
        &self,
        task_id: &str,
        expected_incarnation: Option<uuid::Uuid>,
    ) -> Result<bool> {
        let _pending_publication = self.pending_publication_mutation.lock().await;
        let task_id = task_id.to_string();
        self.mutate(move |state| {
            let Some(current) = state.tasks.get(&task_id) else {
                return Ok(false);
            };
            if expected_incarnation.is_some_and(|expected| current.incarnation_id != expected) {
                return Ok(false);
            }
            let removed = state
                .tasks
                .remove(&task_id)
                .expect("task checked above must still exist during mutation");
            for pending in state.pending_publications.values_mut() {
                pending
                    .owner_incarnations
                    .retain(|owner| *owner != removed.incarnation_id);
            }
            state
                .pending_publications
                .retain(|_, pending| !pending.owner_incarnations.is_empty());
            Ok(true)
        })
        .await
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRegistration {
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipeline_key: Option<PipelineKey>,
    pub route: PipelineRoute,
    pub task_kind: String,
    pub proposal_id: Option<u64>,
    #[serde(default)]
    pub proof_ids: Vec<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeTaskRecord {
    pub task_id: String,
    /// Immutable identity for this concrete task lifetime; never reused after replacement.
    pub incarnation_id: uuid::Uuid,
    pub pipeline_key: PipelineKey,
    pub route: PipelineRoute,
    pub task_kind: String,
    pub proposal_id: Option<u64>,
    pub proof_ids: Vec<String>,
    pub runner_status: RunnerStatus,
    pub image_ref: Option<String>,
    pub proof_uri: Option<String>,
    pub error: Option<String>,
    pub metadata: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_fingerprint: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunnerStatus {
    Allocated,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl RunnerStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allocated => "allocated",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

impl std::fmt::Display for RunnerStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

fn build_task_record(registration: &TaskRegistration) -> Result<RuntimeTaskRecord> {
    let pipeline_key = registration.pipeline_key.map_or_else(
        || {
            registration
                .route
                .pipeline_key()
                .map_err(anyhow::Error::msg)
        },
        Ok,
    )?;
    anyhow::ensure!(
        pipeline_key_matches_route(pipeline_key, registration.route),
        "pipeline_key '{}' does not match route '{}'",
        pipeline_key.as_str(),
        registration.route
    );
    Ok(RuntimeTaskRecord {
        task_id: registration.task_id.clone(),
        incarnation_id: uuid::Uuid::new_v4(),
        pipeline_key,
        route: registration.route,
        task_kind: registration.task_kind.clone(),
        proposal_id: registration.proposal_id,
        proof_ids: registration.proof_ids.clone(),
        runner_status: RunnerStatus::Allocated,
        image_ref: None,
        proof_uri: None,
        error: None,
        metadata: registration.metadata.clone(),
        request_fingerprint: registration.request_fingerprint.clone(),
        updated_at: now_ts(),
    })
}

fn pipeline_key_matches_route(pipeline_key: PipelineKey, route: PipelineRoute) -> bool {
    matches!(
        (pipeline_key, route),
        (
            PipelineKey::ShastaSgx | PipelineKey::ShastaSgxGeth,
            PipelineRoute {
                guest_system: raiko2_pipeline::GuestSystem::Sgx,
                runner: raiko2_pipeline::RunnerKind::Remote,
            }
        )
    ) || route.pipeline_key() == Ok(pipeline_key)
}

fn artifact_record_key(
    network_pair: &str,
    pipeline_key: PipelineKey,
    route: PipelineRoute,
    proof_ref: &str,
) -> String {
    format!(
        "{network_pair}|{}|{route}|{proof_ref}",
        pipeline_key.as_str()
    )
}

fn ensure_artifact_preconditions(
    state: &RuntimeState,
    artifact_preconditions: &[ProofArtifactPrecondition],
) -> Result<()> {
    for expected in artifact_preconditions {
        let key = artifact_record_key(
            &expected.network_pair,
            expected.pipeline_key,
            expected.route,
            &expected.proof_ref,
        );
        let Some(artifact) = state.artifacts.get(&key) else {
            anyhow::bail!("cached proof artifact disappeared before task admission");
        };
        if artifact.lifecycle != ProofArtifactLifecycle::Active
            || artifact.descriptor() != expected.descriptor
        {
            anyhow::bail!("cached proof artifact changed before task admission");
        }
    }
    Ok(())
}

fn ensure_task_fingerprint_available(
    state: &RuntimeState,
    record: &RuntimeTaskRecord,
) -> Result<()> {
    if let Some(fingerprint) = record.request_fingerprint.as_deref()
        && state.tasks.values().any(|task| {
            task.task_id != record.task_id
                && task.request_fingerprint.as_deref() == Some(fingerprint)
        })
    {
        anyhow::bail!("request fingerprint already belongs to another task");
    }
    Ok(())
}

fn task_references(record: &RuntimeTaskRecord, task_ref: &str) -> bool {
    record.task_id == task_ref
        || record.proof_ids.iter().any(|id| id == task_ref)
        || record
            .metadata
            .get("aggregate_task_id")
            .and_then(serde_json::Value::as_str)
            == Some(task_ref)
        || record
            .metadata
            .get("aggregate_input_artifacts")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|artifacts| {
                artifacts.iter().any(|artifact| {
                    artifact
                        .get("proof_ref")
                        .and_then(serde_json::Value::as_str)
                        == Some(task_ref)
                })
            })
}

fn task_network_pair(record: &RuntimeTaskRecord) -> Option<&str> {
    record
        .metadata
        .get("network_pair")
        .and_then(serde_json::Value::as_str)
}

fn task_matches_artifact_identity(
    record: &RuntimeTaskRecord,
    network_pair: &str,
    pipeline_key: PipelineKey,
    route: PipelineRoute,
    proof_ref: &str,
) -> Result<bool> {
    if !task_references(record, proof_ref)
        || record.pipeline_key != pipeline_key
        || record.route != route
    {
        return Ok(false);
    }
    Ok(
        task_network_pair(record).context("runtime task metadata is missing network_pair")?
            == network_pair,
    )
}

fn artifact_task_records(
    state: &RuntimeState,
    network_pair: &str,
    pipeline_key: PipelineKey,
    route: PipelineRoute,
    proof_ref: &str,
) -> Result<Vec<RuntimeTaskRecord>> {
    state
        .tasks
        .values()
        .filter_map(|record| {
            match task_matches_artifact_identity(
                record,
                network_pair,
                pipeline_key,
                route,
                proof_ref,
            ) {
                Ok(true) => Some(Ok(record.clone())),
                Ok(false) => None,
                Err(error) => Some(Err(error)),
            }
        })
        .collect()
}

fn set_json_integer(value: &mut serde_json::Value, fields: &[String], integer: i64) -> Result<()> {
    let Some((head, tail)) = fields.split_first() else {
        anyhow::bail!("metadata path must contain a field");
    };
    if tail.is_empty() {
        let object = value
            .as_object_mut()
            .context("metadata path parent is not an object")?;
        object.insert(head.clone(), integer.into());
        return Ok(());
    }
    let object = value
        .as_object_mut()
        .context("metadata path parent is not an object")?;
    let child = object
        .entry(head.clone())
        .or_insert_with(|| serde_json::json!({}));
    set_json_integer(child, tail, integer)
}

fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .cast_signed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[derive(Debug)]
    struct RuntimeStateProbeStore {
        inner: MemoryProofArtifactStore,
        runtime_state_writes: AtomicUsize,
        force_conflict: AtomicBool,
        commit_then_error: AtomicBool,
        foreign_commit_then_error: AtomicBool,
        fail_before_commit: AtomicUsize,
        block_next_runtime_state_write: AtomicBool,
        runtime_state_write_entered: tokio::sync::Notify,
        allow_runtime_state_write: tokio::sync::Notify,
        block_next_artifact_put: AtomicBool,
        artifact_put_entered: tokio::sync::Notify,
        allow_artifact_put: tokio::sync::Notify,
        block_next_artifact_delete: AtomicBool,
        artifact_delete_completed: tokio::sync::Notify,
        allow_artifact_delete_return: tokio::sync::Notify,
    }

    impl RuntimeStateProbeStore {
        fn new(namespace: &str) -> Result<Self> {
            Ok(Self {
                inner: MemoryProofArtifactStore::new("test".into(), namespace.into())?,
                runtime_state_writes: AtomicUsize::new(0),
                force_conflict: AtomicBool::new(false),
                commit_then_error: AtomicBool::new(false),
                foreign_commit_then_error: AtomicBool::new(false),
                fail_before_commit: AtomicUsize::new(0),
                block_next_runtime_state_write: AtomicBool::new(false),
                runtime_state_write_entered: tokio::sync::Notify::new(),
                allow_runtime_state_write: tokio::sync::Notify::new(),
                block_next_artifact_put: AtomicBool::new(false),
                artifact_put_entered: tokio::sync::Notify::new(),
                allow_artifact_put: tokio::sync::Notify::new(),
                block_next_artifact_delete: AtomicBool::new(false),
                artifact_delete_completed: tokio::sync::Notify::new(),
                allow_artifact_delete_return: tokio::sync::Notify::new(),
            })
        }
    }

    #[async_trait::async_trait]
    impl ProofArtifactStore for RuntimeStateProbeStore {
        fn environment(&self) -> &str {
            self.inner.environment()
        }

        fn namespace(&self) -> &str {
            self.inner.namespace()
        }

        fn backend_name(&self) -> &'static str {
            "test"
        }

        async fn put_if_absent(
            &self,
            key: &ProofArtifactKey,
            bytes: &[u8],
        ) -> Result<ProofArtifactPutResult> {
            if self.block_next_artifact_put.swap(false, Ordering::SeqCst) {
                self.artifact_put_entered.notify_one();
                self.allow_artifact_put.notified().await;
            }
            self.inner.put_if_absent(key, bytes).await
        }

        async fn get(&self, key: &ProofArtifactKey) -> Result<Option<ProofArtifactObject>> {
            self.inner.get(key).await
        }

        async fn get_descriptor(
            &self,
            key: &ProofArtifactKey,
        ) -> Result<Option<ProofArtifactDescriptor>> {
            self.inner.get_descriptor(key).await
        }

        async fn get_prefix(
            &self,
            key: &ProofArtifactKey,
            max_bytes: usize,
        ) -> Result<Option<ProofArtifactPrefix>> {
            self.inner.get_prefix(key, max_bytes).await
        }

        async fn mark_invalidated(
            &self,
            key: &ProofArtifactKey,
            generation: Option<i64>,
            content_hash: &str,
        ) -> Result<()> {
            self.inner
                .mark_invalidated(key, generation, content_hash)
                .await
        }

        async fn is_invalidated(
            &self,
            key: &ProofArtifactKey,
            generation: Option<i64>,
            content_hash: &str,
        ) -> Result<bool> {
            self.inner
                .is_invalidated(key, generation, content_hash)
                .await
        }

        async fn delete(
            &self,
            key: &ProofArtifactKey,
            generation: Option<i64>,
            expected_content_hash: &str,
        ) -> Result<ProofArtifactDeleteResult> {
            let result = self
                .inner
                .delete(key, generation, expected_content_hash)
                .await?;
            if self
                .block_next_artifact_delete
                .swap(false, Ordering::SeqCst)
            {
                self.artifact_delete_completed.notify_one();
                self.allow_artifact_delete_return.notified().await;
            }
            Ok(result)
        }

        async fn load_runtime_state(&self) -> Result<Option<RuntimeStateObject>> {
            self.inner.load_runtime_state().await
        }

        async fn store_runtime_state(
            &self,
            bytes: &[u8],
            expected_generation: Option<i64>,
        ) -> Result<RuntimeStateWriteResult> {
            self.runtime_state_writes.fetch_add(1, Ordering::SeqCst);
            if self
                .block_next_runtime_state_write
                .swap(false, Ordering::SeqCst)
            {
                self.runtime_state_write_entered.notify_one();
                self.allow_runtime_state_write.notified().await;
            }
            if self
                .fail_before_commit
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                anyhow::bail!("injected transport error before commit");
            }
            if self.force_conflict.load(Ordering::SeqCst) {
                return Ok(RuntimeStateWriteResult::Conflict(Some(
                    RuntimeStateObject {
                        bytes: serde_json::to_vec(&RuntimeState::default())?,
                        generation: Some(99),
                    },
                )));
            }
            if self.commit_then_error.swap(false, Ordering::SeqCst) {
                self.inner
                    .store_runtime_state(bytes, expected_generation)
                    .await?;
                anyhow::bail!("injected transport error after commit");
            }
            if self.foreign_commit_then_error.swap(false, Ordering::SeqCst) {
                self.inner
                    .store_runtime_state(
                        &serde_json::to_vec(&RuntimeState::default())?,
                        expected_generation,
                    )
                    .await?;
                anyhow::bail!("injected transport error after foreign commit");
            }
            self.inner
                .store_runtime_state(bytes, expected_generation)
                .await
        }
    }

    #[tokio::test]
    async fn memory_runtime_deduplicates_request_fingerprint() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "one".into())?;
        let registration = TaskRegistration {
            task_id: "task-a".into(),
            pipeline_key: Some(PipelineKey::ShastaNative),
            route: PipelineKey::ShastaNative.route(),
            task_kind: "proposal".into(),
            proposal_id: Some(1),
            proof_ids: Vec::new(),
            metadata: serde_json::json!({}),
            request_fingerprint: Some("same".into()),
        };
        assert!(matches!(
            runtime
                .register_task_if_absent(registration.clone())
                .await?,
            TaskRegistrationOutcome::Created(_)
        ));
        assert!(matches!(
            runtime
                .register_task_if_absent(TaskRegistration {
                    task_id: "task-b".into(),
                    ..registration
                })
                .await?,
            TaskRegistrationOutcome::Existing(_)
        ));
        Ok(())
    }

    #[tokio::test]
    async fn inactive_runtime_is_not_ready() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "inactive".into())?;
        runtime.begin_draining().await;
        assert!(runtime.check_readiness().await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn draining_waits_for_every_submission_checkpoint_permit() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new_memory(
            "test".into(),
            "checkpoint-permits".into(),
        )?);
        runtime
            .register_task(TaskRegistration {
                task_id: "checkpoint-root".into(),
                pipeline_key: Some(PipelineKey::ShastaSp1),
                route: PipelineKey::ShastaSp1.route(),
                task_kind: "proposal".into(),
                proposal_id: Some(1),
                proof_ids: Vec::new(),
                metadata: serde_json::json!({}),
                request_fingerprint: None,
            })
            .await?;
        let first = runtime.acquire_submission_checkpoint_permit()?;
        let second = runtime.acquire_submission_checkpoint_permit()?;
        let mut draining = tokio::spawn({
            let runtime = Arc::clone(&runtime);
            async move { runtime.begin_draining().await }
        });

        let mut admission_closed = false;
        for _ in 0..100 {
            if let Ok(permit) = runtime.acquire_submission_checkpoint_permit() {
                drop(permit);
            } else {
                admission_closed = true;
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(admission_closed);
        assert!(!runtime.accepts_mutations());
        assert!(runtime.remove_task("checkpoint-root").await.is_err());
        runtime
            .checkpoint_tasks_by_ref(&first, "checkpoint-root", |records| {
                records[0].image_ref = Some("checkpoint-persisted".into());
                Ok(())
            })
            .await?;
        assert_eq!(
            runtime
                .get_task("checkpoint-root")
                .await?
                .and_then(|record| record.image_ref),
            Some("checkpoint-persisted".into())
        );

        drop(first);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut draining)
                .await
                .is_err(),
            "one remaining permit must keep the runtime active"
        );
        drop(second);
        draining.await?;

        assert!(!runtime.accepts_mutations());
        Ok(())
    }

    #[tokio::test]
    async fn draining_waits_for_global_lifecycle_operation() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new_memory(
            "test".into(),
            "lifecycle-operation-drain".into(),
        )?);
        let operation = runtime.acquire_lifecycle_operation().await?;
        let mut draining = tokio::spawn({
            let runtime = Arc::clone(&runtime);
            async move { runtime.begin_draining().await }
        });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut draining)
                .await
                .is_err()
        );
        drop(operation);
        draining.await?;
        assert!(runtime.acquire_lifecycle_operation().await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn pre_admitted_checkpoint_recovers_while_runtime_is_draining() -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new("checkpoint-drain-retry")?);
        let runtime = RuntimeManager::with_store(store.clone())?;
        runtime
            .register_task(TaskRegistration {
                task_id: "checkpoint-root".into(),
                pipeline_key: Some(PipelineKey::ShastaSp1),
                route: PipelineKey::ShastaSp1.route(),
                task_kind: "proposal".into(),
                proposal_id: Some(1),
                proof_ids: Vec::new(),
                metadata: serde_json::json!({}),
                request_fingerprint: None,
            })
            .await?;
        let permit = runtime.acquire_submission_checkpoint_permit()?;
        store.fail_before_commit.store(3, Ordering::SeqCst);
        runtime
            .checkpoint_tasks_by_ref(&permit, "checkpoint-root", |records| {
                records[0].image_ref = Some("checkpoint".into());
                Ok(())
            })
            .await
            .expect_err("transient writes must exhaust the first checkpoint attempt");
        assert!(runtime.mutation_failure_is_retryable());

        runtime.start_draining();
        runtime
            .checkpoint_tasks_by_ref(&permit, "checkpoint-root", |records| {
                records[0].image_ref = Some("checkpoint".into());
                Ok(())
            })
            .await?;
        assert_eq!(
            runtime
                .get_task("checkpoint-root")
                .await?
                .and_then(|record| record.image_ref),
            Some("checkpoint".into())
        );
        drop(permit);
        runtime.begin_draining().await;
        Ok(())
    }

    #[tokio::test]
    async fn checkpoint_drain_deadline_forces_runtime_inactive() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "checkpoint-timeout".into())?;
        let permit = runtime.acquire_submission_checkpoint_permit()?;

        let drained = runtime
            .begin_draining_with_deadline(Instant::now() + std::time::Duration::from_millis(20))
            .await;

        assert!(!drained);
        assert!(!runtime.accepts_mutations());
        assert!(runtime.acquire_submission_checkpoint_permit().is_err());
        assert!(
            runtime
                .checkpoint_tasks_by_ref(&permit, "missing", |_| Ok(()))
                .await
                .is_err(),
            "deadline expiry must permanently fence a previously admitted permit"
        );
        drop(permit);
        Ok(())
    }

    #[tokio::test]
    async fn checkpoint_finishing_after_drain_deadline_is_not_installed() -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new("checkpoint-write-timeout")?);
        let runtime = Arc::new(RuntimeManager::with_store(store.clone())?);
        runtime
            .register_task(TaskRegistration {
                task_id: "checkpoint-root".into(),
                pipeline_key: Some(PipelineKey::ShastaSp1),
                route: PipelineKey::ShastaSp1.route(),
                task_kind: "proposal".into(),
                proposal_id: Some(1),
                proof_ids: Vec::new(),
                metadata: serde_json::json!({}),
                request_fingerprint: None,
            })
            .await?;
        let permit = runtime.acquire_submission_checkpoint_permit()?;
        store
            .block_next_runtime_state_write
            .store(true, Ordering::SeqCst);
        let checkpoint = tokio::spawn({
            let runtime = Arc::clone(&runtime);
            async move {
                runtime
                    .checkpoint_tasks_by_ref(&permit, "checkpoint-root", |records| {
                        records[0].image_ref = Some("late-checkpoint".into());
                        Ok(())
                    })
                    .await
            }
        });
        store.runtime_state_write_entered.notified().await;

        assert!(
            !runtime
                .begin_draining_with_deadline(
                    Instant::now() + std::time::Duration::from_millis(20),
                )
                .await
        );
        store.allow_runtime_state_write.notify_one();
        let error = checkpoint
            .await?
            .expect_err("late checkpoint must not report success");
        assert!(format!("{error:#}").contains("runtime checkpoint deadline elapsed"));
        assert_eq!(
            runtime
                .get_task("checkpoint-root")
                .await?
                .and_then(|record| record.image_ref),
            None
        );
        assert!(!runtime.mutation_failure_is_retryable());
        assert!(runtime.acquire_submission_checkpoint_permit().is_err());
        Ok(())
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn draining_fences_every_artifact_lifecycle_write() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "artifact-drain-matrix".into())?;
        let registration = |proof_ref: &str, generation: i64| ProofArtifactRegistration {
            network_pair: "l1-l2".into(),
            proof_ref: proof_ref.into(),
            pipeline_key: PipelineKey::ShastaSp1,
            route: PipelineKey::ShastaSp1.route(),
            proof_uri: format!("memory://{proof_ref}"),
            content_hash: format!("hash-{proof_ref}"),
            generation: Some(generation),
        };
        runtime
            .register_task(TaskRegistration {
                task_id: "root".into(),
                pipeline_key: Some(PipelineKey::ShastaSp1),
                route: PipelineKey::ShastaSp1.route(),
                task_kind: "proposal".into(),
                proposal_id: Some(1),
                proof_ids: vec!["pending".into()],
                metadata: serde_json::json!({"network_pair": "l1-l2"}),
                request_fingerprint: None,
            })
            .await?;
        let active = registration("active", 1);
        let mut pending = registration("pending", 2);
        pending.content_hash = artifact_store::content_hash(b"pending-proof");
        let invalidated = registration("invalidated", 3);
        runtime.upsert_proof_artifact(active.clone()).await?;
        assert_eq!(
            runtime
                .register_pending_proof_artifact(pending.clone())
                .await?,
            ProofArtifactLifecycle::Pending
        );
        runtime
            .register_invalidated_proof_artifact(invalidated.clone())
            .await?;
        let owner = runtime
            .get_task("root")
            .await?
            .context("runtime root")?
            .incarnation_id;
        assert!(
            runtime
                .checkpoint_pending_proof_publication(
                    "l1-l2",
                    PipelineKey::ShastaSp1,
                    PipelineKey::ShastaSp1.route(),
                    "pending",
                    &[owner],
                    b"pending-proof",
                )
                .await?
        );
        let state = runtime.state.read().await;
        let before_artifacts = state.artifacts.clone();
        let before_tasks = state.tasks.clone();
        let before_pending = state.pending_publications.clone();
        drop(state);
        let before_raw = runtime
            .get_pending_proof_publication(
                "l1-l2",
                PipelineKey::ShastaSp1,
                PipelineKey::ShastaSp1.route(),
                "pending",
            )
            .await?;

        runtime.start_draining();
        assert!(runtime.upsert_proof_artifact(active.clone()).await.is_err());
        assert!(
            runtime
                .register_pending_proof_artifact(pending.clone())
                .await
                .is_err()
        );
        assert!(
            runtime
                .register_invalidated_proof_artifact(invalidated.clone())
                .await
                .is_err()
        );
        assert!(
            runtime
                .mark_proof_artifact_descriptor_invalidated(
                    &active.network_pair,
                    active.pipeline_key,
                    active.route,
                    &active.proof_ref,
                    &active.descriptor(),
                )
                .await
                .is_err()
        );
        assert!(
            runtime
                .remove_proof_artifact_if_descriptor(
                    &active.network_pair,
                    active.pipeline_key,
                    active.route,
                    &active.proof_ref,
                    &active.descriptor(),
                )
                .await
                .is_err()
        );
        assert!(
            runtime
                .activate_proof_artifact_with_tasks("pending", pending.clone(), &[owner], |_| Ok(
                    Some(())
                ),)
                .await
                .is_err()
        );
        assert!(
            runtime
                .update_tasks_and_invalidate_artifact(
                    "pending",
                    "l1-l2",
                    PipelineKey::ShastaSp1,
                    PipelineKey::ShastaSp1.route(),
                    |_| Ok(((), true)),
                )
                .await
                .is_err()
        );
        assert!(
            runtime
                .checkpoint_pending_proof_publication(
                    "l1-l2",
                    PipelineKey::ShastaSp1,
                    PipelineKey::ShastaSp1.route(),
                    "pending",
                    &[owner],
                    b"pending-proof",
                )
                .await
                .is_err()
        );
        assert!(
            runtime
                .release_pending_proof_publication_owner(
                    "l1-l2",
                    PipelineKey::ShastaSp1,
                    PipelineKey::ShastaSp1.route(),
                    "pending",
                    owner,
                )
                .await
                .is_err()
        );
        let state = runtime.state.read().await;
        assert_eq!(state.artifacts, before_artifacts);
        assert_eq!(state.tasks, before_tasks);
        assert_eq!(state.pending_publications, before_pending);
        drop(state);
        assert_eq!(
            runtime
                .get_pending_proof_publication(
                    "l1-l2",
                    PipelineKey::ShastaSp1,
                    PipelineKey::ShastaSp1.route(),
                    "pending",
                )
                .await?,
            before_raw
        );
        runtime.begin_draining().await;
        assert!(runtime.upsert_proof_artifact(active).await.is_err());
        assert!(
            runtime
                .register_pending_proof_artifact(pending.clone())
                .await
                .is_err()
        );
        assert!(
            runtime
                .register_invalidated_proof_artifact(invalidated)
                .await
                .is_err()
        );
        assert!(
            runtime
                .activate_proof_artifact_with_tasks("pending", pending, &[owner], |_| Ok(Some(())))
                .await
                .is_err()
        );
        assert!(
            runtime
                .update_tasks_and_invalidate_artifact(
                    "pending",
                    "l1-l2",
                    PipelineKey::ShastaSp1,
                    PipelineKey::ShastaSp1.route(),
                    |_| Ok(((), true)),
                )
                .await
                .is_err()
        );
        let state = runtime.state.read().await;
        assert_eq!(state.artifacts, before_artifacts);
        assert_eq!(state.tasks, before_tasks);
        assert_eq!(state.pending_publications, before_pending);
        Ok(())
    }

    #[tokio::test]
    async fn external_mutation_fence_does_not_write_runtime_state() -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new("external-fence")?);
        let runtime = RuntimeManager::with_store(store.clone())?;

        runtime
            .upsert_pending_proof_publication(
                "l1-l2",
                PipelineKey::ShastaSp1,
                PipelineKey::ShastaSp1.route(),
                "proposal-1",
                b"proof",
            )
            .await?;

        assert_eq!(store.runtime_state_writes.load(Ordering::SeqCst), 0);
        Ok(())
    }

    #[tokio::test]
    async fn runtime_state_conflict_fails_closed_without_installing_foreign_state() -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new("state-conflict")?);
        store.force_conflict.store(true, Ordering::SeqCst);
        let runtime = RuntimeManager::with_store(store.clone())?;

        let error = runtime
            .remove_task("missing")
            .await
            .expect_err("a true generation conflict must fail closed");

        assert!(format!("{error:#}").contains("generation changed"));
        assert_eq!(store.runtime_state_writes.load(Ordering::SeqCst), 1);
        assert!(runtime.is_lifecycle_active());
        assert!(!runtime.accepts_mutations());
        assert_eq!(runtime.current_generation()?, None);
        Ok(())
    }

    #[tokio::test]
    async fn runtime_state_transport_error_recovers_committed_write_by_readback() -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new("state-readback")?);
        store.commit_then_error.store(true, Ordering::SeqCst);
        let runtime = RuntimeManager::with_store(store.clone())?;

        runtime
            .register_task(TaskRegistration {
                task_id: "committed-task".into(),
                pipeline_key: Some(PipelineKey::ShastaNative),
                route: PipelineKey::ShastaNative.route(),
                task_kind: "proposal".into(),
                proposal_id: Some(1),
                proof_ids: Vec::new(),
                metadata: serde_json::json!({}),
                request_fingerprint: None,
            })
            .await?;

        assert!(runtime.accepts_mutations());
        assert!(runtime.get_task("committed-task").await?.is_some());
        assert_eq!(store.runtime_state_writes.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test]
    async fn runtime_state_transport_error_fails_closed_on_foreign_readback() -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new("state-foreign-readback")?);
        store
            .foreign_commit_then_error
            .store(true, Ordering::SeqCst);
        let runtime = RuntimeManager::with_store(store.clone())?;

        let error = runtime
            .register_task(TaskRegistration {
                task_id: "must-not-commit".into(),
                pipeline_key: Some(PipelineKey::ShastaNative),
                route: PipelineKey::ShastaNative.route(),
                task_kind: "proposal".into(),
                proposal_id: Some(1),
                proof_ids: Vec::new(),
                metadata: serde_json::json!({}),
                request_fingerprint: None,
            })
            .await
            .expect_err("foreign read-back must fail closed");

        assert!(format!("{error:#}").contains("conflicted with authoritative read-back"));
        assert_eq!(store.runtime_state_writes.load(Ordering::SeqCst), 1);
        assert!(!runtime.accepts_mutations());
        assert_eq!(runtime.current_generation()?, None);
        Ok(())
    }

    #[tokio::test]
    async fn transient_runtime_state_failure_is_readiness_recoverable() -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new("state-recoverable")?);
        store.fail_before_commit.store(3, Ordering::SeqCst);
        let runtime = RuntimeManager::with_store(store.clone())?;

        runtime
            .remove_task("missing")
            .await
            .expect_err("three transient writes must exhaust the mutation retry");

        assert!(runtime.is_lifecycle_active());
        assert!(runtime.mutation_failure_is_retryable());
        assert!(!runtime.accepts_mutations());
        runtime.check_readiness().await?;
        assert!(runtime.accepts_mutations());
        assert_eq!(store.runtime_state_writes.load(Ordering::SeqCst), 3);
        Ok(())
    }

    #[tokio::test]
    async fn generic_status_sync_cannot_reopen_terminal_task() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "terminal-guard".into())?;
        runtime
            .register_task(TaskRegistration {
                task_id: "completed-task".into(),
                pipeline_key: Some(PipelineKey::ShastaNative),
                route: PipelineKey::ShastaNative.route(),
                task_kind: "proposal".into(),
                proposal_id: Some(1),
                proof_ids: Vec::new(),
                metadata: serde_json::json!({}),
                request_fingerprint: None,
            })
            .await?;
        runtime
            .complete_nonterminal_task("completed-task", "memory://proof")
            .await?;

        runtime
            .sync_status(
                "completed-task",
                RunnerStatus::Allocated,
                Some("late retry".into()),
                None,
            )
            .await?;

        let record = runtime
            .get_task("completed-task")
            .await?
            .expect("registered task");
        assert_eq!(record.runner_status, RunnerStatus::Completed);
        assert_eq!(record.proof_uri.as_deref(), Some("memory://proof"));
        assert_eq!(record.error, None);
        Ok(())
    }

    #[tokio::test]
    async fn pending_proof_is_durable_without_expanding_runtime_state() -> Result<()> {
        let store = Arc::new(MemoryProofArtifactStore::new(
            "test".into(),
            "pending-proof".into(),
        )?);
        let first = RuntimeManager::with_store(store.clone())?;
        first
            .register_task(TaskRegistration {
                task_id: "task-a".into(),
                pipeline_key: Some(PipelineKey::ShastaSp1),
                route: PipelineKey::ShastaSp1.route(),
                task_kind: "proposal".into(),
                proposal_id: Some(1),
                proof_ids: Vec::new(),
                metadata: serde_json::json!({}),
                request_fingerprint: None,
            })
            .await?;

        let proof = b"pending-proof-payload-not-runtime-state";
        first
            .upsert_pending_proof_publication(
                "l1-l2",
                PipelineKey::ShastaSp1,
                PipelineKey::ShastaSp1.route(),
                "proposal-1",
                proof,
            )
            .await?;

        let runtime_state = store
            .load_runtime_state()
            .await?
            .context("runtime state should have been stored")?;
        assert!(
            !runtime_state
                .bytes
                .windows(proof.len())
                .any(|window| window == proof)
        );

        let recovered = RuntimeManager::with_store(store)?;
        recovered.initialize().await?;
        assert_eq!(
            recovered
                .get_pending_proof_publication(
                    "l1-l2",
                    PipelineKey::ShastaSp1,
                    PipelineKey::ShastaSp1.route(),
                    "proposal-1",
                )
                .await?,
            Some(proof.to_vec())
        );
        Ok(())
    }

    #[tokio::test]
    async fn stale_lifecycle_registration_cannot_replace_current_descriptor() -> Result<()> {
        let runtime = RuntimeManager::with_store(Arc::new(MemoryProofArtifactStore::new(
            "test".into(),
            "descriptor-fence".into(),
        )?))?;
        let current = ProofArtifactRegistration {
            network_pair: "l1-l2".into(),
            proof_ref: "proposal-1".into(),
            pipeline_key: PipelineKey::ShastaSp1,
            route: PipelineKey::ShastaSp1.route(),
            proof_uri: "memory://generation-2".into(),
            content_hash: "hash-2".into(),
            generation: Some(2),
        };
        runtime.upsert_proof_artifact(current.clone()).await?;

        let stale = ProofArtifactRegistration {
            proof_uri: "memory://generation-1".into(),
            content_hash: "hash-1".into(),
            generation: Some(1),
            ..current.clone()
        };
        runtime
            .register_invalidated_proof_artifact(stale.clone())
            .await?;
        assert_eq!(
            runtime.register_pending_proof_artifact(stale).await?,
            ProofArtifactLifecycle::Invalidated
        );

        let record = runtime
            .get_proof_artifact_including_invalidated(
                &current.network_pair,
                current.pipeline_key,
                current.route,
                &current.proof_ref,
            )
            .await?
            .context("current lifecycle record")?;
        assert_eq!(record.descriptor(), current.descriptor());
        assert_eq!(record.lifecycle, ProofArtifactLifecycle::Active);
        Ok(())
    }

    #[tokio::test]
    async fn restart_reconciles_locally_committed_artifact_invalidation() -> Result<()> {
        let store = Arc::new(MemoryProofArtifactStore::new(
            "test".into(),
            "restart-invalidation".into(),
        )?);
        let first = RuntimeManager::with_store(store.clone())?;
        let publication = first
            .publish_proof_artifact_bytes(
                "l1-l2",
                PipelineKey::ShastaSp1,
                PipelineKey::ShastaSp1.route(),
                "proposal-1",
                b"proof",
            )
            .await?;
        let object = publication
            .try_object()
            .expect("proof publication should materialize content");
        let registration = ProofArtifactRegistration {
            network_pair: "l1-l2".into(),
            proof_ref: "proposal-1".into(),
            pipeline_key: PipelineKey::ShastaSp1,
            route: PipelineKey::ShastaSp1.route(),
            proof_uri: object.proof_uri.clone(),
            content_hash: object.content_hash.clone(),
            generation: object.generation,
        };
        first.upsert_proof_artifact(registration.clone()).await?;
        let key = artifact_record_key(
            &registration.network_pair,
            registration.pipeline_key,
            registration.route,
            &registration.proof_ref,
        );
        first
            .mutate(move |state| {
                let record = state.artifacts.get_mut(&key).context("artifact record")?;
                record.lifecycle = ProofArtifactLifecycle::Invalidated;
                record.invalidated_at = Some(now_ts());
                Ok(())
            })
            .await?;
        first
            .upsert_pending_proof_publication(
                &registration.network_pair,
                registration.pipeline_key,
                registration.route,
                &registration.proof_ref,
                b"proof",
            )
            .await?;

        let restarted = RuntimeManager::with_store(store)?;
        restarted.initialize().await?;
        assert_eq!(restarted.reconcile_invalidated_proof_artifacts().await?, 1);
        assert!(
            restarted
                .read_proof_artifact_bytes(
                    &registration.network_pair,
                    registration.pipeline_key,
                    registration.route,
                    &registration.proof_ref,
                )
                .await?
                .is_none()
        );
        assert!(
            restarted
                .get_pending_proof_publication(
                    &registration.network_pair,
                    registration.pipeline_key,
                    registration.route,
                    &registration.proof_ref,
                )
                .await?
                .is_none()
        );
        assert!(
            restarted
                .proof_artifact_descriptor_is_invalidated(
                    &registration.network_pair,
                    registration.pipeline_key,
                    registration.route,
                    &registration.proof_ref,
                    &registration.descriptor(),
                )
                .await?
        );
        Ok(())
    }

    #[tokio::test]
    async fn pending_publication_cleanup_is_fenced_by_task_incarnation() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "outbox-incarnation".into())?;
        let registration = TaskRegistration {
            task_id: "root".into(),
            pipeline_key: Some(PipelineKey::ShastaSp1),
            route: PipelineKey::ShastaSp1.route(),
            task_kind: "proposal".into(),
            proposal_id: Some(1),
            proof_ids: vec!["proposal-1".into()],
            metadata: serde_json::json!({}),
            request_fingerprint: Some("same-request".into()),
        };
        let first = match runtime
            .register_task_if_absent(registration.clone())
            .await?
        {
            TaskRegistrationOutcome::Created(record) => record,
            TaskRegistrationOutcome::Existing(_) => anyhow::bail!("unexpected existing task"),
        };
        runtime
            .upsert_pending_proof_publication(
                "l1-l2",
                PipelineKey::ShastaSp1,
                PipelineKey::ShastaSp1.route(),
                "proposal-1",
                b"orphaned-proof-from-crashed-owner",
            )
            .await?;
        assert!(
            runtime
                .checkpoint_pending_proof_publication(
                    "l1-l2",
                    PipelineKey::ShastaSp1,
                    PipelineKey::ShastaSp1.route(),
                    "proposal-1",
                    &[first.incarnation_id],
                    b"proof",
                )
                .await?
        );
        runtime
            .sync_status("root", RunnerStatus::Cancelled, None, None)
            .await?;
        runtime.remove_task("root").await?;
        let second = match runtime.register_task_if_absent(registration).await? {
            TaskRegistrationOutcome::Created(record) => record,
            TaskRegistrationOutcome::Existing(_) => anyhow::bail!("unexpected existing task"),
        };
        assert_ne!(first.incarnation_id, second.incarnation_id);
        assert!(
            runtime
                .checkpoint_pending_proof_publication(
                    "l1-l2",
                    PipelineKey::ShastaSp1,
                    PipelineKey::ShastaSp1.route(),
                    "proposal-1",
                    &[second.incarnation_id],
                    b"proof",
                )
                .await?
        );

        assert!(
            !runtime
                .remove_pending_proof_publication_if_unowned(
                    "l1-l2",
                    PipelineKey::ShastaSp1,
                    PipelineKey::ShastaSp1.route(),
                    "proposal-1",
                )
                .await?
        );
        let pending = runtime
            .get_recoverable_pending_proof_publication(
                "l1-l2",
                PipelineKey::ShastaSp1,
                PipelineKey::ShastaSp1.route(),
                "proposal-1",
            )
            .await?
            .context("replacement outbox")?;
        assert_eq!(pending.owner_incarnations, vec![second.incarnation_id]);
        assert_eq!(pending.bytes, b"proof");
        Ok(())
    }

    #[tokio::test]
    async fn publication_invalidation_refuses_a_live_runtime_owner() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "owner-aware-invalidation".into())?;
        let pipeline = PipelineKey::ShastaSp1;
        let route = pipeline.route();
        let proof_ref = "proposal-1";
        let object = runtime
            .publish_proof_artifact_bytes("l1-l2", pipeline, route, proof_ref, b"proof")
            .await?
            .try_object()
            .expect("proof publication should materialize content")
            .clone();
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: "l1-l2".into(),
                proof_ref: proof_ref.into(),
                pipeline_key: pipeline,
                route,
                proof_uri: object.proof_uri.clone(),
                content_hash: object.content_hash.clone(),
                generation: object.generation,
            })
            .await?;
        runtime
            .register_task(TaskRegistration {
                task_id: "root".into(),
                pipeline_key: Some(pipeline),
                route,
                task_kind: "proposal".into(),
                proposal_id: Some(1),
                proof_ids: vec!["aggregate-task".into()],
                metadata: serde_json::json!({
                    "network_pair": "l1-l2",
                    "aggregate_input_artifacts": [{ "proof_ref": proof_ref }],
                }),
                request_fingerprint: None,
            })
            .await?;

        let mut stale_descriptor = object.descriptor();
        stale_descriptor.generation = stale_descriptor
            .generation
            .map_or(Some(1), |value| Some(value.saturating_add(1)));
        assert_eq!(
            runtime
                .invalidate_proof_artifact_descriptor_if_unowned(
                    "l1-l2",
                    pipeline,
                    route,
                    proof_ref,
                    &stale_descriptor,
                )
                .await?,
            ProofArtifactInvalidationResult::MissingOrChanged
        );
        assert_eq!(
            runtime
                .invalidate_proof_artifact_descriptor_if_unowned(
                    "l1-l2",
                    pipeline,
                    route,
                    proof_ref,
                    &object.descriptor(),
                )
                .await?,
            ProofArtifactInvalidationResult::BlockedByLiveTask
        );
        assert!(
            !runtime
                .invalidate_pending_proof_publication("l1-l2", pipeline, route, proof_ref)
                .await?
        );
        assert!(
            runtime
                .read_proof_artifact_bytes("l1-l2", pipeline, route, proof_ref)
                .await?
                .is_some()
        );
        assert!(
            runtime
                .get_proof_artifact("l1-l2", pipeline, route, proof_ref)
                .await?
                .is_some()
        );
        assert!(
            !runtime
                .proof_artifact_descriptor_is_invalidated(
                    "l1-l2",
                    pipeline,
                    route,
                    proof_ref,
                    &object.descriptor(),
                )
                .await?
        );
        Ok(())
    }

    #[tokio::test]
    async fn descriptor_invalidation_serializes_new_task_admission_until_delete_finishes()
    -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new("artifact-admission-fence")?);
        let runtime = Arc::new(RuntimeManager::with_store(store.clone())?);
        let network_pair = "l1-l2";
        let pipeline = PipelineKey::ShastaSp1;
        let route = pipeline.route();
        let proof_ref = "proposal-2";
        let first = runtime
            .publish_proof_artifact_bytes(network_pair, pipeline, route, proof_ref, b"proof-a")
            .await?
            .try_object()
            .expect("proof publication should materialize content")
            .clone();
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: network_pair.into(),
                proof_ref: proof_ref.into(),
                pipeline_key: pipeline,
                route,
                proof_uri: first.proof_uri.clone(),
                content_hash: first.content_hash.clone(),
                generation: first.generation,
            })
            .await?;
        runtime
            .register_task(TaskRegistration {
                task_id: "recoverable-root".into(),
                pipeline_key: Some(pipeline),
                route,
                task_kind: "proposal".into(),
                proposal_id: Some(2),
                proof_ids: vec![proof_ref.into()],
                metadata: serde_json::json!({ "network_pair": network_pair }),
                request_fingerprint: None,
            })
            .await?;
        runtime
            .sync_status("recoverable-root", RunnerStatus::Cancelled, None, None)
            .await?;

        store
            .block_next_artifact_delete
            .store(true, Ordering::SeqCst);
        let invalidation = tokio::spawn({
            let runtime = Arc::clone(&runtime);
            let descriptor = first.descriptor();
            async move {
                runtime
                    .invalidate_proof_artifact_descriptor_if_unowned(
                        network_pair,
                        pipeline,
                        route,
                        proof_ref,
                        &descriptor,
                    )
                    .await
            }
        });
        store.artifact_delete_completed.notified().await;

        let mut admission = tokio::spawn({
            let runtime = Arc::clone(&runtime);
            let precondition = ProofArtifactPrecondition {
                network_pair: network_pair.into(),
                proof_ref: proof_ref.into(),
                pipeline_key: pipeline,
                route,
                descriptor: first.descriptor(),
            };
            async move {
                runtime
                    .register_task_if_absent_with_artifact_preconditions(
                        TaskRegistration {
                            task_id: "replacement-root".into(),
                            pipeline_key: Some(pipeline),
                            route,
                            task_kind: "proposal".into(),
                            proposal_id: Some(2),
                            proof_ids: vec![proof_ref.into()],
                            metadata: serde_json::json!({ "network_pair": network_pair }),
                            request_fingerprint: Some("replacement-root".into()),
                        },
                        &[precondition],
                    )
                    .await
            }
        });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut admission)
                .await
                .is_err(),
            "new task admission bypassed the artifact invalidation fence"
        );
        let mut recovery = tokio::spawn({
            let runtime = Arc::clone(&runtime);
            async move {
                runtime
                    .reopen_task_for_recovery("recoverable-root", RunnerStatus::Cancelled)
                    .await
            }
        });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut recovery)
                .await
                .is_err(),
            "task recovery bypassed the artifact invalidation fence"
        );

        store.allow_artifact_delete_return.notify_one();
        assert_eq!(
            invalidation.await??,
            ProofArtifactInvalidationResult::Invalidated(ProofArtifactDeleteResult::Removed)
        );
        assert!(admission.await?.is_err());
        assert!(recovery.await??);
        assert!(
            runtime
                .get_proof_artifact_including_invalidated(network_pair, pipeline, route, proof_ref,)
                .await?
                .is_none()
        );
        Ok(())
    }

    #[tokio::test]
    async fn invalidation_owner_scope_uses_full_artifact_identity() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "artifact-owner-scope".into())?;
        let pipeline = PipelineKey::ShastaSp1;
        let route = pipeline.route();
        let proof_ref = "shared-proof-ref";
        let object = runtime
            .publish_proof_artifact_bytes("pair-a", pipeline, route, proof_ref, b"proof-a")
            .await?
            .try_object()
            .expect("proof publication should materialize content")
            .clone();
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: "pair-a".into(),
                proof_ref: proof_ref.into(),
                pipeline_key: pipeline,
                route,
                proof_uri: object.proof_uri.clone(),
                content_hash: object.content_hash.clone(),
                generation: object.generation,
            })
            .await?;
        for (task_id, network_pair) in [("root-a", "pair-a"), ("root-b", "pair-b")] {
            runtime
                .register_task(TaskRegistration {
                    task_id: task_id.into(),
                    pipeline_key: Some(pipeline),
                    route,
                    task_kind: "proposal".into(),
                    proposal_id: Some(1),
                    proof_ids: vec![proof_ref.into()],
                    metadata: serde_json::json!({ "network_pair": network_pair }),
                    request_fingerprint: None,
                })
                .await?;
        }

        let (updated, invalidated, _) = runtime
            .update_tasks_and_invalidate_artifact(proof_ref, "pair-a", pipeline, route, |records| {
                let updated = records.len();
                for record in records {
                    record.runner_status = RunnerStatus::Cancelled;
                }
                Ok((updated, true))
            })
            .await?;
        assert_eq!(updated, 1);
        assert!(invalidated);
        assert_eq!(
            runtime
                .get_task("root-a")
                .await?
                .context("pair-a root")?
                .runner_status,
            RunnerStatus::Cancelled
        );
        assert_eq!(
            runtime
                .get_task("root-b")
                .await?
                .context("pair-b root")?
                .runner_status,
            RunnerStatus::Allocated
        );
        assert!(
            runtime
                .invalidate_pending_proof_publication("pair-a", pipeline, route, proof_ref)
                .await?
        );
        assert!(
            runtime
                .proof_artifact_descriptor_is_invalidated(
                    "pair-a",
                    pipeline,
                    route,
                    proof_ref,
                    &object.descriptor(),
                )
                .await?
        );
        Ok(())
    }

    #[tokio::test]
    async fn removed_owner_cannot_poison_replacement_after_restart() -> Result<()> {
        let store = Arc::new(MemoryProofArtifactStore::new(
            "test".into(),
            "removed-owner-restart".into(),
        )?);
        let registration = TaskRegistration {
            task_id: "root".into(),
            pipeline_key: Some(PipelineKey::ShastaSp1),
            route: PipelineKey::ShastaSp1.route(),
            task_kind: "proposal".into(),
            proposal_id: Some(1),
            proof_ids: vec!["proposal-1".into()],
            metadata: serde_json::json!({}),
            request_fingerprint: Some("same-request".into()),
        };
        let first = RuntimeManager::with_store(store.clone())?;
        let first_owner = first
            .register_task(registration.clone())
            .await?
            .incarnation_id;
        assert!(
            first
                .checkpoint_pending_proof_publication(
                    "l1-l2",
                    PipelineKey::ShastaSp1,
                    PipelineKey::ShastaSp1.route(),
                    "proposal-1",
                    &[first_owner],
                    b"first-proof",
                )
                .await?
        );
        first
            .sync_status("root", RunnerStatus::Cancelled, None, None)
            .await?;
        assert!(first.remove_task("root").await?);
        drop(first);

        let replacement = RuntimeManager::with_store(store)?;
        replacement.initialize().await?;
        let replacement_owner = replacement
            .register_task(registration)
            .await?
            .incarnation_id;
        assert!(
            replacement
                .checkpoint_pending_proof_publication(
                    "l1-l2",
                    PipelineKey::ShastaSp1,
                    PipelineKey::ShastaSp1.route(),
                    "proposal-1",
                    &[replacement_owner],
                    b"different-replacement-proof",
                )
                .await?
        );
        let pending = replacement
            .get_recoverable_pending_proof_publication(
                "l1-l2",
                PipelineKey::ShastaSp1,
                PipelineKey::ShastaSp1.route(),
                "proposal-1",
            )
            .await?
            .context("replacement outbox")?;
        assert_eq!(pending.bytes, b"different-replacement-proof");
        assert_eq!(pending.owner_incarnations, vec![replacement_owner]);
        Ok(())
    }

    #[tokio::test]
    async fn stale_remove_cannot_delete_replacement_incarnation() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "remove-incarnation".into())?;
        let registration = TaskRegistration {
            task_id: "root".into(),
            pipeline_key: Some(PipelineKey::ShastaSp1),
            route: PipelineKey::ShastaSp1.route(),
            task_kind: "proposal".into(),
            proposal_id: Some(1),
            proof_ids: Vec::new(),
            metadata: serde_json::json!({}),
            request_fingerprint: None,
        };
        let first = runtime.register_task(registration.clone()).await?;
        assert!(runtime.remove_task("root").await?);
        let replacement = runtime.register_task(registration).await?;

        assert!(
            !runtime
                .remove_task_if_incarnation("root", first.incarnation_id)
                .await?
        );
        assert_eq!(
            runtime
                .get_task("root")
                .await?
                .expect("replacement")
                .incarnation_id,
            replacement.incarnation_id
        );
        Ok(())
    }

    #[tokio::test]
    async fn cancelled_owner_cleans_outbox_that_finishes_put_late() -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new("cancel-during-outbox-put")?);
        let runtime = Arc::new(RuntimeManager::with_store(store.clone())?);
        let registration = TaskRegistration {
            task_id: "root".into(),
            pipeline_key: Some(PipelineKey::ShastaSp1),
            route: PipelineKey::ShastaSp1.route(),
            task_kind: "proposal".into(),
            proposal_id: Some(1),
            proof_ids: vec!["proposal-1".into()],
            metadata: serde_json::json!({}),
            request_fingerprint: None,
        };
        runtime.register_task(registration.clone()).await?;
        let owner = runtime
            .get_task("root")
            .await?
            .context("runtime owner")?
            .incarnation_id;
        store.block_next_artifact_put.store(true, Ordering::SeqCst);
        let checkpoint = tokio::spawn({
            let runtime = Arc::clone(&runtime);
            async move {
                runtime
                    .checkpoint_pending_proof_publication(
                        "l1-l2",
                        PipelineKey::ShastaSp1,
                        PipelineKey::ShastaSp1.route(),
                        "proposal-1",
                        &[owner],
                        b"late-proof",
                    )
                    .await
            }
        });
        store.artifact_put_entered.notified().await;
        runtime
            .sync_status("root", RunnerStatus::Cancelled, None, None)
            .await?;
        let cleanup = tokio::spawn({
            let runtime = Arc::clone(&runtime);
            async move {
                runtime
                    .remove_pending_proof_publication_if_unowned(
                        "l1-l2",
                        PipelineKey::ShastaSp1,
                        PipelineKey::ShastaSp1.route(),
                        "proposal-1",
                    )
                    .await
            }
        });
        tokio::task::yield_now().await;
        assert!(!cleanup.is_finished());
        store.allow_artifact_put.notify_one();
        assert!(!checkpoint.await??);
        assert!(!cleanup.await??);
        assert!(
            runtime
                .get_pending_proof_publication(
                    "l1-l2",
                    PipelineKey::ShastaSp1,
                    PipelineKey::ShastaSp1.route(),
                    "proposal-1",
                )
                .await?
                .is_none()
        );

        runtime.remove_task("root").await?;
        runtime.register_task(registration).await?;
        let replacement = runtime
            .get_task("root")
            .await?
            .context("replacement owner")?
            .incarnation_id;
        assert!(
            runtime
                .checkpoint_pending_proof_publication(
                    "l1-l2",
                    PipelineKey::ShastaSp1,
                    PipelineKey::ShastaSp1.route(),
                    "proposal-1",
                    &[replacement],
                    b"replacement-proof",
                )
                .await?
        );
        Ok(())
    }
}
