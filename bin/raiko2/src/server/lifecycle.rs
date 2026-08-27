//! Runtime-root lifecycle coordination.

use anyhow::{Result, bail};
use raiko2_engine::EngineExecutionPlan;
use raiko2_queue::{DetachMode, DetachOutcome, RootOwner};
use raiko2_runtime::{
    ArtifactExpectation, ExactDeleteResult, PendingPublicationExpectation, ProofArtifactKey,
    ProofArtifactPrecondition, ProofArtifactRecord, RunnerStatus, RuntimeManager,
    RuntimeMutationOutcome, RuntimeTaskRecord, TaskLifetime, TaskRegistration,
};
use std::sync::Arc;
use tokio::task::JoinSet;
use tracing::warn;

use crate::server::state::{EngineHandle, PipelineFactory};

/// Coordinates durable root transitions with idempotent in-memory queue effects.
#[derive(Clone)]
pub(crate) struct ProofLifecycle {
    runtime: Arc<RuntimeManager>,
    pipelines: Arc<dyn PipelineFactory>,
}

/// Result of atomically preparing and attaching a recovered runtime root.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecoveryOutcome {
    Recovered,
    Active,
    TaskChanged,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TerminalRetentionBatchOutcome {
    pub retired_roots: usize,
    pub skipped_roots: usize,
    pub removed_roots: usize,
    pub retained_root_failures: usize,
    pub skipped_shared_children: usize,
    pub invalidated_artifacts: usize,
    pub removed_artifacts: usize,
    pub retained_artifact_failures: usize,
    pub retry_roots: Vec<TaskLifetime>,
    pub retry_artifacts: Vec<ProofArtifactKey>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ArtifactRetentionBatchOutcome {
    pub invalidated_artifacts: usize,
    pub removed_artifacts: usize,
    pub retained_artifact_failures: usize,
    pub retry_artifacts: Vec<ProofArtifactKey>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PendingRetentionBatchOutcome {
    pub removed_pending_publications: usize,
    pub retained_pending_publication_failures: usize,
    pub retry_pending_publications: Vec<ProofArtifactKey>,
}

#[derive(Debug, Default)]
struct ArtifactFinalizationBatch {
    finalized: Vec<ArtifactExpectation>,
    retry_artifacts: Vec<ProofArtifactKey>,
    failures: usize,
}

#[derive(Debug, Default)]
struct PendingPublicationFinalizationBatch {
    finalized: Vec<PendingPublicationExpectation>,
    retry_pending_publications: Vec<ProofArtifactKey>,
    failures: usize,
}

impl ProofLifecycle {
    pub(crate) fn new(runtime: Arc<RuntimeManager>, pipelines: Arc<dyn PipelineFactory>) -> Self {
        Self { runtime, pipelines }
    }

    /// Closes new root-to-projection transitions before namespace draining begins.
    pub(crate) async fn begin_shutdown(&self) {
        let gate = self.runtime.execution_lifecycle_gate();
        let _gate = gate.lock().await;
        self.runtime.start_draining();
    }

    /// Attaches an execution graph only while its exact runtime root remains active.
    pub(crate) async fn attach(
        &self,
        record: &RuntimeTaskRecord,
        engine: &Arc<dyn EngineHandle>,
        plan: EngineExecutionPlan,
    ) -> Result<()> {
        let gate = self.runtime.execution_lifecycle_gate();
        let _gate = gate.lock().await;
        if !self.runtime.is_lifecycle_active() {
            bail!("runtime root is no longer active for execution attachment");
        }
        let Some(current) = self.runtime.get_task(&record.task_id).await? else {
            bail!("runtime root is no longer active for execution attachment");
        };
        if current.incarnation_id != record.incarnation_id {
            bail!("runtime root is no longer active for execution attachment");
        }
        if current.runner_status == RunnerStatus::Completed {
            return Ok(());
        }
        if !matches!(
            current.runner_status,
            RunnerStatus::Allocated | RunnerStatus::Running
        ) {
            bail!("runtime root is no longer active for execution attachment");
        }
        engine
            .attach_execution_plan(
                RootOwner::new(record.task_id.clone(), record.incarnation_id),
                plan,
            )
            .await?;
        Ok(())
    }

    /// Recovers an exact inactive root and attaches its execution plan as one lifecycle
    /// transition. An active owner always wins over duplicate recovery.
    pub(crate) async fn recover_if_inactive<F>(
        &self,
        expected: &RuntimeTaskRecord,
        build_plan: F,
    ) -> Result<RecoveryOutcome>
    where
        F: FnOnce() -> Result<EngineExecutionPlan>,
    {
        if matches!(
            expected.runner_status,
            RunnerStatus::Completed | RunnerStatus::Cancelled
        ) {
            return Ok(RecoveryOutcome::TaskChanged);
        }

        let gate = self.runtime.execution_lifecycle_gate();
        let _gate = gate.lock().await;
        if !self.runtime.is_lifecycle_active() {
            bail!("runtime root is no longer active for recovery attachment");
        }
        let Some(engine) = self
            .pipelines
            .get(&expected.network_pair, expected.pipeline_key)
        else {
            bail!(
                "execution pipeline is unavailable for recovered task {}",
                expected.task_id
            );
        };
        let owner = RootOwner::new(expected.task_id.clone(), expected.incarnation_id);
        // A failed runtime root is explicitly eligible for recovery before remote
        // submission, even when its in-memory execution graph is still attached.
        // Only live roots need the active-owner guard that prevents a duplicate
        // poll from racing a concurrent attachment.
        if matches!(
            expected.runner_status,
            RunnerStatus::Allocated | RunnerStatus::Running
        ) && engine.has_active_execution(owner.clone()).await?
        {
            return Ok(RecoveryOutcome::Active);
        }
        let Some(prepared) = self
            .runtime
            .prepare_task_for_recovery_if_unchanged(expected)
            .await?
        else {
            return Ok(RecoveryOutcome::TaskChanged);
        };

        let plan = match build_plan() {
            Ok(plan) => plan,
            Err(error) => {
                if let Err(rollback_error) =
                    self.restore_recovery_snapshot(&prepared, expected).await
                {
                    return Err(anyhow::anyhow!(
                        "failed to build recovered execution plan: {error:#}; {rollback_error:#}"
                    ));
                }
                return Err(error);
            }
        };
        if let Err(error) = engine.attach_execution_plan(owner, plan).await {
            if let Err(rollback_error) = self.restore_recovery_snapshot(&prepared, expected).await {
                return Err(anyhow::anyhow!(
                    "failed to attach recovered execution: {error}; {rollback_error:#}"
                ));
            }
            return Err(error.into());
        }
        Ok(RecoveryOutcome::Recovered)
    }

    async fn restore_recovery_snapshot(
        &self,
        prepared: &RuntimeTaskRecord,
        expected: &RuntimeTaskRecord,
    ) -> Result<()> {
        match self
            .runtime
            .restore_task_after_recovery_if_unchanged(prepared, expected)
            .await?
        {
            RuntimeMutationOutcome::Applied => Ok(()),
            outcome => bail!("recovery rollback was not applied: {outcome:?}"),
        }
    }

    /// Replaces one unchanged runtime root and swaps its queue projection while holding the
    /// process-local lifecycle transition gate.
    pub(crate) async fn replace(
        &self,
        expected: &RuntimeTaskRecord,
        registration: TaskRegistration,
        artifact_preconditions: &[ProofArtifactPrecondition],
        replacement_engine: &Arc<dyn EngineHandle>,
        replacement_plan: EngineExecutionPlan,
    ) -> Result<Option<RuntimeTaskRecord>> {
        let previous_engine = self
            .pipelines
            .get(&expected.network_pair, expected.pipeline_key);
        let gate = self.runtime.execution_lifecycle_gate();
        let gate_guard = gate.lock().await;
        let Some(replacement) = self
            .runtime
            .replace_task_if_unchanged_with_artifact_preconditions(
                expected,
                registration,
                artifact_preconditions,
            )
            .await?
        else {
            return Ok(None);
        };

        let projection = async {
            if let Some(engine) = previous_engine {
                engine
                    .detach_execution(
                        RootOwner::new(expected.task_id.clone(), expected.incarnation_id),
                        DetachMode::Remove,
                    )
                    .await?;
            }
            replacement_engine
                .attach_execution_plan(
                    RootOwner::new(replacement.task_id.clone(), replacement.incarnation_id),
                    replacement_plan,
                )
                .await
        }
        .await
        .map_err(anyhow::Error::from);
        drop(gate_guard);
        let publication_cleanup = self
            .runtime
            .release_task_pending_publications(expected)
            .await;
        finish_lifecycle_effect(projection, publication_cleanup)?;
        Ok(Some(replacement))
    }

    /// Cancels the durable root first, then detaches its exact queue projection.
    pub(crate) async fn cancel(
        &self,
        record: &RuntimeTaskRecord,
        error: Option<String>,
    ) -> Result<RuntimeMutationOutcome> {
        self.cancel_transition(record, error).await
    }

    /// Cancels an unchanged root only when its exact execution projection is still inactive.
    pub(crate) async fn cancel_orphaned_if_unchanged(
        &self,
        record: &RuntimeTaskRecord,
        error: String,
    ) -> Result<RuntimeMutationOutcome> {
        let engine = self
            .pipelines
            .get(&record.network_pair, record.pipeline_key);
        let gate = self.runtime.execution_lifecycle_gate();
        let gate = gate.lock().await;
        let owner = RootOwner::new(record.task_id.clone(), record.incarnation_id);
        if let Some(engine) = &engine
            && engine.has_active_execution(owner.clone()).await?
        {
            return Ok(RuntimeMutationOutcome::Blocked);
        }

        let outcome = self
            .runtime
            .cancel_task_if_unchanged(record, Some(error))
            .await?;
        if !matches!(
            outcome,
            RuntimeMutationOutcome::Applied | RuntimeMutationOutcome::AlreadyApplied
        ) {
            return Ok(outcome);
        }
        let projection = if let Some(engine) = engine {
            engine
                .detach_execution(owner, DetachMode::Cancel)
                .await
                .map_err(anyhow::Error::from)
        } else {
            Ok(DetachOutcome::not_attached(DetachMode::Cancel))
        };
        drop(gate);
        let publication_cleanup = self.runtime.release_task_pending_publications(record).await;
        finish_lifecycle_effect(projection, publication_cleanup)?;
        Ok(outcome)
    }

    async fn cancel_transition(
        &self,
        record: &RuntimeTaskRecord,
        error: Option<String>,
    ) -> Result<RuntimeMutationOutcome> {
        let engine = self
            .pipelines
            .get(&record.network_pair, record.pipeline_key);
        let gate = self.runtime.execution_lifecycle_gate();
        let gate = gate.lock().await;
        let outcome = self
            .runtime
            .cancel_task_if_current(&record.lifetime(), error)
            .await?;
        if !matches!(
            outcome,
            RuntimeMutationOutcome::Applied | RuntimeMutationOutcome::AlreadyApplied
        ) {
            return Ok(outcome);
        }
        let projection = if let Some(engine) = engine {
            engine
                .detach_execution(
                    RootOwner::new(record.task_id.clone(), record.incarnation_id),
                    DetachMode::Cancel,
                )
                .await
                .map_err(anyhow::Error::from)
        } else {
            Ok(DetachOutcome::not_attached(DetachMode::Cancel))
        };
        drop(gate);
        let publication_cleanup = self.runtime.release_task_pending_publications(record).await;
        finish_lifecycle_effect(projection, publication_cleanup)?;
        Ok(outcome)
    }

    /// Retires and detaches one bounded terminal-retention batch under the process-local execution
    /// lifecycle gate, then finalizes exact external artifacts without blocking other lifecycles.
    pub(crate) async fn remove_terminal_retention_batch(
        &self,
        records: &[RuntimeTaskRecord],
    ) -> Result<TerminalRetentionBatchOutcome> {
        if records.is_empty() {
            return Ok(TerminalRetentionBatchOutcome::default());
        }

        let lifecycle_gate = self.runtime.execution_lifecycle_gate();
        let lifecycle_gate_guard = lifecycle_gate.lock().await;
        let prepared = self
            .runtime
            .prepare_terminal_task_retention_batch(records)
            .await?;
        let mut outcome = TerminalRetentionBatchOutcome {
            retired_roots: prepared.retired_tasks.len(),
            skipped_roots: prepared.skipped_tasks,
            invalidated_artifacts: prepared.artifact_invalidations.len(),
            ..TerminalRetentionBatchOutcome::default()
        };

        let mut detached_tasks = Vec::with_capacity(prepared.retired_tasks.len());
        for record in prepared.retired_tasks {
            let detached = if let Some(engine) = self
                .pipelines
                .get(&record.network_pair, record.pipeline_key)
            {
                engine
                    .detach_execution(
                        RootOwner::new(record.task_id.clone(), record.incarnation_id),
                        DetachMode::Remove,
                    )
                    .await
                    .map_err(anyhow::Error::from)
            } else {
                Ok(DetachOutcome::not_attached(DetachMode::Remove))
            };
            match detached {
                Ok(detached) => {
                    outcome.skipped_shared_children = outcome
                        .skipped_shared_children
                        .saturating_add(detached.retained.len());
                    detached_tasks.push(record);
                }
                Err(error) => {
                    outcome.retained_root_failures =
                        outcome.retained_root_failures.saturating_add(1);
                    outcome.retry_roots.push(record.lifetime());
                    warn!(
                        task_id = %record.task_id,
                        error = %error,
                        "failed to detach expired runtime task"
                    );
                }
            }
        }
        let finalized_roots = self
            .runtime
            .finalize_terminal_task_retention_batch(&detached_tasks, &[], &[])
            .await?;
        let removed_lifetimes = finalized_roots
            .removed_tasks
            .iter()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        for record in &detached_tasks {
            let lifetime = record.lifetime();
            if !removed_lifetimes.contains(&lifetime) && !outcome.retry_roots.contains(&lifetime) {
                outcome.retry_roots.push(lifetime);
            }
        }
        outcome.removed_roots = finalized_roots.removed_tasks.len();
        outcome.retained_root_failures = outcome
            .retained_root_failures
            .saturating_add(finalized_roots.skipped_tasks);
        drop(lifecycle_gate_guard);

        let artifact_batch = finalize_terminal_retention_artifacts(
            Arc::clone(&self.runtime),
            prepared.artifact_invalidations,
        )
        .await;
        outcome.retained_artifact_failures = artifact_batch.failures;
        outcome.retry_artifacts = artifact_batch.retry_artifacts;

        let removable_artifacts = artifact_batch.finalized;
        if removable_artifacts.is_empty() {
            return Ok(outcome);
        }
        let finalized = self
            .runtime
            .finalize_terminal_task_retention_batch(&[], &removable_artifacts, &[])
            .await?;
        outcome.removed_artifacts = finalized.removed_artifacts.len();
        outcome.retained_artifact_failures = outcome
            .retained_artifact_failures
            .saturating_add(finalized.skipped_artifacts);
        Ok(outcome)
    }

    pub(crate) async fn remove_artifact_retention_batch(
        &self,
        records: &[ProofArtifactRecord],
    ) -> Result<ArtifactRetentionBatchOutcome> {
        let prepared = self
            .runtime
            .prepare_artifact_retention_batch(records)
            .await?;
        let mut outcome = ArtifactRetentionBatchOutcome {
            invalidated_artifacts: prepared.newly_invalidated_artifacts,
            ..ArtifactRetentionBatchOutcome::default()
        };
        let artifact_batch = finalize_terminal_retention_artifacts(
            Arc::clone(&self.runtime),
            prepared.artifact_invalidations,
        )
        .await;
        outcome.retained_artifact_failures = artifact_batch.failures;
        outcome.retry_artifacts = artifact_batch.retry_artifacts;
        let removable_artifacts = artifact_batch.finalized;
        if removable_artifacts.is_empty() {
            return Ok(outcome);
        }
        let finalized = self
            .runtime
            .finalize_terminal_task_retention_batch(&[], &removable_artifacts, &[])
            .await?;
        outcome.removed_artifacts = finalized.removed_artifacts.len();
        outcome.retained_artifact_failures = outcome
            .retained_artifact_failures
            .saturating_add(finalized.skipped_artifacts);
        Ok(outcome)
    }

    pub(crate) async fn remove_pending_retention_batch(
        &self,
        expectations: &[PendingPublicationExpectation],
    ) -> Result<PendingRetentionBatchOutcome> {
        let pending_batch = finalize_terminal_retention_pending_publications(
            Arc::clone(&self.runtime),
            expectations.to_vec(),
        )
        .await;
        let mut outcome = PendingRetentionBatchOutcome {
            retained_pending_publication_failures: pending_batch.failures,
            retry_pending_publications: pending_batch.retry_pending_publications,
            ..PendingRetentionBatchOutcome::default()
        };
        if pending_batch.finalized.is_empty() {
            return Ok(outcome);
        }
        let finalized = self
            .runtime
            .finalize_terminal_task_retention_batch(&[], &[], &pending_batch.finalized)
            .await?;
        outcome.removed_pending_publications = finalized.removed_pending_publications.len();
        outcome.retained_pending_publication_failures = outcome
            .retained_pending_publication_failures
            .saturating_add(finalized.skipped_pending_publications);
        Ok(outcome)
    }

    /// Removes one unchanged root and its exact queue projection as one lifecycle transition.
    pub(crate) async fn remove(
        &self,
        record: &RuntimeTaskRecord,
        mode: DetachMode,
    ) -> Result<(
        RuntimeMutationOutcome,
        DetachOutcome<raiko2_engine::EngineTaskKey>,
    )> {
        let engine = self
            .pipelines
            .get(&record.network_pair, record.pipeline_key);
        let gate = self.runtime.execution_lifecycle_gate();
        let gate = gate.lock().await;
        let outcome = self.runtime.retire_task_if_unchanged(record, None).await?;
        if !matches!(
            outcome,
            RuntimeMutationOutcome::Applied | RuntimeMutationOutcome::AlreadyApplied
        ) {
            return Ok((outcome, DetachOutcome::not_attached(mode)));
        }
        let detached: Result<_> = if let Some(engine) = engine {
            engine
                .detach_execution(
                    RootOwner::new(record.task_id.clone(), record.incarnation_id),
                    mode,
                )
                .await
                .map_err(anyhow::Error::from)
        } else {
            Ok(DetachOutcome::not_attached(mode))
        };
        let removal = match detached {
            Ok(detached) => self
                .runtime
                .remove_task_if_current(&record.lifetime())
                .await
                .map(|removed| (removed, detached)),
            Err(error) => Err(error),
        };
        drop(gate);
        let publication_cleanup = self.runtime.release_task_pending_publications(record).await;
        finish_lifecycle_effect(removal, publication_cleanup)
    }
}

async fn finalize_terminal_retention_artifacts(
    runtime: Arc<RuntimeManager>,
    expectations: Vec<ArtifactExpectation>,
) -> ArtifactFinalizationBatch {
    const CONCURRENCY: usize = 8;

    let mut queue = expectations.into_iter();
    let mut workers = JoinSet::new();
    let mut batch = ArtifactFinalizationBatch::default();
    loop {
        while workers.len() < CONCURRENCY {
            let Some(expectation) = queue.next() else {
                break;
            };
            let runtime = Arc::clone(&runtime);
            workers.spawn(async move {
                let result = runtime
                    .finalize_proof_artifact_invalidation(&expectation)
                    .await;
                (expectation, result)
            });
        }
        let Some(finalized) = workers.join_next().await else {
            break;
        };
        match finalized {
            Ok((expectation, Ok(ExactDeleteResult::Removed | ExactDeleteResult::Missing))) => {
                batch.finalized.push(expectation);
            }
            Ok((expectation, Ok(ExactDeleteResult::Stale))) => {
                match runtime
                    .proof_artifact_invalidation_is_stale(&expectation)
                    .await
                {
                    Ok(true) => {
                        warn!(
                            proof_ref = %expectation.key.proof_ref,
                            "discarding stale proof artifact cleanup candidate"
                        );
                    }
                    Ok(false) => {
                        batch.failures = batch.failures.saturating_add(1);
                        batch.retry_artifacts.push(expectation.key.clone());
                        warn!(
                            proof_ref = %expectation.key.proof_ref,
                            "exact proof deletion observed a changed manifest while the invalidation remains authoritative"
                        );
                    }
                    Err(error) => {
                        batch.failures = batch.failures.saturating_add(1);
                        batch.retry_artifacts.push(expectation.key.clone());
                        warn!(
                            proof_ref = %expectation.key.proof_ref,
                            error = %error,
                            "failed to recheck stale proof artifact cleanup candidate"
                        );
                    }
                }
            }
            Ok((expectation, Err(error))) => {
                match runtime
                    .proof_artifact_invalidation_is_stale(&expectation)
                    .await
                {
                    Ok(true) => {
                        warn!(
                            proof_ref = %expectation.key.proof_ref,
                            "discarding failed cleanup for a stale proof artifact candidate"
                        );
                    }
                    Ok(false) => {
                        batch.failures = batch.failures.saturating_add(1);
                        batch.retry_artifacts.push(expectation.key.clone());
                        warn!(
                            proof_ref = %expectation.key.proof_ref,
                            error = %error,
                            "failed to finalize expired proof artifact invalidation"
                        );
                    }
                    Err(stale_check_error) => {
                        batch.failures = batch.failures.saturating_add(1);
                        batch.retry_artifacts.push(expectation.key.clone());
                        warn!(
                            proof_ref = %expectation.key.proof_ref,
                            error = %stale_check_error,
                            "failed to inspect proof artifact descriptor after finalization error"
                        );
                        warn!(
                            proof_ref = %expectation.key.proof_ref,
                            error = %error,
                            "failed to finalize expired proof artifact invalidation"
                        );
                    }
                }
            }
            Err(error) => {
                batch.failures = batch.failures.saturating_add(1);
                warn!(error = %error, "proof artifact invalidation worker failed");
            }
        }
    }
    batch
}

async fn finalize_terminal_retention_pending_publications(
    runtime: Arc<RuntimeManager>,
    expectations: Vec<PendingPublicationExpectation>,
) -> PendingPublicationFinalizationBatch {
    const CONCURRENCY: usize = 8;

    let mut queue = expectations.into_iter();
    let mut workers = JoinSet::new();
    let mut batch = PendingPublicationFinalizationBatch::default();
    loop {
        while workers.len() < CONCURRENCY {
            let Some(expectation) = queue.next() else {
                break;
            };
            let runtime = Arc::clone(&runtime);
            workers.spawn(async move {
                let result = runtime
                    .finalize_pending_publication_retention(&expectation)
                    .await;
                (expectation, result)
            });
        }
        let Some(finalized) = workers.join_next().await else {
            break;
        };
        match finalized {
            Ok((expectation, Ok(_))) => {
                batch.finalized.push(expectation);
            }
            Ok((expectation, Err(error))) => {
                batch.failures = batch.failures.saturating_add(1);
                batch
                    .retry_pending_publications
                    .push(expectation.key.clone());
                warn!(
                    proof_ref = %expectation.key.proof_ref,
                    error = %error,
                    "failed to finalize expired pending proof publication"
                );
            }
            Err(error) => {
                batch.failures = batch.failures.saturating_add(1);
                warn!(error = %error, "pending proof publication finalization worker failed");
            }
        }
    }
    batch
}

fn finish_lifecycle_effect<T>(effect: Result<T>, publication_cleanup: Result<()>) -> Result<T> {
    match (effect, publication_cleanup) {
        (Ok(output), Ok(())) => Ok(output),
        (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
        (Err(effect_error), Err(cleanup_error)) => Err(anyhow::anyhow!(
            "lifecycle projection effect failed: {effect_error:#}; pending publication cleanup also failed: {cleanup_error:#}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::state::{EngineQueueTaskView, EngineStatusView, StaticPipelineFactory};
    use raiko2_engine::{EngineTaskId, EngineTaskKey};
    use raiko2_pipeline::PipelineKey;
    use raiko2_queue::{AttachOutcome, TaskStoreError};
    use raiko2_runtime::{ProofArtifactRegistration, TaskRegistration};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::sync::Notify;

    type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

    #[tokio::test]
    async fn artifact_finalization_accepts_an_already_missing_manifest() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new_memory(
            "test".into(),
            format!("missing-finalization-{}", uuid::Uuid::new_v4()),
        )?);
        let pipeline = PipelineKey::NativeLocal;
        let route = pipeline.route();
        let object = runtime
            .publish_proof_artifact_bytes(
                "taiko_dev/ethereum",
                pipeline,
                route,
                "proof-ref",
                br#"{"proof":"0x01"}"#,
            )
            .await?
            .try_object()
            .expect("proof publication")
            .clone();
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: "taiko_dev/ethereum".into(),
                proof_ref: "proof-ref".into(),
                pipeline_key: pipeline,
                route,
                proof_uri: object.proof_uri,
                content_hash: object.content_hash,
                generation: object.generation,
            })
            .await?;
        let active = runtime
            .get_proof_artifact("taiko_dev/ethereum", pipeline, route, "proof-ref")
            .await?
            .expect("active artifact");
        let prepared = runtime.prepare_artifact_retention_batch(&[active]).await?;
        let expectation = prepared
            .artifact_invalidations
            .first()
            .expect("prepared invalidation")
            .clone();
        assert_eq!(
            runtime
                .delete_proof_artifact(
                    &expectation.key.network_pair,
                    expectation.key.pipeline_key,
                    expectation.key.route,
                    &expectation.key.proof_ref,
                    expectation.descriptor.generation,
                    &expectation.descriptor.content_hash,
                )
                .await?,
            raiko2_runtime::ProofArtifactDeleteResult::Removed
        );

        let batch =
            finalize_terminal_retention_artifacts(Arc::clone(&runtime), vec![expectation.clone()])
                .await;
        assert_eq!(batch.finalized, vec![expectation.clone()]);
        assert_eq!(batch.failures, 0);
        assert!(batch.retry_artifacts.is_empty());

        let finalized = runtime
            .finalize_terminal_task_retention_batch(&[], &batch.finalized, &[])
            .await?;
        assert_eq!(finalized.removed_artifacts, vec![expectation]);
        Ok(())
    }

    #[derive(Default)]
    struct BlockingProjectionEngine {
        inspection_started: Notify,
        allow_inspection: Notify,
        attached: AtomicBool,
    }

    impl EngineHandle for BlockingProjectionEngine {
        fn get_status(
            &self,
            _id: EngineTaskId,
        ) -> BoxFuture<'_, Result<Option<EngineStatusView>, TaskStoreError>> {
            Box::pin(async { Ok(None) })
        }

        fn get_task_state(
            &self,
            _id: EngineTaskId,
        ) -> BoxFuture<'_, Result<Option<EngineQueueTaskView>, TaskStoreError>> {
            Box::pin(async { Ok(None) })
        }

        fn has_active_execution(
            &self,
            _owner: RootOwner,
        ) -> BoxFuture<'_, Result<bool, TaskStoreError>> {
            Box::pin(async move {
                self.inspection_started.notify_one();
                self.allow_inspection.notified().await;
                Ok(self.attached.load(Ordering::SeqCst))
            })
        }

        fn attach_execution_plan(
            &self,
            _owner: RootOwner,
            _plan: EngineExecutionPlan,
        ) -> BoxFuture<'_, Result<AttachOutcome, TaskStoreError>> {
            Box::pin(async move {
                self.attached.store(true, Ordering::SeqCst);
                Ok(AttachOutcome::Attached)
            })
        }

        fn detach_execution(
            &self,
            _owner: RootOwner,
            mode: DetachMode,
        ) -> BoxFuture<'_, Result<DetachOutcome<EngineTaskKey>, TaskStoreError>> {
            Box::pin(async move {
                self.attached.store(false, Ordering::SeqCst);
                Ok(DetachOutcome::not_attached(mode))
            })
        }
    }

    #[derive(Default)]
    struct BlockingAttachEngine {
        attach_started: Notify,
        allow_attach: Notify,
        attached: AtomicBool,
        attach_calls: AtomicUsize,
    }

    impl EngineHandle for BlockingAttachEngine {
        fn get_status(
            &self,
            _id: EngineTaskId,
        ) -> BoxFuture<'_, Result<Option<EngineStatusView>, TaskStoreError>> {
            Box::pin(async { Ok(None) })
        }

        fn get_task_state(
            &self,
            _id: EngineTaskId,
        ) -> BoxFuture<'_, Result<Option<EngineQueueTaskView>, TaskStoreError>> {
            Box::pin(async { Ok(None) })
        }

        fn has_active_execution(
            &self,
            _owner: RootOwner,
        ) -> BoxFuture<'_, Result<bool, TaskStoreError>> {
            let attached = self.attached.load(Ordering::SeqCst);
            Box::pin(async move { Ok(attached) })
        }

        fn attach_execution_plan(
            &self,
            _owner: RootOwner,
            _plan: EngineExecutionPlan,
        ) -> BoxFuture<'_, Result<AttachOutcome, TaskStoreError>> {
            Box::pin(async move {
                self.attach_calls.fetch_add(1, Ordering::SeqCst);
                self.attach_started.notify_one();
                self.allow_attach.notified().await;
                self.attached.store(true, Ordering::SeqCst);
                Ok(AttachOutcome::Attached)
            })
        }

        fn detach_execution(
            &self,
            _owner: RootOwner,
            mode: DetachMode,
        ) -> BoxFuture<'_, Result<DetachOutcome<EngineTaskKey>, TaskStoreError>> {
            Box::pin(async move { Ok(DetachOutcome::not_attached(mode)) })
        }
    }

    struct FailingAttachEngine;

    impl EngineHandle for FailingAttachEngine {
        fn get_status(
            &self,
            _id: EngineTaskId,
        ) -> BoxFuture<'_, Result<Option<EngineStatusView>, TaskStoreError>> {
            Box::pin(async { Ok(None) })
        }

        fn get_task_state(
            &self,
            _id: EngineTaskId,
        ) -> BoxFuture<'_, Result<Option<EngineQueueTaskView>, TaskStoreError>> {
            Box::pin(async { Ok(None) })
        }

        fn has_active_execution(
            &self,
            _owner: RootOwner,
        ) -> BoxFuture<'_, Result<bool, TaskStoreError>> {
            Box::pin(async { Ok(false) })
        }

        fn attach_execution_plan(
            &self,
            _owner: RootOwner,
            _plan: EngineExecutionPlan,
        ) -> BoxFuture<'_, Result<AttachOutcome, TaskStoreError>> {
            Box::pin(async {
                Err(TaskStoreError::backend(std::io::Error::other(
                    "attachment failed",
                )))
            })
        }

        fn detach_execution(
            &self,
            _owner: RootOwner,
            mode: DetachMode,
        ) -> BoxFuture<'_, Result<DetachOutcome<EngineTaskKey>, TaskStoreError>> {
            Box::pin(async move { Ok(DetachOutcome::not_attached(mode)) })
        }
    }

    #[tokio::test]
    async fn orphan_cancellation_serializes_projection_inspection_with_attach() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new_memory(
            "test".into(),
            format!("orphan-attach-race-{}", uuid::Uuid::new_v4()),
        )?);
        let record = runtime
            .register_task(TaskRegistration {
                task_id: "root".into(),
                pipeline_key: PipelineKey::NativeLocal,
                route: PipelineKey::NativeLocal.route(),
                task_kind: "proposal".into(),
                network_pair: "taiko_dev/ethereum".into(),
                artifact_refs: Vec::new(),
                metadata: serde_json::json!({}),
                request_fingerprint: "root-request".into(),
            })
            .await?;
        let engine = Arc::new(BlockingProjectionEngine::default());
        let mut pipelines = StaticPipelineFactory::default();
        pipelines.insert(
            &record.network_pair,
            record.pipeline_key,
            engine.clone() as Arc<dyn EngineHandle>,
        );
        let lifecycle = ProofLifecycle::new(Arc::clone(&runtime), Arc::new(pipelines));

        let inspection_started = engine.inspection_started.notified();
        let cancel_lifecycle = lifecycle.clone();
        let cancel_record = record.clone();
        let cancellation = tokio::spawn(async move {
            cancel_lifecycle
                .cancel_orphaned_if_unchanged(&cancel_record, "orphaned".into())
                .await
        });
        inspection_started.await;

        let attach_lifecycle = lifecycle.clone();
        let attach_record = record.clone();
        let attach_engine = engine.clone() as Arc<dyn EngineHandle>;
        let attachment = tokio::spawn(async move {
            attach_lifecycle
                .attach(
                    &attach_record,
                    &attach_engine,
                    EngineExecutionPlan {
                        proposals: Vec::new(),
                        aggregate: None,
                    },
                )
                .await
        });
        engine.allow_inspection.notify_one();

        assert_eq!(cancellation.await??, RuntimeMutationOutcome::Applied);
        assert!(attachment.await?.is_err());
        assert!(!engine.attached.load(Ordering::SeqCst));
        assert_eq!(
            runtime
                .get_task(&record.task_id)
                .await?
                .expect("cancelled root")
                .runner_status,
            RunnerStatus::Cancelled
        );
        Ok(())
    }

    #[tokio::test]
    async fn recovery_does_not_reattach_an_owner_attached_after_candidate_evaluation() -> Result<()>
    {
        let runtime = Arc::new(RuntimeManager::new_memory(
            "test".into(),
            format!("recovery-attach-race-{}", uuid::Uuid::new_v4()),
        )?);
        let record = runtime
            .register_task(TaskRegistration {
                task_id: "root".into(),
                pipeline_key: PipelineKey::NativeLocal,
                route: PipelineKey::NativeLocal.route(),
                task_kind: "proposal".into(),
                network_pair: "taiko_dev/ethereum".into(),
                artifact_refs: Vec::new(),
                metadata: serde_json::json!({}),
                request_fingerprint: "root-request".into(),
            })
            .await?;
        let engine = Arc::new(BlockingAttachEngine::default());
        let mut pipelines = StaticPipelineFactory::default();
        pipelines.insert(
            &record.network_pair,
            record.pipeline_key,
            engine.clone() as Arc<dyn EngineHandle>,
        );
        let lifecycle = ProofLifecycle::new(Arc::clone(&runtime), Arc::new(pipelines));

        let attach_started = engine.attach_started.notified();
        let attach_lifecycle = lifecycle.clone();
        let attach_record = record.clone();
        let attach_engine = engine.clone() as Arc<dyn EngineHandle>;
        let attachment = tokio::spawn(async move {
            attach_lifecycle
                .attach(
                    &attach_record,
                    &attach_engine,
                    EngineExecutionPlan {
                        proposals: Vec::new(),
                        aggregate: None,
                    },
                )
                .await
        });
        attach_started.await;

        let recover_lifecycle = lifecycle.clone();
        let recover_record = record.clone();
        let recovery = tokio::spawn(async move {
            recover_lifecycle
                .recover_if_inactive(&recover_record, || {
                    Ok(EngineExecutionPlan {
                        proposals: Vec::new(),
                        aggregate: None,
                    })
                })
                .await
        });
        engine.allow_attach.notify_one();

        attachment.await??;
        assert_eq!(recovery.await??, RecoveryOutcome::Active);
        assert_eq!(engine.attach_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            runtime
                .get_task(&record.task_id)
                .await?
                .expect("runtime root"),
            record
        );
        Ok(())
    }

    #[tokio::test]
    async fn recovery_attachment_failure_restores_the_exact_runtime_snapshot() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new_memory(
            "test".into(),
            format!("recovery-attach-rollback-{}", uuid::Uuid::new_v4()),
        )?);
        let record = runtime
            .register_task(TaskRegistration {
                task_id: "root".into(),
                pipeline_key: PipelineKey::NativeLocal,
                route: PipelineKey::NativeLocal.route(),
                task_kind: "proposal".into(),
                network_pair: "taiko_dev/ethereum".into(),
                artifact_refs: Vec::new(),
                metadata: serde_json::json!({}),
                request_fingerprint: "root-request".into(),
            })
            .await?;
        let mut pipelines = StaticPipelineFactory::default();
        pipelines.insert(
            &record.network_pair,
            record.pipeline_key,
            Arc::new(FailingAttachEngine) as Arc<dyn EngineHandle>,
        );
        let lifecycle = ProofLifecycle::new(Arc::clone(&runtime), Arc::new(pipelines));

        let error = lifecycle
            .recover_if_inactive(&record, || {
                Ok(EngineExecutionPlan {
                    proposals: Vec::new(),
                    aggregate: None,
                })
            })
            .await
            .expect_err("failed recovery attachment");

        assert!(error.to_string().contains("attachment failed"));
        assert_eq!(
            runtime
                .get_task(&record.task_id)
                .await?
                .expect("runtime root"),
            record
        );
        Ok(())
    }
}
