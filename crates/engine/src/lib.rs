//! Raiko2 Engine - queue-driven proving orchestration.
//!
//! ## Module Structure
//!
//! - `worker` - Supervised worker management with auto-restart
//! - `tasks` - Task types and outputs
//!
//! Prover integration is provided via `raiko2-prover` and wired through
//! `Engine` / `tasks::EngineTask`.

#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

pub mod tasks;
pub mod worker;

pub use tasks::{
    AggregateProofInput, AggregationSource, AggregationTaskRequest, EncodedGuestInput,
    EngineTaskId, EngineTaskKey, ProofArtifactRef, ProposalStage, ProposalTaskRequest,
    ProverTaskConfig,
};

use std::sync::Arc;
use std::time::Duration;

use crate::worker::WorkerConfig;
use async_trait::async_trait;
use raiko2_pipeline::{Pipeline, PipelineSpec, PipelineStage, PipelineStageResult, ProverBackend};
use raiko2_primitives::{AggregationGuestInput, Proof, ProofContext, ShastaRequest};
use raiko2_prover::{BoundlessSubmissionResume, Prover, ProverProgress, ProverProgressObserver};
use raiko2_provider::Provider;
use raiko2_queue::{
    MemoryStore, NewTask, Priority, RetryPolicy, Scheduler, SchedulerConfig, TaskExecutionPolicy,
    TaskState, TaskStoreError, TaskView, TaskViewState,
};

use crate::tasks::{EngineOutput, EngineTask};

const PROPOSAL_TASK_PRIORITY: Priority = Priority::Medium;
const AGGREGATION_TASK_PRIORITY: Priority = Priority::High;

pub struct Engine<S>
where
    S: PipelineSpec,
    S::Prover: Prover<S::Backend, GuestInput = S::GuestInput>,
    S::Backend: ProverBackend,
    S::Provider: Provider,
{
    inner: Arc<Inner<S>>,
}

struct Inner<S>
where
    S: PipelineSpec,
{
    spec: S,
    scheduler: Scheduler<EngineTask, EngineOutput<S::GuestInput>, EngineTaskKey>,
    context: ProofContext,
    observer: Option<Arc<dyn EngineObserver>>,
}

#[derive(Clone, Debug)]
pub enum EngineTaskSuccess {
    GuestInput { stage: PipelineStage },
    EncodedInput { stage: PipelineStage },
    Proof { stage: PipelineStage, proof: Proof },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineObserverError {
    ProofPublication(String),
}

impl std::fmt::Display for EngineObserverError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProofPublication(error) => formatter.write_str(error),
        }
    }
}

impl std::error::Error for EngineObserverError {}

#[derive(Debug)]
enum TaskExecutionError {
    Retryable(String),
    Permanent(String),
}

impl TaskExecutionError {
    fn message(&self) -> &str {
        match self {
            Self::Retryable(error) | Self::Permanent(error) => error,
        }
    }
}

impl std::fmt::Display for TaskExecutionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl From<String> for TaskExecutionError {
    fn from(error: String) -> Self {
        Self::Retryable(error)
    }
}

impl From<EngineObserverError> for TaskExecutionError {
    fn from(error: EngineObserverError) -> Self {
        match error {
            EngineObserverError::ProofPublication(error) => Self::Permanent(error),
        }
    }
}

#[async_trait]
pub trait EngineObserver: Send + Sync {
    async fn on_task_started(&self, _id: &EngineTaskId, _task: &EngineTask, _worker: &str) {}

    async fn on_task_progress(
        &self,
        _id: &EngineTaskId,
        _task: &EngineTask,
        _progress: &ProverProgress,
    ) {
    }

    async fn on_task_succeeded(
        &self,
        _id: &EngineTaskId,
        _task: &EngineTask,
        _success: &EngineTaskSuccess,
    ) -> Result<(), EngineObserverError> {
        Ok(())
    }

    async fn on_task_failed(&self, _id: &EngineTaskId, _task: &EngineTask, _error: &str) {}

    async fn on_task_cancelled(&self, _id: &EngineTaskId) {}

    async fn load_sp1_network_request_id(
        &self,
        _id: &EngineTaskId,
        _task: &EngineTask,
    ) -> Option<String> {
        None
    }

    async fn load_boundless_submission(
        &self,
        _id: &EngineTaskId,
        _task: &EngineTask,
    ) -> Option<BoundlessSubmissionResume> {
        None
    }

    async fn load_proof_artifact(
        &self,
        _artifact: &ProofArtifactRef,
    ) -> Result<Option<raiko2_primitives::Proof>, String> {
        Ok(None)
    }

    async fn load_completed_proof(
        &self,
        _id: &EngineTaskId,
        _task: &EngineTask,
    ) -> Result<Option<Proof>, String> {
        Ok(None)
    }
}

async fn notify_stage_started(
    observer: Option<&Arc<dyn EngineObserver>>,
    id: &EngineTaskId,
    task: &EngineTask,
    worker: &str,
) {
    if let Some(observer) = observer {
        observer.on_task_started(id, task, worker).await;
    }
}

async fn notify_stage_succeeded(
    observer: Option<&Arc<dyn EngineObserver>>,
    id: &EngineTaskId,
    task: &EngineTask,
    success: &EngineTaskSuccess,
) -> Result<(), EngineObserverError> {
    if let Some(observer) = observer {
        observer.on_task_succeeded(id, task, success).await?;
    }
    Ok(())
}

async fn notify_stage_failed(
    observer: Option<&Arc<dyn EngineObserver>>,
    id: &EngineTaskId,
    task: &EngineTask,
    error: &str,
) {
    if let Some(observer) = observer {
        observer.on_task_failed(id, task, error).await;
    }
}

struct EngineProgressObserver {
    observer: Arc<dyn EngineObserver>,
    task_id: EngineTaskId,
    task: EngineTask,
}

enum LeaseInterruption {
    Cancelled,
    Lost,
}

#[async_trait]
impl ProverProgressObserver for EngineProgressObserver {
    async fn on_progress(&self, progress: &ProverProgress) {
        self.observer
            .on_task_progress(&self.task_id, &self.task, progress)
            .await;
    }

    async fn load_sp1_network_request_id(&self) -> Option<String> {
        self.observer
            .load_sp1_network_request_id(&self.task_id, &self.task)
            .await
    }

    async fn load_boundless_submission(&self) -> Option<BoundlessSubmissionResume> {
        self.observer
            .load_boundless_submission(&self.task_id, &self.task)
            .await
    }
}

impl<S> Clone for Engine<S>
where
    S: PipelineSpec,
    S::Prover: Prover<S::Backend, GuestInput = S::GuestInput>,
    S::Backend: ProverBackend,
    S::Provider: Provider,
{
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<S> Engine<S>
where
    S: PipelineSpec,
    S::Prover: Prover<S::Backend, GuestInput = S::GuestInput>,
    S::Backend: ProverBackend,
    S::Provider: Provider,
{
    const fn default_scheduler_config() -> SchedulerConfig {
        SchedulerConfig {
            lease_duration: Duration::from_secs(60),
            retry: RetryPolicy::None,
        }
    }

    pub fn new(spec: S, context: ProofContext) -> Self {
        Self::with_store_and_scheduler_config(
            spec,
            context,
            MemoryStore::new(),
            Self::default_scheduler_config(),
        )
    }

    pub fn with_store<Store>(spec: S, context: ProofContext, store: Store) -> Self
    where
        Store: raiko2_queue::TaskStore<EngineTask, EngineOutput<S::GuestInput>, EngineTaskKey>
            + 'static,
    {
        Self::with_store_and_scheduler_config(
            spec,
            context,
            store,
            Self::default_scheduler_config(),
        )
    }

    pub fn with_store_and_scheduler_config<Store>(
        spec: S,
        context: ProofContext,
        store: Store,
        scheduler_config: SchedulerConfig,
    ) -> Self
    where
        Store: raiko2_queue::TaskStore<EngineTask, EngineOutput<S::GuestInput>, EngineTaskKey>
            + 'static,
    {
        Self::with_store_scheduler_config_and_observer(spec, context, store, scheduler_config, None)
    }

    pub fn with_store_scheduler_config_and_observer<Store>(
        spec: S,
        context: ProofContext,
        store: Store,
        scheduler_config: SchedulerConfig,
        observer: Option<Arc<dyn EngineObserver>>,
    ) -> Self
    where
        Store: raiko2_queue::TaskStore<EngineTask, EngineOutput<S::GuestInput>, EngineTaskKey>
            + 'static,
    {
        Self {
            inner: Arc::new(Inner {
                spec,
                scheduler: Scheduler::with_config(store, scheduler_config),
                context,
                observer,
            }),
        }
    }

    fn context_for_proposal(&self, request: &ProposalTaskRequest) -> ProofContext {
        let mut ctx = self.inner.context.clone();
        ctx.request.proposal_id = request.proposal_id;
        ctx.request.l2_block_range = request.l2_block_range;
        ctx.request.shasta = Some(ShastaRequest {
            l1_inclusion_block_number: request.l1_inclusion_block_number,
            last_anchor_block_number: request.last_anchor_block_number,
            checkpoint: request.checkpoint,
        });
        ctx.request
            .blob_proof_type
            .clone_from(&request.blob_proof_type);
        ctx.request.prover.clone_from(&request.prover);
        ctx.request.graffiti.clone_from(&request.graffiti);
        Self::apply_prover_config(&mut ctx.config, &request.prover_config);
        ctx
    }

    fn context_for_aggregation(&self, request: &AggregationTaskRequest) -> ProofContext {
        let mut ctx = self.inner.context.clone();
        Self::apply_prover_config(&mut ctx.config, &request.prover_config);
        ctx
    }

    fn apply_prover_config(config: &mut serde_json::Value, request: &ProverTaskConfig) {
        if !config.is_object() {
            *config = serde_json::json!({});
        }

        let Some(config) = config.as_object_mut() else {
            return;
        };
        if let Some(sp1) = &request.sp1
            && let Ok(value) = serde_json::to_value(sp1)
        {
            config.insert("sp1".to_string(), value);
        }
        if let Some(sp1_system) = &request.sp1_system
            && let Ok(value) = serde_json::to_value(sp1_system)
        {
            config.insert("sp1_system".to_string(), value);
        }
    }

    fn proposal_task_id(&self, request: ProposalTaskRequest) -> EngineTaskId {
        EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: self.inner.spec.pipeline_key(),
            request,
        })
    }

    fn aggregate_task_id(&self, request: AggregationTaskRequest) -> EngineTaskId {
        EngineTaskId::new(EngineTaskKey::Aggregate {
            pipeline: self.inner.spec.pipeline_key(),
            request,
        })
    }

    fn externally_stateful_stage_execution_policy(&self) -> TaskExecutionPolicy {
        let config = self.inner.scheduler.config();
        TaskExecutionPolicy {
            lease_duration: config.lease_duration,
            retry: config.retry.clone(),
        }
    }

    /// # Errors
    ///
    /// Returns an error if the task store cannot enqueue required tasks.
    pub async fn submit_proposal_proof(
        &self,
        request: ProposalTaskRequest,
    ) -> Result<EngineTaskId, TaskStoreError> {
        self.submit_proposal_proof_with_dependencies(request, Vec::new())
            .await
    }

    /// # Errors
    ///
    /// Returns an error if the task store cannot enqueue required tasks.
    pub async fn submit_proposal_proof_with_dependencies(
        &self,
        request: ProposalTaskRequest,
        dependencies: Vec<EngineTaskId>,
    ) -> Result<EngineTaskId, TaskStoreError> {
        let proposal_id = self.proposal_task_id(request.clone());
        self.inner
            .scheduler
            .submit_with_execution_policy(
                proposal_id,
                NewTask {
                    priority: PROPOSAL_TASK_PRIORITY,
                    payload: EngineTask::Proposal { request },
                },
                dependencies,
                self.externally_stateful_stage_execution_policy(),
            )
            .await
    }

    /// # Errors
    ///
    /// Returns an error if the task store cannot enqueue the aggregation task or if any proof
    /// task input does not point to a proposal prove stage in this pipeline.
    pub async fn submit_aggregation_proof_from_inputs(
        &self,
        request: AggregationTaskRequest,
        inputs: Vec<AggregateProofInput>,
    ) -> Result<EngineTaskId, TaskStoreError> {
        if inputs.is_empty() {
            return Err(TaskStoreError::corrupt_msg(
                "aggregation requires at least 1 proof input",
            ));
        }

        let mut proof_tasks = Vec::new();
        for input in &inputs {
            let proof_task = match input {
                AggregateProofInput::PendingProofArtifact {
                    dependency: proof_task,
                    ..
                } => proof_task,
                AggregateProofInput::ProofArtifact(_) => continue,
            };
            match &proof_task.0 {
                EngineTaskKey::Proposal { pipeline, .. }
                    if *pipeline == self.inner.spec.pipeline_key() =>
                {
                    proof_tasks.push((**proof_task).clone());
                }
                EngineTaskKey::Proposal { .. } => {
                    return Err(TaskStoreError::corrupt_msg(
                        "aggregation input must reference proposal tasks in this pipeline",
                    ));
                }
                EngineTaskKey::Aggregate { .. } => {
                    return Err(TaskStoreError::corrupt_msg(
                        "aggregation input cannot reference an aggregate task",
                    ));
                }
            }
        }

        let aggregate_id = self.aggregate_task_id(request.clone());
        self.inner
            .scheduler
            .submit_with_execution_policy(
                aggregate_id,
                NewTask {
                    priority: AGGREGATION_TASK_PRIORITY,
                    payload: EngineTask::Aggregate {
                        request,
                        source: AggregationSource::Inputs(inputs),
                    },
                },
                proof_tasks,
                self.externally_stateful_stage_execution_policy(),
            )
            .await
    }

    /// # Errors
    ///
    /// Returns an error if the task store cannot fetch the task.
    pub async fn get(
        &self,
        id: EngineTaskId,
    ) -> Result<Option<TaskView<EngineOutput<S::GuestInput>, EngineTaskKey>>, TaskStoreError> {
        self.inner.scheduler.get(id).await
    }

    /// # Errors
    ///
    /// error if task store cannot fetch task state.
    pub async fn get_task_state(
        &self,
        id: EngineTaskId,
    ) -> Result<Option<TaskViewState<EngineTaskKey>>, TaskStoreError> {
        self.inner.scheduler.get_state(id).await
    }

    /// # Errors
    ///
    /// Returns an error if task store cannot list tasks.
    pub async fn list_tasks(&self) -> Result<Vec<TaskViewState<EngineTaskKey>>, TaskStoreError> {
        self.inner.scheduler.list().await
    }

    /// # Errors
    ///
    /// Returns an error if the task store cannot cancel the task.
    pub async fn cancel(&self, id: EngineTaskId) -> Result<(), TaskStoreError> {
        self.inner.scheduler.cancel(id.clone()).await?;
        if let Some(observer) = &self.inner.observer {
            observer.on_task_cancelled(&id).await;
        }
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the task store cannot delete the task.
    pub async fn remove(&self, id: EngineTaskId) -> Result<(), TaskStoreError> {
        self.inner.scheduler.remove(id).await
    }

    /// # Errors
    ///
    /// Returns an error if the task store cannot lease or complete work.
    pub async fn run_one(&self, worker: &str) -> Result<bool, TaskStoreError> {
        let Some(lease) = self.inner.scheduler.next_ready(worker).await? else {
            return Ok(false);
        };

        let payload = lease.payload.clone();
        let renew_scheduler = self.inner.scheduler.clone();
        let renew_lease = lease.clone();
        let renew_period = lease
            .execution_policy
            .lease_duration
            .checked_div(2)
            .unwrap_or_else(|| Duration::from_secs(1))
            .max(Duration::from_secs(1));
        let renew_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(renew_period);
            interval.tick().await;
            loop {
                interval.tick().await;
                match renew_scheduler.renew_lease(&renew_lease).await {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(err) => {
                        tracing::warn!(
                            worker = %renew_lease.worker,
                            task = ?renew_lease.id,
                            error = %err,
                            "failed to renew task lease"
                        );
                        break;
                    }
                }
            }
        });

        if let Some(observer) = &self.inner.observer
            && !matches!(payload, EngineTask::Proposal { .. })
        {
            observer.on_task_started(&lease.id, &payload, worker).await;
        }

        let mut execution_result = match self.recover_completed_output(&lease.id, &payload).await {
            Ok(Some(output)) => Ok(output),
            Ok(None) => {
                self.execute_with_task_controls(&lease.id, &lease, payload.clone())
                    .await
            }
            Err(error) => Err(error),
        };
        if let Err(err) = &execution_result {
            tracing::warn!(worker = %worker, task = ?payload, error = %err, "engine task failed");
        }
        let should_notify_queue_task = !matches!(payload, EngineTask::Proposal { .. })
            || execution_result
                .as_ref()
                .err()
                .map(TaskExecutionError::message)
                == Some(task_cancelled_error().as_str())
            || execution_result
                .as_ref()
                .err()
                .map(TaskExecutionError::message)
                == Some(task_lease_lost_error().as_str());
        if should_notify_queue_task && let Ok(output) = &execution_result {
            let success = task_success_from_output(output);
            if let Err(error) =
                notify_stage_succeeded(self.inner.observer.as_ref(), &lease.id, &payload, &success)
                    .await
            {
                execution_result = Err(error.into());
            }
        }
        let permanent_failure = matches!(execution_result, Err(TaskExecutionError::Permanent(_)));
        let result = execution_result.map_err(|error| error.to_string());
        let success = result.as_ref().ok().map(task_success_from_output);
        let error = result.as_ref().err().cloned();
        let completed_id = lease.id.clone();
        let completed = if permanent_failure {
            self.inner
                .scheduler
                .complete_permanent_failure(lease, error.clone().unwrap_or_default())
                .await?
        } else {
            self.inner.scheduler.complete(lease, result).await?
        };
        renew_task.abort();
        if completed
            && should_notify_queue_task
            && let Some(observer) = &self.inner.observer
            && success.is_none()
            && !permanent_failure
            && let Some(error) = error.as_deref()
        {
            observer
                .on_task_failed(&completed_id, &payload, error)
                .await;
        }
        Ok(true)
    }

    pub fn start_workers(&self, concurrency: usize)
    where
        S: 'static,
        S::Prover: Prover<S::Backend, GuestInput = S::GuestInput> + 'static,
        S::Backend: ProverBackend + 'static,
        S::Provider: Provider + 'static,
    {
        let config = WorkerConfig {
            concurrency,
            ..WorkerConfig::default()
        };
        crate::worker::spawn_workers(self.clone(), &config);
    }

    pub fn start_workers_with_maintenance_interval(
        &self,
        concurrency: usize,
        maintenance_interval: Duration,
    ) where
        S: 'static,
        S::Prover: Prover<S::Backend, GuestInput = S::GuestInput> + 'static,
        S::Backend: ProverBackend + 'static,
        S::Provider: Provider + 'static,
    {
        let config = WorkerConfig {
            concurrency,
            maintenance_interval,
            ..WorkerConfig::default()
        };
        crate::worker::spawn_workers(self.clone(), &config);
    }

    async fn get_guest_input(
        &self,
        id: EngineTaskId,
        expected_stage: PipelineStage,
        task_name: &str,
    ) -> Result<S::GuestInput, String> {
        let view = self
            .get_view_or_err(id, || format!("missing {task_name} task"))
            .await?;

        match view.state {
            TaskState::Succeeded {
                output: EngineOutput::GuestInput(input),
            } => {
                if input.stage == expected_stage {
                    Ok(input.output)
                } else {
                    let err = match expected_stage {
                        PipelineStage::Preflight => {
                            "preflight task did not produce preflight output"
                        }
                        PipelineStage::Validation => {
                            "input task did not produce validated GuestInput"
                        }
                        _ => "input task did not produce expected stage output",
                    };
                    Err(err.to_string())
                }
            }
            TaskState::Succeeded { .. } => {
                Err(format!("{task_name} task did not produce GuestInput"))
            }
            _ => Err(format!("{task_name} task not completed")),
        }
    }

    async fn execute_with_task_controls(
        &self,
        task_id: &EngineTaskId,
        lease: &raiko2_queue::TaskLease<EngineTask, EngineTaskKey>,
        payload: EngineTask,
    ) -> Result<EngineOutput<S::GuestInput>, TaskExecutionError> {
        let execute = self.execute(task_id, payload, &lease.worker);
        let interrupted = self.wait_lease_interruption(&lease.id, &lease.worker, lease.attempt);
        tokio::pin!(execute);
        tokio::pin!(interrupted);

        tokio::select! {
            result = &mut execute => result,
            interruption = &mut interrupted => {
                match interruption {
                    Ok(LeaseInterruption::Cancelled) => Err(task_cancelled_error().into()),
                    Ok(LeaseInterruption::Lost) => Err(task_lease_lost_error().into()),
                    Err(err) => Err(err.to_string().into()),
                }
            }
        }
    }

    async fn recover_completed_output(
        &self,
        task_id: &EngineTaskId,
        task: &EngineTask,
    ) -> Result<Option<EngineOutput<S::GuestInput>>, TaskExecutionError> {
        let Some(observer) = &self.inner.observer else {
            return Ok(None);
        };
        let Some(proof) = observer
            .load_completed_proof(task_id, task)
            .await
            .map_err(TaskExecutionError::from)?
        else {
            return Ok(None);
        };
        let stage = match task {
            EngineTask::Aggregate { .. } => PipelineStage::Aggregate,
            _ => PipelineStage::Prove,
        };
        Ok(Some(EngineOutput::Proof(Box::new(
            PipelineStageResult::new(stage, proof),
        ))))
    }

    async fn wait_lease_interruption(
        &self,
        id: &EngineTaskId,
        worker: &str,
        attempt: u32,
    ) -> Result<LeaseInterruption, TaskStoreError> {
        let notifier = self.inner.scheduler.notifier();
        loop {
            let Some(view) = self.inner.scheduler.get(id.clone()).await? else {
                return Ok(LeaseInterruption::Lost);
            };

            match view.state {
                TaskState::Running {
                    worker: current_worker,
                    attempt: current_attempt,
                } if current_worker == worker && current_attempt == attempt => {}
                TaskState::Cancelled => return Ok(LeaseInterruption::Cancelled),
                _ => return Ok(LeaseInterruption::Lost),
            }

            notifier.notified().await;
        }
    }

    async fn get_view_or_err(
        &self,
        id: EngineTaskId,
        missing_msg: impl FnOnce() -> String,
    ) -> Result<TaskView<EngineOutput<S::GuestInput>, EngineTaskKey>, String> {
        self.inner
            .scheduler
            .get(id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(missing_msg)
    }

    async fn get_encoded_input(&self, id: EngineTaskId) -> Result<EncodedGuestInput, String> {
        let view = self
            .get_view_or_err(id, || "missing input task".to_string())
            .await?;

        match view.state {
            TaskState::Succeeded {
                output: EngineOutput::EncodedInput(input),
            } => {
                if input.stage == PipelineStage::Encode {
                    Ok(input.output)
                } else {
                    Err("input task did not produce encoded GuestInput".to_string())
                }
            }
            TaskState::Succeeded { .. } => {
                Err("input task did not produce encoded input".to_string())
            }
            _ => Err("input task not completed".to_string()),
        }
    }

    async fn get_proof_artifact(
        &self,
        artifact: ProofArtifactRef,
    ) -> Result<raiko2_primitives::Proof, String> {
        let observer = self
            .inner
            .observer
            .as_ref()
            .ok_or_else(|| "proof artifact resolver is not configured".to_string())?;
        observer
            .load_proof_artifact(&artifact)
            .await?
            .ok_or_else(|| format!("proof artifact {} is missing", artifact.proof_ref))
    }

    async fn resolve_aggregation_source(
        &self,
        source: AggregationSource,
    ) -> Result<Vec<raiko2_primitives::Proof>, String> {
        match source {
            AggregationSource::Inputs(inputs) => {
                let mut proofs = Vec::with_capacity(inputs.len());
                for input in inputs {
                    match input {
                        AggregateProofInput::ProofArtifact(artifact)
                        | AggregateProofInput::PendingProofArtifact { artifact, .. } => {
                            proofs.push(self.get_proof_artifact(artifact).await?);
                        }
                    }
                }
                Ok(proofs)
            }
        }
    }

    async fn execute_proposal_stage<T>(
        &self,
        task_id: &EngineTaskId,
        task: &EngineTask,
        worker: &str,
        stage: PipelineStage,
        execute: impl std::future::Future<Output = Result<PipelineStageResult<T>, String>>,
    ) -> Result<PipelineStageResult<T>, TaskExecutionError> {
        notify_stage_started(self.inner.observer.as_ref(), task_id, task, worker).await;
        match execute.await {
            Ok(output) => {
                let success = match stage {
                    PipelineStage::Encode => EngineTaskSuccess::EncodedInput { stage },
                    PipelineStage::Prove | PipelineStage::Aggregate => {
                        return Err("proof stages require proof output".to_string().into());
                    }
                    PipelineStage::Preflight | PipelineStage::Validation => {
                        EngineTaskSuccess::GuestInput { stage }
                    }
                };
                notify_stage_succeeded(self.inner.observer.as_ref(), task_id, task, &success)
                    .await
                    .map_err(TaskExecutionError::from)?;
                Ok(output)
            }
            Err(error) => {
                notify_stage_failed(self.inner.observer.as_ref(), task_id, task, &error).await;
                Err(error.into())
            }
        }
    }

    async fn execute_proposal(
        &self,
        task_id: &EngineTaskId,
        request: ProposalTaskRequest,
        worker: &str,
    ) -> Result<EngineOutput<S::GuestInput>, TaskExecutionError> {
        let ctx = self.context_for_proposal(&request);
        let pipeline = Pipeline::new(&self.inner.spec);

        let preflight_task = EngineTask::Preflight {
            request: request.clone(),
        };
        let preflight = self
            .execute_proposal_stage(
                task_id,
                &preflight_task,
                worker,
                PipelineStage::Preflight,
                async { pipeline.preflight(&ctx).await.map_err(|e| e.to_string()) },
            )
            .await?;

        let validation_task = EngineTask::Validate {
            request: request.clone(),
            preflight_task: task_id.clone(),
        };
        let validated = self
            .execute_proposal_stage(
                task_id,
                &validation_task,
                worker,
                PipelineStage::Validation,
                async {
                    pipeline
                        .validate(&ctx, preflight.output)
                        .map_err(|e| e.to_string())
                },
            )
            .await?;

        let encode_task = EngineTask::Encode {
            request: request.clone(),
            input_task: task_id.clone(),
        };
        notify_stage_started(self.inner.observer.as_ref(), task_id, &encode_task, worker).await;
        let encoded = match self
            .inner
            .spec
            .prover()
            .encode(&validated.output, &ctx.config)
            .map(|output| PipelineStageResult::new(PipelineStage::Encode, output))
            .map_err(|e| e.to_string())
        {
            Ok(encoded) => {
                notify_stage_succeeded(
                    self.inner.observer.as_ref(),
                    task_id,
                    &encode_task,
                    &EngineTaskSuccess::EncodedInput {
                        stage: PipelineStage::Encode,
                    },
                )
                .await
                .map_err(TaskExecutionError::from)?;
                encoded
            }
            Err(error) => {
                notify_stage_failed(self.inner.observer.as_ref(), task_id, &encode_task, &error)
                    .await;
                return Err(error.into());
            }
        };

        self.prove_proposal_encoded(task_id, request, task_id.clone(), encoded.output, worker)
            .await
    }

    async fn prove_proposal(
        &self,
        task_id: &EngineTaskId,
        request: ProposalTaskRequest,
        input_task: EngineTaskId,
    ) -> Result<EngineOutput<S::GuestInput>, String> {
        let progress_task = EngineTask::ProveProposal {
            request: request.clone(),
            input_task: input_task.clone(),
        };
        // Keep dependency output until downstream completes.
        let encoded = self.get_encoded_input(input_task).await?;
        let ctx = self.context_for_proposal(&request);

        let proof = self
            .inner
            .spec
            .prover()
            .prove_encoded_with_observer(
                encoded,
                &ctx.config,
                self.inner.spec.backend(),
                self.inner.observer.as_ref().map(|observer| {
                    Arc::new(EngineProgressObserver {
                        observer: Arc::clone(observer),
                        task_id: task_id.clone(),
                        task: progress_task.clone(),
                    }) as Arc<dyn ProverProgressObserver>
                }),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(EngineOutput::Proof(Box::new(PipelineStageResult::new(
            PipelineStage::Prove,
            proof,
        ))))
    }

    async fn prove_proposal_encoded(
        &self,
        task_id: &EngineTaskId,
        request: ProposalTaskRequest,
        input_task: EngineTaskId,
        encoded: EncodedGuestInput,
        worker: &str,
    ) -> Result<EngineOutput<S::GuestInput>, TaskExecutionError> {
        let progress_task = EngineTask::ProveProposal {
            request: request.clone(),
            input_task,
        };
        notify_stage_started(
            self.inner.observer.as_ref(),
            task_id,
            &progress_task,
            worker,
        )
        .await;
        let ctx = self.context_for_proposal(&request);
        let proof = match self
            .inner
            .spec
            .prover()
            .prove_encoded_with_observer(
                encoded,
                &ctx.config,
                self.inner.spec.backend(),
                self.inner.observer.as_ref().map(|observer| {
                    Arc::new(EngineProgressObserver {
                        observer: Arc::clone(observer),
                        task_id: task_id.clone(),
                        task: progress_task.clone(),
                    }) as Arc<dyn ProverProgressObserver>
                }),
            )
            .await
            .map_err(|e| e.to_string())
        {
            Ok(proof) => proof,
            Err(error) => {
                notify_stage_failed(
                    self.inner.observer.as_ref(),
                    task_id,
                    &progress_task,
                    &error,
                )
                .await;
                return Err(error.into());
            }
        };
        notify_stage_succeeded(
            self.inner.observer.as_ref(),
            task_id,
            &progress_task,
            &EngineTaskSuccess::Proof {
                stage: PipelineStage::Prove,
                proof: proof.clone(),
            },
        )
        .await
        .map_err(TaskExecutionError::from)?;
        Ok(EngineOutput::Proof(Box::new(PipelineStageResult::new(
            PipelineStage::Prove,
            proof,
        ))))
    }

    async fn execute(
        &self,
        task_id: &EngineTaskId,
        task: EngineTask,
        worker: &str,
    ) -> Result<EngineOutput<S::GuestInput>, TaskExecutionError> {
        match task {
            EngineTask::Proposal { request } => {
                self.execute_proposal(task_id, request, worker).await
            }
            EngineTask::Preflight { request } => {
                let ctx = self.context_for_proposal(&request);
                let pipeline = Pipeline::new(&self.inner.spec);
                pipeline
                    .preflight(&ctx)
                    .await
                    .map(|input| EngineOutput::GuestInput(Box::new(input)))
                    .map_err(|e| TaskExecutionError::from(e.to_string()))
            }
            EngineTask::Validate {
                request,
                preflight_task,
            } => {
                let preflight_input = self
                    .get_guest_input(preflight_task, PipelineStage::Preflight, "preflight")
                    .await?;

                let ctx = self.context_for_proposal(&request);
                let pipeline = Pipeline::new(&self.inner.spec);
                let validated = pipeline
                    .validate(&ctx, preflight_input)
                    .map_err(|e| e.to_string())?;
                Ok(EngineOutput::GuestInput(Box::new(validated)))
            }
            EngineTask::Encode {
                request,
                input_task,
            } => {
                let guest_input = self
                    .get_guest_input(input_task, PipelineStage::Validation, "input")
                    .await?;

                let ctx = self.context_for_proposal(&request);
                let encoded = self
                    .inner
                    .spec
                    .prover()
                    .encode(&guest_input, &ctx.config)
                    .map_err(|e| e.to_string())?;

                Ok(EngineOutput::EncodedInput(Box::new(
                    PipelineStageResult::new(PipelineStage::Encode, encoded),
                )))
            }
            EngineTask::ProveProposal {
                request,
                input_task,
            } => self
                .prove_proposal(task_id, request, input_task)
                .await
                .map_err(TaskExecutionError::from),
            EngineTask::Aggregate { request, source } => {
                let ctx = self.context_for_aggregation(&request);
                let progress_task = EngineTask::Aggregate {
                    request: request.clone(),
                    source: source.clone(),
                };
                let proofs = self.resolve_aggregation_source(source).await?;
                let proof = self
                    .inner
                    .spec
                    .prover()
                    .aggregate_with_observer(
                        AggregationGuestInput { proofs },
                        &ctx.config,
                        self.inner.spec.backend(),
                        self.inner.observer.as_ref().map(|observer| {
                            Arc::new(EngineProgressObserver {
                                observer: Arc::clone(observer),
                                task_id: task_id.clone(),
                                task: progress_task.clone(),
                            }) as Arc<dyn ProverProgressObserver>
                        }),
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(EngineOutput::Proof(Box::new(PipelineStageResult::new(
                    PipelineStage::Aggregate,
                    proof,
                ))))
            }
        }
    }
}

fn task_success_from_output<I>(output: &EngineOutput<I>) -> EngineTaskSuccess {
    match output {
        EngineOutput::GuestInput(input) => EngineTaskSuccess::GuestInput { stage: input.stage },
        EngineOutput::EncodedInput(input) => EngineTaskSuccess::EncodedInput { stage: input.stage },
        EngineOutput::Proof(proof) => EngineTaskSuccess::Proof {
            stage: proof.stage,
            proof: proof.output.clone(),
        },
    }
}

fn task_cancelled_error() -> String {
    "task cancelled".to_string()
}

fn task_lease_lost_error() -> String {
    "task lease lost before completion".to_string()
}

#[async_trait::async_trait]
impl<S> crate::worker::Runnable for Engine<S>
where
    S: PipelineSpec + 'static,
    S::Prover: Prover<S::Backend, GuestInput = S::GuestInput> + 'static,
    S::Backend: ProverBackend + 'static,
    S::Provider: Provider + 'static,
{
    async fn run_one(&self, worker_id: &str) -> Result<bool, String> {
        Engine::run_one(self, worker_id)
            .await
            .map_err(|err| err.to_string())
    }

    async fn maintenance_tick(&self) -> Result<(), String> {
        self.inner
            .scheduler
            .maintenance_tick()
            .await
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    fn notifier(&self) -> Arc<tokio::sync::Notify> {
        self.inner.scheduler.notifier()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use alloy_primitives::Bytes;
    use raiko2_pipeline::{
        NoopManifestBuilder, NoopValidation, PipelineKey, PipelineSpec, Preflight, ProofStage,
        ProverBackend,
    };
    use raiko2_primitives::{
        AggregationGuestInput, Proof, ProofContext, ProofRequest, ProverConfig, RaikoError,
        RaikoResult,
    };
    use raiko2_primitives_shasta::GuestInput;
    use raiko2_prover::{GuestInputCodec, Prover};
    use raiko2_provider::Provider;
    use raiko2_queue::{RetryPolicy, SchedulerConfig, TaskState};

    use crate::tasks::{
        AggregateProofInput, AggregationTaskRequest, EngineOutput, ProofArtifactRef,
        ProposalTaskRequest, ProverTaskConfig,
    };
    use crate::{
        Engine, EngineObserver, EngineObserverError, EngineTask, EngineTaskId, EngineTaskKey,
        EngineTaskSuccess, PROPOSAL_TASK_PRIORITY,
    };

    #[derive(Default)]
    struct PublicationFailingObserver {
        proof_successes: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl EngineObserver for PublicationFailingObserver {
        async fn on_task_succeeded(
            &self,
            _id: &EngineTaskId,
            _task: &EngineTask,
            success: &EngineTaskSuccess,
        ) -> Result<(), EngineObserverError> {
            if matches!(success, EngineTaskSuccess::Proof { .. }) {
                self.proof_successes.fetch_add(1, Ordering::SeqCst);
                return Err(EngineObserverError::ProofPublication(
                    "injected failure".to_string(),
                ));
            }
            Ok(())
        }
    }

    struct MockProver;

    impl GuestInputCodec<GuestInput> for MockProver {
        fn encode(&self, input: &GuestInput, _config: &ProverConfig) -> RaikoResult<Bytes> {
            Ok(Bytes::from(input.taiko.proposal_id.to_le_bytes().to_vec()))
        }
    }

    #[async_trait::async_trait]
    impl Prover<TestBackend> for MockProver {
        type GuestInput = GuestInput;

        fn encode(&self, input: &Self::GuestInput, config: &ProverConfig) -> RaikoResult<Bytes> {
            GuestInputCodec::encode(self, input, config)
        }

        async fn prove_encoded(
            &self,
            input: Bytes,
            _config: &ProverConfig,
            _backend: &TestBackend,
        ) -> RaikoResult<Proof> {
            let raw = input.as_ref();
            if raw.len() != 8 {
                return Err(RaikoError::Guest(
                    "Encoded input missing proposal id".to_string(),
                ));
            }
            let mut buf = [0u8; 8];
            buf.copy_from_slice(raw);
            let proposal_id = u64::from_le_bytes(buf);
            assert_eq!(proposal_id, 1);
            Ok(Proof {
                proof: Some("mock-proof".to_string()),
                ..Default::default()
            })
        }

        async fn aggregate(
            &self,
            _input: AggregationGuestInput,
            _config: &ProverConfig,
            _backend: &TestBackend,
        ) -> RaikoResult<Proof> {
            Ok(Proof {
                proof: Some("mock-agg-proof".to_string()),
                ..Default::default()
            })
        }
    }

    struct FailingProver;

    impl GuestInputCodec<GuestInput> for FailingProver {
        fn encode(&self, input: &GuestInput, _config: &ProverConfig) -> RaikoResult<Bytes> {
            Ok(Bytes::from(input.taiko.proposal_id.to_le_bytes().to_vec()))
        }
    }

    #[async_trait::async_trait]
    impl Prover<TestBackend> for FailingProver {
        type GuestInput = GuestInput;

        fn encode(&self, input: &Self::GuestInput, config: &ProverConfig) -> RaikoResult<Bytes> {
            GuestInputCodec::encode(self, input, config)
        }

        async fn prove_encoded(
            &self,
            _input: Bytes,
            _config: &ProverConfig,
            _backend: &TestBackend,
        ) -> RaikoResult<Proof> {
            Err(RaikoError::Guest("boom".to_string()))
        }

        async fn aggregate(
            &self,
            _input: AggregationGuestInput,
            _config: &ProverConfig,
            _backend: &TestBackend,
        ) -> RaikoResult<Proof> {
            Ok(Proof::default())
        }
    }

    struct SlowProver;

    impl GuestInputCodec<GuestInput> for SlowProver {
        fn encode(&self, input: &GuestInput, _config: &ProverConfig) -> RaikoResult<Bytes> {
            Ok(Bytes::from(input.taiko.proposal_id.to_le_bytes().to_vec()))
        }
    }

    #[async_trait::async_trait]
    impl Prover<TestBackend> for SlowProver {
        type GuestInput = GuestInput;

        fn encode(&self, input: &Self::GuestInput, config: &ProverConfig) -> RaikoResult<Bytes> {
            GuestInputCodec::encode(self, input, config)
        }

        async fn prove_encoded(
            &self,
            _input: Bytes,
            _config: &ProverConfig,
            _backend: &TestBackend,
        ) -> RaikoResult<Proof> {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok(Proof::default())
        }

        async fn aggregate(
            &self,
            _input: AggregationGuestInput,
            _config: &ProverConfig,
            _backend: &TestBackend,
        ) -> RaikoResult<Proof> {
            Ok(Proof::default())
        }
    }

    struct TestBackend;

    impl ProverBackend for TestBackend {
        fn elf(&self, _stage: ProofStage) -> RaikoResult<&[u8]> {
            Ok(&[])
        }
    }

    struct TestSpec<Pv> {
        prover: Pv,
        backend: TestBackend,
        provider: MockProvider,
        pipeline_key: PipelineKey,
    }

    impl<Pv> TestSpec<Pv> {
        fn new(prover: Pv) -> Self {
            Self {
                prover,
                backend: TestBackend,
                provider: MockProvider,
                pipeline_key: PipelineKey::ShastaNative,
            }
        }

        const fn with_pipeline_key(mut self, pipeline_key: PipelineKey) -> Self {
            self.pipeline_key = pipeline_key;
            self
        }
    }
    const NOOP_VALIDATION: NoopValidation<GuestInput> = NoopValidation::new();
    const NOOP_MANIFEST: NoopManifestBuilder = NoopManifestBuilder;

    #[async_trait::async_trait]
    impl<Pv> Preflight for TestSpec<Pv>
    where
        Pv: Send + Sync,
    {
        type Input = GuestInput;

        async fn preflight<P: Provider>(
            &self,
            ctx: &ProofContext,
            _provider: &P,
        ) -> RaikoResult<GuestInput> {
            let mut input = GuestInput::default();
            input.taiko.proposal_id = ctx.request.proposal_id;
            Ok(input)
        }
    }

    impl<Pv> PipelineSpec for TestSpec<Pv>
    where
        Pv: Send + Sync,
    {
        type GuestInput = GuestInput;
        type Preflight = Self;
        type Validation = NoopValidation<GuestInput>;
        type ManifestBuilder = NoopManifestBuilder;
        type Prover = Pv;
        type Backend = TestBackend;
        type Provider = MockProvider;

        fn pipeline_key(&self) -> PipelineKey {
            self.pipeline_key
        }

        fn prover(&self) -> &Self::Prover {
            &self.prover
        }

        fn backend(&self) -> &Self::Backend {
            &self.backend
        }

        fn provider(&self) -> &Self::Provider {
            &self.provider
        }

        fn preflight(&self) -> &Self::Preflight {
            self
        }

        fn validation(&self) -> &Self::Validation {
            &NOOP_VALIDATION
        }

        fn manifest_builder(&self) -> &Self::ManifestBuilder {
            &NOOP_MANIFEST
        }
    }

    struct MockProvider;

    #[async_trait::async_trait]
    impl Provider for MockProvider {
        async fn batch_blocks(
            &self,
            _blocks: &[u64],
        ) -> RaikoResult<Vec<reth_ethereum_primitives::Block>> {
            Ok(vec![])
        }

        async fn batch_accounts(
            &self,
            _blocks: &[u64],
            _accounts: &[Vec<alloy_primitives::Address>],
        ) -> RaikoResult<Vec<alloy_primitives::map::AddressMap<alloy_trie::TrieAccount>>> {
            Ok(vec![])
        }

        async fn batch_witnesses(
            &self,
            _blocks: &[u64],
        ) -> RaikoResult<Vec<raiko2_primitives::ExecutionWitness>> {
            Ok(vec![])
        }

        async fn batch_l1_headers(
            &self,
            _blocks: &[u64],
        ) -> RaikoResult<Vec<alloy_consensus::Header>> {
            Ok(vec![])
        }
    }

    fn test_context() -> ProofContext {
        ProofContext::new(ProofRequest::default(), ProverConfig::default())
    }

    fn proposal_request(proposal_id: u64) -> ProposalTaskRequest {
        ProposalTaskRequest {
            proposal_id,
            l2_block_range: None,
            l1_inclusion_block_number: 0,
            last_anchor_block_number: 0,
            checkpoint: None,
            blob_proof_type: None,
            prover: None,
            graffiti: None,
            prover_config: ProverTaskConfig::default(),
        }
    }

    fn aggregation_request(request_id: &str) -> AggregationTaskRequest {
        AggregationTaskRequest {
            request_id: request_id.to_string(),
            proposal_ids: vec![1, 2],
            prover_config: ProverTaskConfig::default(),
        }
    }

    fn proof_artifact(proof_ref: &str) -> ProofArtifactRef {
        ProofArtifactRef {
            network_pair: "taiko_dev/ethereum".to_string(),
            pipeline_key: PipelineKey::ShastaNative,
            proof_ref: proof_ref.to_string(),
        }
    }

    fn pending_proof_input(proof_ref: &str, dependency: EngineTaskId) -> AggregateProofInput {
        AggregateProofInput::PendingProofArtifact {
            artifact: proof_artifact(proof_ref),
            dependency: Box::new(dependency),
        }
    }

    fn boundless_test_engine(scheduler_config: SchedulerConfig) -> Engine<TestSpec<MockProver>> {
        Engine::with_store_and_scheduler_config(
            TestSpec::new(MockProver).with_pipeline_key(PipelineKey::ShastaRisc0Network),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            scheduler_config,
        )
    }

    #[tokio::test]
    async fn proposal_is_enqueued_as_one_task() -> Result<(), Box<dyn std::error::Error>> {
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec::new(MockProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            Engine::<TestSpec<MockProver>>::default_scheduler_config(),
        );
        let request = proposal_request(1);

        engine.submit_proposal_proof(request.clone()).await?;

        let task_id = engine.proposal_task_id(request);
        let view = engine
            .get(task_id)
            .await?
            .ok_or_else(|| std::io::Error::other("expected proposal task view"))?;
        assert_eq!(view.priority, PROPOSAL_TASK_PRIORITY);

        Ok(())
    }

    #[tokio::test]
    async fn submit_proposal_proof_runs_dependency_pipeline()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec::new(MockProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            Engine::<TestSpec<MockProver>>::default_scheduler_config(),
        );

        let job_id = engine.submit_proposal_proof(proposal_request(1)).await?;

        assert!(Box::pin(engine.run_one("w1")).await?);

        let view = engine
            .get(job_id)
            .await?
            .ok_or_else(|| std::io::Error::other("expected task view"))?;
        match view.state {
            TaskState::Succeeded {
                output: EngineOutput::Proof(proof),
            } => {
                assert_eq!(proof.output.proof.as_deref(), Some("mock-proof"));
            }
            other => {
                return Err(
                    std::io::Error::other(format!("unexpected task state: {other:?}")).into(),
                );
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn submit_aggregation_proof_enqueues_aggregate_task()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec::new(MockProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            Engine::<TestSpec<MockProver>>::default_scheduler_config(),
        );

        let first = engine.proposal_task_id(proposal_request(1));
        let second = engine.proposal_task_id(proposal_request(2));
        let request = aggregation_request("agg-1");
        let aggregate_id = engine
            .submit_aggregation_proof_from_inputs(
                request.clone(),
                vec![
                    pending_proof_input("proposal-1", first.clone()),
                    pending_proof_input("proposal-2", second.clone()),
                ],
            )
            .await?;

        let view = engine
            .get(aggregate_id.clone())
            .await?
            .ok_or_else(|| std::io::Error::other("expected aggregate task view"))?;
        assert!(matches!(view.state, TaskState::Pending { .. }));
        assert_eq!(
            aggregate_id,
            EngineTaskId::new(EngineTaskKey::Aggregate {
                pipeline: raiko2_pipeline::PipelineKey::ShastaNative,
                request,
            })
        );
        Ok(())
    }

    #[tokio::test]
    async fn submit_aggregation_proof_accepts_single_proposal_task()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec::new(MockProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            Engine::<TestSpec<MockProver>>::default_scheduler_config(),
        );

        let proof_task = engine.proposal_task_id(proposal_request(1));
        let request = AggregationTaskRequest {
            request_id: "agg-single".to_string(),
            proposal_ids: vec![1],
            prover_config: ProverTaskConfig::default(),
        };
        let aggregate_id = engine
            .submit_aggregation_proof_from_inputs(
                request.clone(),
                vec![pending_proof_input("proposal-1", proof_task.clone())],
            )
            .await?;

        let view = engine
            .get(aggregate_id.clone())
            .await?
            .ok_or_else(|| std::io::Error::other("expected aggregate task view"))?;
        assert!(matches!(view.state, TaskState::Pending { .. }));
        assert_eq!(
            aggregate_id,
            EngineTaskId::new(EngineTaskKey::Aggregate {
                pipeline: raiko2_pipeline::PipelineKey::ShastaNative,
                request,
            })
        );
        Ok(())
    }

    #[tokio::test]
    async fn submit_aggregation_proof_rejects_empty_input() -> Result<(), Box<dyn std::error::Error>>
    {
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec::new(MockProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            Engine::<TestSpec<MockProver>>::default_scheduler_config(),
        );

        let err = engine
            .submit_aggregation_proof_from_inputs(aggregation_request("agg-empty"), Vec::new())
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("aggregation requires at least 1 proof input")
        );
        Ok(())
    }

    #[tokio::test]
    async fn submit_aggregation_proof_rejects_wrong_pipeline_proposal_task()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec::new(MockProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            Engine::<TestSpec<MockProver>>::default_scheduler_config(),
        );

        let other_pipeline_task = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: raiko2_pipeline::PipelineKey::ShastaSp1,
            request: proposal_request(1),
        });
        let err = engine
            .submit_aggregation_proof_from_inputs(
                aggregation_request("agg-wrong-pipeline"),
                vec![pending_proof_input("proposal-1", other_pipeline_task)],
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("proposal tasks in this pipeline"));
        Ok(())
    }

    #[tokio::test]
    async fn submit_proposal_proof_with_dependencies_delays_next_proposal()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec::new(MockProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            Engine::<TestSpec<MockProver>>::default_scheduler_config(),
        );

        let first_prove = engine.submit_proposal_proof(proposal_request(1)).await?;
        let second_request = proposal_request(2);
        let second_proposal = engine.proposal_task_id(second_request.clone());
        engine
            .submit_proposal_proof_with_dependencies(second_request, vec![first_prove])
            .await?;

        let ready = engine
            .inner
            .scheduler
            .next_ready("w1")
            .await?
            .ok_or_else(|| std::io::Error::other("expected ready task"))?;
        assert_eq!(ready.id, engine.proposal_task_id(proposal_request(1)));

        let second_view = engine
            .get(second_proposal)
            .await?
            .ok_or_else(|| std::io::Error::other("expected second task view"))?;
        assert!(matches!(second_view.state, TaskState::Pending { .. }));
        Ok(())
    }

    #[tokio::test]
    async fn retry_policy_none_fails_task_immediately() -> Result<(), Box<dyn std::error::Error>> {
        let scheduler_config = SchedulerConfig {
            lease_duration: Duration::from_secs(60),
            retry: RetryPolicy::None,
        };
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec::new(FailingProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            scheduler_config,
        );

        let job_id = engine.submit_proposal_proof(proposal_request(1)).await?;

        assert!(Box::pin(engine.run_one("w1")).await?);

        let view = engine
            .get(job_id)
            .await?
            .ok_or_else(|| std::io::Error::other("expected task view"))?;
        assert!(matches!(view.state, TaskState::Failed { .. }));
        Ok(())
    }

    #[tokio::test]
    async fn submitted_proposal_and_aggregate_tasks_use_scheduler_retry()
    -> Result<(), Box<dyn std::error::Error>> {
        let scheduler_config = SchedulerConfig {
            lease_duration: Duration::from_secs(45),
            retry: RetryPolicy::Fixed {
                max_attempts: 4,
                delay: Duration::from_millis(25),
            },
        };
        let engine = boundless_test_engine(scheduler_config.clone());
        let task_policy = raiko2_queue::TaskExecutionPolicy {
            lease_duration: scheduler_config.lease_duration,
            retry: scheduler_config.retry.clone(),
        };
        let request = proposal_request(9);
        let proposal_id = engine.submit_proposal_proof(request.clone()).await?;

        let proposal = engine
            .inner
            .scheduler
            .next_ready("w1")
            .await?
            .ok_or_else(|| std::io::Error::other("expected proposal lease"))?;
        assert_eq!(proposal.id, proposal_id);
        assert_eq!(proposal.execution_policy, task_policy);

        let aggregate_id = engine
            .submit_aggregation_proof_from_inputs(
                aggregation_request("agg"),
                vec![AggregateProofInput::ProofArtifact(proof_artifact(
                    "aggregate-input",
                ))],
            )
            .await?;
        let aggregate = engine
            .inner
            .scheduler
            .next_ready("w1")
            .await?
            .ok_or_else(|| std::io::Error::other("expected aggregate lease"))?;
        assert_eq!(aggregate.id, aggregate_id);
        assert_eq!(aggregate.execution_policy, task_policy);
        Ok(())
    }

    #[tokio::test]
    async fn failed_prove_stage_uses_scheduler_retry_policy()
    -> Result<(), Box<dyn std::error::Error>> {
        let scheduler_config = SchedulerConfig {
            lease_duration: Duration::from_secs(60),
            retry: RetryPolicy::Fixed {
                max_attempts: 4,
                delay: Duration::from_millis(1),
            },
        };
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec::new(FailingProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            scheduler_config,
        );

        let job_id = engine.submit_proposal_proof(proposal_request(1)).await?;

        assert!(Box::pin(engine.run_one("w1")).await?);

        let view = engine
            .get(job_id)
            .await?
            .ok_or_else(|| std::io::Error::other("expected task view"))?;
        assert!(matches!(view.state, TaskState::Retrying { attempt: 1, .. }));
        Ok(())
    }

    #[tokio::test]
    async fn proof_publication_failure_is_permanent_and_does_not_rerun_proving()
    -> Result<(), Box<dyn std::error::Error>> {
        let observer = Arc::new(PublicationFailingObserver::default());
        let engine = Engine::with_store_scheduler_config_and_observer(
            TestSpec::new(MockProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            SchedulerConfig {
                lease_duration: Duration::from_secs(60),
                retry: RetryPolicy::Fixed {
                    max_attempts: 4,
                    delay: Duration::from_millis(1),
                },
            },
            Some(observer.clone()),
        );
        let job_id = engine.submit_proposal_proof(proposal_request(1)).await?;

        assert!(Box::pin(engine.run_one("w1")).await?);

        let view = engine
            .get(job_id)
            .await?
            .ok_or_else(|| std::io::Error::other("expected task view"))?;
        assert!(matches!(view.state, TaskState::Failed { .. }));
        assert_eq!(observer.proof_successes.load(Ordering::SeqCst), 1);
        assert!(!Box::pin(engine.run_one("w2")).await?);
        Ok(())
    }

    #[tokio::test]
    async fn run_one_stops_running_task_after_cancel() -> Result<(), Box<dyn std::error::Error>> {
        let scheduler_config = SchedulerConfig {
            lease_duration: Duration::from_secs(60),
            retry: RetryPolicy::None,
        };
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec::new(SlowProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            scheduler_config,
        );
        let job_id = engine.submit_proposal_proof(proposal_request(1)).await?;

        let worker_engine = engine.clone();
        let handle = tokio::spawn(async move { Box::pin(worker_engine.run_one("w1")).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        engine.cancel(job_id.clone()).await?;

        let result = tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .map_err(|_| std::io::Error::other("run_one did not stop after cancel"))??;
        assert!(result?);

        let view = engine
            .get(job_id)
            .await?
            .ok_or_else(|| std::io::Error::other("expected task view"))?;
        assert!(matches!(view.state, TaskState::Cancelled));
        Ok(())
    }

    #[tokio::test]
    async fn validate_requires_preflight_output_stage() -> Result<(), Box<dyn std::error::Error>> {
        use crate::tasks::{EngineTask, EngineTaskId, EngineTaskKey};
        use raiko2_pipeline::{PipelineStage, PipelineStageResult};
        use raiko2_queue::NewTask;

        let engine = Engine::with_store_and_scheduler_config(
            TestSpec::new(MockProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            Engine::<TestSpec<MockProver>>::default_scheduler_config(),
        );

        let proposal_id = 1;
        let request = proposal_request(proposal_id);
        let preflight_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: raiko2_pipeline::PipelineKey::ShastaNative,
            request: request.clone(),
        });

        // Manually submit and complete a preflight task with WRONG stage output (e.g., Validation)
        engine
            .inner
            .scheduler
            .submit(
                preflight_id.clone(),
                NewTask {
                    priority: PROPOSAL_TASK_PRIORITY,
                    payload: EngineTask::Preflight {
                        request: request.clone(),
                    },
                },
                vec![],
            )
            .await?;

        let lease = engine
            .inner
            .scheduler
            .next_ready("w1")
            .await?
            .ok_or_else(|| std::io::Error::other("expected ready lease"))?;

        // Complete it with PipelineStage::Validation instead of PipelineStage::Preflight
        let wrong_output = EngineOutput::GuestInput(Box::new(PipelineStageResult::new(
            PipelineStage::Validation,
            GuestInput::default(),
        )));

        engine
            .inner
            .scheduler
            .complete(lease, Ok(wrong_output))
            .await?;

        // Run the validation task directly via engine.execute to check error
        let result = engine
            .execute(
                &EngineTaskId::new(EngineTaskKey::Proposal {
                    pipeline: raiko2_pipeline::PipelineKey::ShastaNative,
                    request: request.clone(),
                }),
                EngineTask::Validate {
                    request,
                    preflight_task: preflight_id,
                },
                "w1",
            )
            .await;

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "preflight task did not produce preflight output"
        );
        Ok(())
    }

    #[tokio::test]
    async fn encode_requires_validated_guest_input() -> Result<(), Box<dyn std::error::Error>> {
        use crate::tasks::{EngineTask, EngineTaskId, EngineTaskKey};
        use raiko2_pipeline::{PipelineStage, PipelineStageResult};
        use raiko2_queue::NewTask;

        let engine = Engine::with_store_and_scheduler_config(
            TestSpec::new(MockProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            Engine::<TestSpec<MockProver>>::default_scheduler_config(),
        );

        let proposal_id = 1;
        let request = proposal_request(proposal_id);
        let validation_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: raiko2_pipeline::PipelineKey::ShastaNative,
            request: request.clone(),
        });

        // Manually submit and complete a validation task with WRONG stage output (e.g., Preflight)
        engine
            .inner
            .scheduler
            .submit(
                validation_id.clone(),
                NewTask {
                    priority: PROPOSAL_TASK_PRIORITY,
                    payload: EngineTask::Validate {
                        request: request.clone(),
                        preflight_task: EngineTaskId::new(EngineTaskKey::Proposal {
                            pipeline: raiko2_pipeline::PipelineKey::ShastaNative,
                            request: request.clone(),
                        }),
                    },
                },
                vec![],
            )
            .await?;

        let lease = engine
            .inner
            .scheduler
            .next_ready("w1")
            .await?
            .ok_or_else(|| std::io::Error::other("expected ready lease"))?;

        // Complete it with PipelineStage::Preflight instead of PipelineStage::Validation
        let wrong_output = EngineOutput::GuestInput(Box::new(PipelineStageResult::new(
            PipelineStage::Preflight,
            GuestInput::default(),
        )));

        engine
            .inner
            .scheduler
            .complete(lease, Ok(wrong_output))
            .await?;

        // Run the encode task directly via engine.execute to check error
        let result = engine
            .execute(
                &EngineTaskId::new(EngineTaskKey::Proposal {
                    pipeline: raiko2_pipeline::PipelineKey::ShastaNative,
                    request: request.clone(),
                }),
                EngineTask::Encode {
                    request,
                    input_task: validation_id,
                },
                "w1",
            )
            .await;

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "input task did not produce validated GuestInput"
        );
        Ok(())
    }
}
