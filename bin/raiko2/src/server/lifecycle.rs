//! Runtime-root lifecycle coordination.

use anyhow::{Result, bail};
use raiko2_engine::EngineExecutionPlan;
use raiko2_queue::{AttachOutcome, DetachMode, DetachOutcome, RootOwner};
use raiko2_runtime::{RuntimeManager, RuntimeMutationOutcome, RuntimeTaskRecord};
use std::sync::Arc;

use crate::server::state::{EngineHandle, PipelineFactory};
use crate::server::task_metadata::TaskMetadata;

/// Coordinates durable root transitions with idempotent in-memory queue effects.
#[derive(Clone)]
pub(crate) struct ProofLifecycle {
    runtime: Arc<RuntimeManager>,
    pipelines: Arc<dyn PipelineFactory>,
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
    ) -> Result<AttachOutcome> {
        let gate = self.runtime.execution_lifecycle_gate();
        let _gate = gate.lock().await;
        if !self.runtime.is_lifecycle_active()
            || !self
                .runtime
                .is_task_active_if_current(&record.lifetime())
                .await?
        {
            bail!("runtime root is no longer active for execution attachment");
        }
        Ok(engine
            .attach_execution_plan(
                RootOwner::new(record.task_id.clone(), record.incarnation_id),
                plan,
            )
            .await?)
    }

    /// Cancels the durable root first, then detaches its exact queue projection.
    pub(crate) async fn cancel(
        &self,
        record: &RuntimeTaskRecord,
        metadata: &TaskMetadata,
        error: Option<String>,
    ) -> Result<RuntimeMutationOutcome> {
        let engine = self
            .pipelines
            .get(&metadata.network_pair, record.pipeline_key);
        let gate = self.runtime.execution_lifecycle_gate();
        let _gate = gate.lock().await;
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
        if let Some(engine) = engine {
            engine
                .detach_execution(
                    RootOwner::new(record.task_id.clone(), record.incarnation_id),
                    DetachMode::Cancel,
                )
                .await?;
        }
        Ok(outcome)
    }

    /// Retires a root before destructive cleanup and removes its exact queue projection.
    pub(crate) async fn retire(
        &self,
        record: &RuntimeTaskRecord,
        metadata: &TaskMetadata,
        mode: DetachMode,
    ) -> Result<(
        RuntimeMutationOutcome,
        DetachOutcome<raiko2_engine::EngineTaskKey>,
    )> {
        let engine = self
            .pipelines
            .get(&metadata.network_pair, record.pipeline_key);
        let gate = self.runtime.execution_lifecycle_gate();
        let _gate = gate.lock().await;
        let outcome = self
            .runtime
            .retire_task_if_current(&record.lifetime(), None)
            .await?;
        if !matches!(
            outcome,
            RuntimeMutationOutcome::Applied | RuntimeMutationOutcome::AlreadyApplied
        ) {
            return Ok((outcome, DetachOutcome::not_attached(mode)));
        }
        let detached = if let Some(engine) = engine {
            engine
                .detach_execution(
                    RootOwner::new(record.task_id.clone(), record.incarnation_id),
                    mode,
                )
                .await?
        } else {
            DetachOutcome::not_attached(mode)
        };
        Ok((outcome, detached))
    }
}
