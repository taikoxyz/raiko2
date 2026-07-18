#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![expect(
    clippy::missing_errors_doc,
    reason = "the runtime crate currently permits undocumented internal-facing public APIs"
)]

mod artifact_store;
mod publication;

pub use artifact_store::{
    GcsProofArtifactStore, MemoryProofArtifactStore, ProofArtifactDescriptor, ProofArtifactKey,
    ProofArtifactObject, ProofArtifactPrefix, ProofArtifactPutResult, ProofArtifactStore,
    RuntimeStateObject, RuntimeStateWriteResult, validate_scope_component,
};
pub use publication::ProofArtifactPublicationInvalidated;

use anyhow::{Context, Result};
use raiko2_pipeline::{PipelineKey, PipelineRoute};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, RwLock};

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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct RuntimeState {
    tasks: HashMap<String, RuntimeTaskRecord>,
    artifacts: HashMap<String, ProofArtifactRecord>,
}

#[derive(Debug)]
pub struct RuntimeManager {
    store: Arc<dyn ProofArtifactStore>,
    state: RwLock<RuntimeState>,
    generation: StdMutex<Option<i64>>,
    mutation: Mutex<()>,
    lifecycle_gate: RwLock<()>,
    active: AtomicBool,
    state_coherent: AtomicBool,
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
            mutation: Mutex::new(()),
            lifecycle_gate: RwLock::new(()),
            active: AtomicBool::new(true),
            state_coherent: AtomicBool::new(true),
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
        let _lifecycle = self.lifecycle_gate.write().await;
        self.active.store(false, Ordering::Release);
    }

    #[must_use]
    pub fn accepts_mutations(&self) -> bool {
        self.active.load(Ordering::Acquire) && self.state_coherent.load(Ordering::Acquire)
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
        if !self.state_coherent.load(Ordering::Acquire) {
            self.install_runtime_state_object(stored).await?;
        }
        self.ensure_active()
    }

    fn ensure_active_lifecycle(&self) -> Result<()> {
        anyhow::ensure!(self.active.load(Ordering::Acquire), "runtime is draining");
        Ok(())
    }

    fn ensure_active(&self) -> Result<()> {
        self.ensure_active_lifecycle()?;
        anyhow::ensure!(
            self.state_coherent.load(Ordering::Acquire),
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
        const MAX_ATTEMPTS: usize = 3;

        let _lifecycle = self.lifecycle_gate.read().await;
        self.ensure_active()?;
        let _mutation = self.mutation.lock().await;
        let mut last_error = None;
        for attempt in 1..=MAX_ATTEMPTS {
            self.ensure_active()?;
            let current = self.state.read().await.clone();
            let current_bytes =
                serde_json::to_vec(&current).context("encode runtime store state")?;
            let expected_generation = self.current_generation()?;
            let mut next = current.clone();
            let output = update(&mut next)?;
            let next_bytes = serde_json::to_vec(&next).context("encode runtime store state")?;

            match self
                .store
                .store_runtime_state(&next_bytes, expected_generation)
                .await
            {
                Ok(RuntimeStateWriteResult::Stored { generation }) => {
                    self.install_runtime_state(next, generation).await?;
                    return Ok(output);
                }
                Ok(RuntimeStateWriteResult::Conflict(observed)) => {
                    if let Some(observed) = observed.as_ref()
                        && observed.bytes == next_bytes
                    {
                        self.install_runtime_state(next, observed.generation)
                            .await?;
                        return Ok(output);
                    }
                    self.install_runtime_state_object(observed).await?;
                    last_error = Some(anyhow::anyhow!(
                        "runtime state generation changed during mutation"
                    ));
                }
                Err(write_error) => {
                    let observed = match self.store.load_runtime_state().await {
                        Ok(observed) => observed,
                        Err(read_error) => {
                            self.state_coherent.store(false, Ordering::Release);
                            return Err(write_error).context(format!(
                                "runtime state write outcome is unknown and read-back failed: {read_error:#}"
                            ));
                        }
                    };
                    if let Some(observed) = observed.as_ref()
                        && observed.bytes == next_bytes
                    {
                        self.install_runtime_state(next, observed.generation)
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
                    self.install_runtime_state_object(observed).await?;
                    last_error = Some(if remote_matches_current {
                        write_error.context("runtime state write failed before commit")
                    } else {
                        write_error.context(
                            "runtime state write outcome conflicted with authoritative read-back",
                        )
                    });
                }
            }

            if attempt < MAX_ATTEMPTS {
                tokio::task::yield_now().await;
            }
        }
        self.state_coherent.store(false, Ordering::Release);
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("runtime state mutation failed")))
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
        *self.state.write().await = state;
        *self
            .generation
            .lock()
            .map_err(|_| anyhow::anyhow!("runtime generation lock poisoned"))? = generation;
        self.state_coherent.store(true, Ordering::Release);
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

    async fn fence_external_mutation(&self) -> Result<tokio::sync::RwLockReadGuard<'_, ()>> {
        self.mutate(|_| Ok(())).await?;
        let lifecycle = self.lifecycle_gate.read().await;
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
        let lifecycle = self.fence_external_mutation().await?;
        let key = Self::artifact_key(network_pair, pipeline_key, route, proof_ref);
        let publication = self.store.put_if_absent(&key, bytes).await?;
        drop(lifecycle);
        let _lifecycle = self
            .fence_external_mutation()
            .await
            .context("global runtime fence changed during artifact publication")?;
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
    ) -> Result<()> {
        let _lifecycle = self.fence_external_mutation().await?;
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

    pub async fn register_task_if_absent(
        &self,
        registration: TaskRegistration,
    ) -> Result<TaskRegistrationOutcome> {
        let fingerprint = registration
            .request_fingerprint
            .as_deref()
            .context("request_fingerprint is required for idempotent registration")?
            .to_string();
        let record = build_task_record(&registration)?;
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
            state.tasks.insert(record.task_id.clone(), record.clone());
            Ok(TaskRegistrationOutcome::Created(record.clone()))
        })
        .await
    }

    pub async fn upsert_task(&self, record: &RuntimeTaskRecord) -> Result<()> {
        let record = record.clone();
        self.mutate(move |state| {
            if let Some(fingerprint) = record.request_fingerprint.as_deref()
                && state.tasks.values().any(|task| {
                    task.task_id != record.task_id
                        && task.request_fingerprint.as_deref() == Some(fingerprint)
                })
            {
                anyhow::bail!("request fingerprint already belongs to another task");
            }
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
        let key = artifact_record_key(
            &registration.network_pair,
            registration.pipeline_key,
            registration.route,
            &registration.proof_ref,
        );
        let record = ProofArtifactRecord {
            environment: self.environment().to_string(),
            network_pair: registration.network_pair,
            proof_ref: registration.proof_ref,
            pipeline_key: registration.pipeline_key,
            route: registration.route,
            proof_uri: registration.proof_uri,
            content_hash: registration.content_hash,
            generation: registration.generation,
            invalidated_at: None,
            updated_at: now_ts(),
        };
        self.mutate(move |state| {
            if let Some(existing) = state.artifacts.get(&key)
                && existing.invalidated_at.is_some()
                && existing.content_hash == record.content_hash
                && existing.generation == record.generation
            {
                return Ok(());
            }
            state.artifacts.insert(key.clone(), record.clone());
            Ok(())
        })
        .await
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
            .filter(|record| record.invalidated_at.is_none()))
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

    pub async fn remove_proof_artifact(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<Option<ProofArtifactRecord>> {
        let key = artifact_record_key(network_pair, pipeline_key, route, proof_ref);
        self.mutate(move |state| Ok(state.artifacts.remove(&key)))
            .await
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
    ) -> Result<bool> {
        let Some(record) = self
            .get_proof_artifact_including_invalidated(network_pair, pipeline_key, route, proof_ref)
            .await?
        else {
            return Ok(false);
        };
        if record.content_hash != content_hash {
            return Ok(false);
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
    ) -> Result<bool> {
        let object_key = Self::artifact_key(network_pair, pipeline_key, route, proof_ref);
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
                record.invalidated_at.get_or_insert_with(now_ts);
                record.updated_at = now_ts();
                Ok(Some(record.generation))
            })
            .await?;
        if let Some(generation) = invalidated_generation {
            let lifecycle = self.fence_external_mutation().await?;
            self.store
                .mark_invalidated(&object_key, generation, &descriptor.content_hash)
                .await?;
            drop(lifecycle);
            let _lifecycle = self.fence_external_mutation().await?;
            self.store
                .delete(&object_key, generation, &descriptor.content_hash)
                .await?;
        }
        Ok(invalidated_generation.is_some())
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
            record.descriptor() == *descriptor && record.invalidated_at.is_some()
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
            .read_proof_artifact_bytes(network_pair, pipeline_key, route, proof_ref)
            .await?
            .is_some_and(|object| object.descriptor() == *descriptor))
    }

    pub async fn upsert_pending_proof_publication(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        proof_bytes: &[u8],
    ) -> Result<()> {
        let _lifecycle = self.fence_external_mutation().await?;
        let key = Self::pending_artifact_key(network_pair, pipeline_key, route, proof_ref);
        match self.store.put_if_absent(&key, proof_bytes).await? {
            ProofArtifactPutResult::Created(_) | ProofArtifactPutResult::AlreadyExists(_) => Ok(()),
            ProofArtifactPutResult::Conflict(_) => {
                anyhow::bail!("different pending proof already exists")
            }
        }
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

    pub async fn remove_pending_proof_publication(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<bool> {
        self.remove_local_pending(network_pair, pipeline_key, route, proof_ref)
            .await
    }

    pub(crate) async fn remove_committed_pending_proof_publication(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<bool> {
        self.remove_local_pending(network_pair, pipeline_key, route, proof_ref)
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
        let _lifecycle = self.fence_external_mutation().await?;
        self.store
            .delete(&key, object.generation, &object.content_hash)
            .await?;
        Ok(true)
    }

    pub async fn invalidate_pending_proof_publication(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<bool> {
        let object_key = Self::artifact_key(network_pair, pipeline_key, route, proof_ref);
        let canonical = self.store.get(&object_key).await?;
        if let Some(object) = canonical.as_ref() {
            self.upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: network_pair.to_string(),
                proof_ref: proof_ref.to_string(),
                pipeline_key,
                route,
                proof_uri: object.proof_uri.clone(),
                content_hash: object.content_hash.clone(),
                generation: object.generation,
            })
            .await?;
            self.mark_proof_artifact_descriptor_invalidated(
                network_pair,
                pipeline_key,
                route,
                proof_ref,
                &object.descriptor(),
            )
            .await?;
        }
        let pending = self
            .get_pending_proof_publication(network_pair, pipeline_key, route, proof_ref)
            .await?;
        let removed = self
            .remove_pending_proof_publication(network_pair, pipeline_key, route, proof_ref)
            .await?;
        Ok(removed || pending.is_some() || canonical.is_some())
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
        let task_id = task_id.to_string();
        self.mutate(move |state| Ok(state.tasks.remove(&task_id).is_some()))
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeTaskRecord {
    pub task_id: String,
    pub pipeline_key: PipelineKey,
    pub route: PipelineRoute,
    pub task_kind: String,
    pub proposal_id: Option<u64>,
    pub proof_ids: Vec<String>,
    pub runner_status: RunnerStatus,
    pub image_ref: Option<String>,
    pub provider_request_id: Option<String>,
    pub remote_tx_hash: Option<String>,
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
        pipeline_key,
        route: registration.route,
        task_kind: registration.task_kind.clone(),
        proposal_id: registration.proposal_id,
        proof_ids: registration.proof_ids.clone(),
        runner_status: RunnerStatus::Allocated,
        image_ref: None,
        provider_request_id: None,
        remote_tx_hash: None,
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
}
