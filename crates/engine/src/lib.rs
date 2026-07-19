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
    EngineAggregationPlan, EngineExecutionPlan, EngineTaskId, EngineTaskKey, ProofArtifactRef,
    ProposalStage, ProposalTaskRequest, ProverTaskConfig,
};

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::worker::WorkerConfig;
use async_trait::async_trait;
use raiko2_pipeline::{Pipeline, PipelineSpec, PipelineStage, PipelineStageResult, ProverBackend};
use raiko2_primitives::{AggregationGuestInput, Proof, ProofContext, ShastaRequest};
use raiko2_prover::{
    NetworkProverBackend, PendingProofCheckpoint, Prover, ProverProgress, ProverProgressObserver,
    SubmissionCheckpointPermit,
};
use raiko2_provider::Provider;
use raiko2_queue::{
    AttachOutcome, DetachMode, DetachOutcome, ExecutionGraph, ExecutionNode, MemoryStore, NewTask,
    Priority, ProjectionView, RetryPolicy, RootOwner, Scheduler, SchedulerConfig,
    TaskCompletionDisposition, TaskExecutionPolicy, TaskLease, TaskState, TaskStoreError, TaskView,
    TaskViewState,
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
    last_maintenance_success_ms: AtomicU64,
    worker_groups: Mutex<Vec<Arc<crate::worker::WorkerGroup>>>,
}

#[derive(Clone, Debug)]
pub enum EngineTaskSuccess {
    GuestInput { stage: PipelineStage },
    EncodedInput { stage: PipelineStage },
    Proof { stage: PipelineStage, proof: Proof },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineObserverError {
    RuntimeSync(String),
    RuntimeInactive(String),
    ProgressRejected(String),
    ProofPublication(String),
    ProofInvalidated(String),
}

impl std::fmt::Display for EngineObserverError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RuntimeSync(error)
            | Self::RuntimeInactive(error)
            | Self::ProgressRejected(error)
            | Self::ProofPublication(error)
            | Self::ProofInvalidated(error) => formatter.write_str(error),
        }
    }
}

impl std::error::Error for EngineObserverError {}

#[derive(Debug)]
enum TaskExecutionError {
    Retryable(String),
    Coordination(String),
    Stage {
        error: String,
        observer_task: Box<EngineTask>,
    },
    ProofPublication {
        error: String,
        proof: Box<Proof>,
    },
    ProofInvalidated(String),
}

impl TaskExecutionError {
    fn message(&self) -> &str {
        match self {
            Self::Retryable(error)
            | Self::Coordination(error)
            | Self::Stage { error, .. }
            | Self::ProofPublication { error, .. }
            | Self::ProofInvalidated(error) => error,
        }
    }

    fn observer_task(&self) -> Option<&EngineTask> {
        match self {
            Self::Stage { observer_task, .. } => Some(observer_task),
            _ => None,
        }
    }

    fn stage(error: String, observer_task: EngineTask) -> Self {
        Self::Stage {
            error,
            observer_task: Box::new(observer_task),
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
            EngineObserverError::RuntimeSync(error)
            | EngineObserverError::RuntimeInactive(error)
            | EngineObserverError::ProgressRejected(error)
            | EngineObserverError::ProofPublication(error) => Self::Coordination(error),
            EngineObserverError::ProofInvalidated(error) => Self::ProofInvalidated(error),
        }
    }
}

const fn durability_retry_policy() -> RetryPolicy {
    RetryPolicy::Exponential {
        max_attempts: u32::MAX,
        base_delay: Duration::from_secs(1),
        max_delay: Duration::from_secs(5 * 60),
    }
}

fn apply_execution_failure_policy(
    lease: &mut TaskLease<EngineTask, EngineTaskKey>,
    payload: &EngineTask,
    execution_result: &Result<EngineOutput<impl Sized>, TaskExecutionError>,
) {
    match execution_result {
        Err(TaskExecutionError::ProofPublication { proof, .. }) => {
            lease.payload = payload.clone().with_pending_publication((**proof).clone());
            lease.execution_policy.retry = durability_retry_policy();
        }
        Err(TaskExecutionError::Coordination(_)) => {
            lease.execution_policy.retry = durability_retry_policy();
        }
        Err(TaskExecutionError::ProofInvalidated(_)) => {
            lease.execution_policy.retry = RetryPolicy::None;
        }
        _ => {}
    }
}

fn should_notify_queue_task<I>(
    payload: &EngineTask,
    execution_result: &Result<EngineOutput<I>, TaskExecutionError>,
    recovered_output: bool,
) -> bool {
    recovered_output
        || !matches!(payload.publication_source(), EngineTask::Proposal { .. })
        || matches!(execution_result, Err(TaskExecutionError::Stage { .. }))
        || matches!(execution_result, Ok(EngineOutput::Proof(_)))
        || execution_result
            .as_ref()
            .err()
            .map(TaskExecutionError::message)
            == Some(task_cancelled_error().as_str())
        || execution_result
            .as_ref()
            .err()
            .map(TaskExecutionError::message)
            == Some(task_lease_lost_error().as_str())
}

fn log_execution_failure<I>(
    worker: &str,
    task: &EngineTask,
    result: &Result<EngineOutput<I>, TaskExecutionError>,
) {
    if let Err(error) = result {
        tracing::warn!(worker, task = ?task, %error, "engine task failed");
    }
}

fn proof_observer_task(id: &EngineTaskId, task: &EngineTask) -> EngineTask {
    match &id.0 {
        EngineTaskKey::Proposal { request, .. } => EngineTask::ProveProposal {
            request: request.clone(),
            input_task: id.clone(),
        },
        EngineTaskKey::Aggregate { .. } => task.publication_source().clone(),
    }
}

pub struct ProofCompletionPermit {
    guard: Box<dyn std::any::Any + Send + Sync>,
}

pub struct TaskExecutionPermit {
    guard: Box<dyn std::any::Any + Send + Sync>,
}

/// Keeps an observer-owned terminal transition gate held while the engine validates the lease,
/// persists terminal runtime state, and removes the corresponding queue projection.
pub struct TerminalFailurePermit {
    guard: Box<dyn std::any::Any + Send + Sync>,
}

/// Keeps the root-owner transition gate held until the engine removes failed projections.
///
/// A terminal queue failure first becomes durable runtime state, then the engine removes every
/// affected `RootOwner` while this permit is alive. The opaque guard prevents recovery from
/// reattaching an owner between those two effects.
pub struct TerminalFailureProjection {
    root_owners: Vec<RootOwner>,
    _lifecycle_guard: Box<dyn std::any::Any + Send>,
}

impl TerminalFailureProjection {
    /// Returns the exact runtime-root owners that must leave the execution projection.
    #[must_use]
    pub fn root_owners(&self) -> &[RootOwner] {
        &self.root_owners
    }
}

impl TerminalFailurePermit {
    pub fn tracked(guard: impl std::any::Any + Send + Sync + 'static) -> Self {
        Self {
            guard: Box::new(guard),
        }
    }

    #[must_use]
    pub fn guard<T: std::any::Any>(&self) -> Option<&T> {
        self.guard.downcast_ref()
    }

    #[must_use]
    pub fn into_projection(self, root_owners: Vec<RootOwner>) -> TerminalFailureProjection {
        TerminalFailureProjection {
            root_owners,
            _lifecycle_guard: self.guard,
        }
    }

    fn untracked() -> Self {
        Self::tracked(())
    }
}

impl TaskExecutionPermit {
    pub fn tracked(guard: impl std::any::Any + Send + Sync + 'static) -> Self {
        Self {
            guard: Box::new(guard),
        }
    }

    #[must_use]
    pub fn guard<T: std::any::Any>(&self) -> Option<&T> {
        self.guard.downcast_ref()
    }

    fn untracked() -> Self {
        Self::tracked(())
    }
}

impl ProofCompletionPermit {
    pub fn tracked(guard: impl std::any::Any + Send + Sync + 'static) -> Self {
        Self {
            guard: Box::new(guard),
        }
    }

    #[must_use]
    pub fn guard<T: std::any::Any>(&self) -> Option<&T> {
        self.guard.downcast_ref()
    }

    fn untracked() -> Self {
        Self::tracked(())
    }
}

#[async_trait]
pub trait EngineObserver: Send + Sync {
    async fn acquire_task_execution_permit(
        &self,
        _id: &EngineTaskId,
        _task: &EngineTask,
    ) -> Result<TaskExecutionPermit, EngineObserverError> {
        Ok(TaskExecutionPermit::untracked())
    }

    async fn acquire_submission_checkpoint_permit(
        &self,
    ) -> Result<SubmissionCheckpointPermit, raiko2_prover::ProgressPersistenceError> {
        Ok(SubmissionCheckpointPermit::tracked(()))
    }

    async fn on_task_started(
        &self,
        _id: &EngineTaskId,
        _task: &EngineTask,
        _worker: &str,
        _execution_permit: &TaskExecutionPermit,
    ) {
    }

    async fn on_task_progress(
        &self,
        _id: &EngineTaskId,
        _task: &EngineTask,
        _progress: &ProverProgress,
        _permit: &SubmissionCheckpointPermit,
        _execution_permit: &TaskExecutionPermit,
    ) -> Result<(), EngineObserverError> {
        Ok(())
    }

    async fn on_task_cancelled(&self, _id: &EngineTaskId) {}

    async fn checkpoint_completed_proof(
        &self,
        _id: &EngineTaskId,
        _task: &EngineTask,
        _proof: &Proof,
        _execution_permit: &TaskExecutionPermit,
    ) -> Result<ProofCompletionPermit, EngineObserverError> {
        Ok(ProofCompletionPermit::untracked())
    }

    async fn on_task_succeeded(
        &self,
        _id: &EngineTaskId,
        _task: &EngineTask,
        _success: &EngineTaskSuccess,
        _permit: Option<&ProofCompletionPermit>,
        _execution_permit: &TaskExecutionPermit,
    ) -> Result<(), EngineObserverError> {
        Ok(())
    }

    async fn on_task_failed(
        &self,
        _id: &EngineTaskId,
        _task: &EngineTask,
        _error: &str,
        permit: TerminalFailurePermit,
    ) -> Result<TerminalFailureProjection, EngineObserverError> {
        Ok(permit.into_projection(Vec::new()))
    }

    async fn acquire_terminal_failure_permit(
        &self,
        _id: &EngineTaskId,
        _task: &EngineTask,
        _execution_permit: &TaskExecutionPermit,
    ) -> Result<TerminalFailurePermit, EngineObserverError> {
        Ok(TerminalFailurePermit::untracked())
    }

    async fn load_pending_proof_checkpoint(
        &self,
        _id: &EngineTaskId,
        _task: &EngineTask,
        _backend: NetworkProverBackend,
    ) -> Result<Option<PendingProofCheckpoint>, raiko2_prover::ProgressPersistenceError> {
        Ok(None)
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
    execution_permit: &TaskExecutionPermit,
) {
    if let Some(observer) = observer {
        observer
            .on_task_started(id, task, worker, execution_permit)
            .await;
    }
}

async fn notify_stage_succeeded(
    observer: Option<&Arc<dyn EngineObserver>>,
    id: &EngineTaskId,
    task: &EngineTask,
    success: &EngineTaskSuccess,
    permit: Option<&ProofCompletionPermit>,
    execution_permit: &TaskExecutionPermit,
) -> Result<(), EngineObserverError> {
    if let Some(observer) = observer {
        observer
            .on_task_succeeded(id, task, success, permit, execution_permit)
            .await?;
    }
    Ok(())
}

struct EngineProgressObserver {
    observer: Arc<dyn EngineObserver>,
    task_id: EngineTaskId,
    task: EngineTask,
    execution_permit: Arc<TaskExecutionPermit>,
}

enum LeaseInterruption {
    Cancelled,
    Lost,
}

struct AbortOnDropTask(Option<tokio::task::JoinHandle<()>>);

impl AbortOnDropTask {
    const fn new(handle: tokio::task::JoinHandle<()>) -> Self {
        Self(Some(handle))
    }

    async fn abort_and_wait(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.abort();
            if let Err(error) = handle.await
                && !error.is_cancelled()
            {
                tracing::warn!(%error, "lease renewal task failed before abort completed");
            }
        }
    }
}

impl Drop for AbortOnDropTask {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.abort();
        }
    }
}

#[async_trait]
impl ProverProgressObserver for EngineProgressObserver {
    async fn acquire_submission_checkpoint_permit(
        &self,
    ) -> Result<SubmissionCheckpointPermit, raiko2_prover::ProgressPersistenceError> {
        self.observer.acquire_submission_checkpoint_permit().await
    }

    async fn on_progress(
        &self,
        progress: &ProverProgress,
        permit: &SubmissionCheckpointPermit,
    ) -> Result<(), raiko2_prover::ProgressPersistenceError> {
        match self
            .observer
            .on_task_progress(
                &self.task_id,
                &self.task,
                progress,
                permit,
                &self.execution_permit,
            )
            .await
        {
            Ok(()) => Ok(()),
            Err(
                EngineObserverError::RuntimeInactive(error)
                | EngineObserverError::ProgressRejected(error),
            ) => Err(raiko2_prover::ProgressPersistenceError::Permanent(error)),
            Err(error) => Err(raiko2_prover::ProgressPersistenceError::Retryable(
                error.to_string(),
            )),
        }
    }

    async fn load_pending_proof_checkpoint(
        &self,
        backend: NetworkProverBackend,
    ) -> Result<Option<PendingProofCheckpoint>, raiko2_prover::ProgressPersistenceError> {
        self.observer
            .load_pending_proof_checkpoint(&self.task_id, &self.task, backend)
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
                last_maintenance_success_ms: AtomicU64::new(0),
                worker_groups: Mutex::new(Vec::new()),
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

    fn progress_observer(
        &self,
        task_id: &EngineTaskId,
        task: &EngineTask,
        execution_permit: &Arc<TaskExecutionPermit>,
    ) -> Option<Arc<dyn ProverProgressObserver>> {
        self.inner.observer.as_ref().map(|observer| {
            Arc::new(EngineProgressObserver {
                observer: Arc::clone(observer),
                task_id: task_id.clone(),
                task: task.clone(),
                execution_permit: Arc::clone(execution_permit),
            }) as Arc<dyn ProverProgressObserver>
        })
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

    /// Attaches an entire root-owned proving plan atomically to the queue projection.
    ///
    /// # Errors
    ///
    /// Returns an error when the plan is malformed or the queue cannot attach it.
    pub async fn attach_execution_plan(
        &self,
        owner: RootOwner,
        plan: EngineExecutionPlan,
    ) -> Result<AttachOutcome, TaskStoreError> {
        let mut nodes =
            Vec::with_capacity(plan.proposals.len() + usize::from(plan.aggregate.is_some()));
        let mut proposal_ids = std::collections::HashSet::new();

        for request in plan.proposals {
            let task_id = self.proposal_task_id(request.clone());
            if !proposal_ids.insert(task_id.clone()) {
                continue;
            }
            nodes.push(ExecutionNode {
                id: task_id.clone(),
                task: NewTask {
                    priority: PROPOSAL_TASK_PRIORITY,
                    payload: EngineTask::Proposal { request },
                },
                dependencies: Vec::new(),
                execution_policy: self.externally_stateful_stage_execution_policy(),
            });
        }

        if let Some(aggregate) = plan.aggregate {
            if aggregate.inputs.is_empty() {
                return Err(TaskStoreError::corrupt_msg(
                    "aggregation requires at least 1 proof input",
                ));
            }
            let mut dependencies = Vec::new();
            for input in &aggregate.inputs {
                let AggregateProofInput::PendingProofArtifact { dependency, .. } = input else {
                    continue;
                };
                match &dependency.0 {
                    EngineTaskKey::Proposal { pipeline, .. }
                        if *pipeline == self.inner.spec.pipeline_key()
                            && proposal_ids.contains(dependency.as_ref()) =>
                    {
                        if !dependencies.contains(dependency.as_ref()) {
                            dependencies.push((**dependency).clone());
                        }
                    }
                    EngineTaskKey::Proposal { .. } => {
                        return Err(TaskStoreError::corrupt_msg(
                            "aggregation input must reference a proposal in this execution plan",
                        ));
                    }
                    EngineTaskKey::Aggregate { .. } => {
                        return Err(TaskStoreError::corrupt_msg(
                            "aggregation input cannot reference an aggregate task",
                        ));
                    }
                }
            }
            nodes.push(ExecutionNode {
                id: self.aggregate_task_id(aggregate.request.clone()),
                task: NewTask {
                    priority: AGGREGATION_TASK_PRIORITY,
                    payload: EngineTask::Aggregate {
                        request: aggregate.request,
                        source: AggregationSource::Inputs(aggregate.inputs),
                    },
                },
                dependencies,
                execution_policy: self.externally_stateful_stage_execution_policy(),
            });
        }

        if nodes.is_empty() {
            return Err(TaskStoreError::corrupt_msg(
                "execution plan must contain at least one task",
            ));
        }
        self.attach_execution(owner, ExecutionGraph::new(nodes))
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

    async fn attach_execution(
        &self,
        owner: RootOwner,
        graph: ExecutionGraph<EngineTask, EngineTaskKey>,
    ) -> Result<AttachOutcome, TaskStoreError> {
        self.inner.scheduler.attach(owner, graph).await
    }

    /// Detaches exactly one runtime-root incarnation from the in-memory execution projection.
    ///
    /// # Errors
    ///
    /// Returns an error if the queue cannot update the projection.
    pub async fn detach_execution(
        &self,
        owner: &RootOwner,
        mode: DetachMode,
    ) -> Result<DetachOutcome<EngineTaskKey>, TaskStoreError> {
        let outcome = self.inner.scheduler.detach(owner, mode).await?;
        if let Some(observer) = &self.inner.observer {
            for task_id in &outcome.retired {
                observer.on_task_cancelled(task_id).await;
            }
        }
        Ok(outcome)
    }

    /// Returns the currently attached task graph for one exact runtime-root incarnation.
    ///
    /// # Errors
    ///
    /// Returns an error if the queue cannot read the projection.
    pub async fn inspect_execution(
        &self,
        owner: &RootOwner,
    ) -> Result<Option<ProjectionView<EngineTaskKey>>, TaskStoreError> {
        self.inner.scheduler.inspect(owner).await
    }

    async fn checkpoint_completed_proof(
        &self,
        lease: &mut TaskLease<EngineTask, EngineTaskKey>,
        payload: &EngineTask,
        terminal_observer_task: &mut EngineTask,
        execution_result: &mut Result<EngineOutput<S::GuestInput>, TaskExecutionError>,
        execution_permit: &TaskExecutionPermit,
    ) -> Result<Option<ProofCompletionPermit>, TaskStoreError> {
        let Ok(EngineOutput::Proof(proof)) = execution_result else {
            return Ok(None);
        };
        *terminal_observer_task = proof_observer_task(&lease.id, payload);
        let completed_proof = proof.output.clone();
        let checkpoint_payload = payload
            .clone()
            .with_pending_publication(completed_proof.clone());
        let checkpoint_policy = TaskExecutionPolicy {
            retry: durability_retry_policy(),
            ..lease.execution_policy.clone()
        };
        let checkpointed = self
            .inner
            .scheduler
            .checkpoint_payload(lease, checkpoint_payload.clone(), checkpoint_policy.clone())
            .await?;
        if checkpointed {
            lease.payload = checkpoint_payload;
            lease.execution_policy = checkpoint_policy;
        } else {
            *execution_result = Err(task_lease_lost_error().into());
            return Ok(None);
        }
        if let Some(observer) = &self.inner.observer {
            match observer
                .checkpoint_completed_proof(
                    &lease.id,
                    terminal_observer_task,
                    &completed_proof,
                    execution_permit,
                )
                .await
            {
                Ok(permit) => return Ok(Some(permit)),
                Err(EngineObserverError::ProofInvalidated(error)) => {
                    *execution_result = Err(TaskExecutionError::ProofInvalidated(error));
                }
                Err(error) => {
                    *execution_result = Err(TaskExecutionError::ProofPublication {
                        error: error.to_string(),
                        proof: Box::new(completed_proof),
                    });
                }
            }
        }
        Ok(None)
    }

    async fn notify_execution_success(
        &self,
        id: &EngineTaskId,
        task: &EngineTask,
        execution_result: &mut Result<EngineOutput<S::GuestInput>, TaskExecutionError>,
        permit: Option<&ProofCompletionPermit>,
        execution_permit: &TaskExecutionPermit,
    ) {
        let Ok(output) = execution_result else {
            return;
        };
        let success = task_success_from_output(output);
        if let Err(error) = notify_stage_succeeded(
            self.inner.observer.as_ref(),
            id,
            task,
            &success,
            permit,
            execution_permit,
        )
        .await
        {
            *execution_result = Err(match (success, error) {
                (EngineTaskSuccess::Proof { .. }, EngineObserverError::ProofInvalidated(error)) => {
                    TaskExecutionError::ProofInvalidated(error)
                }
                (
                    EngineTaskSuccess::Proof { proof, .. },
                    EngineObserverError::ProofPublication(error)
                    | EngineObserverError::RuntimeSync(error)
                    | EngineObserverError::RuntimeInactive(error)
                    | EngineObserverError::ProgressRejected(error),
                ) => TaskExecutionError::ProofPublication {
                    error,
                    proof: Box::new(proof),
                },
                (
                    EngineTaskSuccess::GuestInput { .. } | EngineTaskSuccess::EncodedInput { .. },
                    error,
                ) => error.into(),
            });
        }
    }

    fn spawn_lease_renewal(
        &self,
        lease: &TaskLease<EngineTask, EngineTaskKey>,
    ) -> tokio::task::JoinHandle<()> {
        let scheduler = self.inner.scheduler.clone();
        let lease = lease.clone();
        let renew_period = lease
            .execution_policy
            .lease_duration
            .checked_div(2)
            .unwrap_or_else(|| Duration::from_secs(1))
            .max(Duration::from_secs(1));
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(renew_period);
            interval.tick().await;
            loop {
                interval.tick().await;
                match scheduler.renew_lease(&lease).await {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(error) => {
                        tracing::warn!(
                            worker = %lease.worker,
                            task = ?lease.id,
                            %error,
                            "failed to renew task lease"
                        );
                        break;
                    }
                }
            }
        })
    }

    async fn fail_task_after_lease_loss(
        &self,
        lease: &TaskLease<EngineTask, EngineTaskKey>,
        observer_task: &EngineTask,
        execution_permit: &TaskExecutionPermit,
    ) -> Result<(), TaskStoreError> {
        let error = task_lease_lost_error();
        let mut lease = lease.clone();
        lease.execution_policy.retry = RetryPolicy::None;
        self.complete_terminal_failure(lease, observer_task, error, execution_permit)
            .await?;
        Ok(())
    }

    async fn execute_leased_task(
        &self,
        lease: &TaskLease<EngineTask, EngineTaskKey>,
        payload: &EngineTask,
        terminal_observer_task: &EngineTask,
        worker: &str,
        execution_permit: &Arc<TaskExecutionPermit>,
        permit_error: Option<EngineObserverError>,
    ) -> (
        Result<EngineOutput<S::GuestInput>, TaskExecutionError>,
        bool,
    ) {
        if let Some(error) = permit_error {
            return (Err(error.into()), false);
        }
        if let Some(observer) = &self.inner.observer
            && !matches!(payload, EngineTask::PublishProof { .. })
            && !matches!(payload.publication_source(), EngineTask::Proposal { .. })
        {
            observer
                .on_task_started(&lease.id, terminal_observer_task, worker, execution_permit)
                .await;
        }
        match self.recover_completed_output(&lease.id, payload).await {
            Ok(Some(output)) => (Ok(output), true),
            Ok(None) => (
                self.execute_with_task_controls(
                    &lease.id,
                    lease,
                    payload.clone(),
                    Arc::clone(execution_permit),
                )
                .await,
                false,
            ),
            Err(error) => (Err(error), false),
        }
    }

    async fn retry_after_terminal_observer_error(
        &self,
        mut lease: TaskLease<EngineTask, EngineTaskKey>,
        execution_error: &str,
        observer_error: &EngineObserverError,
    ) -> Result<TaskCompletionDisposition, TaskStoreError> {
        tracing::warn!(
            task = ?lease.id,
            error = %observer_error,
            "terminal runtime state is not durable; retrying the queue task"
        );
        lease.execution_policy.retry = durability_retry_policy();
        self.inner
            .scheduler
            .complete_with_disposition(
                lease,
                Err(format!(
                    "{execution_error}; terminal runtime persistence failed: {observer_error}"
                )),
            )
            .await
    }

    async fn complete_terminal_failure(
        &self,
        lease: TaskLease<EngineTask, EngineTaskKey>,
        task: &EngineTask,
        error: String,
        execution_permit: &TaskExecutionPermit,
    ) -> Result<TaskCompletionDisposition, TaskStoreError> {
        let Some(observer) = &self.inner.observer else {
            return self
                .inner
                .scheduler
                .complete_with_disposition(lease, Err(error))
                .await;
        };
        let terminal_permit = match observer
            .acquire_terminal_failure_permit(&lease.id, task, execution_permit)
            .await
        {
            Ok(permit) => permit,
            Err(observer_error) => {
                return self
                    .retry_after_terminal_observer_error(lease, &error, &observer_error)
                    .await;
            }
        };
        if !self.inner.scheduler.renew_lease(&lease).await? {
            return Ok(TaskCompletionDisposition::Stale);
        }
        let projection = match observer
            .on_task_failed(&lease.id, task, &error, terminal_permit)
            .await
        {
            Ok(projection) => projection,
            Err(observer_error) => {
                return self
                    .retry_after_terminal_observer_error(lease, &error, &observer_error)
                    .await;
            }
        };
        let id = lease.id.clone();
        let completion = self
            .inner
            .scheduler
            .complete_with_disposition(lease, Err(error))
            .await;
        self.detach_terminal_failure_projection(&id, projection)
            .await;
        completion
    }

    async fn detach_terminal_failure_projection(
        &self,
        id: &EngineTaskId,
        projection: TerminalFailureProjection,
    ) {
        for owner in projection.root_owners() {
            if let Err(error) = self.inner.scheduler.detach(owner, DetachMode::Remove).await {
                tracing::warn!(
                    task = ?id,
                    root_task_id = %owner.task_id,
                    root_incarnation_id = %owner.incarnation_id,
                    %error,
                    "failed to remove terminal root from execution projection"
                );
            }
        }
    }

    async fn acquire_current_execution_permit(
        &self,
        lease: &TaskLease<EngineTask, EngineTaskKey>,
        payload: &EngineTask,
    ) -> Result<Option<(Arc<TaskExecutionPermit>, Option<EngineObserverError>)>, TaskStoreError>
    {
        let permit = match &self.inner.observer {
            Some(observer) => {
                observer
                    .acquire_task_execution_permit(&lease.id, payload)
                    .await
            }
            None => Ok(TaskExecutionPermit::untracked()),
        };
        if !self.inner.scheduler.renew_lease(lease).await? {
            return Ok(None);
        }
        Ok(Some(match permit {
            Ok(permit) => (Arc::new(permit), None),
            Err(error) => (Arc::new(TaskExecutionPermit::untracked()), Some(error)),
        }))
    }

    /// # Errors
    ///
    /// Returns an error if the task store cannot lease or complete work.
    pub async fn run_one(&self, worker: &str) -> Result<bool, TaskStoreError> {
        let Some(mut lease) = self.inner.scheduler.next_ready(worker).await? else {
            return Ok(false);
        };

        let payload = lease.payload.clone();
        let mut renew_task = AbortOnDropTask::new(self.spawn_lease_renewal(&lease));
        let Some((execution_permit, permit_error)) = self
            .acquire_current_execution_permit(&lease, &payload)
            .await?
        else {
            renew_task.abort_and_wait().await;
            return Ok(true);
        };

        let mut terminal_observer_task = payload.clone();
        if matches!(payload, EngineTask::PublishProof { .. }) {
            terminal_observer_task = proof_observer_task(&lease.id, &payload);
        }
        let (mut execution_result, recovered_output) = self
            .execute_leased_task(
                &lease,
                &payload,
                &terminal_observer_task,
                worker,
                &execution_permit,
                permit_error,
            )
            .await;
        if let Some(observer_task) = execution_result
            .as_ref()
            .err()
            .and_then(TaskExecutionError::observer_task)
        {
            terminal_observer_task = observer_task.clone();
        }
        log_execution_failure(worker, &payload, &execution_result);
        let proof_completion_permit = match self
            .checkpoint_completed_proof(
                &mut lease,
                &payload,
                &mut terminal_observer_task,
                &mut execution_result,
                &execution_permit,
            )
            .await
        {
            Ok(permit) => permit,
            Err(error) => {
                renew_task.abort_and_wait().await;
                return Err(error);
            }
        };
        let lease_was_lost = execution_result
            .as_ref()
            .err()
            .is_some_and(|error| error.message() == task_lease_lost_error());
        if lease_was_lost {
            renew_task.abort_and_wait().await;
            self.fail_task_after_lease_loss(&lease, &terminal_observer_task, &execution_permit)
                .await?;
            return Ok(true);
        }
        let should_notify_queue_task =
            should_notify_queue_task(&payload, &execution_result, recovered_output);
        if should_notify_queue_task {
            self.notify_execution_success(
                &lease.id,
                &terminal_observer_task,
                &mut execution_result,
                proof_completion_permit.as_ref(),
                &execution_permit,
            )
            .await;
        }
        apply_execution_failure_policy(&mut lease, &payload, &execution_result);
        let result = execution_result.map_err(|error| error.to_string());
        let completion = match result {
            Err(error) if lease.execution_policy.failure_is_terminal(lease.attempt) => {
                self.complete_terminal_failure(
                    lease,
                    &terminal_observer_task,
                    error,
                    &execution_permit,
                )
                .await
            }
            result => {
                self.inner
                    .scheduler
                    .complete_with_disposition(lease, result)
                    .await
            }
        };
        renew_task.abort_and_wait().await;
        completion?;
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
        self.register_worker_group(crate::worker::spawn_workers(self.clone(), &config));
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
        self.register_worker_group(crate::worker::spawn_workers(self.clone(), &config));
    }

    fn register_worker_group(&self, group: crate::worker::WorkerGroup) {
        self.inner
            .worker_groups
            .lock()
            .expect("worker group lock poisoned")
            .push(Arc::new(group));
    }

    pub async fn shutdown_workers(&self) {
        let groups = match self.inner.worker_groups.lock() {
            Ok(mut groups) => groups.drain(..).collect::<Vec<_>>(),
            Err(poisoned) => {
                tracing::error!("worker group lock poisoned during engine shutdown");
                poisoned.into_inner().drain(..).collect::<Vec<_>>()
            }
        };
        for group in groups {
            group.shutdown().await;
        }
    }

    #[must_use]
    pub fn queue_maintenance_ready(&self, max_age: Duration) -> bool {
        let last_success = self
            .inner
            .last_maintenance_success_ms
            .load(Ordering::Acquire);
        let max_age_ms = u64::try_from(max_age.as_millis()).unwrap_or(u64::MAX);
        last_success != 0 && now_millis().saturating_sub(last_success) <= max_age_ms
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
        execution_permit: Arc<TaskExecutionPermit>,
    ) -> Result<EngineOutput<S::GuestInput>, TaskExecutionError> {
        let execute = self.execute(task_id, payload, &lease.worker, execution_permit);
        let interrupted =
            self.wait_lease_interruption(&lease.id, &lease.lease_token, lease.attempt);
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
        let stage = match task.publication_source() {
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
        lease_token: &str,
        attempt: u32,
    ) -> Result<LeaseInterruption, TaskStoreError> {
        let notifier = self.inner.scheduler.notifier();
        loop {
            let notification = notifier.notified();
            tokio::pin!(notification);
            notification.as_mut().enable();
            let Some(view) = self.inner.scheduler.get(id.clone()).await? else {
                return Ok(LeaseInterruption::Lost);
            };

            match view.state {
                TaskState::Running {
                    lease_token: current_lease_token,
                    attempt: current_attempt,
                } if current_lease_token == lease_token && current_attempt == attempt => {}
                TaskState::Cancelled => return Ok(LeaseInterruption::Cancelled),
                _ => return Ok(LeaseInterruption::Lost),
            }

            notification.await;
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
        execution_permit: &Arc<TaskExecutionPermit>,
        execute: impl std::future::Future<Output = Result<PipelineStageResult<T>, String>>,
    ) -> Result<PipelineStageResult<T>, TaskExecutionError> {
        notify_stage_started(
            self.inner.observer.as_ref(),
            task_id,
            task,
            worker,
            execution_permit,
        )
        .await;
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
                notify_stage_succeeded(
                    self.inner.observer.as_ref(),
                    task_id,
                    task,
                    &success,
                    None,
                    execution_permit,
                )
                .await
                .map_err(TaskExecutionError::from)?;
                Ok(output)
            }
            Err(error) => Err(TaskExecutionError::stage(error, task.clone())),
        }
    }

    async fn execute_proposal(
        &self,
        task_id: &EngineTaskId,
        request: ProposalTaskRequest,
        worker: &str,
        execution_permit: &Arc<TaskExecutionPermit>,
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
                execution_permit,
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
                execution_permit,
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
        notify_stage_started(
            self.inner.observer.as_ref(),
            task_id,
            &encode_task,
            worker,
            execution_permit,
        )
        .await;
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
                    None,
                    execution_permit,
                )
                .await
                .map_err(TaskExecutionError::from)?;
                encoded
            }
            Err(error) => return Err(TaskExecutionError::stage(error, encode_task)),
        };

        self.prove_proposal_encoded(
            task_id,
            request,
            task_id.clone(),
            encoded.output,
            worker,
            execution_permit,
        )
        .await
    }

    async fn prove_proposal(
        &self,
        task_id: &EngineTaskId,
        request: ProposalTaskRequest,
        input_task: EngineTaskId,
        execution_permit: &Arc<TaskExecutionPermit>,
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
                self.progress_observer(task_id, &progress_task, execution_permit),
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
        execution_permit: &Arc<TaskExecutionPermit>,
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
            execution_permit,
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
                self.progress_observer(task_id, &progress_task, execution_permit),
            )
            .await
            .map_err(|e| e.to_string())
        {
            Ok(proof) => proof,
            Err(error) => return Err(TaskExecutionError::stage(error, progress_task)),
        };
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
        execution_permit: Arc<TaskExecutionPermit>,
    ) -> Result<EngineOutput<S::GuestInput>, TaskExecutionError> {
        match task {
            EngineTask::Proposal { request } => {
                self.execute_proposal(task_id, request, worker, &execution_permit)
                    .await
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
                .prove_proposal(task_id, request, input_task, &execution_permit)
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
                        self.progress_observer(task_id, &progress_task, &execution_permit),
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(EngineOutput::Proof(Box::new(PipelineStageResult::new(
                    PipelineStage::Aggregate,
                    proof,
                ))))
            }
            EngineTask::PublishProof { proof, .. } => {
                let stage = match &task_id.0 {
                    EngineTaskKey::Proposal { .. } => PipelineStage::Prove,
                    EngineTaskKey::Aggregate { .. } => PipelineStage::Aggregate,
                };
                Ok(EngineOutput::Proof(Box::new(PipelineStageResult::new(
                    stage, *proof,
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
            .map_err(|err| err.to_string())?;
        self.inner
            .last_maintenance_success_ms
            .store(now_millis(), Ordering::Release);
        Ok(())
    }

    fn notifier(&self) -> Arc<tokio::sync::Notify> {
        self.inner.scheduler.notifier()
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
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
    use raiko2_queue::{
        AttachOutcome, DetachMode, RetryPolicy, RootOwner, SchedulerConfig, TaskState,
    };

    use crate::tasks::{
        AggregateProofInput, AggregationTaskRequest, EngineOutput, ProofArtifactRef,
        ProposalTaskRequest, ProverTaskConfig,
    };
    use crate::{
        AbortOnDropTask, Engine, EngineAggregationPlan, EngineExecutionPlan, EngineObserver,
        EngineObserverError, EngineTask, EngineTaskId, EngineTaskKey, EngineTaskSuccess,
        PROPOSAL_TASK_PRIORITY, TerminalFailureProjection,
    };

    struct TaskDropSignal(Arc<AtomicUsize>);

    impl Drop for TaskDropSignal {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Release);
        }
    }

    #[tokio::test]
    async fn dropping_lease_renewal_guard_aborts_task() {
        let dropped = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(tokio::sync::Notify::new());
        let task = tokio::spawn({
            let dropped = Arc::clone(&dropped);
            let started = Arc::clone(&started);
            async move {
                let _drop_signal = TaskDropSignal(dropped);
                started.notify_one();
                std::future::pending::<()>().await;
            }
        });
        started.notified().await;

        drop(AbortOnDropTask::new(task));
        for _ in 0..100 {
            if dropped.load(Ordering::Acquire) == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }

        assert_eq!(dropped.load(Ordering::Acquire), 1);
    }

    struct PublicationFailingObserver {
        task_starts: AtomicUsize,
        proof_successes: AtomicUsize,
        task_failures: AtomicUsize,
        failures: usize,
    }

    impl PublicationFailingObserver {
        fn new(failures: usize) -> Self {
            Self {
                task_starts: AtomicUsize::new(0),
                proof_successes: AtomicUsize::new(0),
                task_failures: AtomicUsize::new(0),
                failures,
            }
        }
    }

    #[async_trait::async_trait]
    impl EngineObserver for PublicationFailingObserver {
        async fn on_task_started(
            &self,
            _id: &EngineTaskId,
            _task: &EngineTask,
            _worker: &str,
            _execution_permit: &crate::TaskExecutionPermit,
        ) {
            self.task_starts.fetch_add(1, Ordering::SeqCst);
        }

        async fn on_task_succeeded(
            &self,
            _id: &EngineTaskId,
            _task: &EngineTask,
            success: &EngineTaskSuccess,
            _permit: Option<&crate::ProofCompletionPermit>,
            _execution_permit: &crate::TaskExecutionPermit,
        ) -> Result<(), EngineObserverError> {
            if matches!(success, EngineTaskSuccess::Proof { .. }) {
                let attempt = self.proof_successes.fetch_add(1, Ordering::SeqCst);
                if attempt < self.failures {
                    return Err(EngineObserverError::ProofPublication(
                        "injected failure".to_string(),
                    ));
                }
            }
            Ok(())
        }

        async fn on_task_failed(
            &self,
            _id: &EngineTaskId,
            _task: &EngineTask,
            _error: &str,
            permit: crate::TerminalFailurePermit,
        ) -> Result<TerminalFailureProjection, EngineObserverError> {
            self.task_failures.fetch_add(1, Ordering::SeqCst);
            Ok(permit.into_projection(Vec::new()))
        }
    }

    #[derive(Default)]
    struct RuntimeSyncFailingObserver {
        stage_successes: AtomicUsize,
        task_failures: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl EngineObserver for RuntimeSyncFailingObserver {
        async fn on_task_succeeded(
            &self,
            _id: &EngineTaskId,
            _task: &EngineTask,
            success: &EngineTaskSuccess,
            _permit: Option<&crate::ProofCompletionPermit>,
            _execution_permit: &crate::TaskExecutionPermit,
        ) -> Result<(), EngineObserverError> {
            if !matches!(success, EngineTaskSuccess::Proof { .. })
                && self.stage_successes.fetch_add(1, Ordering::SeqCst) == 0
            {
                return Err(EngineObserverError::RuntimeSync(
                    "injected intermediate runtime sync failure".to_string(),
                ));
            }
            Ok(())
        }

        async fn on_task_failed(
            &self,
            _id: &EngineTaskId,
            _task: &EngineTask,
            _error: &str,
            permit: crate::TerminalFailurePermit,
        ) -> Result<TerminalFailureProjection, EngineObserverError> {
            self.task_failures.fetch_add(1, Ordering::SeqCst);
            Ok(permit.into_projection(Vec::new()))
        }
    }

    struct PublicationInvalidatingObserver;

    #[async_trait::async_trait]
    impl EngineObserver for PublicationInvalidatingObserver {
        async fn checkpoint_completed_proof(
            &self,
            _id: &EngineTaskId,
            _task: &EngineTask,
            _proof: &Proof,
            _execution_permit: &crate::TaskExecutionPermit,
        ) -> Result<crate::ProofCompletionPermit, EngineObserverError> {
            Err(EngineObserverError::ProofInvalidated(
                "injected invalidation".to_string(),
            ))
        }
    }

    struct TerminalFailureObserver {
        owner: RootOwner,
    }

    #[async_trait::async_trait]
    impl EngineObserver for TerminalFailureObserver {
        async fn checkpoint_completed_proof(
            &self,
            _id: &EngineTaskId,
            _task: &EngineTask,
            _proof: &Proof,
            _execution_permit: &crate::TaskExecutionPermit,
        ) -> Result<crate::ProofCompletionPermit, EngineObserverError> {
            Err(EngineObserverError::ProofInvalidated(
                "injected invalidation".to_string(),
            ))
        }

        async fn on_task_failed(
            &self,
            _id: &EngineTaskId,
            _task: &EngineTask,
            _error: &str,
            permit: crate::TerminalFailurePermit,
        ) -> Result<TerminalFailureProjection, EngineObserverError> {
            Ok(permit.into_projection(vec![self.owner.clone()]))
        }
    }

    struct TerminalSyncFailingObserver {
        owner: RootOwner,
        failures: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl EngineObserver for TerminalSyncFailingObserver {
        async fn on_task_failed(
            &self,
            _id: &EngineTaskId,
            _task: &EngineTask,
            _error: &str,
            permit: crate::TerminalFailurePermit,
        ) -> Result<TerminalFailureProjection, EngineObserverError> {
            if self.failures.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(EngineObserverError::RuntimeSync(
                    "injected terminal sync failure".to_string(),
                ));
            }
            Ok(permit.into_projection(vec![self.owner.clone()]))
        }
    }

    struct RecoveringObserver {
        proof_successes: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl EngineObserver for RecoveringObserver {
        async fn load_completed_proof(
            &self,
            _id: &EngineTaskId,
            _task: &EngineTask,
        ) -> Result<Option<Proof>, String> {
            Ok(Some(Proof {
                proof: Some("recovered-proof".to_string()),
                ..Proof::default()
            }))
        }

        async fn on_task_succeeded(
            &self,
            _id: &EngineTaskId,
            _task: &EngineTask,
            success: &EngineTaskSuccess,
            _permit: Option<&crate::ProofCompletionPermit>,
            _execution_permit: &crate::TaskExecutionPermit,
        ) -> Result<(), EngineObserverError> {
            if matches!(success, EngineTaskSuccess::Proof { .. }) {
                self.proof_successes.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct CancellationObserver {
        retired_tasks: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl EngineObserver for CancellationObserver {
        async fn on_task_cancelled(&self, _id: &EngineTaskId) {
            self.retired_tasks.fetch_add(1, Ordering::SeqCst);
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

    struct CountingProver {
        proof_runs: Arc<AtomicUsize>,
    }

    impl GuestInputCodec<GuestInput> for CountingProver {
        fn encode(&self, input: &GuestInput, _config: &ProverConfig) -> RaikoResult<Bytes> {
            Ok(Bytes::from(input.taiko.proposal_id.to_le_bytes().to_vec()))
        }
    }

    #[async_trait::async_trait]
    impl Prover<TestBackend> for CountingProver {
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
            self.proof_runs.fetch_add(1, Ordering::SeqCst);
            Ok(Proof {
                proof: Some("counted-proof".to_string()),
                ..Proof::default()
            })
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
            route: PipelineKey::ShastaNative.route(),
            proof_ref: proof_ref.to_string(),
        }
    }

    fn pending_proof_input(proof_ref: &str, dependency: EngineTaskId) -> AggregateProofInput {
        AggregateProofInput::PendingProofArtifact {
            artifact: proof_artifact(proof_ref),
            dependency: Box::new(dependency),
        }
    }

    macro_rules! attach_proposal {
        ($engine:expr, $root:expr, $request:expr) => {{
            let request = $request;
            let owner = RootOwner::new($root, uuid::Uuid::new_v4());
            let task_id = $engine.proposal_task_id(request.clone());
            $engine
                .attach_execution_plan(
                    owner.clone(),
                    EngineExecutionPlan {
                        proposals: vec![request],
                        aggregate: None,
                    },
                )
                .await?;
            (owner, task_id)
        }};
    }

    macro_rules! attach_aggregate {
        ($engine:expr, $root:expr, $proposals:expr, $request:expr, $inputs:expr) => {{
            let request = $request;
            let owner = RootOwner::new($root, uuid::Uuid::new_v4());
            let task_id = $engine.aggregate_task_id(request.clone());
            $engine
                .attach_execution_plan(
                    owner.clone(),
                    EngineExecutionPlan {
                        proposals: $proposals,
                        aggregate: Some(EngineAggregationPlan {
                            request,
                            inputs: $inputs,
                        }),
                    },
                )
                .await?;
            (owner, task_id)
        }};
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

        let (_owner, task_id) = attach_proposal!(engine, "proposal-priority", request);
        let view = engine
            .get(task_id)
            .await?
            .ok_or_else(|| std::io::Error::other("expected proposal task view"))?;
        assert_eq!(view.priority, PROPOSAL_TASK_PRIORITY);

        Ok(())
    }

    #[tokio::test]
    async fn execution_plan_attaches_the_complete_root_graph_atomically()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec::new(MockProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            Engine::<TestSpec<MockProver>>::default_scheduler_config(),
        );
        let first = proposal_request(1);
        let second = proposal_request(2);
        let second_id = engine.proposal_task_id(second.clone());
        let owner = RootOwner::new("root", uuid::Uuid::new_v4());
        let plan = EngineExecutionPlan {
            proposals: vec![first, second],
            aggregate: Some(EngineAggregationPlan {
                request: aggregation_request("root"),
                inputs: vec![pending_proof_input("proposal-2", second_id)],
            }),
        };

        assert_eq!(
            engine
                .attach_execution_plan(owner.clone(), plan.clone())
                .await?,
            AttachOutcome::Attached
        );
        assert_eq!(
            engine.attach_execution_plan(owner.clone(), plan).await?,
            AttachOutcome::AlreadyAttached
        );
        let graph = engine
            .inspect_execution(&owner)
            .await?
            .expect("attached execution graph");
        assert_eq!(graph.tasks.len(), 3);
        Ok(())
    }

    #[tokio::test]
    async fn attached_proposal_runs_dependency_pipeline() -> Result<(), Box<dyn std::error::Error>>
    {
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec::new(MockProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            Engine::<TestSpec<MockProver>>::default_scheduler_config(),
        );

        let (_owner, job_id) = attach_proposal!(engine, "proposal-run", proposal_request(1));

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
    async fn attached_aggregate_enqueues_complete_execution_graph()
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
        let (_owner, aggregate_id) = attach_aggregate!(
            engine,
            "aggregate-two-proposals",
            vec![proposal_request(1), proposal_request(2)],
            request.clone(),
            vec![
                pending_proof_input("proposal-1", first.clone()),
                pending_proof_input("proposal-2", second.clone()),
            ]
        );

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
        let execution = engine
            .inspect_execution(&_owner)
            .await?
            .expect("attached execution graph");
        assert_eq!(execution.tasks.len(), 3);
        Ok(())
    }

    #[tokio::test]
    async fn attached_aggregate_accepts_single_proposal_task()
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
        let (_owner, aggregate_id) = attach_aggregate!(
            engine,
            "aggregate-single-proposal",
            vec![proposal_request(1)],
            request.clone(),
            vec![pending_proof_input("proposal-1", proof_task.clone())]
        );

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
    async fn attached_aggregate_rejects_empty_input() -> Result<(), Box<dyn std::error::Error>> {
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec::new(MockProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            Engine::<TestSpec<MockProver>>::default_scheduler_config(),
        );

        let err = engine
            .attach_execution_plan(
                RootOwner::new("aggregate-empty", uuid::Uuid::new_v4()),
                EngineExecutionPlan {
                    proposals: vec![],
                    aggregate: Some(EngineAggregationPlan {
                        request: aggregation_request("agg-empty"),
                        inputs: Vec::new(),
                    }),
                },
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("aggregation requires at least 1 proof input")
        );
        Ok(())
    }

    #[tokio::test]
    async fn attached_aggregate_rejects_wrong_pipeline_proposal_task()
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
            .attach_execution_plan(
                RootOwner::new("aggregate-wrong-pipeline", uuid::Uuid::new_v4()),
                EngineExecutionPlan {
                    proposals: vec![],
                    aggregate: Some(EngineAggregationPlan {
                        request: aggregation_request("agg-wrong-pipeline"),
                        inputs: vec![pending_proof_input("proposal-1", other_pipeline_task)],
                    }),
                },
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("proposal in this execution plan"));
        Ok(())
    }

    #[tokio::test]
    async fn shared_proposal_has_one_definition_across_batch_positions()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec::new(MockProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            Engine::<TestSpec<MockProver>>::default_scheduler_config(),
        );

        let shared_request = proposal_request(2);
        let shared_proposal = engine.proposal_task_id(shared_request.clone());
        let batch_owner = RootOwner::new("proposal-batch", uuid::Uuid::new_v4());
        engine
            .attach_execution_plan(
                batch_owner,
                EngineExecutionPlan {
                    proposals: vec![proposal_request(1), shared_request.clone()],
                    aggregate: None,
                },
            )
            .await?;
        let standalone_owner = RootOwner::new("proposal-standalone", uuid::Uuid::new_v4());
        let outcome = engine
            .attach_execution_plan(
                standalone_owner.clone(),
                EngineExecutionPlan {
                    proposals: vec![shared_request],
                    aggregate: None,
                },
            )
            .await?;
        assert_eq!(outcome, AttachOutcome::Attached);

        let projection = engine
            .inspect_execution(&standalone_owner)
            .await?
            .ok_or_else(|| std::io::Error::other("expected standalone projection"))?;
        let shared = projection
            .tasks
            .iter()
            .find(|task| task.id == shared_proposal)
            .ok_or_else(|| std::io::Error::other("expected shared proposal"))?;
        assert_eq!(shared.owner_count, 2);
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

        let (_owner, job_id) = attach_proposal!(engine, "retry-none", proposal_request(1));

        assert!(Box::pin(engine.run_one("w1")).await?);

        let view = engine
            .get(job_id)
            .await?
            .ok_or_else(|| std::io::Error::other("expected task view"))?;
        assert!(matches!(view.state, TaskState::Failed { .. }));
        Ok(())
    }

    #[tokio::test]
    async fn attached_proposal_and_aggregate_tasks_use_scheduler_retry()
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
        let (_proposal_owner, proposal_id) =
            attach_proposal!(engine, "retry-policy-proposal", request.clone());

        let proposal = engine
            .inner
            .scheduler
            .next_ready("w1")
            .await?
            .ok_or_else(|| std::io::Error::other("expected proposal lease"))?;
        assert_eq!(proposal.id, proposal_id);
        assert_eq!(proposal.execution_policy, task_policy);

        let (_aggregate_owner, aggregate_id) = attach_aggregate!(
            engine,
            "retry-policy-aggregate",
            vec![],
            aggregation_request("agg"),
            vec![AggregateProofInput::ProofArtifact(proof_artifact(
                "aggregate-input"
            ))]
        );
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

        let (_owner, job_id) = attach_proposal!(engine, "retry-failed-prove", proposal_request(1));

        assert!(Box::pin(engine.run_one("w1")).await?);

        let view = engine
            .get(job_id)
            .await?
            .ok_or_else(|| std::io::Error::other("expected task view"))?;
        assert!(matches!(view.state, TaskState::Retrying { attempt: 1, .. }));
        Ok(())
    }

    #[tokio::test]
    async fn intermediate_runtime_sync_failure_retries_without_terminal_callback()
    -> Result<(), Box<dyn std::error::Error>> {
        let observer = Arc::new(RuntimeSyncFailingObserver::default());
        let engine = Engine::with_store_scheduler_config_and_observer(
            TestSpec::new(MockProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            SchedulerConfig {
                lease_duration: Duration::from_secs(60),
                retry: RetryPolicy::None,
            },
            Some(observer.clone()),
        );
        let (_owner, job_id) =
            attach_proposal!(engine, "intermediate-sync-retry", proposal_request(1));

        assert!(Box::pin(engine.run_one("w1")).await?);

        let view = engine
            .get(job_id)
            .await?
            .ok_or_else(|| std::io::Error::other("expected task view"))?;
        assert!(matches!(view.state, TaskState::Retrying { attempt: 1, .. }));
        assert_eq!(observer.stage_successes.load(Ordering::SeqCst), 1);
        assert_eq!(observer.task_failures.load(Ordering::SeqCst), 0);
        Ok(())
    }

    #[tokio::test]
    async fn proof_publication_failure_retries_durable_output_without_reproving()
    -> Result<(), Box<dyn std::error::Error>> {
        let observer = Arc::new(PublicationFailingObserver::new(1));
        let proof_runs = Arc::new(AtomicUsize::new(0));
        let engine = Engine::with_store_scheduler_config_and_observer(
            TestSpec::new(CountingProver {
                proof_runs: Arc::clone(&proof_runs),
            }),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            SchedulerConfig {
                lease_duration: Duration::from_secs(60),
                retry: RetryPolicy::None,
            },
            Some(observer.clone()),
        );
        let (_owner, job_id) = attach_proposal!(engine, "publication-retry", proposal_request(1));

        assert!(Box::pin(engine.run_one("w1")).await?);

        let view = engine
            .get(job_id.clone())
            .await?
            .ok_or_else(|| std::io::Error::other("expected task view"))?;
        assert!(matches!(view.state, TaskState::Retrying { .. }));
        assert_eq!(proof_runs.load(Ordering::SeqCst), 1);
        assert_eq!(observer.proof_successes.load(Ordering::SeqCst), 1);
        assert_eq!(observer.task_failures.load(Ordering::SeqCst), 0);
        let starts_after_proving = observer.task_starts.load(Ordering::SeqCst);

        tokio::time::sleep(Duration::from_millis(1_100)).await;
        engine.inner.scheduler.maintenance_tick().await?;
        assert!(Box::pin(engine.run_one("w2")).await?);

        let view = engine
            .get(job_id)
            .await?
            .ok_or_else(|| std::io::Error::other("expected task view"))?;
        assert!(matches!(view.state, TaskState::Succeeded { .. }));
        assert_eq!(proof_runs.load(Ordering::SeqCst), 1);
        assert_eq!(observer.proof_successes.load(Ordering::SeqCst), 2);
        assert_eq!(observer.task_failures.load(Ordering::SeqCst), 0);
        assert_eq!(
            observer.task_starts.load(Ordering::SeqCst),
            starts_after_proving,
            "publication-only retries must not reopen task execution telemetry"
        );
        Ok(())
    }

    #[tokio::test]
    async fn stale_lease_failure_does_not_overwrite_recreated_task_or_notify_observer()
    -> Result<(), Box<dyn std::error::Error>> {
        let observer = Arc::new(PublicationFailingObserver::new(0));
        let engine = Engine::with_store_scheduler_config_and_observer(
            TestSpec::new(MockProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            SchedulerConfig {
                lease_duration: Duration::from_secs(60),
                retry: RetryPolicy::None,
            },
            Some(observer.clone()),
        );
        let owner = RootOwner::new("stale-lease", uuid::Uuid::new_v4());
        let request = proposal_request(1);
        let job_id = engine.proposal_task_id(request.clone());
        engine
            .attach_execution_plan(
                owner.clone(),
                EngineExecutionPlan {
                    proposals: vec![request.clone()],
                    aggregate: None,
                },
            )
            .await?;
        let lease = engine
            .inner
            .scheduler
            .next_ready("old-worker")
            .await?
            .ok_or_else(|| std::io::Error::other("expected running lease"))?;
        engine.detach_execution(&owner, DetachMode::Remove).await?;
        let replacement_owner = RootOwner::new("stale-lease-replacement", uuid::Uuid::new_v4());
        engine
            .attach_execution_plan(
                replacement_owner,
                EngineExecutionPlan {
                    proposals: vec![request],
                    aggregate: None,
                },
            )
            .await?;
        let replacement_id = job_id.clone();
        assert_eq!(replacement_id, job_id);
        let replacement = engine
            .inner
            .scheduler
            .next_ready("old-worker")
            .await?
            .ok_or_else(|| std::io::Error::other("expected replacement lease"))?;
        assert_ne!(lease.lease_token, replacement.lease_token);

        let execution_permit = crate::TaskExecutionPermit::untracked();
        engine
            .fail_task_after_lease_loss(&lease, &lease.payload, &execution_permit)
            .await?;

        let view = engine
            .get(job_id)
            .await?
            .ok_or_else(|| std::io::Error::other("expected task view"))?;
        assert!(matches!(view.state, TaskState::Running { .. }));
        assert_eq!(observer.task_failures.load(Ordering::SeqCst), 0);
        assert!(
            engine
                .inner
                .scheduler
                .complete(replacement, Err("test cleanup".to_string()))
                .await?
        );
        Ok(())
    }

    #[tokio::test]
    async fn queue_maintenance_readiness_requires_a_fresh_successful_tick()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec::new(MockProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            Engine::<TestSpec<MockProver>>::default_scheduler_config(),
        );

        assert!(!engine.queue_maintenance_ready(Duration::from_secs(1)));
        crate::worker::Runnable::maintenance_tick(&engine)
            .await
            .map_err(std::io::Error::other)?;
        assert!(engine.queue_maintenance_ready(Duration::from_secs(1)));

        engine
            .inner
            .last_maintenance_success_ms
            .store(super::now_millis().saturating_sub(100), Ordering::Release);
        assert!(!engine.queue_maintenance_ready(Duration::from_millis(50)));
        Ok(())
    }

    #[tokio::test]
    async fn invalidated_proof_publication_is_terminal() -> Result<(), Box<dyn std::error::Error>> {
        let proof_runs = Arc::new(AtomicUsize::new(0));
        let engine = Engine::with_store_scheduler_config_and_observer(
            TestSpec::new(CountingProver {
                proof_runs: Arc::clone(&proof_runs),
            }),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            Engine::<TestSpec<CountingProver>>::default_scheduler_config(),
            Some(Arc::new(PublicationInvalidatingObserver)),
        );
        let (_owner, job_id) =
            attach_proposal!(engine, "publication-invalidated", proposal_request(1));

        assert!(Box::pin(engine.run_one("w1")).await?);

        let view = engine
            .get(job_id)
            .await?
            .ok_or_else(|| std::io::Error::other("expected task view"))?;
        assert!(matches!(view.state, TaskState::Failed { .. }));
        assert_eq!(proof_runs.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test]
    async fn terminal_failure_removes_root_owner_before_a_shared_stage_is_reused()
    -> Result<(), Box<dyn std::error::Error>> {
        let proof_runs = Arc::new(AtomicUsize::new(0));
        let owner = RootOwner::new("failed-root", uuid::Uuid::new_v4());
        let request = proposal_request(1);
        let observer = Arc::new(TerminalFailureObserver {
            owner: owner.clone(),
        });
        let engine = Engine::with_store_scheduler_config_and_observer(
            TestSpec::new(CountingProver {
                proof_runs: Arc::clone(&proof_runs),
            }),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            Engine::<TestSpec<CountingProver>>::default_scheduler_config(),
            Some(observer),
        );
        let task_id = engine.proposal_task_id(request.clone());
        let plan = EngineExecutionPlan {
            proposals: vec![request],
            aggregate: None,
        };
        engine
            .attach_execution_plan(owner.clone(), plan.clone())
            .await?;

        assert!(Box::pin(engine.run_one("w1")).await?);
        assert!(engine.inspect_execution(&owner).await?.is_none());

        let replacement = RootOwner::new("replacement-root", uuid::Uuid::new_v4());
        assert_eq!(
            engine
                .attach_execution_plan(replacement.clone(), plan)
                .await?,
            AttachOutcome::Attached
        );
        let view = engine
            .get(task_id)
            .await?
            .ok_or_else(|| std::io::Error::other("expected reattached task view"))?;
        assert!(matches!(view.state, TaskState::Ready));
        assert_eq!(proof_runs.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test]
    async fn terminal_failure_retries_until_runtime_state_is_durable()
    -> Result<(), Box<dyn std::error::Error>> {
        let owner = RootOwner::new("terminal-sync-retry", uuid::Uuid::new_v4());
        let observer = Arc::new(TerminalSyncFailingObserver {
            owner: owner.clone(),
            failures: AtomicUsize::new(0),
        });
        let engine = Engine::with_store_scheduler_config_and_observer(
            TestSpec::new(FailingProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            SchedulerConfig {
                lease_duration: Duration::from_secs(60),
                retry: RetryPolicy::None,
            },
            Some(observer.clone()),
        );
        let request = proposal_request(1);
        let task_id = engine.proposal_task_id(request.clone());
        engine
            .attach_execution_plan(
                owner.clone(),
                EngineExecutionPlan {
                    proposals: vec![request],
                    aggregate: None,
                },
            )
            .await?;

        assert!(Box::pin(engine.run_one("w1")).await?);
        let view = engine
            .get(task_id.clone())
            .await?
            .ok_or_else(|| std::io::Error::other("expected retrying task"))?;
        assert!(matches!(view.state, TaskState::Retrying { .. }));
        assert!(engine.inspect_execution(&owner).await?.is_some());

        tokio::time::sleep(Duration::from_millis(1_100)).await;
        engine.inner.scheduler.maintenance_tick().await?;
        assert!(Box::pin(engine.run_one("w2")).await?);

        assert!(engine.inspect_execution(&owner).await?.is_none());
        assert!(engine.get(task_id).await?.is_none());
        assert_eq!(observer.failures.load(Ordering::SeqCst), 2);
        Ok(())
    }

    #[tokio::test]
    async fn recovered_proposal_output_is_finalized_through_observer()
    -> Result<(), Box<dyn std::error::Error>> {
        let observer = Arc::new(RecoveringObserver {
            proof_successes: AtomicUsize::new(0),
        });
        let proof_runs = Arc::new(AtomicUsize::new(0));
        let engine = Engine::with_store_scheduler_config_and_observer(
            TestSpec::new(CountingProver {
                proof_runs: Arc::clone(&proof_runs),
            }),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            Engine::<TestSpec<CountingProver>>::default_scheduler_config(),
            Some(observer.clone()),
        );
        let (_owner, job_id) = attach_proposal!(engine, "recovered-output", proposal_request(1));

        assert!(Box::pin(engine.run_one("w1")).await?);

        let view = engine
            .get(job_id)
            .await?
            .ok_or_else(|| std::io::Error::other("expected task view"))?;
        assert!(matches!(view.state, TaskState::Succeeded { .. }));
        assert_eq!(proof_runs.load(Ordering::SeqCst), 0);
        assert_eq!(observer.proof_successes.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test]
    async fn run_one_stops_running_task_after_root_detach() -> Result<(), Box<dyn std::error::Error>>
    {
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
        let (owner, job_id) = attach_proposal!(engine, "detach-running", proposal_request(1));

        let worker_engine = engine.clone();
        let handle = tokio::spawn(async move { Box::pin(worker_engine.run_one("w1")).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        engine.detach_execution(&owner, DetachMode::Cancel).await?;

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
    async fn shared_task_notifies_cancellation_only_after_its_last_owner_detaches()
    -> Result<(), Box<dyn std::error::Error>> {
        let observer = Arc::new(CancellationObserver::default());
        let engine = Engine::with_store_scheduler_config_and_observer(
            TestSpec::new(MockProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            Engine::<TestSpec<MockProver>>::default_scheduler_config(),
            Some(observer.clone()),
        );
        let request = proposal_request(1);
        let plan = EngineExecutionPlan {
            proposals: vec![request],
            aggregate: None,
        };
        let first = RootOwner::new("first", uuid::Uuid::new_v4());
        let second = RootOwner::new("second", uuid::Uuid::new_v4());
        engine
            .attach_execution_plan(first.clone(), plan.clone())
            .await?;
        engine.attach_execution_plan(second.clone(), plan).await?;

        let first_outcome = engine.detach_execution(&first, DetachMode::Cancel).await?;
        assert!(first_outcome.retired.is_empty());
        assert_eq!(observer.retired_tasks.load(Ordering::SeqCst), 0);

        let second_outcome = engine.detach_execution(&second, DetachMode::Cancel).await?;
        assert_eq!(second_outcome.retired.len(), 1);
        assert_eq!(observer.retired_tasks.load(Ordering::SeqCst), 1);
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
                Arc::new(crate::TaskExecutionPermit::untracked()),
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
                Arc::new(crate::TaskExecutionPermit::untracked()),
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
