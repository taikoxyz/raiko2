#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![expect(
    clippy::missing_errors_doc,
    reason = "the runtime crate currently permits undocumented internal-facing public APIs"
)]

mod artifact_store;
mod publication;

pub use artifact_store::{
    ProofArtifactConflict, ProofArtifactDeleteResult, ProofArtifactDescriptor, ProofArtifactKey,
    ProofArtifactObject, ProofArtifactPrefix, ProofArtifactPutResult, validate_scope_component,
};
pub use publication::ProofArtifactPublicationInvalidated;

/// Test-only exports of the same storage interfaces used in production.
#[cfg(any(test, feature = "test-utils"))]
pub mod test_support {
    pub use crate::artifact_store::{
        ExactInvalidationResult, MemoryProofArtifactStore, ProofObjectStore, RuntimeStateObject,
        RuntimeStateStore, RuntimeStateWriteResult, RuntimeStore, RuntimeStoreScope,
    };
    pub use crate::{
        ProofArtifactDeleteResult, ProofArtifactDescriptor, ProofArtifactKey, ProofArtifactObject,
        ProofArtifactPrefix, ProofArtifactPutResult,
    };
}

use anyhow::{Context, Result};
use artifact_store::{
    ExactInvalidationResult, GcsProofArtifactStore, MemoryProofArtifactStore, RuntimeStateObject,
    RuntimeStateWriteResult, RuntimeStore,
};
#[cfg(test)]
use artifact_store::{ProofObjectStore, RuntimeStateStore, RuntimeStoreScope};
#[cfg(test)]
use raiko2_pipeline::forks::shasta::preflight_cache::CANONICAL_PREFLIGHT_SCHEMA_V1;
use raiko2_pipeline::forks::shasta::preflight_cache::{
    CanonicalPreflightDescriptor, CanonicalPreflightInvalidateResult, CanonicalPreflightKeyV1,
    CanonicalPreflightObject, CanonicalPreflightPutResult,
};
use raiko2_pipeline::{
    PipelineKey, PipelineRoute, forks::shasta::preflight_cache::CanonicalPreflightStore,
};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
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
#[serde(deny_unknown_fields)]
struct RuntimeState {
    tasks: HashMap<String, RuntimeTaskRecord>,
    artifacts: HashMap<String, ProofArtifactRecord>,
    pending_publications: HashMap<String, PendingProofPublicationRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct PendingProofPublicationRecord {
    network_pair: String,
    pipeline_key: PipelineKey,
    route: PipelineRoute,
    proof_ref: String,
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
    store: Arc<dyn RuntimeStore>,
    canonical_preflight_store: Arc<dyn CanonicalPreflightStore>,
    state: RwLock<RuntimeState>,
    generation: StdMutex<Option<i64>>,
    lifecycle_commit: StdMutex<()>,
    mutation: Mutex<()>,
    artifact_lifecycle_locks: [Mutex<()>; ARTIFACT_LIFECYCLE_LOCK_SHARDS],
    execution_lifecycle_gate: Arc<Mutex<()>>,
    namespace_commit_fence: RwLock<()>,
    submission_checkpoints: Arc<SubmissionCheckpointAdmission>,
    initialized: AtomicBool,
    active: AtomicBool,
    draining: AtomicBool,
    state_coherence: AtomicU8,
}

#[cfg(any(test, feature = "test-utils"))]
#[derive(Debug)]
struct DisabledCanonicalPreflightStore;

#[cfg(any(test, feature = "test-utils"))]
#[async_trait::async_trait]
impl CanonicalPreflightStore for DisabledCanonicalPreflightStore {
    async fn get_canonical_preflight(
        &self,
        _key: &CanonicalPreflightKeyV1,
    ) -> Result<Option<CanonicalPreflightObject>> {
        Ok(None)
    }

    async fn put_canonical_preflight_if_absent(
        &self,
        _key: &CanonicalPreflightKeyV1,
        _bytes: &[u8],
    ) -> Result<CanonicalPreflightPutResult> {
        anyhow::bail!("canonical preflight persistence is disabled for this runtime")
    }

    async fn invalidate_canonical_preflight_exact(
        &self,
        _key: &CanonicalPreflightKeyV1,
        _descriptor: &CanonicalPreflightDescriptor,
    ) -> Result<CanonicalPreflightInvalidateResult> {
        Ok(CanonicalPreflightInvalidateResult::Missing)
    }
}

#[async_trait::async_trait]
impl CanonicalPreflightStore for RuntimeManager {
    async fn get_canonical_preflight(
        &self,
        key: &CanonicalPreflightKeyV1,
    ) -> Result<Option<CanonicalPreflightObject>> {
        self.canonical_preflight_store
            .get_canonical_preflight(key)
            .await
    }

    async fn put_canonical_preflight_if_absent(
        &self,
        key: &CanonicalPreflightKeyV1,
        bytes: &[u8],
    ) -> Result<CanonicalPreflightPutResult> {
        let _commit = self.begin_object_commit()?;
        self.canonical_preflight_store
            .put_canonical_preflight_if_absent(key, bytes)
            .await
    }

    async fn invalidate_canonical_preflight_exact(
        &self,
        key: &CanonicalPreflightKeyV1,
        descriptor: &CanonicalPreflightDescriptor,
    ) -> Result<CanonicalPreflightInvalidateResult> {
        let _commit = self.begin_object_commit()?;
        self.canonical_preflight_store
            .invalidate_canonical_preflight_exact(key, descriptor)
            .await
    }
}

const ARTIFACT_LIFECYCLE_LOCK_SHARDS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpiredTaskCursor {
    pub updated_at: i64,
    pub task_id: String,
}

/// Exact identity of one authoritative runtime-task lifetime.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskLifetime {
    pub task_id: String,
    pub incarnation_id: uuid::Uuid,
}

/// Explicit outcome of a conditional authoritative runtime mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMutationOutcome {
    Applied,
    AlreadyApplied,
    Stale,
    Blocked,
    Missing,
    Conflict,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactExpectation {
    pub key: ProofArtifactKey,
    pub descriptor: ProofArtifactDescriptor,
    pub lifecycle: ProofArtifactLifecycle,
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
#[serde(deny_unknown_fields)]
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

    #[must_use]
    pub fn precondition(&self) -> ProofArtifactPrecondition {
        ProofArtifactPrecondition {
            network_pair: self.network_pair.clone(),
            proof_ref: self.proof_ref.clone(),
            pipeline_key: self.pipeline_key,
            route: self.route,
            descriptor: self.descriptor(),
        }
    }

    #[must_use]
    pub fn expectation(&self) -> ArtifactExpectation {
        ArtifactExpectation {
            key: ProofArtifactKey {
                network_pair: self.network_pair.clone(),
                pipeline_key: self.pipeline_key,
                route: self.route,
                proof_ref: self.proof_ref.clone(),
            },
            descriptor: self.descriptor(),
            lifecycle: self.lifecycle,
        }
    }
}

impl RuntimeManager {
    pub fn canonical_preflight_store(self: &Arc<Self>) -> Arc<dyn CanonicalPreflightStore> {
        self.clone()
    }

    pub async fn tasks_referencing(&self, task_ref: &str) -> Vec<RuntimeTaskRecord> {
        let mut records = self
            .state
            .read()
            .await
            .tasks
            .values()
            .filter(|record| task_references(record, task_ref))
            .cloned()
            .collect::<Vec<_>>();
        records.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.task_id.cmp(&right.task_id))
        });
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

    #[cfg(any(test, feature = "test-utils"))]
    pub fn new(test_root: impl AsRef<std::path::Path>) -> Result<Self> {
        let namespace = format!(
            "test-{}",
            artifact_store::content_hash(test_root.as_ref().to_string_lossy().as_bytes())
        );
        let store = Arc::new(MemoryProofArtifactStore::new(
            "local".to_string(),
            namespace,
        )?);
        Ok(Self::from_shared_store(store))
    }

    pub fn new_memory(environment: String, namespace: String) -> Result<Self> {
        let store = Arc::new(MemoryProofArtifactStore::new(environment, namespace)?);
        Ok(Self::from_shared_store(store))
    }

    #[cfg_attr(
        any(test, feature = "test-utils"),
        doc = "Constructs a runtime from a test store."
    )]
    #[cfg(any(test, feature = "test-utils"))]
    pub fn with_store(store: Arc<dyn RuntimeStore>) -> Self {
        Self::from_store(store)
    }

    #[cfg(any(test, feature = "test-utils"))]
    fn from_store(store: Arc<dyn RuntimeStore>) -> Self {
        Self::from_stores(store, Arc::new(DisabledCanonicalPreflightStore))
    }

    fn from_shared_store<S>(store: Arc<S>) -> Self
    where
        S: RuntimeStore + CanonicalPreflightStore + 'static,
    {
        let runtime_store: Arc<dyn RuntimeStore> = store.clone();
        let canonical_preflight_store: Arc<dyn CanonicalPreflightStore> = store;
        Self::from_stores(runtime_store, canonical_preflight_store)
    }

    fn from_stores(
        store: Arc<dyn RuntimeStore>,
        canonical_preflight_store: Arc<dyn CanonicalPreflightStore>,
    ) -> Self {
        Self {
            store,
            canonical_preflight_store,
            state: RwLock::new(RuntimeState::default()),
            generation: StdMutex::new(None),
            lifecycle_commit: StdMutex::new(()),
            mutation: Mutex::new(()),
            artifact_lifecycle_locks: std::array::from_fn(|_| Mutex::new(())),
            execution_lifecycle_gate: Arc::new(Mutex::new(())),
            namespace_commit_fence: RwLock::new(()),
            submission_checkpoints: Arc::new(SubmissionCheckpointAdmission::default()),
            initialized: AtomicBool::new(false),
            active: AtomicBool::new(true),
            draining: AtomicBool::new(false),
            state_coherence: AtomicU8::new(StateCoherence::Coherent as u8),
        }
    }

    pub async fn new_gcs(
        environment: String,
        namespace: String,
        bucket_id: String,
        prefix: String,
    ) -> Result<Self> {
        let store =
            Arc::new(GcsProofArtifactStore::new(environment, namespace, bucket_id, prefix).await?);
        Ok(Self::from_shared_store(store))
    }

    pub async fn begin_draining(&self) {
        self.start_draining();
        let _commits = self.namespace_commit_fence.write().await;
        self.submission_checkpoints.wait_until_drained().await;
        self.deactivate();
    }

    #[must_use]
    pub fn execution_lifecycle_gate(&self) -> Arc<Mutex<()>> {
        Arc::clone(&self.execution_lifecycle_gate)
    }

    /// Closes provider-submission admission, waits until `deadline` for accepted checkpoints, then
    /// makes the runtime inactive regardless of whether the deadline elapsed.
    ///
    /// Returns `true` when every accepted checkpoint completed before the deadline.
    pub async fn begin_draining_with_deadline(&self, deadline: Instant) -> bool {
        self.start_draining();
        let drained = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
            let _commits = self.namespace_commit_fence.write().await;
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
            StateCoherence::Coherent => {
                let observed_generation = stored.as_ref().and_then(|state| state.generation);
                if observed_generation != self.current_generation()? {
                    self.state_coherence
                        .store(StateCoherence::Violated as u8, Ordering::Release);
                    anyhow::bail!(
                        "runtime state generation changed outside the authoritative repository"
                    );
                }
            }
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
        anyhow::ensure!(
            !self.initialized.load(Ordering::Acquire),
            "runtime is already initialized"
        );
        self.reload_authoritative_state().await?;
        self.initialized.store(true, Ordering::Release);
        Ok(())
    }

    /// Clears the complete persistent namespace before the runtime is initialized.
    ///
    /// This is an explicit startup operation. Callers must uphold the deployment
    /// invariant that no other process is active in this namespace.
    pub async fn reset_namespace(&self) -> Result<usize> {
        self.ensure_active()?;
        let _commit = self.namespace_commit_fence.write().await;
        self.ensure_active()?;
        let _mutation = self.mutation.lock().await;
        self.ensure_active()?;
        anyhow::ensure!(
            !self.initialized.load(Ordering::Acquire),
            "runtime namespace reset is only valid before initialization"
        );
        let cleared = self.store.reset_namespace().await?;
        self.install_runtime_state(RuntimeState::default(), None)
            .await?;
        Ok(cleared)
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
        let _commit = if checkpoint_permit.is_some() {
            None
        } else {
            Some(
                self.namespace_commit_fence
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
            let expected_generation = self.current_generation()?;
            let mut next = current.clone();
            let output = update(&mut next)?;
            validate_runtime_state(&next, self.environment())?;
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
                    // The object generation is the CAS identity. Runtime state contains hash maps,
                    // so deserializing and serializing an unchanged snapshot is not guaranteed to
                    // reproduce the original byte order. Comparing those bytes would turn a
                    // transport error before commit into a false coherence violation.
                    let remote_matches_current = match observed.as_ref() {
                        Some(observed) => observed.generation == expected_generation,
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
                validate_runtime_state(&state, self.environment())?;
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

    fn begin_object_commit(&self) -> Result<tokio::sync::RwLockReadGuard<'_, ()>> {
        self.ensure_active()?;
        let commit = self
            .namespace_commit_fence
            .try_read()
            .context("runtime is draining")?;
        self.ensure_active()?;
        Ok(commit)
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

    fn artifact_lifecycle_lock(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> &Mutex<()> {
        // Only same-artifact object-store steps require ordering. Runtime task transitions remain
        // independent and use the authoritative state CAS instead of taking this lock.
        let mut hasher = DefaultHasher::new();
        Self::artifact_key(network_pair, pipeline_key, route, proof_ref).hash(&mut hasher);
        let shard = hasher.finish() % ARTIFACT_LIFECYCLE_LOCK_SHARDS as u64;
        &self.artifact_lifecycle_locks
            [usize::try_from(shard).expect("artifact lock shard always fits usize")]
    }

    async fn has_active_artifact_owner(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        owner_incarnation: uuid::Uuid,
    ) -> bool {
        self.state.read().await.tasks.values().any(|task| {
            task.incarnation_id == owner_incarnation
                && task.pipeline_key == pipeline_key
                && task.route == route
                && task.network_pair == network_pair
                && task_references(task, proof_ref)
                && matches!(
                    task.runner_status,
                    RunnerStatus::Allocated | RunnerStatus::Running
                )
        })
    }

    pub async fn publish_proof_artifact_bytes(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        bytes: &[u8],
    ) -> Result<ProofArtifactPutResult> {
        let _artifact_lifecycle = self
            .artifact_lifecycle_lock(network_pair, pipeline_key, route, proof_ref)
            .lock()
            .await;
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
        let key = Self::artifact_key(network_pair, pipeline_key, route, proof_ref);
        let commit = self.begin_object_commit()?;
        let publication = self.store.put_if_absent(&key, bytes).await?;
        drop(commit);
        self.ensure_active()
            .context("global runtime fence changed during artifact publication")?;
        Ok(publication)
    }

    pub async fn publish_active_proof_artifact_bytes(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        owner_incarnation: uuid::Uuid,
        bytes: &[u8],
    ) -> Result<ProofArtifactPutResult> {
        anyhow::ensure!(
            self.has_active_artifact_owner(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
                owner_incarnation,
            )
            .await,
            "active proof artifact requires a durable task owner"
        );
        anyhow::ensure!(
            self.checkpoint_pending_proof_publication(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
                &[owner_incarnation],
                bytes,
            )
            .await?,
            "active proof artifact owner changed before checkpoint"
        );
        let publication = self
            .commit_proof_artifact_publication(network_pair, pipeline_key, route, proof_ref, bytes)
            .await?;
        if matches!(publication, ProofArtifactPutResult::Conflict(_)) {
            self.release_pending_proof_publication_owner(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
                owner_incarnation,
            )
            .await?;
            anyhow::bail!("different active proof artifact already exists");
        }
        let artifact = publication
            .try_object()
            .context("active proof artifact conflict references missing content")?;
        if self
            .get_proof_artifact(network_pair, pipeline_key, route, proof_ref)
            .await?
            .is_some_and(|record| record.descriptor() == artifact.descriptor())
        {
            self.release_pending_proof_publication_owner(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
                owner_incarnation,
            )
            .await?;
            return Ok(publication);
        }
        let registration = ProofArtifactRegistration {
            network_pair: network_pair.to_string(),
            proof_ref: proof_ref.to_string(),
            pipeline_key,
            route,
            proof_uri: artifact.proof_uri.clone(),
            content_hash: artifact.content_hash.clone(),
            generation: artifact.generation,
        };
        let activated = self
            .activate_proof_artifact_with_tasks(
                proof_ref,
                registration,
                &[owner_incarnation],
                |_| Ok(Some(())),
            )
            .await?;
        if activated.is_none() {
            anyhow::ensure!(
                self.invalidate_pending_proof_publication(
                    network_pair,
                    pipeline_key,
                    route,
                    proof_ref,
                )
                .await?,
                "active proof artifact owner changed while invalidation was blocked"
            );
            anyhow::bail!("active proof artifact owner changed before activation");
        }
        self.remove_pending_proof_publication_if_unowned(
            network_pair,
            pipeline_key,
            route,
            proof_ref,
        )
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

    #[cfg(any(test, feature = "test-utils"))]
    pub async fn delete_proof_artifact(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        generation: Option<i64>,
        expected_content_hash: &str,
    ) -> Result<ProofArtifactDeleteResult> {
        self.ensure_active()?;
        let key = Self::artifact_key(network_pair, pipeline_key, route, proof_ref);
        let Some(descriptor) = self.store.get_descriptor(&key).await? else {
            return Ok(ProofArtifactDeleteResult::Missing);
        };
        anyhow::ensure!(
            descriptor.generation == generation && descriptor.content_hash == expected_content_hash,
            "proof artifact changed before conditional delete"
        );
        let _commit = self.begin_object_commit()?;
        self.store.delete_exact(&key, &descriptor).await
    }

    #[cfg(any(test, feature = "test-utils"))]
    pub async fn register_task(&self, registration: TaskRegistration) -> Result<RuntimeTaskRecord> {
        let record = build_task_record(&registration)?;
        self.upsert_task(&record).await?;
        Ok(record)
    }

    /// Atomically replaces one unchanged runtime-task snapshot.
    ///
    /// Returning `None` means the caller's snapshot is no longer authoritative. In that case this
    /// method leaves both the current task and pending-publication ownership untouched.
    pub async fn replace_task_if_unchanged_with_artifact_preconditions(
        &self,
        expected: &RuntimeTaskRecord,
        registration: TaskRegistration,
        artifact_preconditions: &[ProofArtifactPrecondition],
    ) -> Result<Option<RuntimeTaskRecord>> {
        let record = build_task_record(&registration)?;
        let expected = expected.clone();
        let artifact_preconditions = artifact_preconditions.to_vec();
        self.mutate(move |state| {
            let Some(current) = state.tasks.get(&expected.task_id) else {
                return Ok(None);
            };
            if current != &expected {
                return Ok(None);
            }
            ensure_artifact_preconditions(state, &artifact_preconditions)?;
            anyhow::ensure!(
                record.task_id == expected.task_id || !state.tasks.contains_key(&record.task_id),
                "replacement task id already belongs to another task"
            );
            let removed = state
                .tasks
                .remove(&expected.task_id)
                .context("runtime task disappeared during conditional replacement")?;
            for pending in state.pending_publications.values_mut() {
                pending
                    .owner_incarnations
                    .retain(|owner| *owner != removed.incarnation_id);
            }
            ensure_task_fingerprint_available(state, &record)?;
            state.tasks.insert(record.task_id.clone(), record.clone());
            Ok(Some(record.clone()))
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
        let fingerprint = registration.request_fingerprint.clone();
        let record = build_task_record(&registration)?;
        let artifact_preconditions = artifact_preconditions.to_vec();
        self.mutate(move |state| {
            if let Some(existing) = state.tasks.get(&record.task_id).cloned().or_else(|| {
                state
                    .tasks
                    .values()
                    .find(|task| task.request_fingerprint == fingerprint)
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

    #[cfg(any(test, feature = "test-utils"))]
    pub async fn upsert_task(&self, record: &RuntimeTaskRecord) -> Result<()> {
        let record = record.clone();
        self.mutate(move |state| {
            ensure_task_fingerprint_available(state, &record)?;
            state.tasks.insert(record.task_id.clone(), record.clone());
            Ok(())
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
            .find(|task| task.request_fingerprint == fingerprint)
            .cloned())
    }

    /// Completes one concrete task lifetime only while every proof artifact it consumed remains
    /// the exact active registration validated by the caller.
    pub async fn complete_nonterminal_task(
        &self,
        task_id: &str,
        expected_incarnation: uuid::Uuid,
        proof_uri: &str,
        artifact_preconditions: &[ProofArtifactPrecondition],
    ) -> Result<bool> {
        let task_id = task_id.to_string();
        let proof_uri = proof_uri.to_string();
        let artifact_preconditions = artifact_preconditions.to_vec();
        self.mutate(move |state| {
            let Some(task) = state.tasks.get(&task_id) else {
                return Ok(false);
            };
            if task.incarnation_id != expected_incarnation || task.runner_status.is_terminal() {
                return Ok(false);
            }
            if ensure_artifact_preconditions(state, &artifact_preconditions).is_err() {
                return Ok(false);
            }
            let task = state
                .tasks
                .get_mut(&task_id)
                .context("runtime task disappeared during artifact completion")?;
            task.runner_status = RunnerStatus::Completed;
            task.proof_uri = Some(proof_uri.clone());
            task.error = None;
            task.updated_at = now_ts();
            Ok(true)
        })
        .await
    }

    /// Prepares one unchanged non-terminal task snapshot for recovery.
    pub async fn prepare_task_for_recovery_if_unchanged(
        &self,
        expected: &RuntimeTaskRecord,
    ) -> Result<Option<RuntimeTaskRecord>> {
        let expected = expected.clone();
        self.mutate(move |state| {
            let Some(task) = state.tasks.get_mut(&expected.task_id) else {
                return Ok(None);
            };
            if task.incarnation_id != expected.incarnation_id {
                return Ok(None);
            }
            if task != &expected
                || matches!(
                    task.runner_status,
                    RunnerStatus::Completed | RunnerStatus::Cancelled
                )
            {
                return Ok(None);
            }
            task.runner_status = RunnerStatus::Allocated;
            task.error = None;
            task.updated_at = now_ts();
            Ok(Some(task.clone()))
        })
        .await
    }

    /// Restores an exact task snapshot only when no state changed after recovery prepared it.
    pub async fn restore_task_after_recovery_if_unchanged(
        &self,
        prepared: &RuntimeTaskRecord,
        original: &RuntimeTaskRecord,
    ) -> Result<RuntimeMutationOutcome> {
        anyhow::ensure!(
            prepared.task_id == original.task_id
                && prepared.incarnation_id == original.incarnation_id,
            "recovery rollback snapshots belong to different task lifetimes"
        );
        let mut expected_original = prepared.clone();
        expected_original.runner_status = original.runner_status;
        expected_original.error.clone_from(&original.error);
        expected_original.updated_at = original.updated_at;
        anyhow::ensure!(
            expected_original == *original,
            "recovery rollback may only restore status, error, and update time"
        );

        let prepared = prepared.clone();
        let original = original.clone();
        self.mutate(move |state| {
            let Some(current) = state.tasks.get_mut(&prepared.task_id) else {
                return Ok(RuntimeMutationOutcome::Missing);
            };
            if current.incarnation_id != prepared.incarnation_id {
                return Ok(RuntimeMutationOutcome::Stale);
            }
            if current != &prepared {
                return Ok(RuntimeMutationOutcome::Blocked);
            }
            current.clone_from(&original);
            Ok(RuntimeMutationOutcome::Applied)
        })
        .await
    }

    /// Marks one exact non-terminal task snapshot failed without overwriting concurrent progress.
    pub async fn fail_task_if_unchanged(
        &self,
        expected: &RuntimeTaskRecord,
        error: String,
    ) -> Result<RuntimeMutationOutcome> {
        let expected = expected.clone();
        self.mutate(move |state| {
            let Some(current) = state.tasks.get_mut(&expected.task_id) else {
                return Ok(RuntimeMutationOutcome::Missing);
            };
            if current.incarnation_id != expected.incarnation_id {
                return Ok(RuntimeMutationOutcome::Stale);
            }
            if current != &expected || current.runner_status.is_terminal() {
                return Ok(RuntimeMutationOutcome::Blocked);
            }
            current.runner_status = RunnerStatus::Failed;
            current.error = Some(error.clone());
            current.updated_at = now_ts();
            Ok(RuntimeMutationOutcome::Applied)
        })
        .await
    }

    /// Cancels exactly one task lifetime without affecting a replacement task.
    pub async fn cancel_task_if_current(
        &self,
        lifetime: &TaskLifetime,
        error: Option<String>,
    ) -> Result<RuntimeMutationOutcome> {
        let task_id = lifetime.task_id.clone();
        let incarnation_id = lifetime.incarnation_id;
        self.mutate(move |state| {
            let Some(task) = state.tasks.get_mut(&task_id) else {
                return Ok(RuntimeMutationOutcome::Missing);
            };
            if task.incarnation_id != incarnation_id {
                return Ok(RuntimeMutationOutcome::Stale);
            }
            match task.runner_status {
                RunnerStatus::Cancelled => Ok(RuntimeMutationOutcome::AlreadyApplied),
                RunnerStatus::Completed | RunnerStatus::Failed => {
                    Ok(RuntimeMutationOutcome::Blocked)
                }
                RunnerStatus::Allocated | RunnerStatus::Running => {
                    task.runner_status = RunnerStatus::Cancelled;
                    task.error.clone_from(&error);
                    task.updated_at = now_ts();
                    Ok(RuntimeMutationOutcome::Applied)
                }
            }
        })
        .await
    }

    /// Cancels one non-terminal task only while its complete observed snapshot is unchanged.
    pub async fn cancel_task_if_unchanged(
        &self,
        expected: &RuntimeTaskRecord,
        error: Option<String>,
    ) -> Result<RuntimeMutationOutcome> {
        let expected = expected.clone();
        self.mutate(move |state| {
            let Some(task) = state.tasks.get_mut(&expected.task_id) else {
                return Ok(RuntimeMutationOutcome::Missing);
            };
            if task.incarnation_id != expected.incarnation_id {
                return Ok(RuntimeMutationOutcome::Stale);
            }
            if task != &expected || task.runner_status.is_terminal() {
                return Ok(RuntimeMutationOutcome::Blocked);
            }
            task.runner_status = RunnerStatus::Cancelled;
            task.error.clone_from(&error);
            task.updated_at = now_ts();
            Ok(RuntimeMutationOutcome::Applied)
        })
        .await
    }

    /// Retires one unchanged task snapshot before destructive queue cleanup.
    ///
    /// Unlike cancellation, retirement also applies to completed and failed roots because the
    /// caller is about to remove the root and must prevent a concurrent recovery from attaching
    /// a new execution graph. A changed non-cancelled snapshot is blocked so stale invalidation or
    /// cleanup cannot retire a root that recovery has reopened in the same incarnation.
    pub async fn retire_task_if_unchanged(
        &self,
        expected: &RuntimeTaskRecord,
        error: Option<String>,
    ) -> Result<RuntimeMutationOutcome> {
        let expected = expected.clone();
        self.mutate(move |state| {
            let Some(task) = state.tasks.get_mut(&expected.task_id) else {
                return Ok(RuntimeMutationOutcome::Missing);
            };
            if task.incarnation_id != expected.incarnation_id {
                return Ok(RuntimeMutationOutcome::Stale);
            }
            if task != &expected {
                return Ok(RuntimeMutationOutcome::Blocked);
            }
            if task.runner_status == RunnerStatus::Cancelled {
                return Ok(RuntimeMutationOutcome::AlreadyApplied);
            }
            task.runner_status = RunnerStatus::Cancelled;
            task.error.clone_from(&error);
            task.proof_uri = None;
            // Retirement is cleanup admission for an already-observed snapshot, not a new
            // retention event. Preserve the terminal timestamp so a failed queue detach remains
            // eligible for the next cleanup scan instead of being retained for another full TTL.
            Ok(RuntimeMutationOutcome::Applied)
        })
        .await
    }

    #[cfg(any(test, feature = "test-utils"))]
    pub async fn upsert_proof_artifact(
        &self,
        registration: ProofArtifactRegistration,
    ) -> Result<()> {
        let _artifact_lifecycle = self
            .artifact_lifecycle_lock(
                &registration.network_pair,
                registration.pipeline_key,
                registration.route,
                &registration.proof_ref,
            )
            .lock()
            .await;
        self.upsert_proof_artifact_with_lifecycle(registration, ProofArtifactLifecycle::Active)
            .await
    }

    pub(crate) async fn register_pending_proof_artifact(
        &self,
        registration: ProofArtifactRegistration,
    ) -> Result<ProofArtifactLifecycle> {
        ensure_canonical_artifact_registration(&registration)?;
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
            let has_owner = pending_publication_has_live_owner(
                state,
                &key,
                &network_pair,
                pipeline_key,
                route,
                &proof_ref,
            );
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
        ensure_canonical_artifact_registration(&registration)?;
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

    #[cfg(any(test, feature = "test-utils"))]
    async fn upsert_proof_artifact_with_lifecycle(
        &self,
        registration: ProofArtifactRegistration,
        lifecycle: ProofArtifactLifecycle,
    ) -> Result<()> {
        ensure_canonical_artifact_registration(&registration)?;
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
        let _artifact_lifecycle = self
            .artifact_lifecycle_lock(
                &registration.network_pair,
                registration.pipeline_key,
                registration.route,
                &registration.proof_ref,
            )
            .lock()
            .await;
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
            anyhow::ensure!(
                artifact.lifecycle == ProofArtifactLifecycle::Active
                    || pending.content_hash == descriptor.content_hash,
                "pending proof publication content changed before activation"
            );

            let mut records =
                artifact_task_records(state, &network_pair, pipeline_key, route, &task_ref);
            records.sort_by(|left, right| left.task_id.cmp(&right.task_id));
            let active_incarnations = records
                .iter()
                .filter(|record| {
                    !matches!(
                        record.runner_status,
                        RunnerStatus::Failed | RunnerStatus::Cancelled
                    )
                })
                .map(|record| record.incarnation_id)
                .collect::<HashSet<_>>();
            if !owner_incarnations
                .iter()
                .any(|owner| active_incarnations.contains(owner))
            {
                return Ok(None);
            }
            anyhow::ensure!(
                owner_incarnations
                    .iter()
                    .all(|owner| active_incarnations.contains(owner)),
                "proof publication activation includes a stale runtime owner"
            );
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
            state
                .pending_publications
                .get_mut(&key)
                .context("pending proof publication ownership disappeared")?
                .owner_incarnations
                .clear();
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
        let _artifact_lifecycle = self
            .artifact_lifecycle_lock(network_pair, pipeline_key, route, task_ref)
            .lock()
            .await;
        let task_ref = task_ref.to_string();
        let network_pair = network_pair.to_string();
        let key = artifact_record_key(&network_pair, pipeline_key, route, &task_ref);
        self.mutate(move |state| {
            let mut records =
                artifact_task_records(state, &network_pair, pipeline_key, route, &task_ref);
            records.sort_by(|left, right| left.task_id.cmp(&right.task_id));
            let (output, requested_invalidation) = update(&mut records)?;
            for record in records {
                state.tasks.insert(record.task_id.clone(), record);
            }
            let live_incarnations =
                artifact_task_records(state, &network_pair, pipeline_key, route, &task_ref)
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
            }
            let invalidate = requested_invalidation && live_incarnations.is_empty();
            let descriptor = if invalidate {
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
        artifacts.sort_by(|left, right| {
            left.updated_at
                .cmp(&right.updated_at)
                .then_with(|| left.network_pair.cmp(&right.network_pair))
                .then_with(|| left.pipeline_key.as_str().cmp(right.pipeline_key.as_str()))
                .then_with(|| {
                    left.route
                        .guest_system
                        .as_str()
                        .cmp(right.route.guest_system.as_str())
                })
                .then_with(|| left.route.runner.as_str().cmp(right.route.runner.as_str()))
                .then_with(|| left.proof_ref.cmp(&right.proof_ref))
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
        let Some(_generation) = invalidated_generation else {
            return Ok(None);
        };
        self.finalize_proof_artifact_invalidation(&ArtifactExpectation {
            key: Self::artifact_key(network_pair, pipeline_key, route, proof_ref),
            descriptor: descriptor.clone(),
            lifecycle: ProofArtifactLifecycle::Invalidated,
        })
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
        let _artifact_lifecycle = self
            .artifact_lifecycle_lock(network_pair, pipeline_key, route, proof_ref)
            .lock()
            .await;
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
                )
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
            .finalize_proof_artifact_invalidation(&ArtifactExpectation {
                key: Self::artifact_key(network_pair, pipeline_key, route, proof_ref),
                descriptor: descriptor.clone(),
                lifecycle: ProofArtifactLifecycle::Invalidated,
            })
            .await?;
        self.remove_proof_artifact_if_descriptor(
            network_pair,
            pipeline_key,
            route,
            proof_ref,
            descriptor,
        )
        .await?;
        self.remove_pending_proof_publication_if_unowned_locked(
            network_pair,
            pipeline_key,
            route,
            proof_ref,
        )
        .await?;
        Ok(ProofArtifactInvalidationResult::Invalidated(delete_result))
    }

    pub async fn finalize_proof_artifact_invalidation(
        &self,
        expectation: &ArtifactExpectation,
    ) -> Result<ProofArtifactDeleteResult> {
        anyhow::ensure!(
            expectation.lifecycle == ProofArtifactLifecycle::Invalidated,
            "proof artifact must be durably invalidated before external finalization"
        );
        let _commit = self.begin_object_commit()?;
        match self
            .store
            .invalidate_exact(&expectation.key, &expectation.descriptor)
            .await?
        {
            ExactInvalidationResult::Invalidated(result) => Ok(result),
            ExactInvalidationResult::AlreadyInvalidated | ExactInvalidationResult::Missing => {
                Ok(ProofArtifactDeleteResult::Missing)
            }
            ExactInvalidationResult::Stale => {
                anyhow::bail!("proof artifact changed before exact invalidation")
            }
        }
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
            self.finalize_proof_artifact_invalidation(&record.expectation())
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
        self.store.is_invalidated(&object_key, descriptor).await
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

    #[cfg(any(test, feature = "test-utils"))]
    pub async fn upsert_pending_proof_publication(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        proof_bytes: &[u8],
    ) -> Result<()> {
        let _artifact_lifecycle = self
            .artifact_lifecycle_lock(network_pair, pipeline_key, route, proof_ref)
            .lock()
            .await;
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
        let key = Self::pending_artifact_key(network_pair, pipeline_key, route, proof_ref);
        let _commit = self.begin_object_commit()?;
        self.store.put_if_absent(&key, proof_bytes).await
    }

    async fn record_pending_proof_publication_intent(
        &self,
        record: PendingProofPublicationRecord,
    ) -> Result<bool> {
        let key = artifact_record_key(
            &record.network_pair,
            record.pipeline_key,
            record.route,
            &record.proof_ref,
        );
        self.mutate(move |state| {
            let live_incarnations = state
                .tasks
                .values()
                .filter(|task| {
                    task_matches_artifact_identity(
                        task,
                        &record.network_pair,
                        record.pipeline_key,
                        record.route,
                        &record.proof_ref,
                    ) && !matches!(
                        task.runner_status,
                        RunnerStatus::Failed | RunnerStatus::Cancelled
                    )
                })
                .map(|task| task.incarnation_id)
                .collect::<HashSet<_>>();
            let mut owners = record
                .owner_incarnations
                .iter()
                .filter(|owner| live_incarnations.contains(owner))
                .copied()
                .collect::<Vec<_>>();
            if owners.is_empty() {
                return Ok(false);
            }
            let different_live_intent =
                state.pending_publications.get(&key).is_some_and(|pending| {
                    pending.content_hash != record.content_hash
                        && pending
                            .owner_incarnations
                            .iter()
                            .any(|owner| live_incarnations.contains(owner))
                });
            anyhow::ensure!(
                !different_live_intent,
                "different pending proof is owned by another task incarnation"
            );
            if let Some(existing) = state
                .pending_publications
                .get(&key)
                .filter(|pending| pending.content_hash == record.content_hash)
            {
                owners.extend(
                    existing
                        .owner_incarnations
                        .iter()
                        .filter(|owner| live_incarnations.contains(owner))
                        .copied(),
                );
            }
            owners.sort_unstable();
            owners.dedup();
            let mut next = record.clone();
            next.owner_incarnations = owners;
            state.pending_publications.insert(key.clone(), next);
            Ok(true)
        })
        .await
    }

    async fn materialize_pending_proof_publication(
        &self,
        key: &ProofArtifactKey,
        proof_bytes: &[u8],
    ) -> Result<()> {
        let mut publication = self
            .put_pending_proof_publication_bytes(
                &key.network_pair,
                key.pipeline_key,
                key.route,
                &key.proof_ref,
                proof_bytes,
            )
            .await?;
        if matches!(publication, ProofArtifactPutResult::Conflict(_)) {
            anyhow::ensure!(
                self.remove_local_pending(
                    &key.network_pair,
                    key.pipeline_key,
                    key.route,
                    &key.proof_ref,
                )
                .await?,
                "conflicting pending proof disappeared before exact replacement"
            );
            publication = self
                .put_pending_proof_publication_bytes(
                    &key.network_pair,
                    key.pipeline_key,
                    key.route,
                    &key.proof_ref,
                    proof_bytes,
                )
                .await?;
        }
        anyhow::ensure!(
            !matches!(publication, ProofArtifactPutResult::Conflict(_)),
            "different pending proof still exists after exact replacement"
        );
        Ok(())
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
        let _artifact_lifecycle = self
            .artifact_lifecycle_lock(network_pair, pipeline_key, route, proof_ref)
            .lock()
            .await;
        let key = Self::artifact_key(network_pair, pipeline_key, route, proof_ref);
        let content_hash = artifact_store::content_hash(proof_bytes);
        let checkpointed = self
            .record_pending_proof_publication_intent(PendingProofPublicationRecord {
                network_pair: network_pair.to_string(),
                pipeline_key,
                route,
                proof_ref: proof_ref.to_string(),
                content_hash: content_hash.clone(),
                owner_incarnations: owner_incarnations.to_vec(),
            })
            .await?;
        if !checkpointed {
            return Ok(false);
        }
        self.materialize_pending_proof_publication(&key, proof_bytes)
            .await?;
        let state_key = artifact_record_key(network_pair, pipeline_key, route, proof_ref);
        let still_owned = {
            let state = self.state.read().await;
            state
                .pending_publications
                .get(&state_key)
                .is_some_and(|pending| pending.content_hash == content_hash)
                && pending_publication_has_live_owner(
                    &state,
                    &state_key,
                    network_pair,
                    pipeline_key,
                    route,
                    proof_ref,
                )
        };
        if !still_owned {
            self.remove_pending_proof_publication_if_unowned_locked(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
            )
            .await?;
            return Ok(false);
        }
        Ok(true)
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
                    task_matches_artifact_identity(
                        task,
                        network_pair,
                        pipeline_key,
                        route,
                        &proof_ref_owned,
                    ) && !matches!(
                        task.runner_status,
                        RunnerStatus::Failed | RunnerStatus::Cancelled
                    ) && pending.owner_incarnations.contains(&task.incarnation_id)
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

    /// Reconciles publication intents that no current runtime task still owns.
    ///
    /// An activated artifact with a live consumer needs only pending-blob cleanup. Every other
    /// unfinished publication is invalidated before its pending intent is removed, including the
    /// crash window after canonical object creation but before lifecycle registration.
    pub async fn reconcile_unowned_pending_proof_publications(&self) -> Result<usize> {
        let pending = {
            let state = self.state.read().await;
            state
                .pending_publications
                .iter()
                .filter_map(|(key, record)| {
                    if pending_publication_has_live_owner(
                        &state,
                        key,
                        &record.network_pair,
                        record.pipeline_key,
                        record.route,
                        &record.proof_ref,
                    ) {
                        return None;
                    }
                    let has_live_task = artifact_task_records(
                        &state,
                        &record.network_pair,
                        record.pipeline_key,
                        record.route,
                        &record.proof_ref,
                    )
                    .iter()
                    .any(|task| {
                        !matches!(
                            task.runner_status,
                            RunnerStatus::Failed | RunnerStatus::Cancelled
                        )
                    });
                    let active_for_live_task = has_live_task
                        && state.artifacts.get(key).is_some_and(|artifact| {
                            artifact.lifecycle == ProofArtifactLifecycle::Active
                        });
                    Some((
                        record.network_pair.clone(),
                        record.pipeline_key,
                        record.route,
                        record.proof_ref.clone(),
                        !active_for_live_task,
                    ))
                })
                .collect::<Vec<_>>()
        };
        let mut removed = 0usize;
        for (network_pair, pipeline_key, route, proof_ref, invalidate) in pending {
            let reconciled = if invalidate {
                self.invalidate_pending_proof_publication(
                    &network_pair,
                    pipeline_key,
                    route,
                    &proof_ref,
                )
                .await?
            } else {
                self.remove_pending_proof_publication_if_unowned(
                    &network_pair,
                    pipeline_key,
                    route,
                    &proof_ref,
                )
                .await?
            };
            if reconciled {
                removed = removed.saturating_add(1);
            }
        }
        Ok(removed)
    }

    pub async fn remove_pending_proof_publication_if_unowned(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<bool> {
        let _artifact_lifecycle = self
            .artifact_lifecycle_lock(network_pair, pipeline_key, route, proof_ref)
            .lock()
            .await;
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
        let pending_record_present = {
            let state = self.state.read().await;
            if pending_publication_has_live_owner(
                &state,
                &state_key,
                network_pair,
                pipeline_key,
                route,
                proof_ref,
            ) {
                return Ok(false);
            }
            state.pending_publications.contains_key(&state_key)
        };
        let removed_object = self
            .remove_local_pending(network_pair, pipeline_key, route, proof_ref)
            .await?;
        if !pending_record_present && !removed_object {
            return Ok(false);
        }
        let network_pair = network_pair.to_string();
        let proof_ref = proof_ref.to_string();
        let removed_record = self
            .mutate(move |state| {
                if pending_publication_has_live_owner(
                    state,
                    &state_key,
                    &network_pair,
                    pipeline_key,
                    route,
                    &proof_ref,
                ) {
                    return Ok(false);
                }
                Ok(state.pending_publications.remove(&state_key).is_some())
            })
            .await?;
        Ok(removed_object || removed_record)
    }

    pub async fn release_pending_proof_publication_owner(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        owner_incarnation: uuid::Uuid,
    ) -> Result<bool> {
        let _artifact_lifecycle = self
            .artifact_lifecycle_lock(network_pair, pipeline_key, route, proof_ref)
            .lock()
            .await;
        let state_key = artifact_record_key(network_pair, pipeline_key, route, proof_ref);
        let pending_record_exists = self
            .mutate(move |state| {
                let Some(pending) = state.pending_publications.get_mut(&state_key) else {
                    return Ok(false);
                };
                pending
                    .owner_incarnations
                    .retain(|owner| *owner != owner_incarnation);
                Ok(true)
            })
            .await?;
        if !pending_record_exists {
            return Ok(false);
        }

        if self
            .pending_publication_has_live_owner(network_pair, pipeline_key, route, proof_ref)
            .await
        {
            return Ok(false);
        }
        if self
            .invalidate_pending_proof_publication_locked(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
            )
            .await?
        {
            return Ok(true);
        }
        self.remove_pending_proof_publication_if_unowned_locked(
            network_pair,
            pipeline_key,
            route,
            proof_ref,
        )
        .await
    }

    /// Releases every pending publication owned by one exact runtime-task lifetime.
    pub async fn release_task_pending_publications(
        &self,
        record: &RuntimeTaskRecord,
    ) -> Result<()> {
        for proof_ref in &record.artifact_refs {
            self.release_pending_proof_publication_owner(
                &record.network_pair,
                record.pipeline_key,
                record.route,
                proof_ref,
                record.incarnation_id,
            )
            .await?;
        }
        Ok(())
    }

    async fn pending_publication_has_live_owner(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> bool {
        let state_key = artifact_record_key(network_pair, pipeline_key, route, proof_ref);
        let state = self.state.read().await;
        pending_publication_has_live_owner(
            &state,
            &state_key,
            network_pair,
            pipeline_key,
            route,
            proof_ref,
        )
    }

    async fn remove_local_pending(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<bool> {
        let key = Self::pending_artifact_key(network_pair, pipeline_key, route, proof_ref);
        let Some(descriptor) = self.store.get_descriptor(&key).await? else {
            return Ok(false);
        };
        let _commit = self.begin_object_commit()?;
        self.store.delete_exact(&key, &descriptor).await?;
        Ok(true)
    }

    /// Invalidates an unowned publication intent without treating a replacement task that merely
    /// references the same artifact key as the old publication owner.
    ///
    /// An active artifact remains protected by every live consumer. A pending artifact is fenced
    /// by the exact owner incarnations recorded in its durable publication intent, so replacing a
    /// root cannot strand the old incarnation's outbox forever.
    pub async fn invalidate_pending_proof_publication(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<bool> {
        let _artifact_lifecycle = self
            .artifact_lifecycle_lock(network_pair, pipeline_key, route, proof_ref)
            .lock()
            .await;
        self.invalidate_pending_proof_publication_locked(
            network_pair,
            pipeline_key,
            route,
            proof_ref,
        )
        .await
    }

    async fn invalidate_pending_proof_publication_locked(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<bool> {
        let object_key = Self::artifact_key(network_pair, pipeline_key, route, proof_ref);
        let canonical = self.store.get_descriptor(&object_key).await?;
        let canonical_record = canonical.as_ref().map(|descriptor| {
            self.proof_artifact_record(
                ProofArtifactRegistration {
                    network_pair: network_pair.to_string(),
                    proof_ref: proof_ref.to_string(),
                    pipeline_key,
                    route,
                    proof_uri: descriptor.proof_uri.clone(),
                    content_hash: descriptor.content_hash.clone(),
                    generation: descriptor.generation,
                },
                ProofArtifactLifecycle::Invalidated,
            )
            .1
        });
        let state_object_key = object_key.clone();
        let invalidated = self
            .mutate(move |state| {
                Ok(mark_unowned_pending_publication_invalidated(
                    state,
                    &state_object_key,
                    canonical_record.as_ref(),
                ))
            })
            .await?;
        let Some(invalidate_canonical) = invalidated else {
            return Ok(false);
        };

        if invalidate_canonical && let Some(descriptor) = canonical {
            self.finalize_proof_artifact_invalidation(&ArtifactExpectation {
                key: object_key,
                descriptor,
                lifecycle: ProofArtifactLifecycle::Invalidated,
            })
            .await?;
        }
        self.remove_pending_proof_publication_if_unowned_locked(
            network_pair,
            pipeline_key,
            route,
            proof_ref,
        )
        .await?;
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

    /// Removes exactly one retired task lifetime and its pending-publication ownership.
    pub async fn remove_task_if_current(
        &self,
        lifetime: &TaskLifetime,
    ) -> Result<RuntimeMutationOutcome> {
        let task_id = lifetime.task_id.clone();
        let incarnation_id = lifetime.incarnation_id;
        self.mutate(move |state| {
            let Some(current) = state.tasks.get(&task_id) else {
                return Ok(RuntimeMutationOutcome::Missing);
            };
            if current.incarnation_id != incarnation_id {
                return Ok(RuntimeMutationOutcome::Stale);
            }
            if current.runner_status != RunnerStatus::Cancelled {
                return Ok(RuntimeMutationOutcome::Blocked);
            }
            let Some(removed) = state.tasks.remove(&task_id) else {
                return Ok(RuntimeMutationOutcome::Conflict);
            };
            for pending in state.pending_publications.values_mut() {
                pending
                    .owner_incarnations
                    .retain(|owner| *owner != removed.incarnation_id);
            }
            Ok(RuntimeMutationOutcome::Applied)
        })
        .await
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRegistration {
    pub task_id: String,
    pub pipeline_key: PipelineKey,
    pub route: PipelineRoute,
    pub task_kind: String,
    pub network_pair: String,
    pub artifact_refs: Vec<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub request_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeTaskRecord {
    pub task_id: String,
    /// Immutable identity for this concrete task lifetime; never reused after replacement.
    pub incarnation_id: uuid::Uuid,
    pub pipeline_key: PipelineKey,
    pub route: PipelineRoute,
    pub task_kind: String,
    /// Canonical network-pair scope for every artifact and task ownership decision.
    pub network_pair: String,
    /// Canonical proof references consumed or produced by this root.
    pub artifact_refs: Vec<String>,
    pub runner_status: RunnerStatus,
    pub image_ref: Option<String>,
    pub proof_uri: Option<String>,
    pub error: Option<String>,
    pub metadata: serde_json::Value,
    pub request_fingerprint: String,
    pub updated_at: i64,
}

impl RuntimeTaskRecord {
    #[must_use]
    pub fn lifetime(&self) -> TaskLifetime {
        TaskLifetime {
            task_id: self.task_id.clone(),
            incarnation_id: self.incarnation_id,
        }
    }
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
    anyhow::ensure!(
        !registration.task_id.is_empty(),
        "task registration id is empty"
    );
    anyhow::ensure!(
        !registration.task_kind.is_empty(),
        "task registration kind is empty"
    );
    anyhow::ensure!(
        registration.pipeline_key.supports_route(registration.route),
        "pipeline_key '{}' does not match route '{}'",
        registration.pipeline_key.as_str(),
        registration.route
    );
    anyhow::ensure!(
        !registration.network_pair.is_empty(),
        "task registration requires network_pair"
    );
    anyhow::ensure!(
        registration
            .artifact_refs
            .iter()
            .all(|proof_ref| !proof_ref.is_empty()),
        "task registration contains an empty artifact reference"
    );
    let unique_artifact_refs = registration.artifact_refs.iter().collect::<HashSet<_>>();
    anyhow::ensure!(
        unique_artifact_refs.len() == registration.artifact_refs.len(),
        "task registration contains duplicate artifact references"
    );
    anyhow::ensure!(
        !registration.request_fingerprint.is_empty(),
        "task registration request fingerprint is empty"
    );
    Ok(RuntimeTaskRecord {
        task_id: registration.task_id.clone(),
        incarnation_id: uuid::Uuid::new_v4(),
        pipeline_key: registration.pipeline_key,
        route: registration.route,
        task_kind: registration.task_kind.clone(),
        network_pair: registration.network_pair.clone(),
        artifact_refs: registration.artifact_refs.clone(),
        runner_status: RunnerStatus::Allocated,
        image_ref: None,
        proof_uri: None,
        error: None,
        metadata: registration.metadata.clone(),
        request_fingerprint: registration.request_fingerprint.clone(),
        updated_at: now_ts(),
    })
}

fn artifact_record_key(
    network_pair: &str,
    pipeline_key: PipelineKey,
    route: PipelineRoute,
    proof_ref: &str,
) -> String {
    let route = route.to_string();
    let mut key = String::from("artifact:v1:");
    for component in [
        network_pair,
        pipeline_key.as_str(),
        route.as_str(),
        proof_ref,
    ] {
        write!(&mut key, "{}:{component}", component.len())
            .expect("writing an artifact identity to a String cannot fail");
    }
    key
}

fn ensure_canonical_artifact_registration(registration: &ProofArtifactRegistration) -> Result<()> {
    anyhow::ensure!(
        registration.pipeline_key.supports_route(registration.route),
        "proof artifact pipeline does not support its route"
    );
    Ok(())
}

fn validate_runtime_state(state: &RuntimeState, environment: &str) -> Result<()> {
    let mut incarnations = HashSet::new();
    let mut request_fingerprints = HashSet::new();
    for (task_id, record) in &state.tasks {
        validate_runtime_task_record(
            task_id,
            record,
            &mut incarnations,
            &mut request_fingerprints,
        )?;
    }
    for (key, record) in &state.artifacts {
        validate_proof_artifact_record(key, record, environment)?;
    }
    for (key, record) in &state.pending_publications {
        validate_pending_publication_record(key, record, &state.tasks)?;
    }
    Ok(())
}

fn validate_runtime_task_record<'a>(
    task_id: &str,
    record: &'a RuntimeTaskRecord,
    incarnations: &mut HashSet<uuid::Uuid>,
    request_fingerprints: &mut HashSet<&'a str>,
) -> Result<()> {
    anyhow::ensure!(
        task_id == record.task_id,
        "runtime task key does not match record"
    );
    anyhow::ensure!(!record.task_id.is_empty(), "runtime task id is empty");
    anyhow::ensure!(
        record.incarnation_id != uuid::Uuid::nil(),
        "runtime task incarnation is nil"
    );
    anyhow::ensure!(
        incarnations.insert(record.incarnation_id),
        "runtime task incarnation is duplicated"
    );
    anyhow::ensure!(
        record
            .pipeline_key
            .canonicalize_persisted_route(record.route)
            .is_some(),
        "runtime task pipeline does not support its route"
    );
    anyhow::ensure!(!record.task_kind.is_empty(), "runtime task kind is empty");
    anyhow::ensure!(
        !record.network_pair.is_empty(),
        "runtime task network pair is empty"
    );
    anyhow::ensure!(
        record
            .artifact_refs
            .iter()
            .all(|proof_ref| !proof_ref.is_empty()),
        "runtime task contains an empty artifact reference"
    );
    anyhow::ensure!(
        record.artifact_refs.iter().collect::<HashSet<_>>().len() == record.artifact_refs.len(),
        "runtime task contains duplicate artifact references"
    );
    anyhow::ensure!(
        !record.request_fingerprint.is_empty(),
        "runtime task request fingerprint is empty"
    );
    anyhow::ensure!(
        request_fingerprints.insert(&record.request_fingerprint),
        "runtime task request fingerprint is duplicated"
    );
    Ok(())
}

fn validate_proof_artifact_record(
    key: &str,
    record: &ProofArtifactRecord,
    environment: &str,
) -> Result<()> {
    anyhow::ensure!(
        key == artifact_record_key(
            &record.network_pair,
            record.pipeline_key,
            record.route,
            &record.proof_ref,
        ),
        "runtime artifact key does not match record"
    );
    anyhow::ensure!(
        record.environment == environment,
        "runtime artifact environment does not match the store"
    );
    anyhow::ensure!(
        record
            .pipeline_key
            .canonicalize_persisted_route(record.route)
            .is_some(),
        "runtime artifact pipeline does not support its route"
    );
    anyhow::ensure!(
        !record.network_pair.is_empty()
            && !record.proof_ref.is_empty()
            && !record.proof_uri.is_empty()
            && !record.content_hash.is_empty(),
        "runtime artifact identity or descriptor is empty"
    );
    anyhow::ensure!(
        (record.lifecycle == ProofArtifactLifecycle::Invalidated)
            == record.invalidated_at.is_some(),
        "runtime artifact invalidation timestamp does not match its lifecycle"
    );
    Ok(())
}

fn validate_pending_publication_record(
    key: &str,
    record: &PendingProofPublicationRecord,
    tasks: &HashMap<String, RuntimeTaskRecord>,
) -> Result<()> {
    anyhow::ensure!(
        key == artifact_record_key(
            &record.network_pair,
            record.pipeline_key,
            record.route,
            &record.proof_ref,
        ),
        "pending publication key does not match record"
    );
    anyhow::ensure!(
        record
            .pipeline_key
            .canonicalize_persisted_route(record.route)
            .is_some(),
        "pending publication pipeline does not support its route"
    );
    anyhow::ensure!(
        !record.network_pair.is_empty()
            && !record.proof_ref.is_empty()
            && !record.content_hash.is_empty(),
        "pending publication identity or content hash is empty"
    );
    anyhow::ensure!(
        record
            .owner_incarnations
            .iter()
            .collect::<HashSet<_>>()
            .len()
            == record.owner_incarnations.len(),
        "pending publication contains duplicate owners"
    );
    anyhow::ensure!(
        record.owner_incarnations.iter().all(|owner| {
            tasks.values().any(|task| {
                task.incarnation_id == *owner
                    && task_matches_artifact_identity(
                        task,
                        &record.network_pair,
                        record.pipeline_key,
                        record.route,
                        &record.proof_ref,
                    )
            })
        }),
        "pending publication owner does not match its artifact identity"
    );
    Ok(())
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
    if state.tasks.values().any(|task| {
        task.task_id != record.task_id && task.request_fingerprint == record.request_fingerprint
    }) {
        anyhow::bail!("request fingerprint already belongs to another task");
    }
    Ok(())
}

fn task_references(record: &RuntimeTaskRecord, task_ref: &str) -> bool {
    record.task_id == task_ref || record.artifact_refs.iter().any(|id| id == task_ref)
}

fn task_matches_artifact_identity(
    record: &RuntimeTaskRecord,
    network_pair: &str,
    pipeline_key: PipelineKey,
    route: PipelineRoute,
    proof_ref: &str,
) -> bool {
    if !task_references(record, proof_ref)
        || record.pipeline_key != pipeline_key
        || record.route != route
    {
        return false;
    }
    record.network_pair == network_pair
}

fn pending_publication_has_live_owner(
    state: &RuntimeState,
    state_key: &str,
    network_pair: &str,
    pipeline_key: PipelineKey,
    route: PipelineRoute,
    proof_ref: &str,
) -> bool {
    state
        .pending_publications
        .get(state_key)
        .is_some_and(|pending| {
            pending.owner_incarnations.iter().any(|owner| {
                state.tasks.values().any(|task| {
                    task.incarnation_id == *owner
                        && task_matches_artifact_identity(
                            task,
                            network_pair,
                            pipeline_key,
                            route,
                            proof_ref,
                        )
                        && !matches!(
                            task.runner_status,
                            RunnerStatus::Failed | RunnerStatus::Cancelled
                        )
                })
            })
        })
}

fn mark_unowned_pending_publication_invalidated(
    state: &mut RuntimeState,
    key: &ProofArtifactKey,
    canonical_record: Option<&ProofArtifactRecord>,
) -> Option<bool> {
    let state_key = artifact_record_key(
        &key.network_pair,
        key.pipeline_key,
        key.route,
        &key.proof_ref,
    );
    if pending_publication_has_live_owner(
        state,
        &state_key,
        &key.network_pair,
        key.pipeline_key,
        key.route,
        &key.proof_ref,
    ) {
        return None;
    }
    let has_live_task = artifact_task_records(
        state,
        &key.network_pair,
        key.pipeline_key,
        key.route,
        &key.proof_ref,
    )
    .iter()
    .any(|task| {
        !matches!(
            task.runner_status,
            RunnerStatus::Failed | RunnerStatus::Cancelled
        )
    });
    let pending = state.pending_publications.get(&state_key);
    let artifact = state.artifacts.get(&state_key);
    let canonical_descriptor = canonical_record.map(ProofArtifactRecord::descriptor);
    let canonical_has_lifecycle = |lifecycle| {
        artifact.is_some_and(|record| {
            canonical_descriptor
                .as_ref()
                .is_some_and(|descriptor| record.descriptor() == *descriptor)
                && record.lifecycle == lifecycle
        })
    };
    if has_live_task
        && (pending.is_none() || canonical_has_lifecycle(ProofArtifactLifecycle::Active))
    {
        return None;
    }
    let pending_matches_canonical = pending.is_some_and(|pending| {
        canonical_descriptor
            .as_ref()
            .is_some_and(|descriptor| pending.content_hash == descriptor.content_hash)
    });
    let invalidate_canonical = canonical_record.is_some()
        && (canonical_has_lifecycle(ProofArtifactLifecycle::Invalidated)
            || pending_matches_canonical
            || !has_live_task);
    if invalidate_canonical && let Some(canonical_record) = canonical_record {
        match state.artifacts.get_mut(&state_key) {
            None => {
                state.artifacts.insert(state_key, canonical_record.clone());
            }
            Some(record) if record.descriptor() == canonical_record.descriptor() => {
                record.lifecycle = ProofArtifactLifecycle::Invalidated;
                record.invalidated_at.get_or_insert_with(now_ts);
                record.updated_at = now_ts();
            }
            Some(_) => {}
        }
    }
    Some(invalidate_canonical)
}

fn artifact_task_records(
    state: &RuntimeState,
    network_pair: &str,
    pipeline_key: PipelineKey,
    route: PipelineRoute,
    proof_ref: &str,
) -> Vec<RuntimeTaskRecord> {
    state
        .tasks
        .values()
        .filter(|record| {
            task_matches_artifact_identity(record, network_pair, pipeline_key, route, proof_ref)
        })
        .cloned()
        .collect()
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
    use std::time::Duration;

    fn canonical_preflight_test_key(proposal_id: u64) -> CanonicalPreflightKeyV1 {
        CanonicalPreflightKeyV1 {
            schema: CANONICAL_PREFLIGHT_SCHEMA_V1,
            l1_chain_id: 1,
            l2_chain_id: 167_001,
            proposal_id,
            l2_block_range: raiko2_primitives::L2BlockRange {
                start: proposal_id,
                end: proposal_id,
            },
            l1_inclusion_block_number: proposal_id,
            last_anchor_block_number: proposal_id.saturating_sub(1),
            checkpoint: None,
            l1_inclusion_hash: [0x11; 32].into(),
            proposal_event_digest: [0x22; 32].into(),
            chain_rules_fingerprint: [0x33; 32].into(),
        }
    }

    async fn current_test_task(
        runtime: &RuntimeManager,
        task_id: &str,
    ) -> Result<RuntimeTaskRecord> {
        runtime
            .get_task(task_id)
            .await?
            .with_context(|| format!("missing test runtime task {task_id}"))
    }

    async fn cancel_test_task(
        runtime: &RuntimeManager,
        task_id: &str,
    ) -> Result<RuntimeMutationOutcome> {
        let record = current_test_task(runtime, task_id).await?;
        runtime
            .cancel_task_if_current(&record.lifetime(), None)
            .await
    }

    async fn remove_test_task(
        runtime: &RuntimeManager,
        task_id: &str,
    ) -> Result<RuntimeMutationOutcome> {
        let record = current_test_task(runtime, task_id).await?;
        let retired = runtime.retire_task_if_unchanged(&record, None).await?;
        anyhow::ensure!(
            matches!(
                retired,
                RuntimeMutationOutcome::Applied | RuntimeMutationOutcome::AlreadyApplied
            ),
            "test task {task_id} could not be retired before removal: {retired:?}"
        );
        runtime.remove_task_if_current(&record.lifetime()).await
    }

    #[tokio::test]
    async fn active_task_cannot_be_removed_without_retirement() -> Result<()> {
        let runtime =
            RuntimeManager::new_memory("test".into(), "remove-requires-retirement".into())?;
        let root = register_native_task(&runtime, "root").await?;

        assert_eq!(
            runtime.remove_task_if_current(&root.lifetime()).await?,
            RuntimeMutationOutcome::Blocked
        );
        assert!(runtime.get_task("root").await?.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn namespace_reset_is_rejected_after_initialization() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "reset-live-runtime".into())?;
        runtime.initialize().await?;

        let error = runtime
            .reset_namespace()
            .await
            .expect_err("initialized runtime must reject namespace reset");
        assert!(error.to_string().contains("before initialization"));
        Ok(())
    }

    #[tokio::test]
    async fn draining_fences_canonical_preflight_manifest_mutations() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new_memory(
            "test".into(),
            "preflight-drain-fence".into(),
        )?);
        runtime.initialize().await?;
        let store = runtime.canonical_preflight_store();
        let existing_key = canonical_preflight_test_key(1);
        let existing = store
            .put_canonical_preflight_if_absent(&existing_key, b"existing")
            .await?
            .try_object()
            .context("expected canonical preflight object")?
            .clone();

        runtime.start_draining();

        let put_error = store
            .put_canonical_preflight_if_absent(&canonical_preflight_test_key(2), b"new")
            .await
            .expect_err("draining runtime must reject canonical preflight publication");
        assert!(put_error.to_string().contains("runtime is draining"));

        let invalidate_error = store
            .invalidate_canonical_preflight_exact(&existing_key, &existing.descriptor())
            .await
            .expect_err("draining runtime must reject canonical preflight invalidation");
        assert!(invalidate_error.to_string().contains("runtime is draining"));
        assert!(
            store
                .get_canonical_preflight(&existing_key)
                .await?
                .is_some()
        );
        Ok(())
    }

    #[test]
    fn artifact_state_keys_are_unambiguous() {
        let pipeline = PipelineKey::ShastaRisc0;
        let route = pipeline.route();
        let middle = format!("{}|{route}", pipeline.as_str());

        let first = artifact_record_key("left", pipeline, route, &format!("center|{middle}|right"));
        let second =
            artifact_record_key(&format!("left|{middle}|center"), pipeline, route, "right");

        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn exact_root_commands_do_not_touch_a_replacement() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "exact-root-commands".into())?;
        let registration = TaskRegistration {
            task_id: "root".into(),
            pipeline_key: PipelineKey::ShastaNative,
            route: PipelineKey::ShastaNative.route(),
            task_kind: "proposal".into(),
            network_pair: "l1-l2".into(),
            artifact_refs: Vec::new(),
            metadata: serde_json::json!({}),
            request_fingerprint: "root-request".into(),
        };
        let first = runtime.register_task(registration.clone()).await?;
        assert_eq!(
            runtime
                .cancel_task_if_current(&first.lifetime(), None)
                .await?,
            RuntimeMutationOutcome::Applied
        );
        let replacement = runtime.register_task(registration).await?;
        assert_ne!(first.incarnation_id, replacement.incarnation_id);
        assert_eq!(
            runtime.remove_task_if_current(&first.lifetime()).await?,
            RuntimeMutationOutcome::Stale
        );
        assert_eq!(
            runtime
                .get_task("root")
                .await?
                .map(|task| task.incarnation_id),
            Some(replacement.incarnation_id)
        );
        Ok(())
    }

    #[tokio::test]
    async fn stale_snapshot_cannot_cancel_or_retire_concurrent_progress() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "exact-cancel-snapshot".into())?;
        let stale = runtime
            .register_task(TaskRegistration {
                task_id: "root".into(),
                pipeline_key: PipelineKey::ShastaNative,
                route: PipelineKey::ShastaNative.route(),
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: Vec::new(),
                metadata: serde_json::json!({}),
                request_fingerprint: "root-request".into(),
            })
            .await?;
        runtime
            .update_tasks_by_ref("root", |records| {
                let current = records.first_mut().context("registered root")?;
                current.runner_status = RunnerStatus::Running;
                current.image_ref = Some("submitted-image".into());
                current.updated_at = current.updated_at.saturating_add(1);
                Ok(())
            })
            .await?;

        assert_eq!(
            runtime
                .cancel_task_if_unchanged(&stale, Some("stale cleanup".into()))
                .await?,
            RuntimeMutationOutcome::Blocked
        );
        assert_eq!(
            runtime
                .retire_task_if_unchanged(&stale, Some("stale invalidation".into()))
                .await?,
            RuntimeMutationOutcome::Blocked
        );
        let current = current_test_task(&runtime, "root").await?;
        assert_eq!(current.runner_status, RunnerStatus::Running);
        assert_eq!(current.image_ref.as_deref(), Some("submitted-image"));

        assert_eq!(
            runtime
                .cancel_task_if_unchanged(&current, Some("current cleanup".into()))
                .await?,
            RuntimeMutationOutcome::Applied
        );
        let cancelled = current_test_task(&runtime, "root").await?;
        assert_eq!(
            runtime.retire_task_if_unchanged(&current, None).await?,
            RuntimeMutationOutcome::Blocked,
            "a pre-cancellation snapshot must not bypass the exact retirement CAS"
        );
        assert_eq!(
            runtime.retire_task_if_unchanged(&cancelled, None).await?,
            RuntimeMutationOutcome::AlreadyApplied
        );
        Ok(())
    }

    #[tokio::test]
    async fn stale_recovery_snapshot_cannot_discard_new_checkpoint_metadata() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "exact-recovery-snapshot".into())?;
        let registration = TaskRegistration {
            task_id: "root".into(),
            pipeline_key: PipelineKey::ShastaNative,
            route: PipelineKey::ShastaNative.route(),
            task_kind: "proposal".into(),
            network_pair: "l1-l2".into(),
            artifact_refs: Vec::new(),
            metadata: serde_json::json!({ "checkpoint": "old" }),
            request_fingerprint: "root-request".into(),
        };
        let root = runtime.register_task(registration).await?;
        assert_eq!(
            runtime
                .fail_task_if_unchanged(&root, "retryable".into())
                .await?,
            RuntimeMutationOutcome::Applied,
        );
        let stale = current_test_task(&runtime, "root").await?;
        runtime
            .mutate(|state| {
                let task = state.tasks.get_mut("root").context("registered root")?;
                task.metadata = serde_json::json!({ "checkpoint": "new" });
                task.updated_at = now_ts();
                Ok(())
            })
            .await?;

        assert!(
            runtime
                .prepare_task_for_recovery_if_unchanged(&stale)
                .await?
                .is_none()
        );
        let current = current_test_task(&runtime, "root").await?;
        assert_eq!(current.runner_status, RunnerStatus::Failed);
        assert_eq!(current.metadata, serde_json::json!({ "checkpoint": "new" }));
        Ok(())
    }

    #[tokio::test]
    async fn recovery_rollback_is_an_exact_snapshot_cas() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "exact-recovery-rollback".into())?;
        let root = runtime
            .register_task(TaskRegistration {
                task_id: "root".into(),
                pipeline_key: PipelineKey::ShastaNative,
                route: PipelineKey::ShastaNative.route(),
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: Vec::new(),
                metadata: serde_json::json!({}),
                request_fingerprint: "root-request".into(),
            })
            .await?;
        assert_eq!(
            runtime
                .fail_task_if_unchanged(&root, "retryable".into())
                .await?,
            RuntimeMutationOutcome::Applied,
        );
        let original = current_test_task(&runtime, "root").await?;
        let reopened = runtime
            .prepare_task_for_recovery_if_unchanged(&original)
            .await?
            .context("reopened root")?;

        assert_eq!(
            runtime
                .restore_task_after_recovery_if_unchanged(&reopened, &original)
                .await?,
            RuntimeMutationOutcome::Applied,
        );
        assert_eq!(current_test_task(&runtime, "root").await?, original);

        let reopened = runtime
            .prepare_task_for_recovery_if_unchanged(&original)
            .await?
            .context("reopened root")?;
        runtime
            .update_tasks_by_ref("root", |records| {
                let current = records
                    .iter_mut()
                    .find(|record| record.incarnation_id == reopened.incarnation_id)
                    .context("reopened root")?;
                current.runner_status = RunnerStatus::Running;
                current.error = Some("concurrent progress".into());
                current.updated_at = now_ts();
                Ok(())
            })
            .await?;
        assert_eq!(
            runtime
                .restore_task_after_recovery_if_unchanged(&reopened, &original)
                .await?,
            RuntimeMutationOutcome::Blocked,
        );
        let current = current_test_task(&runtime, "root").await?;
        assert_eq!(current.runner_status, RunnerStatus::Running);
        assert_eq!(current.error.as_deref(), Some("concurrent progress"));
        Ok(())
    }

    #[tokio::test]
    async fn stale_failure_snapshot_cannot_overwrite_concurrent_progress() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "exact-failure-cas".into())?;
        let stale = runtime
            .register_task(TaskRegistration {
                task_id: "root".into(),
                pipeline_key: PipelineKey::ShastaNative,
                route: PipelineKey::ShastaNative.route(),
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: Vec::new(),
                metadata: serde_json::json!({}),
                request_fingerprint: "root-request".into(),
            })
            .await?;
        runtime
            .update_tasks_by_ref("root", |records| {
                let current = records.first_mut().context("registered root")?;
                current.runner_status = RunnerStatus::Running;
                current.image_ref = Some("submitted-image".into());
                current.updated_at = now_ts();
                Ok(())
            })
            .await?;

        assert_eq!(
            runtime
                .fail_task_if_unchanged(&stale, "stale failure".into())
                .await?,
            RuntimeMutationOutcome::Blocked
        );
        let current = current_test_task(&runtime, "root").await?;
        assert_eq!(current.runner_status, RunnerStatus::Running);
        assert_eq!(current.image_ref.as_deref(), Some("submitted-image"));
        assert_eq!(current.error, None);
        Ok(())
    }

    #[tokio::test]
    async fn legacy_sgxgeth_state_loads_without_rekeying_artifacts() -> Result<()> {
        let network_pair = "taiko_dev/taiko_dev_l1";
        let proof_ref = "legacy-sgxgeth-proof";
        let pipeline_key = PipelineKey::ShastaSgxGeth;
        let legacy_route = "sgx/remote"
            .parse::<PipelineRoute>()
            .expect("parse legacy SGXGETH route");
        let canonical_route = pipeline_key.route();
        let incarnation_id = uuid::Uuid::new_v4();
        let store = Arc::new(MemoryProofArtifactStore::new(
            "test".into(),
            "legacy-sgxgeth-route".into(),
        )?);

        let artifact_key =
            RuntimeManager::artifact_key(network_pair, pipeline_key, legacy_route, proof_ref);
        let artifact = store
            .put_if_absent(&artifact_key, b"legacy-proof")
            .await?
            .try_object()
            .context("legacy artifact publication")?
            .clone();
        let pending_key = RuntimeManager::pending_artifact_key(
            network_pair,
            pipeline_key,
            legacy_route,
            proof_ref,
        );
        let pending = store
            .put_if_absent(&pending_key, b"legacy-pending-proof")
            .await?
            .try_object()
            .context("legacy pending publication")?
            .clone();
        let task = RuntimeTaskRecord {
            task_id: "legacy-sgxgeth-root".into(),
            incarnation_id,
            pipeline_key,
            route: legacy_route,
            task_kind: "proposal".into(),
            network_pair: network_pair.into(),
            artifact_refs: vec![proof_ref.into()],
            runner_status: RunnerStatus::Running,
            image_ref: None,
            proof_uri: None,
            error: None,
            metadata: serde_json::json!({}),
            request_fingerprint: "legacy-sgxgeth-request".into(),
            updated_at: 1,
        };
        let state_key = artifact_record_key(network_pair, pipeline_key, legacy_route, proof_ref);
        let state = RuntimeState {
            tasks: HashMap::from([(task.task_id.clone(), task)]),
            artifacts: HashMap::from([(
                state_key.clone(),
                ProofArtifactRecord {
                    environment: "test".into(),
                    network_pair: network_pair.into(),
                    proof_ref: proof_ref.into(),
                    pipeline_key,
                    route: legacy_route,
                    proof_uri: artifact.proof_uri,
                    content_hash: artifact.content_hash,
                    generation: artifact.generation,
                    lifecycle: ProofArtifactLifecycle::Active,
                    invalidated_at: None,
                    updated_at: 1,
                },
            )]),
            pending_publications: HashMap::from([(
                state_key,
                PendingProofPublicationRecord {
                    network_pair: network_pair.into(),
                    pipeline_key,
                    route: legacy_route,
                    proof_ref: proof_ref.into(),
                    content_hash: pending.content_hash,
                    owner_incarnations: vec![incarnation_id],
                },
            )]),
        };
        assert!(matches!(
            store
                .store_runtime_state(&serde_json::to_vec(&state)?, None)
                .await?,
            RuntimeStateWriteResult::Stored { .. }
        ));

        let runtime = RuntimeManager::with_store(store);
        runtime.initialize().await?;

        let stored = runtime
            .get_task("legacy-sgxgeth-root")
            .await?
            .context("legacy task")?;
        assert_eq!(stored.route, legacy_route);
        assert_eq!(
            runtime
                .read_proof_artifact_bytes(network_pair, pipeline_key, legacy_route, proof_ref)
                .await?
                .context("legacy artifact")?
                .bytes,
            b"legacy-proof"
        );
        assert!(
            runtime
                .read_proof_artifact_bytes(network_pair, pipeline_key, canonical_route, proof_ref)
                .await?
                .is_none()
        );
        assert_eq!(
            runtime
                .get_recoverable_pending_proof_publication(
                    network_pair,
                    pipeline_key,
                    legacy_route,
                    proof_ref,
                )
                .await?
                .context("legacy pending publication")?
                .bytes,
            b"legacy-pending-proof"
        );
        Ok(())
    }

    #[tokio::test]
    async fn new_sgxgeth_registrations_reject_legacy_route() -> Result<()> {
        let pipeline_key = PipelineKey::ShastaSgxGeth;
        let legacy_route = "sgx/remote"
            .parse::<PipelineRoute>()
            .expect("parse legacy SGXGETH route");
        let task_error = build_task_record(&TaskRegistration {
            task_id: "new-sgxgeth-root".into(),
            pipeline_key,
            route: legacy_route,
            task_kind: "proposal".into(),
            network_pair: "taiko_dev/taiko_dev_l1".into(),
            artifact_refs: vec!["new-sgxgeth-proof".into()],
            metadata: serde_json::json!({}),
            request_fingerprint: "new-sgxgeth-request".into(),
        })
        .expect_err("new task registration must require the canonical route");
        assert!(task_error.to_string().contains("does not match route"));

        let runtime =
            RuntimeManager::new_memory("test".into(), "new-sgxgeth-registration-strict".into())?;
        let artifact_error = runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: "taiko_dev/taiko_dev_l1".into(),
                proof_ref: "new-sgxgeth-proof".into(),
                pipeline_key,
                route: legacy_route,
                proof_uri: "memory://new-sgxgeth-proof".into(),
                content_hash: "new-sgxgeth-hash".into(),
                generation: None,
            })
            .await
            .expect_err("new artifact registration must require the canonical route");
        assert!(
            artifact_error
                .to_string()
                .contains("does not support its route")
        );
        Ok(())
    }

    #[test]
    fn persisted_route_compatibility_rejects_other_mismatches() -> Result<()> {
        let mut task = build_task_record(&TaskRegistration {
            task_id: "mismatched-sgxgeth-root".into(),
            pipeline_key: PipelineKey::ShastaSgxGeth,
            route: PipelineKey::ShastaSgxGeth.route(),
            task_kind: "proposal".into(),
            network_pair: "taiko_dev/taiko_dev_l1".into(),
            artifact_refs: Vec::new(),
            metadata: serde_json::json!({}),
            request_fingerprint: "mismatched-sgxgeth-request".into(),
        })?;
        task.route = "risc0/local"
            .parse::<PipelineRoute>()
            .expect("parse mismatched route");
        let state = RuntimeState {
            tasks: HashMap::from([(task.task_id.clone(), task)]),
            ..RuntimeState::default()
        };

        let error = validate_runtime_state(&state, "test")
            .expect_err("nonhistorical persisted route mismatch must fail closed");
        assert!(error.to_string().contains("does not support its route"));
        Ok(())
    }

    #[tokio::test]
    async fn superseded_runtime_state_schema_fails_closed() -> Result<()> {
        let record = build_task_record(&TaskRegistration {
            task_id: "legacy-root".into(),
            pipeline_key: PipelineKey::ShastaRisc0Network,
            route: PipelineKey::ShastaRisc0Network.route(),
            task_kind: "aggregate".into(),
            network_pair: "taiko_dev/taiko_dev_l1".into(),
            artifact_refs: vec!["legacy-proof".into()],
            metadata: serde_json::json!({ "network_pair": "taiko_dev/taiko_dev_l1" }),
            request_fingerprint: "legacy-request".into(),
        })?;
        let artifact = serde_json::to_value(ProofArtifactRecord {
            environment: "test".into(),
            network_pair: "taiko_dev/taiko_dev_l1".into(),
            proof_ref: "legacy-proof".into(),
            pipeline_key: PipelineKey::ShastaRisc0Network,
            route: PipelineKey::ShastaRisc0Network.route(),
            proof_uri: "memory://legacy-proof".into(),
            content_hash: "legacy-hash".into(),
            generation: None,
            lifecycle: ProofArtifactLifecycle::Active,
            invalidated_at: None,
            updated_at: 1,
        })?;
        let legacy_artifact_key = artifact_record_key(
            "taiko_dev/taiko_dev_l1",
            PipelineKey::ShastaRisc0Network,
            PipelineKey::ShastaRisc0Network.route(),
            "legacy-proof",
        );
        let current_state = serde_json::json!({
            "tasks": { "legacy-root": serde_json::to_value(record)? },
            "artifacts": { legacy_artifact_key.clone(): artifact },
            "pending_publications": {},
        });

        let mut missing_incarnation = current_state.clone();
        missing_incarnation["tasks"]["legacy-root"]
            .as_object_mut()
            .context("legacy task must be an object")?
            .remove("incarnation_id");
        let mut missing_network_pair = current_state.clone();
        missing_network_pair["tasks"]["legacy-root"]
            .as_object_mut()
            .context("legacy task must be an object")?
            .remove("network_pair");
        let mut missing_artifact_refs = current_state.clone();
        missing_artifact_refs["tasks"]["legacy-root"]
            .as_object_mut()
            .context("legacy task must be an object")?
            .remove("artifact_refs");
        let mut missing_request_fingerprint = current_state.clone();
        missing_request_fingerprint["tasks"]["legacy-root"]
            .as_object_mut()
            .context("legacy task must be an object")?
            .remove("request_fingerprint");
        let mut superseded_proof_ids = current_state.clone();
        superseded_proof_ids["tasks"]["legacy-root"]
            .as_object_mut()
            .context("legacy task must be an object")?
            .insert("proof_ids".into(), serde_json::json!(["legacy-proof"]));
        let mut missing_artifact_lifecycle = current_state;
        missing_artifact_lifecycle["artifacts"]
            .get_mut(&legacy_artifact_key)
            .and_then(serde_json::Value::as_object_mut)
            .context("legacy artifact must be an object")?
            .remove("lifecycle");

        for (namespace, legacy_state) in [
            ("legacy-missing-incarnation", missing_incarnation),
            ("legacy-missing-network-pair", missing_network_pair),
            ("legacy-missing-artifact-refs", missing_artifact_refs),
            (
                "legacy-missing-request-fingerprint",
                missing_request_fingerprint,
            ),
            ("legacy-superseded-proof-ids", superseded_proof_ids),
            (
                "legacy-missing-artifact-lifecycle",
                missing_artifact_lifecycle,
            ),
        ] {
            let store = Arc::new(MemoryProofArtifactStore::new(
                "test".into(),
                namespace.into(),
            )?);
            assert!(matches!(
                store
                    .store_runtime_state(&serde_json::to_vec(&legacy_state)?, None)
                    .await?,
                RuntimeStateWriteResult::Stored { .. }
            ));
            let runtime = RuntimeManager::with_store(store);
            runtime
                .initialize()
                .await
                .expect_err("legacy runtime state must require an empty namespace cutover");
        }
        Ok(())
    }

    #[derive(Debug)]
    struct RuntimeStateProbeStore {
        inner: MemoryProofArtifactStore,
        runtime_state_writes: AtomicUsize,
        force_conflict: AtomicBool,
        commit_then_error: AtomicBool,
        foreign_commit_then_error: AtomicBool,
        fail_before_commit: AtomicUsize,
        rewrite_next_runtime_state_readback: AtomicBool,
        block_next_runtime_state_write: AtomicBool,
        runtime_state_write_entered: tokio::sync::Notify,
        allow_runtime_state_write: tokio::sync::Notify,
        fail_next_artifact_put: AtomicBool,
        block_next_artifact_put: AtomicBool,
        artifact_put_entered: tokio::sync::Notify,
        allow_artifact_put: tokio::sync::Notify,
        fail_next_artifact_delete: AtomicBool,
        block_next_artifact_delete: AtomicBool,
        artifact_delete_completed: tokio::sync::Notify,
        allow_artifact_delete_return: tokio::sync::Notify,
        dangling_artifacts: StdMutex<HashSet<ProofArtifactKey>>,
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
                rewrite_next_runtime_state_readback: AtomicBool::new(false),
                block_next_runtime_state_write: AtomicBool::new(false),
                runtime_state_write_entered: tokio::sync::Notify::new(),
                allow_runtime_state_write: tokio::sync::Notify::new(),
                fail_next_artifact_put: AtomicBool::new(false),
                block_next_artifact_put: AtomicBool::new(false),
                artifact_put_entered: tokio::sync::Notify::new(),
                allow_artifact_put: tokio::sync::Notify::new(),
                fail_next_artifact_delete: AtomicBool::new(false),
                block_next_artifact_delete: AtomicBool::new(false),
                artifact_delete_completed: tokio::sync::Notify::new(),
                allow_artifact_delete_return: tokio::sync::Notify::new(),
                dangling_artifacts: StdMutex::new(HashSet::new()),
            })
        }

        fn mark_artifact_content_missing(&self, key: ProofArtifactKey) -> Result<()> {
            self.dangling_artifacts
                .lock()
                .map_err(|_| anyhow::anyhow!("dangling artifact set poisoned"))?
                .insert(key);
            Ok(())
        }
    }

    impl RuntimeStoreScope for RuntimeStateProbeStore {
        fn environment(&self) -> &str {
            self.inner.environment()
        }

        fn namespace(&self) -> &str {
            self.inner.namespace()
        }

        fn backend_name(&self) -> &'static str {
            "test"
        }
    }

    #[async_trait::async_trait]
    impl ProofObjectStore for RuntimeStateProbeStore {
        async fn put_if_absent(
            &self,
            key: &ProofArtifactKey,
            bytes: &[u8],
        ) -> Result<ProofArtifactPutResult> {
            if self.fail_next_artifact_put.swap(false, Ordering::SeqCst) {
                anyhow::bail!("injected artifact put failure before commit");
            }
            if self.block_next_artifact_put.swap(false, Ordering::SeqCst) {
                self.artifact_put_entered.notify_one();
                self.allow_artifact_put.notified().await;
            }
            self.inner.put_if_absent(key, bytes).await
        }

        async fn get(&self, key: &ProofArtifactKey) -> Result<Option<ProofArtifactObject>> {
            let content_missing = self
                .dangling_artifacts
                .lock()
                .map_err(|_| anyhow::anyhow!("dangling artifact set poisoned"))?
                .contains(key);
            if content_missing && self.inner.get_descriptor(key).await?.is_some() {
                anyhow::bail!("proof manifest references missing content");
            }
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

        async fn invalidate_exact(
            &self,
            key: &ProofArtifactKey,
            descriptor: &ProofArtifactDescriptor,
        ) -> Result<ExactInvalidationResult> {
            let result = self.inner.invalidate_exact(key, descriptor).await?;
            if self
                .block_next_artifact_delete
                .swap(false, Ordering::SeqCst)
            {
                self.artifact_delete_completed.notify_one();
                self.allow_artifact_delete_return.notified().await;
            }
            Ok(result)
        }

        async fn is_invalidated(
            &self,
            key: &ProofArtifactKey,
            descriptor: &ProofArtifactDescriptor,
        ) -> Result<bool> {
            self.inner.is_invalidated(key, descriptor).await
        }

        async fn delete_exact(
            &self,
            key: &ProofArtifactKey,
            descriptor: &ProofArtifactDescriptor,
        ) -> Result<ProofArtifactDeleteResult> {
            if self.fail_next_artifact_delete.swap(false, Ordering::SeqCst) {
                anyhow::bail!("injected artifact delete failure before commit");
            }
            let result = self.inner.delete_exact(key, descriptor).await?;
            if self
                .block_next_artifact_delete
                .swap(false, Ordering::SeqCst)
            {
                self.artifact_delete_completed.notify_one();
                self.allow_artifact_delete_return.notified().await;
            }
            Ok(result)
        }
    }

    #[async_trait::async_trait]
    impl RuntimeStateStore for RuntimeStateProbeStore {
        async fn load_runtime_state(&self) -> Result<Option<RuntimeStateObject>> {
            let mut stored = self.inner.load_runtime_state().await?;
            if self
                .rewrite_next_runtime_state_readback
                .swap(false, Ordering::SeqCst)
                && let Some(stored) = stored.as_mut()
            {
                let state = serde_json::from_slice::<serde_json::Value>(&stored.bytes)?;
                stored.bytes = serde_json::to_vec_pretty(&state)?;
            }
            Ok(stored)
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
            pipeline_key: PipelineKey::ShastaNative,
            route: PipelineKey::ShastaNative.route(),
            task_kind: "proposal".into(),
            network_pair: "l1-l2".into(),
            artifact_refs: Vec::new(),
            metadata: serde_json::json!({}),
            request_fingerprint: "same".into(),
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
                pipeline_key: PipelineKey::ShastaSp1,
                route: PipelineKey::ShastaSp1.route(),
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: Vec::new(),
                metadata: serde_json::json!({}),
                request_fingerprint: "checkpoint-root".into(),
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
        let root = current_test_task(runtime.as_ref(), "checkpoint-root").await?;
        assert!(
            runtime
                .remove_task_if_current(&root.lifetime())
                .await
                .is_err()
        );
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
    async fn draining_waits_for_in_flight_object_commit() -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new("object-commit-drain")?);
        store.block_next_artifact_put.store(true, Ordering::SeqCst);
        let runtime = Arc::new(RuntimeManager::with_store(store.clone()));
        let publication = tokio::spawn({
            let runtime = Arc::clone(&runtime);
            async move {
                runtime
                    .publish_proof_artifact_bytes(
                        "l1-l2",
                        PipelineKey::ShastaSp1,
                        PipelineKey::ShastaSp1.route(),
                        "proof",
                        b"proof",
                    )
                    .await
            }
        });
        store.artifact_put_entered.notified().await;
        let mut draining = tokio::spawn({
            let runtime = Arc::clone(&runtime);
            async move { runtime.begin_draining().await }
        });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut draining)
                .await
                .is_err()
        );
        store.allow_artifact_put.notify_one();
        publication
            .await?
            .expect_err("draining must reject the post-commit saga step");
        draining.await?;
        assert!(!runtime.accepts_mutations());
        Ok(())
    }

    #[tokio::test]
    async fn unrelated_artifacts_do_not_share_a_lifecycle_lock() -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new("artifact-lock-scope")?);
        store.block_next_artifact_put.store(true, Ordering::SeqCst);
        let runtime = Arc::new(RuntimeManager::with_store(store.clone()));
        let pipeline = PipelineKey::ShastaSp1;
        let route = pipeline.route();
        let blocked_ref = "blocked-proof";
        let independent_ref = (0..ARTIFACT_LIFECYCLE_LOCK_SHARDS * 2)
            .map(|index| format!("independent-proof-{index}"))
            .find(|candidate| {
                !std::ptr::eq(
                    runtime.artifact_lifecycle_lock("l1-l2", pipeline, route, blocked_ref),
                    runtime.artifact_lifecycle_lock("l1-l2", pipeline, route, candidate),
                )
            })
            .context("failed to select a different artifact lock shard")?;
        let blocked = tokio::spawn({
            let runtime = Arc::clone(&runtime);
            async move {
                runtime
                    .publish_proof_artifact_bytes("l1-l2", pipeline, route, blocked_ref, b"blocked")
                    .await
            }
        });
        store.artifact_put_entered.notified().await;

        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            runtime.publish_proof_artifact_bytes(
                "l1-l2",
                pipeline,
                route,
                &independent_ref,
                b"independent",
            ),
        )
        .await
        .context("unrelated artifact publication waited on a namespace-wide lock")??;

        store.allow_artifact_put.notify_one();
        blocked.await??;
        Ok(())
    }

    #[tokio::test]
    async fn pre_admitted_checkpoint_recovers_while_runtime_is_draining() -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new("checkpoint-drain-retry")?);
        let runtime = RuntimeManager::with_store(store.clone());
        runtime
            .register_task(TaskRegistration {
                task_id: "checkpoint-root".into(),
                pipeline_key: PipelineKey::ShastaSp1,
                route: PipelineKey::ShastaSp1.route(),
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: Vec::new(),
                metadata: serde_json::json!({}),
                request_fingerprint: "checkpoint-root".into(),
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
        let runtime = Arc::new(RuntimeManager::with_store(store.clone()));
        runtime
            .register_task(TaskRegistration {
                task_id: "checkpoint-root".into(),
                pipeline_key: PipelineKey::ShastaSp1,
                route: PipelineKey::ShastaSp1.route(),
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: Vec::new(),
                metadata: serde_json::json!({}),
                request_fingerprint: "checkpoint-root".into(),
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
                pipeline_key: PipelineKey::ShastaSp1,
                route: PipelineKey::ShastaSp1.route(),
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: vec!["pending".into()],
                metadata: serde_json::json!({"network_pair": "l1-l2"}),
                request_fingerprint: "pending-root".into(),
            })
            .await?;
        let active = registration("active", 1);
        let mut pending = registration("pending", 2);
        pending.content_hash = artifact_store::content_hash(b"pending-proof");
        let invalidated = registration("invalidated", 3);
        runtime.upsert_proof_artifact(active.clone()).await?;
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
        assert_eq!(
            runtime
                .register_pending_proof_artifact(pending.clone())
                .await?,
            ProofArtifactLifecycle::Pending
        );
        runtime
            .register_invalidated_proof_artifact(invalidated.clone())
            .await?;
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
        let runtime = RuntimeManager::with_store(store.clone());

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
        let runtime = RuntimeManager::with_store(store.clone());

        let error = runtime
            .mutate(|_| Ok(()))
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
    async fn readiness_rejects_out_of_band_runtime_generation_change() -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new(
            "readiness-generation-conflict",
        )?);
        let runtime = RuntimeManager::with_store(store.clone());
        runtime
            .register_task(TaskRegistration {
                task_id: "root".into(),
                pipeline_key: PipelineKey::ShastaNative,
                route: PipelineKey::ShastaNative.route(),
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: Vec::new(),
                metadata: serde_json::json!({}),
                request_fingerprint: "root-request".into(),
            })
            .await?;
        let stored = store
            .inner
            .load_runtime_state()
            .await?
            .context("runtime state")?;
        assert!(matches!(
            store
                .inner
                .store_runtime_state(&stored.bytes, stored.generation)
                .await?,
            RuntimeStateWriteResult::Stored { .. }
        ));

        let error = runtime
            .check_readiness()
            .await
            .expect_err("foreign generation must fail readiness");
        assert!(format!("{error:#}").contains("outside the authoritative repository"));
        assert!(!runtime.accepts_mutations());
        Ok(())
    }

    #[tokio::test]
    async fn runtime_state_transport_error_recovers_committed_write_by_readback() -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new("state-readback")?);
        store.commit_then_error.store(true, Ordering::SeqCst);
        let runtime = RuntimeManager::with_store(store.clone());

        runtime
            .register_task(TaskRegistration {
                task_id: "committed-task".into(),
                pipeline_key: PipelineKey::ShastaNative,
                route: PipelineKey::ShastaNative.route(),
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: Vec::new(),
                metadata: serde_json::json!({}),
                request_fingerprint: "committed-task".into(),
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
        let runtime = RuntimeManager::with_store(store.clone());

        let error = runtime
            .register_task(TaskRegistration {
                task_id: "must-not-commit".into(),
                pipeline_key: PipelineKey::ShastaNative,
                route: PipelineKey::ShastaNative.route(),
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: Vec::new(),
                metadata: serde_json::json!({}),
                request_fingerprint: "must-not-commit".into(),
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
        let runtime = RuntimeManager::with_store(store.clone());

        runtime
            .mutate(|_| Ok(()))
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
    async fn precommit_transport_error_uses_generation_not_json_byte_order() -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new("state-reformatted-readback")?);
        let runtime = RuntimeManager::with_store(store.clone());
        runtime
            .register_task(TaskRegistration {
                task_id: "root".into(),
                pipeline_key: PipelineKey::ShastaNative,
                route: PipelineKey::ShastaNative.route(),
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: Vec::new(),
                metadata: serde_json::json!({}),
                request_fingerprint: "root-request".into(),
            })
            .await?;
        store.fail_before_commit.store(3, Ordering::SeqCst);
        store
            .rewrite_next_runtime_state_readback
            .store(true, Ordering::SeqCst);

        runtime
            .mutate(|state| {
                state
                    .tasks
                    .get_mut("root")
                    .context("registered root")?
                    .updated_at += 1;
                Ok(())
            })
            .await
            .expect_err("transport failures must exhaust the retry budget");

        assert!(runtime.mutation_failure_is_retryable());
        runtime.check_readiness().await?;
        assert!(runtime.accepts_mutations());
        Ok(())
    }

    #[tokio::test]
    async fn stale_artifact_completion_cannot_complete_replacement_task() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "stale-artifact-task".into())?;
        let first = register_native_task(&runtime, "root").await?;
        let precondition = active_artifact_precondition(&runtime, "stale-task-proof").await?;

        assert!(matches!(
            remove_test_task(&runtime, "root").await?,
            RuntimeMutationOutcome::Applied
        ));
        let replacement = register_native_task(&runtime, "root").await?;

        assert!(
            !runtime
                .complete_nonterminal_task(
                    "root",
                    first.incarnation_id,
                    "memory://stale-task-proof",
                    &[precondition],
                )
                .await?
        );
        let current = runtime.get_task("root").await?.expect("replacement task");
        assert_eq!(current.incarnation_id, replacement.incarnation_id);
        assert_eq!(current.runner_status, RunnerStatus::Allocated);
        assert_eq!(current.proof_uri, None);
        Ok(())
    }

    #[tokio::test]
    async fn stale_artifact_completion_requires_exact_active_descriptor() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "stale-artifact-record".into())?;
        let task = register_native_task(&runtime, "root").await?;
        let precondition = active_artifact_precondition(&runtime, "stale-record-proof").await?;
        runtime
            .mark_proof_artifact_descriptor_invalidated(
                &precondition.network_pair,
                precondition.pipeline_key,
                precondition.route,
                &precondition.proof_ref,
                &precondition.descriptor,
            )
            .await?;
        let proof_uri = precondition.descriptor.proof_uri.clone();

        assert!(!runtime
            .complete_nonterminal_task(
                "root",
                task.incarnation_id,
                &proof_uri,
                &[precondition],
            )
            .await?);
        let current = runtime.get_task("root").await?.expect("runtime task");
        assert_eq!(current.runner_status, RunnerStatus::Allocated);
        assert_eq!(current.proof_uri, None);
        Ok(())
    }

    async fn register_native_task(
        runtime: &RuntimeManager,
        task_id: &str,
    ) -> Result<RuntimeTaskRecord> {
        runtime
            .register_task(TaskRegistration {
                task_id: task_id.into(),
                pipeline_key: PipelineKey::ShastaNative,
                route: PipelineKey::ShastaNative.route(),
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: Vec::new(),
                metadata: serde_json::json!({}),
                request_fingerprint: format!("request-{task_id}"),
            })
            .await
    }

    async fn active_artifact_precondition(
        runtime: &RuntimeManager,
        proof_ref: &str,
    ) -> Result<ProofArtifactPrecondition> {
        let pipeline_key = PipelineKey::ShastaNative;
        let route = pipeline_key.route();
        let object = runtime
            .publish_proof_artifact_bytes(
                "taiko_dev/ethereum",
                pipeline_key,
                route,
                proof_ref,
                br#"{"proof":"0x01"}"#,
            )
            .await?
            .try_object()
            .context("expected proof artifact object")?
            .clone();
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: "taiko_dev/ethereum".into(),
                proof_ref: proof_ref.into(),
                pipeline_key,
                route,
                proof_uri: object.proof_uri.clone(),
                content_hash: object.content_hash.clone(),
                generation: object.generation,
            })
            .await?;
        Ok(ProofArtifactPrecondition {
            network_pair: "taiko_dev/ethereum".into(),
            proof_ref: proof_ref.into(),
            pipeline_key,
            route,
            descriptor: object.descriptor(),
        })
    }

    #[tokio::test]
    async fn pending_proof_is_durable_without_expanding_runtime_state() -> Result<()> {
        let store = Arc::new(MemoryProofArtifactStore::new(
            "test".into(),
            "pending-proof".into(),
        )?);
        let first = RuntimeManager::with_store(store.clone());
        first
            .register_task(TaskRegistration {
                task_id: "task-a".into(),
                pipeline_key: PipelineKey::ShastaSp1,
                route: PipelineKey::ShastaSp1.route(),
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: Vec::new(),
                metadata: serde_json::json!({}),
                request_fingerprint: "task-a".into(),
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

        let recovered = RuntimeManager::with_store(store);
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
        )?));
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
        let first = RuntimeManager::with_store(store.clone());
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

        let restarted = RuntimeManager::with_store(store);
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
            pipeline_key: PipelineKey::ShastaSp1,
            route: PipelineKey::ShastaSp1.route(),
            task_kind: "proposal".into(),
            network_pair: "l1-l2".into(),
            artifact_refs: vec!["proposal-1".into()],
            metadata: serde_json::json!({}),
            request_fingerprint: "same-request".into(),
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
        cancel_test_task(&runtime, "root").await?;
        remove_test_task(&runtime, "root").await?;
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
        assert!(
            !runtime
                .invalidate_pending_proof_publication(
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
    async fn replacement_root_does_not_own_the_previous_incarnations_publication() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "replacement-outbox".into())?;
        let pipeline = PipelineKey::ShastaSp1;
        let route = pipeline.route();
        let proof_ref = "proposal-1";
        let registration = TaskRegistration {
            task_id: "root".into(),
            pipeline_key: pipeline,
            route,
            task_kind: "proposal".into(),
            network_pair: "l1-l2".into(),
            artifact_refs: vec![proof_ref.into()],
            metadata: serde_json::json!({}),
            request_fingerprint: "same-request".into(),
        };
        let first = runtime.register_task(registration.clone()).await?;
        assert!(
            runtime
                .checkpoint_pending_proof_publication(
                    "l1-l2",
                    pipeline,
                    route,
                    proof_ref,
                    &[first.incarnation_id],
                    b"old-proof",
                )
                .await?
        );
        let old_artifact = runtime
            .publish_proof_artifact_bytes("l1-l2", pipeline, route, proof_ref, b"old-proof")
            .await?
            .try_object()
            .context("old canonical artifact")?
            .clone();
        cancel_test_task(&runtime, "root").await?;
        remove_test_task(&runtime, "root").await?;
        let replacement = runtime.register_task(registration).await?;

        assert!(
            runtime
                .invalidate_pending_proof_publication("l1-l2", pipeline, route, proof_ref,)
                .await?
        );
        assert!(
            runtime
                .get_pending_proof_publication("l1-l2", pipeline, route, proof_ref)
                .await?
                .is_none()
        );
        assert!(
            runtime
                .proof_artifact_descriptor_is_invalidated(
                    "l1-l2",
                    pipeline,
                    route,
                    proof_ref,
                    &old_artifact.descriptor(),
                )
                .await?
        );
        assert_eq!(
            runtime
                .get_task("root")
                .await?
                .context("replacement root")?
                .incarnation_id,
            replacement.incarnation_id
        );
        Ok(())
    }

    #[tokio::test]
    async fn pending_owner_is_bound_to_the_full_artifact_identity() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "pending-owner-identity".into())?;
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let proof_ref = "shared-ref";
        let root = runtime
            .register_task(TaskRegistration {
                task_id: "root".into(),
                pipeline_key: pipeline,
                route,
                task_kind: "proposal".into(),
                network_pair: "network-a".into(),
                artifact_refs: vec![proof_ref.into()],
                metadata: serde_json::json!({}),
                request_fingerprint: "request".into(),
            })
            .await?;

        assert!(
            !runtime
                .checkpoint_pending_proof_publication(
                    "network-b",
                    pipeline,
                    route,
                    proof_ref,
                    &[root.incarnation_id],
                    b"proof",
                )
                .await?
        );
        assert!(
            !runtime
                .checkpoint_pending_proof_publication(
                    "network-a",
                    PipelineKey::ShastaSp1,
                    PipelineKey::ShastaSp1.route(),
                    proof_ref,
                    &[root.incarnation_id],
                    b"proof",
                )
                .await?
        );
        assert!(
            !runtime
                .checkpoint_pending_proof_publication(
                    "network-a",
                    pipeline,
                    PipelineKey::ShastaRisc0.route(),
                    proof_ref,
                    &[root.incarnation_id],
                    b"proof",
                )
                .await?
        );
        assert!(
            runtime
                .checkpoint_pending_proof_publication(
                    "network-a",
                    pipeline,
                    route,
                    proof_ref,
                    &[root.incarnation_id],
                    b"proof",
                )
                .await?
        );
        Ok(())
    }

    #[tokio::test]
    async fn active_artifact_publication_requires_a_durable_task_owner() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "active-owner".into())?;
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let proof_ref = "external-input-0";

        let error = runtime
            .publish_active_proof_artifact_bytes(
                "l1-l2",
                pipeline,
                route,
                proof_ref,
                uuid::Uuid::new_v4(),
                b"proof",
            )
            .await
            .expect_err("anonymous active artifacts must be rejected");

        assert!(error.to_string().contains("durable task owner"));
        assert!(runtime.list_proof_artifacts().await?.is_empty());
        assert!(
            runtime
                .read_proof_artifact_bytes("l1-l2", pipeline, route, proof_ref)
                .await?
                .is_none()
        );
        Ok(())
    }

    #[tokio::test]
    async fn terminal_task_cannot_publish_active_input_artifacts() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "terminal-input-owner".into())?;
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();

        let failed = match runtime
            .register_task_if_absent(TaskRegistration {
                task_id: "failed-root".into(),
                pipeline_key: pipeline,
                route,
                task_kind: "aggregate".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: vec!["failed-input".into()],
                metadata: serde_json::json!({}),
                request_fingerprint: "failed-request".into(),
            })
            .await?
        {
            TaskRegistrationOutcome::Created(record) => record,
            TaskRegistrationOutcome::Existing(_) => anyhow::bail!("unexpected existing task"),
        };
        assert_eq!(
            runtime
                .fail_task_if_unchanged(&failed, "failed".into())
                .await?,
            RuntimeMutationOutcome::Applied
        );

        let completed = match runtime
            .register_task_if_absent(TaskRegistration {
                task_id: "completed-root".into(),
                pipeline_key: pipeline,
                route,
                task_kind: "aggregate".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: vec!["completed-input".into()],
                metadata: serde_json::json!({}),
                request_fingerprint: "completed-request".into(),
            })
            .await?
        {
            TaskRegistrationOutcome::Created(record) => record,
            TaskRegistrationOutcome::Existing(_) => anyhow::bail!("unexpected existing task"),
        };
        assert!(
            runtime
                .complete_nonterminal_task(
                    &completed.task_id,
                    completed.incarnation_id,
                    "proof://completed",
                    &[],
                )
                .await?
        );

        for (proof_ref, owner) in [
            ("failed-input", failed.incarnation_id),
            ("completed-input", completed.incarnation_id),
        ] {
            let error = runtime
                .publish_active_proof_artifact_bytes(
                    "l1-l2", pipeline, route, proof_ref, owner, b"proof",
                )
                .await
                .expect_err("terminal task must not publish an active input artifact");
            assert!(error.to_string().contains("durable task owner"));
            assert!(
                runtime
                    .read_proof_artifact_bytes("l1-l2", pipeline, route, proof_ref)
                    .await?
                    .is_none()
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn activation_rechecks_pending_owner_liveness() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "activation-owner-fence".into())?;
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let proof_ref = "proposal-1";
        let root = runtime
            .register_task(TaskRegistration {
                task_id: "root".into(),
                pipeline_key: pipeline,
                route,
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: vec![proof_ref.into()],
                metadata: serde_json::json!({}),
                request_fingerprint: "request".into(),
            })
            .await?;
        assert!(
            runtime
                .checkpoint_pending_proof_publication(
                    "l1-l2",
                    pipeline,
                    route,
                    proof_ref,
                    &[root.incarnation_id],
                    b"proof",
                )
                .await?
        );
        let object = runtime
            .publish_proof_artifact_bytes("l1-l2", pipeline, route, proof_ref, b"proof")
            .await?
            .try_object()
            .context("published proof")?
            .clone();
        let registration = ProofArtifactRegistration {
            network_pair: "l1-l2".into(),
            proof_ref: proof_ref.into(),
            pipeline_key: pipeline,
            route,
            proof_uri: object.proof_uri,
            content_hash: object.content_hash,
            generation: object.generation,
        };
        assert_eq!(
            runtime
                .register_pending_proof_artifact(registration.clone())
                .await?,
            ProofArtifactLifecycle::Pending
        );
        assert_eq!(
            runtime
                .cancel_task_if_current(&root.lifetime(), None)
                .await?,
            RuntimeMutationOutcome::Applied
        );

        assert!(
            runtime
                .activate_proof_artifact_with_tasks(
                    proof_ref,
                    registration,
                    &[root.incarnation_id],
                    |_| Ok(Some(())),
                )
                .await?
                .is_none()
        );
        assert!(
            runtime
                .get_proof_artifact("l1-l2", pipeline, route, proof_ref)
                .await?
                .is_none()
        );
        assert!(
            runtime
                .invalidate_pending_proof_publication("l1-l2", pipeline, route, proof_ref)
                .await?,
            "ownerless activation must remain eligible for exact saga cleanup"
        );
        assert!(
            runtime
                .read_proof_artifact_bytes("l1-l2", pipeline, route, proof_ref)
                .await?
                .is_none(),
            "cancelled active publication must not leave a manifest that poisons re-publication"
        );
        Ok(())
    }

    #[tokio::test]
    async fn activation_accepts_a_distinct_owner_added_after_checkpoint() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "activation-owner-refresh".into())?;
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let proof_ref = "proposal-1";
        let first = runtime
            .register_task(TaskRegistration {
                task_id: "root-a".into(),
                pipeline_key: pipeline,
                route,
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: vec![proof_ref.into()],
                metadata: serde_json::json!({}),
                request_fingerprint: "request-a".into(),
            })
            .await?;
        assert!(
            runtime
                .checkpoint_pending_proof_publication(
                    "l1-l2",
                    pipeline,
                    route,
                    proof_ref,
                    &[first.incarnation_id],
                    b"proof",
                )
                .await?
        );
        let second = runtime
            .register_task(TaskRegistration {
                task_id: "root-b".into(),
                pipeline_key: pipeline,
                route,
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: vec![proof_ref.into()],
                metadata: serde_json::json!({}),
                request_fingerprint: "request-b".into(),
            })
            .await?;
        let object = runtime
            .publish_proof_artifact_bytes("l1-l2", pipeline, route, proof_ref, b"proof")
            .await?
            .try_object()
            .context("published proof")?
            .clone();
        let registration = ProofArtifactRegistration {
            network_pair: "l1-l2".into(),
            proof_ref: proof_ref.into(),
            pipeline_key: pipeline,
            route,
            proof_uri: object.proof_uri,
            content_hash: object.content_hash,
            generation: object.generation,
        };
        assert_eq!(
            runtime
                .register_pending_proof_artifact(registration.clone())
                .await?,
            ProofArtifactLifecycle::Pending
        );

        assert!(
            runtime
                .activate_proof_artifact_with_tasks(
                    proof_ref,
                    registration,
                    &[first.incarnation_id, second.incarnation_id],
                    |_| Ok(Some(())),
                )
                .await?
                .is_some()
        );
        assert!(
            runtime
                .get_proof_artifact("l1-l2", pipeline, route, proof_ref)
                .await?
                .is_some()
        );
        Ok(())
    }

    #[tokio::test]
    async fn cancelled_owner_cannot_activate_an_inflight_artifact() -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new(
            "cancel-during-active-publication",
        )?);
        let runtime = Arc::new(RuntimeManager::with_store(store.clone()));
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let proof_ref = "external-input-0";
        let owner = runtime
            .register_task(TaskRegistration {
                task_id: "root".into(),
                pipeline_key: pipeline,
                route,
                task_kind: "aggregate".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: vec![proof_ref.into()],
                metadata: serde_json::json!({}),
                request_fingerprint: "request".into(),
            })
            .await?
            .incarnation_id;

        store.block_next_artifact_put.store(true, Ordering::SeqCst);
        let publication = tokio::spawn({
            let runtime = Arc::clone(&runtime);
            async move {
                runtime
                    .publish_active_proof_artifact_bytes(
                        "l1-l2",
                        pipeline,
                        route,
                        proof_ref,
                        owner,
                        br#"{"proof":"0xproof"}"#,
                    )
                    .await
            }
        });

        store.artifact_put_entered.notified().await;
        store.block_next_artifact_put.store(true, Ordering::SeqCst);
        store.allow_artifact_put.notify_one();
        store.artifact_put_entered.notified().await;
        assert_eq!(
            cancel_test_task(runtime.as_ref(), "root").await?,
            RuntimeMutationOutcome::Applied
        );
        store.allow_artifact_put.notify_one();

        let error = publication
            .await?
            .expect_err("cancelled owner must not activate an inflight artifact");
        assert!(
            error.to_string().contains("invalidated")
                || error
                    .to_string()
                    .contains("owner changed before activation"),
            "unexpected publication error: {error:#}"
        );
        assert!(
            runtime
                .get_proof_artifact("l1-l2", pipeline, route, proof_ref)
                .await?
                .is_none()
        );
        assert!(
            runtime
                .read_proof_artifact_bytes("l1-l2", pipeline, route, proof_ref)
                .await?
                .is_none(),
            "cancelled active publication must not leave a manifest that poisons re-publication"
        );
        Ok(())
    }

    #[tokio::test]
    async fn canonical_publication_serializes_unowned_invalidation() -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new(
            "canonical-publication-invalidation",
        )?);
        let runtime = Arc::new(RuntimeManager::with_store(store.clone()));
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let proof_ref = "proposal-publication";
        let proof = br#"{"proof":"0x01"}"#;
        let owner = runtime
            .register_task(TaskRegistration {
                task_id: "root".into(),
                pipeline_key: pipeline,
                route,
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: vec![proof_ref.into()],
                metadata: serde_json::json!({}),
                request_fingerprint: "request".into(),
            })
            .await?
            .incarnation_id;
        assert!(
            runtime
                .checkpoint_pending_proof_publication(
                    "l1-l2",
                    pipeline,
                    route,
                    proof_ref,
                    &[owner],
                    proof,
                )
                .await?
        );

        store.block_next_artifact_put.store(true, Ordering::SeqCst);
        let publication = tokio::spawn({
            let runtime = Arc::clone(&runtime);
            async move {
                runtime
                    .commit_proof_artifact_publication("l1-l2", pipeline, route, proof_ref, proof)
                    .await
            }
        });
        store.artifact_put_entered.notified().await;
        assert_eq!(
            cancel_test_task(runtime.as_ref(), "root").await?,
            RuntimeMutationOutcome::Applied
        );

        let mut invalidation = tokio::spawn({
            let runtime = Arc::clone(&runtime);
            async move {
                runtime
                    .invalidate_pending_proof_publication("l1-l2", pipeline, route, proof_ref)
                    .await
            }
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut invalidation)
                .await
                .is_err(),
            "unowned invalidation bypassed the canonical publication transaction"
        );

        store.allow_artifact_put.notify_one();
        publication
            .await?
            .expect_err("cancelled publication must be invalidated");
        assert!(invalidation.await??);
        assert!(
            runtime
                .read_proof_artifact_bytes("l1-l2", pipeline, route, proof_ref)
                .await?
                .is_none()
        );
        Ok(())
    }

    #[tokio::test]
    async fn active_artifact_publication_rejects_different_bytes() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "active-conflict".into())?;
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let proof_ref = "external-input-0";
        let owner = match runtime
            .register_task_if_absent(TaskRegistration {
                task_id: "root".into(),
                pipeline_key: pipeline,
                route,
                task_kind: "aggregate".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: vec![proof_ref.into()],
                metadata: serde_json::json!({
                    "network_pair": "l1-l2",
                    "aggregate_input_artifacts": [{ "proof_ref": proof_ref }],
                }),
                request_fingerprint: "request".into(),
            })
            .await?
        {
            TaskRegistrationOutcome::Created(record) => record.incarnation_id,
            TaskRegistrationOutcome::Existing(_) => anyhow::bail!("unexpected existing task"),
        };

        let first = br#"{"proof":"0xfirst"}"#;
        runtime
            .publish_active_proof_artifact_bytes("l1-l2", pipeline, route, proof_ref, owner, first)
            .await?;
        runtime
            .publish_active_proof_artifact_bytes("l1-l2", pipeline, route, proof_ref, owner, first)
            .await?;
        assert_eq!(
            runtime
                .get_proof_artifact("l1-l2", pipeline, route, proof_ref)
                .await?
                .expect("idempotent active artifact")
                .lifecycle,
            ProofArtifactLifecycle::Active
        );
        let error = runtime
            .publish_active_proof_artifact_bytes(
                "l1-l2",
                pipeline,
                route,
                proof_ref,
                owner,
                br#"{"proof":"0xdifferent"}"#,
            )
            .await
            .expect_err("different bytes must remain a conflict");

        assert!(
            error
                .to_string()
                .contains("different active proof artifact")
        );
        assert!(
            runtime
                .get_pending_proof_publication("l1-l2", pipeline, route, proof_ref)
                .await?
                .is_none()
        );
        assert_eq!(
            runtime
                .read_proof_artifact_bytes("l1-l2", pipeline, route, proof_ref)
                .await?
                .expect("original artifact")
                .bytes,
            first
        );
        Ok(())
    }

    #[tokio::test]
    async fn active_canonical_artifact_satisfies_a_conflicting_checkpoint() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "active-conflict-reuse".into())?;
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let proof_ref = "proposal-conflict";
        let root = runtime
            .register_task(TaskRegistration {
                task_id: "root".into(),
                pipeline_key: pipeline,
                route,
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: vec![proof_ref.into()],
                metadata: serde_json::json!({}),
                request_fingerprint: "request".into(),
            })
            .await?;
        let canonical_bytes = br#"{"proof":"0xcanonical"}"#;
        let canonical = runtime
            .publish_proof_artifact_bytes("l1-l2", pipeline, route, proof_ref, canonical_bytes)
            .await?
            .try_object()
            .context("canonical proof object")?
            .clone();
        let registration = ProofArtifactRegistration {
            network_pair: "l1-l2".into(),
            proof_ref: proof_ref.into(),
            pipeline_key: pipeline,
            route,
            proof_uri: canonical.proof_uri.clone(),
            content_hash: canonical.content_hash.clone(),
            generation: canonical.generation,
        };
        runtime.upsert_proof_artifact(registration.clone()).await?;
        let conflicting_bytes = br#"{"proof":"0xlate"}"#;
        assert!(
            runtime
                .checkpoint_pending_proof_publication(
                    "l1-l2",
                    pipeline,
                    route,
                    proof_ref,
                    &[root.incarnation_id],
                    conflicting_bytes,
                )
                .await?
        );

        let publication = runtime
            .commit_proof_artifact_publication(
                "l1-l2",
                pipeline,
                route,
                proof_ref,
                conflicting_bytes,
            )
            .await?;
        assert!(matches!(publication, ProofArtifactPutResult::Conflict(_)));
        assert!(
            runtime
                .activate_proof_artifact_with_tasks(
                    proof_ref,
                    registration,
                    &[root.incarnation_id],
                    |records| {
                        let record = records
                            .iter_mut()
                            .find(|record| record.incarnation_id == root.incarnation_id)
                            .context("active publication owner")?;
                        record.runner_status = RunnerStatus::Completed;
                        record.proof_uri = Some(canonical.proof_uri.clone());
                        Ok(Some(()))
                    },
                )
                .await?
                .is_some()
        );
        assert!(
            runtime
                .remove_pending_proof_publication_if_unowned("l1-l2", pipeline, route, proof_ref,)
                .await?
        );
        assert_eq!(
            runtime
                .read_proof_artifact_bytes("l1-l2", pipeline, route, proof_ref)
                .await?
                .context("active canonical proof")?
                .bytes,
            canonical_bytes
        );
        assert_eq!(
            runtime
                .get_task("root")
                .await?
                .context("completed root")?
                .runner_status,
            RunnerStatus::Completed
        );
        Ok(())
    }

    #[tokio::test]
    async fn task_ref_lookup_includes_external_aggregate_inputs() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "aggregate-input-lookup".into())?;
        let pipeline = PipelineKey::ShastaSp1;
        let proof_ref = "external-input-0";
        runtime
            .register_task(TaskRegistration {
                task_id: "external-aggregate-root".into(),
                pipeline_key: pipeline,
                route: pipeline.route(),
                task_kind: "aggregate".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: vec!["aggregate-output".into(), proof_ref.into()],
                metadata: serde_json::json!({
                    "network_pair": "l1-l2",
                    "aggregate_input_artifacts": [{ "proof_ref": proof_ref }],
                }),
                request_fingerprint: "external-aggregate-request".into(),
            })
            .await?;

        let tasks = runtime.tasks_referencing(proof_ref).await;
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].task_id, "external-aggregate-root");
        Ok(())
    }

    #[tokio::test]
    async fn recoverable_pending_publication_rejects_hash_mismatch() -> Result<()> {
        let store = Arc::new(MemoryProofArtifactStore::new(
            "test".into(),
            "pending-hash-mismatch".into(),
        )?);
        let runtime = RuntimeManager::with_store(store.clone());
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let proof_ref = "external-input-0";
        let owner = runtime
            .register_task(TaskRegistration {
                task_id: "root".into(),
                pipeline_key: pipeline,
                route,
                task_kind: "aggregate".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: vec![proof_ref.into()],
                metadata: serde_json::json!({
                    "network_pair": "l1-l2",
                    "aggregate_input_artifacts": [{ "proof_ref": proof_ref }],
                }),
                request_fingerprint: "request".into(),
            })
            .await?
            .incarnation_id;
        assert!(
            runtime
                .checkpoint_pending_proof_publication(
                    "l1-l2",
                    pipeline,
                    route,
                    proof_ref,
                    &[owner],
                    b"expected",
                )
                .await?
        );

        let key = RuntimeManager::pending_artifact_key("l1-l2", pipeline, route, proof_ref);
        let original = store.get(&key).await?.context("pending object")?;
        store.delete_exact(&key, &original.descriptor()).await?;
        store.put_if_absent(&key, b"corrupted").await?;

        let error = runtime
            .get_recoverable_pending_proof_publication("l1-l2", pipeline, route, proof_ref)
            .await
            .expect_err("pending bytes must match the authoritative hash");
        assert!(error.to_string().contains("content hash mismatch"));
        assert!(runtime.list_proof_artifacts().await?.is_empty());
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
                pipeline_key: pipeline,
                route,
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: vec!["aggregate-task".into(), proof_ref.into()],
                metadata: serde_json::json!({
                    "network_pair": "l1-l2",
                    "aggregate_input_artifacts": [{ "proof_ref": proof_ref }],
                }),
                request_fingerprint: "root-request".into(),
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
    #[expect(
        clippy::too_many_lines,
        reason = "the test keeps the invalidation and admission race in one deterministic trace"
    )]
    async fn descriptor_invalidation_fences_task_admission_before_delete_finishes() -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new("artifact-admission-fence")?);
        let runtime = Arc::new(RuntimeManager::with_store(store.clone()));
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
                pipeline_key: pipeline,
                route,
                task_kind: "proposal".into(),
                network_pair: network_pair.into(),
                artifact_refs: vec![proof_ref.into()],
                metadata: serde_json::json!({ "network_pair": network_pair }),
                request_fingerprint: "recoverable-root".into(),
            })
            .await?;
        cancel_test_task(runtime.as_ref(), "recoverable-root").await?;

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

        let precondition = ProofArtifactPrecondition {
            network_pair: network_pair.into(),
            proof_ref: proof_ref.into(),
            pipeline_key: pipeline,
            route,
            descriptor: first.descriptor(),
        };
        let admission = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            runtime.register_task_if_absent_with_artifact_preconditions(
                TaskRegistration {
                    task_id: "replacement-root".into(),
                    pipeline_key: pipeline,
                    route,
                    task_kind: "proposal".into(),
                    network_pair: network_pair.into(),
                    artifact_refs: vec![proof_ref.into()],
                    metadata: serde_json::json!({ "network_pair": network_pair }),
                    request_fingerprint: "replacement-root".into(),
                },
                &[precondition],
            ),
        )
        .await
        .context("task admission waited for external artifact deletion")?;
        assert!(
            admission.is_err(),
            "invalidated artifact precondition admitted a replacement task"
        );
        let recovery_root = current_test_task(runtime.as_ref(), "recoverable-root").await?;
        let recovery = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            runtime.prepare_task_for_recovery_if_unchanged(&recovery_root),
        )
        .await
        .context("task recovery waited for external artifact deletion")??;
        assert!(recovery.is_none());

        store.allow_artifact_delete_return.notify_one();
        assert_eq!(
            invalidation.await??,
            ProofArtifactInvalidationResult::Invalidated(ProofArtifactDeleteResult::Removed)
        );
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
                    pipeline_key: pipeline,
                    route,
                    task_kind: "proposal".into(),
                    network_pair: network_pair.into(),
                    artifact_refs: vec![proof_ref.into()],
                    metadata: serde_json::json!({ "network_pair": network_pair }),
                    request_fingerprint: format!("request-{task_id}"),
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
            pipeline_key: PipelineKey::ShastaSp1,
            route: PipelineKey::ShastaSp1.route(),
            task_kind: "proposal".into(),
            network_pair: "l1-l2".into(),
            artifact_refs: vec!["proposal-1".into()],
            metadata: serde_json::json!({}),
            request_fingerprint: "same-request".into(),
        };
        let first = RuntimeManager::with_store(store.clone());
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
        cancel_test_task(&first, "root").await?;
        assert!(matches!(
            remove_test_task(&first, "root").await?,
            RuntimeMutationOutcome::Applied
        ));
        drop(first);

        let replacement = RuntimeManager::with_store(store);
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
            pipeline_key: PipelineKey::ShastaSp1,
            route: PipelineKey::ShastaSp1.route(),
            task_kind: "proposal".into(),
            network_pair: "l1-l2".into(),
            artifact_refs: Vec::new(),
            metadata: serde_json::json!({}),
            request_fingerprint: "root-request".into(),
        };
        let first = runtime.register_task(registration.clone()).await?;
        assert!(matches!(
            remove_test_task(&runtime, "root").await?,
            RuntimeMutationOutcome::Applied
        ));
        let replacement = runtime.register_task(registration).await?;

        assert!(matches!(
            runtime.remove_task_if_current(&first.lifetime()).await?,
            RuntimeMutationOutcome::Stale
        ));
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
    async fn pending_outbox_state_failure_does_not_create_an_untracked_object() -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new("pending-state-failure")?);
        let runtime = RuntimeManager::with_store(store.clone());
        let pipeline = PipelineKey::ShastaSp1;
        let route = pipeline.route();
        let proof_ref = "proposal-1";
        let owner = runtime
            .register_task(TaskRegistration {
                task_id: "root".into(),
                pipeline_key: pipeline,
                route,
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: vec![proof_ref.into()],
                metadata: serde_json::json!({}),
                request_fingerprint: "root-request".into(),
            })
            .await?
            .incarnation_id;
        store.fail_before_commit.store(3, Ordering::SeqCst);

        runtime
            .checkpoint_pending_proof_publication(
                "l1-l2",
                pipeline,
                route,
                proof_ref,
                &[owner],
                b"proof",
            )
            .await
            .expect_err("publication intent persistence must precede the pending object write");

        assert!(
            runtime
                .get_pending_proof_publication("l1-l2", pipeline, route, proof_ref)
                .await?
                .is_none()
        );
        assert!(runtime.state.read().await.pending_publications.is_empty());
        drop(runtime);

        let recovered = RuntimeManager::with_store(store);
        recovered.initialize().await?;
        assert_eq!(
            recovered
                .reconcile_unowned_pending_proof_publications()
                .await?,
            0
        );
        Ok(())
    }

    #[tokio::test]
    async fn pending_outbox_object_failure_keeps_its_durable_intent() -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new("pending-object-failure")?);
        let runtime = RuntimeManager::with_store(store.clone());
        let pipeline = PipelineKey::ShastaSp1;
        let route = pipeline.route();
        let proof_ref = "proposal-1";
        let owner = runtime
            .register_task(TaskRegistration {
                task_id: "root".into(),
                pipeline_key: pipeline,
                route,
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: vec![proof_ref.into()],
                metadata: serde_json::json!({}),
                request_fingerprint: "root-request".into(),
            })
            .await?
            .incarnation_id;
        store.fail_next_artifact_put.store(true, Ordering::SeqCst);

        runtime
            .checkpoint_pending_proof_publication(
                "l1-l2",
                pipeline,
                route,
                proof_ref,
                &[owner],
                b"proof",
            )
            .await
            .expect_err("pending object materialization failure must retain the intent");
        assert_eq!(runtime.state.read().await.pending_publications.len(), 1);
        assert!(
            runtime
                .get_pending_proof_publication("l1-l2", pipeline, route, proof_ref)
                .await?
                .is_none()
        );
        drop(runtime);

        let recovered = RuntimeManager::with_store(store);
        recovered.initialize().await?;
        assert!(
            recovered
                .checkpoint_pending_proof_publication(
                    "l1-l2",
                    pipeline,
                    route,
                    proof_ref,
                    &[owner],
                    b"proof",
                )
                .await?
        );
        let pending = recovered
            .get_recoverable_pending_proof_publication("l1-l2", pipeline, route, proof_ref)
            .await?
            .context("recovered publication")?;
        assert_eq!(pending.bytes, b"proof");
        assert_eq!(pending.owner_incarnations, vec![owner]);
        Ok(())
    }

    #[tokio::test]
    async fn materialized_live_outbox_is_first_write_wins() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "pending-first-write".into())?;
        let pipeline = PipelineKey::ShastaSp1;
        let route = pipeline.route();
        let proof_ref = "proposal-1";
        let owner = runtime
            .register_task(TaskRegistration {
                task_id: "root".into(),
                pipeline_key: pipeline,
                route,
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: vec![proof_ref.into()],
                metadata: serde_json::json!({}),
                request_fingerprint: "root-request".into(),
            })
            .await?
            .incarnation_id;
        assert!(
            runtime
                .checkpoint_pending_proof_publication(
                    "l1-l2",
                    pipeline,
                    route,
                    proof_ref,
                    &[owner],
                    b"first-proof",
                )
                .await?
        );

        runtime
            .checkpoint_pending_proof_publication(
                "l1-l2",
                pipeline,
                route,
                proof_ref,
                &[owner],
                b"different-proof",
            )
            .await
            .expect_err("a live materialized intent must not be replaced");

        let pending = runtime
            .get_recoverable_pending_proof_publication("l1-l2", pipeline, route, proof_ref)
            .await?
            .context("first publication")?;
        assert_eq!(pending.bytes, b"first-proof");
        assert_eq!(pending.owner_incarnations, vec![owner]);
        Ok(())
    }

    #[tokio::test]
    async fn same_content_checkpoints_merge_live_owners() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "pending-owner-merge".into())?;
        let pipeline = PipelineKey::ShastaSp1;
        let route = pipeline.route();
        let proof_ref = "proposal-1";
        let mut owners = Vec::new();
        for task_id in ["root-a", "root-b"] {
            owners.push(
                runtime
                    .register_task(TaskRegistration {
                        task_id: task_id.into(),
                        pipeline_key: pipeline,
                        route,
                        task_kind: "proposal".into(),
                        network_pair: "l1-l2".into(),
                        artifact_refs: vec![proof_ref.into()],
                        metadata: serde_json::json!({}),
                        request_fingerprint: format!("request-{task_id}"),
                    })
                    .await?
                    .incarnation_id,
            );
        }

        for owner in &owners {
            assert!(
                runtime
                    .checkpoint_pending_proof_publication(
                        "l1-l2",
                        pipeline,
                        route,
                        proof_ref,
                        &[*owner],
                        b"shared-proof",
                    )
                    .await?
            );
        }

        owners.sort_unstable();
        let mut checkpointed = runtime
            .get_recoverable_pending_proof_publication("l1-l2", pipeline, route, proof_ref)
            .await?
            .context("shared pending publication")?
            .owner_incarnations;
        checkpointed.sort_unstable();
        assert_eq!(checkpointed, owners);
        Ok(())
    }

    #[tokio::test]
    async fn unmaterialized_live_intent_is_first_write_wins() -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new("pending-intent-first-write")?);
        let runtime = RuntimeManager::with_store(store.clone());
        let pipeline = PipelineKey::ShastaSp1;
        let route = pipeline.route();
        let proof_ref = "proposal-1";
        let owner = runtime
            .register_task(TaskRegistration {
                task_id: "root".into(),
                pipeline_key: pipeline,
                route,
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: vec![proof_ref.into()],
                metadata: serde_json::json!({}),
                request_fingerprint: "root-request".into(),
            })
            .await?
            .incarnation_id;
        store.fail_next_artifact_put.store(true, Ordering::SeqCst);
        runtime
            .checkpoint_pending_proof_publication(
                "l1-l2",
                pipeline,
                route,
                proof_ref,
                &[owner],
                b"first-proof",
            )
            .await
            .expect_err("injected object write failure must leave the first intent durable");

        runtime
            .checkpoint_pending_proof_publication(
                "l1-l2",
                pipeline,
                route,
                proof_ref,
                &[owner],
                b"different-proof",
            )
            .await
            .expect_err("a live durable intent must not be replaced before materialization");
        assert!(
            runtime
                .get_pending_proof_publication("l1-l2", pipeline, route, proof_ref)
                .await?
                .is_none()
        );
        assert_eq!(
            runtime
                .state
                .read()
                .await
                .pending_publications
                .values()
                .next()
                .context("durable pending intent")?
                .content_hash,
            artifact_store::content_hash(b"first-proof")
        );
        Ok(())
    }

    #[tokio::test]
    async fn pending_outbox_delete_failure_remains_recoverable_after_restart() -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new("pending-delete-recovery")?);
        let runtime = RuntimeManager::with_store(store.clone());
        let pipeline = PipelineKey::ShastaSp1;
        let route = pipeline.route();
        let proof_ref = "proposal-1";
        let owner = runtime
            .register_task(TaskRegistration {
                task_id: "root".into(),
                pipeline_key: pipeline,
                route,
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: vec![proof_ref.into()],
                metadata: serde_json::json!({}),
                request_fingerprint: "root-request".into(),
            })
            .await?
            .incarnation_id;
        assert!(
            runtime
                .checkpoint_pending_proof_publication(
                    "l1-l2",
                    pipeline,
                    route,
                    proof_ref,
                    &[owner],
                    b"proof",
                )
                .await?
        );
        cancel_test_task(&runtime, "root").await?;

        store
            .fail_next_artifact_delete
            .store(true, Ordering::SeqCst);
        runtime
            .remove_pending_proof_publication_if_unowned("l1-l2", pipeline, route, proof_ref)
            .await
            .expect_err("failed object deletion must keep its durable cleanup intent");
        drop(runtime);

        let recovered = RuntimeManager::with_store(store);
        recovered.initialize().await?;
        assert!(
            recovered
                .get_pending_proof_publication("l1-l2", pipeline, route, proof_ref)
                .await?
                .is_some()
        );
        assert_eq!(
            recovered
                .reconcile_unowned_pending_proof_publications()
                .await?,
            1
        );
        assert!(
            recovered
                .get_pending_proof_publication("l1-l2", pipeline, route, proof_ref)
                .await?
                .is_none()
        );
        assert_eq!(
            recovered
                .reconcile_unowned_pending_proof_publications()
                .await?,
            0
        );
        Ok(())
    }

    #[tokio::test]
    async fn restart_invalidates_a_cancelled_unregistered_canonical_publication() -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new("cancelled-canonical-recovery")?);
        let runtime = RuntimeManager::with_store(store.clone());
        let pipeline = PipelineKey::ShastaSp1;
        let route = pipeline.route();
        let network_pair = "l1-l2";
        let proof_ref = "proposal-1";
        let owner = runtime
            .register_task(TaskRegistration {
                task_id: "root".into(),
                pipeline_key: pipeline,
                route,
                task_kind: "proposal".into(),
                network_pair: network_pair.into(),
                artifact_refs: vec![proof_ref.into()],
                metadata: serde_json::json!({}),
                request_fingerprint: "root-request".into(),
            })
            .await?
            .incarnation_id;
        assert!(
            runtime
                .checkpoint_pending_proof_publication(
                    network_pair,
                    pipeline,
                    route,
                    proof_ref,
                    &[owner],
                    b"proof",
                )
                .await?
        );
        let descriptor = runtime
            .publish_proof_artifact_bytes(network_pair, pipeline, route, proof_ref, b"proof")
            .await?
            .try_object()
            .context("canonical proof object")?
            .descriptor();
        cancel_test_task(&runtime, "root").await?;
        drop(runtime);

        let recovered = RuntimeManager::with_store(store);
        recovered.initialize().await?;
        assert_eq!(
            recovered
                .reconcile_unowned_pending_proof_publications()
                .await?,
            1
        );
        assert!(
            recovered
                .get_pending_proof_publication(network_pair, pipeline, route, proof_ref)
                .await?
                .is_none()
        );
        let artifact = recovered
            .get_proof_artifact_including_invalidated(network_pair, pipeline, route, proof_ref)
            .await?
            .context("invalidated lifecycle record")?;
        assert_eq!(artifact.lifecycle, ProofArtifactLifecycle::Invalidated);
        assert_eq!(artifact.descriptor(), descriptor);
        assert!(
            recovered
                .proof_artifact_descriptor_is_invalidated(
                    network_pair,
                    pipeline,
                    route,
                    proof_ref,
                    &descriptor,
                )
                .await?
        );
        Ok(())
    }

    #[tokio::test]
    async fn restart_reconciles_dangling_canonical_and_pending_manifests() -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new(
            "dangling-publication-recovery",
        )?);
        let runtime = RuntimeManager::with_store(store.clone());
        let pipeline = PipelineKey::ShastaSp1;
        let route = pipeline.route();
        let network_pair = "l1-l2";
        let proof_ref = "proposal-dangling";
        let owner = runtime
            .register_task(TaskRegistration {
                task_id: "root-dangling".into(),
                pipeline_key: pipeline,
                route,
                task_kind: "proposal".into(),
                network_pair: network_pair.into(),
                artifact_refs: vec![proof_ref.into()],
                metadata: serde_json::json!({}),
                request_fingerprint: "root-dangling-request".into(),
            })
            .await?
            .incarnation_id;
        assert!(
            runtime
                .checkpoint_pending_proof_publication(
                    network_pair,
                    pipeline,
                    route,
                    proof_ref,
                    &[owner],
                    b"proof",
                )
                .await?
        );
        let descriptor = runtime
            .publish_proof_artifact_bytes(network_pair, pipeline, route, proof_ref, b"proof")
            .await?
            .try_object()
            .context("canonical proof object")?
            .descriptor();
        cancel_test_task(&runtime, "root-dangling").await?;

        let canonical_key = RuntimeManager::artifact_key(network_pair, pipeline, route, proof_ref);
        let pending_key =
            RuntimeManager::pending_artifact_key(network_pair, pipeline, route, proof_ref);
        store.mark_artifact_content_missing(canonical_key.clone())?;
        store.mark_artifact_content_missing(pending_key.clone())?;
        assert!(store.get(&canonical_key).await.is_err());
        assert!(store.get(&pending_key).await.is_err());
        drop(runtime);

        let recovered = RuntimeManager::with_store(store.clone());
        recovered.initialize().await?;
        assert_eq!(
            recovered
                .reconcile_unowned_pending_proof_publications()
                .await?,
            1
        );
        let artifact = recovered
            .get_proof_artifact_including_invalidated(network_pair, pipeline, route, proof_ref)
            .await?
            .context("invalidated lifecycle record")?;
        assert_eq!(artifact.lifecycle, ProofArtifactLifecycle::Invalidated);
        assert_eq!(artifact.descriptor(), descriptor);
        assert!(
            recovered
                .proof_artifact_descriptor_is_invalidated(
                    network_pair,
                    pipeline,
                    route,
                    proof_ref,
                    &descriptor,
                )
                .await?
        );
        assert_eq!(store.get_descriptor(&pending_key).await?, None);
        Ok(())
    }

    #[tokio::test]
    async fn activated_outbox_is_retained_until_object_cleanup_succeeds() -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new("activated-pending-delete")?);
        let runtime = RuntimeManager::with_store(store.clone());
        let pipeline = PipelineKey::ShastaSp1;
        let route = pipeline.route();
        let network_pair = "l1-l2";
        let proof_ref = "proposal-1";
        let owner = runtime
            .register_task(TaskRegistration {
                task_id: "root".into(),
                pipeline_key: pipeline,
                route,
                task_kind: "proposal".into(),
                network_pair: network_pair.into(),
                artifact_refs: vec![proof_ref.into()],
                metadata: serde_json::json!({}),
                request_fingerprint: "root-request".into(),
            })
            .await?
            .incarnation_id;
        assert!(
            runtime
                .checkpoint_pending_proof_publication(
                    network_pair,
                    pipeline,
                    route,
                    proof_ref,
                    &[owner],
                    b"proof",
                )
                .await?
        );
        let object = runtime
            .publish_proof_artifact_bytes(network_pair, pipeline, route, proof_ref, b"proof")
            .await?
            .try_object()
            .context("canonical proof object")?
            .clone();
        let registration = ProofArtifactRegistration {
            network_pair: network_pair.into(),
            proof_ref: proof_ref.into(),
            pipeline_key: pipeline,
            route,
            proof_uri: object.proof_uri.clone(),
            content_hash: object.content_hash.clone(),
            generation: object.generation,
        };
        assert_eq!(
            runtime
                .register_pending_proof_artifact(registration.clone())
                .await?,
            ProofArtifactLifecycle::Pending
        );
        assert!(
            runtime
                .activate_proof_artifact_with_tasks(proof_ref, registration, &[owner], |records| {
                    let record = records
                        .iter_mut()
                        .find(|record| record.incarnation_id == owner)
                        .context("runtime owner")?;
                    record.runner_status = RunnerStatus::Completed;
                    record.proof_uri = Some(object.proof_uri.clone());
                    Ok(Some(()))
                },)
                .await?
                .is_some()
        );

        store
            .fail_next_artifact_delete
            .store(true, Ordering::SeqCst);
        runtime
            .remove_pending_proof_publication_if_unowned(network_pair, pipeline, route, proof_ref)
            .await
            .expect_err("activated outbox cleanup failure must remain durable");
        drop(runtime);

        let recovered = RuntimeManager::with_store(store);
        recovered.initialize().await?;
        assert_eq!(
            recovered
                .reconcile_unowned_pending_proof_publications()
                .await?,
            1
        );
        assert!(
            recovered
                .get_pending_proof_publication(network_pair, pipeline, route, proof_ref)
                .await?
                .is_none()
        );
        Ok(())
    }

    #[tokio::test]
    async fn cancelled_owner_cleans_outbox_that_finishes_put_late() -> Result<()> {
        let store = Arc::new(RuntimeStateProbeStore::new("cancel-during-outbox-put")?);
        let runtime = Arc::new(RuntimeManager::with_store(store.clone()));
        let registration = TaskRegistration {
            task_id: "root".into(),
            pipeline_key: PipelineKey::ShastaSp1,
            route: PipelineKey::ShastaSp1.route(),
            task_kind: "proposal".into(),
            network_pair: "l1-l2".into(),
            artifact_refs: vec!["proposal-1".into()],
            metadata: serde_json::json!({}),
            request_fingerprint: "root-request".into(),
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
        cancel_test_task(runtime.as_ref(), "root").await?;
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

        remove_test_task(runtime.as_ref(), "root").await?;
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
