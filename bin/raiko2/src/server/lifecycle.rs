//! Runtime-root lifecycle coordination.

use anyhow::{Result, bail};
use raiko2_engine::EngineExecutionPlan;
use raiko2_queue::{DetachMode, DetachOutcome, RootOwner};
use raiko2_runtime::{
    ProofArtifactPrecondition, RunnerStatus, RuntimeManager, RuntimeMutationOutcome,
    RuntimeTaskRecord, TaskRegistration,
};
use std::sync::Arc;

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
        self.replace_inner(
            expected,
            registration,
            artifact_preconditions,
            None,
            replacement_engine,
            replacement_plan,
        )
        .await
    }

    /// Replaces a completed root only after fencing the exact stale canonical artifact that made
    /// the root unreadable to this host.
    pub(crate) async fn replace_and_invalidate_artifact(
        &self,
        expected: &RuntimeTaskRecord,
        registration: TaskRegistration,
        stale_artifact: &ProofArtifactPrecondition,
        replacement_engine: &Arc<dyn EngineHandle>,
        replacement_plan: EngineExecutionPlan,
    ) -> Result<Option<RuntimeTaskRecord>> {
        self.replace_inner(
            expected,
            registration,
            &[],
            Some(stale_artifact),
            replacement_engine,
            replacement_plan,
        )
        .await
    }

    async fn replace_inner(
        &self,
        expected: &RuntimeTaskRecord,
        registration: TaskRegistration,
        artifact_preconditions: &[ProofArtifactPrecondition],
        stale_artifact: Option<&ProofArtifactPrecondition>,
        replacement_engine: &Arc<dyn EngineHandle>,
        replacement_plan: EngineExecutionPlan,
    ) -> Result<Option<RuntimeTaskRecord>> {
        let previous_engine = self
            .pipelines
            .get(&expected.network_pair, expected.pipeline_key);
        let gate = self.runtime.execution_lifecycle_gate();
        let gate_guard = gate.lock().await;
        let replacement = if let Some(stale_artifact) = stale_artifact {
            self.runtime
                .replace_task_if_unchanged_and_invalidate_artifact(
                    expected,
                    registration,
                    stale_artifact,
                )
                .await?
        } else {
            self.runtime
                .replace_task_if_unchanged_with_artifact_preconditions(
                    expected,
                    registration,
                    artifact_preconditions,
                )
                .await?
        };
        let Some(replacement) = replacement else {
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
    use raiko2_runtime::TaskRegistration;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::sync::Notify;

    type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

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
                pipeline_key: PipelineKey::ShastaNative,
                route: PipelineKey::ShastaNative.route(),
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
                pipeline_key: PipelineKey::ShastaNative,
                route: PipelineKey::ShastaNative.route(),
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
                pipeline_key: PipelineKey::ShastaNative,
                route: PipelineKey::ShastaNative.route(),
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
