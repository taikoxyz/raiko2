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
    AggregationSource, AggregationTaskRequest, EncodedGuestInput, EngineTaskId, EngineTaskKey,
    ProposalStage, ProposalTaskRequest, ProverTaskConfig,
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
    TaskState, TaskStoreError, TaskView,
};

use crate::tasks::{EngineOutput, EngineTask};

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
    ) {
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
            task_timeout: Duration::from_secs(7_200),
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

    fn proposal_task_id(&self, request: ProposalTaskRequest, stage: ProposalStage) -> EngineTaskId {
        EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: self.inner.spec.pipeline_key(),
            request,
            stage,
        })
    }

    fn aggregate_task_id(&self, request: AggregationTaskRequest) -> EngineTaskId {
        EngineTaskId::new(EngineTaskKey::Aggregate {
            pipeline: self.inner.spec.pipeline_key(),
            request,
        })
    }

    fn stage_execution_policy(&self) -> TaskExecutionPolicy {
        TaskExecutionPolicy {
            lease_duration: self.inner.scheduler.config().lease_duration,
            retry: RetryPolicy::None,
        }
    }

    fn externally_stateful_stage_execution_policy(&self) -> TaskExecutionPolicy {
        let config = self.inner.scheduler.config();
        TaskExecutionPolicy {
            lease_duration: config.lease_duration,
            retry: RetryPolicy::None,
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
        let preflight_id = self.proposal_task_id(request.clone(), ProposalStage::Preflight);
        let preflight_task = self
            .inner
            .scheduler
            .submit_with_execution_policy(
                preflight_id,
                NewTask {
                    priority: Priority::Low,
                    payload: EngineTask::Preflight {
                        request: request.clone(),
                    },
                },
                dependencies,
                self.stage_execution_policy(),
            )
            .await?;

        let validation_id = self.proposal_task_id(request.clone(), ProposalStage::Validation);
        let validation_task = self
            .inner
            .scheduler
            .submit_with_execution_policy(
                validation_id,
                NewTask {
                    priority: Priority::Low,
                    payload: EngineTask::Validate {
                        request: request.clone(),
                        preflight_task: preflight_task.clone(),
                    },
                },
                vec![preflight_task],
                self.stage_execution_policy(),
            )
            .await?;

        let encode_id = self.proposal_task_id(request.clone(), ProposalStage::Encode);
        let encode_task = self
            .inner
            .scheduler
            .submit_with_execution_policy(
                encode_id,
                NewTask {
                    priority: Priority::Low,
                    payload: EngineTask::Encode {
                        request: request.clone(),
                        input_task: validation_task.clone(),
                    },
                },
                vec![validation_task],
                self.stage_execution_policy(),
            )
            .await?;

        let prove_id = self.proposal_task_id(request.clone(), ProposalStage::Prove);
        self.inner
            .scheduler
            .submit_with_execution_policy(
                prove_id,
                NewTask {
                    priority: Priority::Medium,
                    payload: EngineTask::ProveProposal {
                        request,
                        input_task: encode_task.clone(),
                    },
                },
                vec![encode_task],
                self.externally_stateful_stage_execution_policy(),
            )
            .await
    }

    /// # Errors
    ///
    /// Returns an error if the task store cannot enqueue the aggregation task or if any proof
    /// task id does not point to a proposal prove stage in this pipeline.
    pub async fn submit_aggregation_proof(
        &self,
        proof_tasks: Vec<EngineTaskId>,
    ) -> Result<EngineTaskId, TaskStoreError> {
        if proof_tasks.is_empty() {
            return Err(TaskStoreError::corrupt_msg(
                "aggregation requires at least 1 proof task",
            ));
        }

        let mut proposal_ids = Vec::with_capacity(proof_tasks.len());
        for proof_task in &proof_tasks {
            match &proof_task.0 {
                EngineTaskKey::Proposal {
                    pipeline,
                    request,
                    stage: ProposalStage::Prove,
                } if *pipeline == self.inner.spec.pipeline_key() => {
                    proposal_ids.push(request.proposal_id);
                }
                EngineTaskKey::Proposal { stage, .. } => {
                    return Err(TaskStoreError::corrupt_msg(format!(
                        "aggregation input must reference proposal prove tasks, got {stage:?}"
                    )));
                }
                EngineTaskKey::Aggregate { .. } => {
                    return Err(TaskStoreError::corrupt_msg(
                        "aggregation input cannot reference an aggregate task",
                    ));
                }
            }
        }

        let request = AggregationTaskRequest {
            request_id: proposal_ids
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join("-"),
            proposal_ids,
            prover_config: ProverTaskConfig::default(),
        };
        self.submit_aggregation_proof_from_tasks(request, proof_tasks)
            .await
    }

    /// # Errors
    ///
    /// Returns an error if the task store cannot enqueue the aggregation task or if any proof
    /// task id does not point to a proposal prove stage in this pipeline.
    pub async fn submit_aggregation_proof_from_tasks(
        &self,
        request: AggregationTaskRequest,
        proof_tasks: Vec<EngineTaskId>,
    ) -> Result<EngineTaskId, TaskStoreError> {
        if proof_tasks.is_empty() {
            return Err(TaskStoreError::corrupt_msg(
                "aggregation requires at least 1 proof task",
            ));
        }

        for proof_task in &proof_tasks {
            match &proof_task.0 {
                EngineTaskKey::Proposal {
                    pipeline,
                    stage: ProposalStage::Prove,
                    ..
                } if *pipeline == self.inner.spec.pipeline_key() => {}
                EngineTaskKey::Proposal { stage, .. } => {
                    return Err(TaskStoreError::corrupt_msg(format!(
                        "aggregation input must reference proposal prove tasks, got {stage:?}"
                    )));
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
                    priority: Priority::High,
                    payload: EngineTask::Aggregate {
                        request,
                        source: AggregationSource::ProofTasks(proof_tasks.clone()),
                    },
                },
                proof_tasks,
                self.externally_stateful_stage_execution_policy(),
            )
            .await
    }

    /// # Errors
    ///
    /// Returns an error if the task store cannot enqueue the aggregation task or if the proof set
    /// is invalid.
    pub async fn submit_aggregation_proof_from_proofs(
        &self,
        request: AggregationTaskRequest,
        proofs: Vec<Proof>,
    ) -> Result<EngineTaskId, TaskStoreError> {
        if proofs.is_empty() {
            return Err(TaskStoreError::corrupt_msg(
                "aggregation requires at least 1 proof",
            ));
        }

        let aggregate_id = self.aggregate_task_id(request.clone());
        self.inner
            .scheduler
            .submit_with_execution_policy(
                aggregate_id,
                NewTask {
                    priority: Priority::High,
                    payload: EngineTask::Aggregate {
                        request,
                        source: AggregationSource::Proofs(proofs),
                    },
                },
                Vec::new(),
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
        let task_timeout = self.inner.scheduler.config().task_timeout;
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

        if let Some(observer) = &self.inner.observer {
            observer.on_task_started(&lease.id, &payload, worker).await;
        }

        let remaining_timeout = remaining_task_timeout(lease.deadline_at_ms);
        let result = if remaining_timeout.is_zero() {
            Err(task_timeout_error(task_timeout))
        } else {
            self.execute_with_task_controls(&lease.id, &lease, payload.clone(), remaining_timeout)
                .await
        };
        renew_task.abort();
        if let Err(err) = &result {
            tracing::warn!(worker = %worker, task = ?payload, error = %err, "engine task failed");
        }
        let success = result.as_ref().ok().map(task_success_from_output);
        let error = result.as_ref().err().cloned();
        let completed_id = lease.id.clone();
        let completed = self.inner.scheduler.complete(lease, result).await?;
        if completed && let Some(observer) = &self.inner.observer {
            if let Some(success) = success.as_ref() {
                observer
                    .on_task_succeeded(&completed_id, &payload, success)
                    .await;
            } else if let Some(error) = error.as_deref() {
                observer
                    .on_task_failed(&completed_id, &payload, error)
                    .await;
            }
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
        remaining_timeout: Duration,
    ) -> Result<EngineOutput<S::GuestInput>, String> {
        let execute = self.execute(task_id, payload);
        let interrupted = self.wait_lease_interruption(&lease.id, &lease.worker, lease.attempt);
        tokio::pin!(execute);
        tokio::pin!(interrupted);

        tokio::select! {
            result = &mut execute => result,
            () = tokio::time::sleep(remaining_timeout) => {
                Err(task_timeout_error(self.inner.scheduler.config().task_timeout))
            }
            interruption = &mut interrupted => {
                match interruption {
                    Ok(LeaseInterruption::Cancelled) => Err(task_cancelled_error()),
                    Ok(LeaseInterruption::Lost) => Err(task_lease_lost_error()),
                    Err(err) => Err(err.to_string()),
                }
            }
        }
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

    async fn get_proof(&self, id: EngineTaskId) -> Result<raiko2_primitives::Proof, String> {
        let view = self
            .get_view_or_err(id, || "missing dependency proof task".to_string())
            .await?;

        match view.state {
            TaskState::Succeeded {
                output: EngineOutput::Proof(proof),
            } => {
                if proof.stage == PipelineStage::Prove {
                    Ok(proof.output)
                } else {
                    Err("dependency task did not produce proposal proof".to_string())
                }
            }
            TaskState::Succeeded { .. } => Err("dependency task did not produce Proof".to_string()),
            _ => Err("dependency task not completed".to_string()),
        }
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

    async fn execute(
        &self,
        task_id: &EngineTaskId,
        task: EngineTask,
    ) -> Result<EngineOutput<S::GuestInput>, String> {
        match task {
            EngineTask::Preflight { request } => {
                let ctx = self.context_for_proposal(&request);
                let pipeline = Pipeline::new(&self.inner.spec);
                pipeline
                    .preflight(&ctx)
                    .await
                    .map(|input| EngineOutput::GuestInput(Box::new(input)))
                    .map_err(|e| e.to_string())
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
            } => self.prove_proposal(task_id, request, input_task).await,
            EngineTask::Aggregate { request, source } => {
                let ctx = self.context_for_aggregation(&request);
                let progress_task = EngineTask::Aggregate {
                    request: request.clone(),
                    source: source.clone(),
                };
                let proofs = match source {
                    AggregationSource::ProofTasks(proof_tasks) => {
                        let mut proofs = Vec::with_capacity(proof_tasks.len());
                        for proof_task in proof_tasks {
                            proofs.push(self.get_proof(proof_task).await?);
                        }
                        proofs
                    }
                    AggregationSource::Proofs(proofs) => proofs,
                };
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

fn task_timeout_error(task_timeout: Duration) -> String {
    format!("task timed out after {}ms", task_timeout.as_millis())
}

fn task_cancelled_error() -> String {
    "task cancelled".to_string()
}

fn task_lease_lost_error() -> String {
    "task lease lost before completion".to_string()
}

fn remaining_task_timeout(deadline_at_ms: u64) -> Duration {
    let now_ms = now_millis();
    if deadline_at_ms <= now_ms {
        Duration::ZERO
    } else {
        Duration::from_millis(deadline_at_ms - now_ms)
    }
}

fn now_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or_default()
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
    use std::time::Duration;

    use alloy_primitives::Bytes;
    use raiko2_pipeline::{
        NoopManifestBuilder, NoopValidation, PipelineKey, PipelineSpec, PipelineStage,
        PipelineStageResult, Preflight, ProofStage, ProverBackend,
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
        AggregationTaskRequest, EngineOutput, ProposalTaskRequest, ProverTaskConfig,
    };
    use crate::{Engine, EngineTaskId, EngineTaskKey, ProposalStage};

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
        fn elf(&self, _stage: ProofStage) -> RaikoResult<&'static [u8]> {
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

    fn boundless_test_engine(scheduler_config: SchedulerConfig) -> Engine<TestSpec<MockProver>> {
        Engine::with_store_and_scheduler_config(
            TestSpec::new(MockProver).with_pipeline_key(PipelineKey::ShastaRisc0Boundless),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            scheduler_config,
        )
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

        assert!(engine.run_one("w1").await?);
        assert!(engine.run_one("w1").await?);
        assert!(engine.run_one("w1").await?);
        assert!(engine.run_one("w1").await?);
        assert!(!engine.run_one("w1").await?);

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

        let first = engine.proposal_task_id(proposal_request(1), ProposalStage::Prove);
        let second = engine.proposal_task_id(proposal_request(2), ProposalStage::Prove);
        let aggregate_id = engine
            .submit_aggregation_proof(vec![first.clone(), second.clone()])
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
                request: AggregationTaskRequest {
                    request_id: "1-2".to_string(),
                    proposal_ids: vec![1, 2],
                    prover_config: ProverTaskConfig::default(),
                },
            })
        );
        Ok(())
    }

    #[tokio::test]
    async fn submit_aggregation_proof_accepts_single_prove_task()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec::new(MockProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            Engine::<TestSpec<MockProver>>::default_scheduler_config(),
        );

        let proof_task = engine.proposal_task_id(proposal_request(1), ProposalStage::Prove);
        let aggregate_id = engine
            .submit_aggregation_proof(vec![proof_task.clone()])
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
                request: AggregationTaskRequest {
                    request_id: "1".to_string(),
                    proposal_ids: vec![1],
                    prover_config: ProverTaskConfig::default(),
                },
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
            .submit_aggregation_proof(Vec::new())
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("aggregation requires at least 1 proof task")
        );
        Ok(())
    }

    #[tokio::test]
    async fn submit_aggregation_proof_rejects_non_prove_task()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec::new(MockProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            Engine::<TestSpec<MockProver>>::default_scheduler_config(),
        );

        let err = engine
            .submit_aggregation_proof(vec![
                engine.proposal_task_id(proposal_request(1), ProposalStage::Validation),
            ])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("proposal prove tasks"));

        let err = engine
            .submit_aggregation_proof(vec![
                engine.proposal_task_id(proposal_request(1), ProposalStage::Validation),
                engine.proposal_task_id(proposal_request(2), ProposalStage::Prove),
            ])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("proposal prove tasks"));
        Ok(())
    }

    #[tokio::test]
    async fn submit_proposal_proof_with_dependencies_delays_next_preflight()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec::new(MockProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            Engine::<TestSpec<MockProver>>::default_scheduler_config(),
        );

        let first_prove = engine.submit_proposal_proof(proposal_request(1)).await?;
        let second_request = proposal_request(2);
        let second_preflight =
            engine.proposal_task_id(second_request.clone(), ProposalStage::Preflight);
        engine
            .submit_proposal_proof_with_dependencies(second_request, vec![first_prove])
            .await?;

        let ready = engine
            .inner
            .scheduler
            .next_ready("w1")
            .await?
            .ok_or_else(|| std::io::Error::other("expected ready task"))?;
        assert_eq!(
            ready.id,
            engine.proposal_task_id(proposal_request(1), ProposalStage::Preflight)
        );

        let second_view = engine
            .get(second_preflight)
            .await?
            .ok_or_else(|| std::io::Error::other("expected second task view"))?;
        assert!(matches!(second_view.state, TaskState::Pending { .. }));
        Ok(())
    }

    #[tokio::test]
    async fn retry_policy_none_fails_task_immediately() -> Result<(), Box<dyn std::error::Error>> {
        let scheduler_config = SchedulerConfig {
            lease_duration: Duration::from_secs(60),
            task_timeout: Duration::from_secs(60),
            retry: RetryPolicy::None,
        };
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec::new(FailingProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            scheduler_config,
        );

        let job_id = engine.submit_proposal_proof(proposal_request(1)).await?;

        assert!(engine.run_one("w1").await?);
        assert!(engine.run_one("w1").await?);
        assert!(engine.run_one("w1").await?);
        assert!(engine.run_one("w1").await?);

        let view = engine
            .get(job_id)
            .await?
            .ok_or_else(|| std::io::Error::other("expected task view"))?;
        assert!(matches!(view.state, TaskState::Failed { .. }));
        Ok(())
    }

    #[tokio::test]
    async fn submitted_stage_tasks_disable_queue_retry() -> Result<(), Box<dyn std::error::Error>> {
        let scheduler_config = SchedulerConfig {
            lease_duration: Duration::from_secs(45),
            task_timeout: Duration::from_secs(300),
            retry: RetryPolicy::Fixed {
                max_attempts: 4,
                delay: Duration::from_millis(25),
            },
        };
        let engine = boundless_test_engine(scheduler_config.clone());
        let local_stage_policy = raiko2_queue::TaskExecutionPolicy {
            lease_duration: scheduler_config.lease_duration,
            retry: RetryPolicy::None,
        };
        let remote_stage_policy = raiko2_queue::TaskExecutionPolicy {
            lease_duration: scheduler_config.lease_duration,
            retry: RetryPolicy::None,
        };
        let request = proposal_request(9);
        let prove_id = engine.submit_proposal_proof(request.clone()).await?;

        let preflight = engine
            .inner
            .scheduler
            .next_ready("w1")
            .await?
            .ok_or_else(|| std::io::Error::other("expected preflight lease"))?;
        assert_eq!(preflight.execution_policy, local_stage_policy);
        engine
            .inner
            .scheduler
            .complete(
                preflight,
                Ok(EngineOutput::GuestInput(Box::new(
                    PipelineStageResult::new(PipelineStage::Preflight, GuestInput::default()),
                ))),
            )
            .await?;

        let validation = engine
            .inner
            .scheduler
            .next_ready("w1")
            .await?
            .ok_or_else(|| std::io::Error::other("expected validation lease"))?;
        assert_eq!(validation.execution_policy, local_stage_policy);
        engine
            .inner
            .scheduler
            .complete(
                validation,
                Ok(EngineOutput::GuestInput(Box::new(
                    PipelineStageResult::new(PipelineStage::Validation, GuestInput::default()),
                ))),
            )
            .await?;

        let encode = engine
            .inner
            .scheduler
            .next_ready("w1")
            .await?
            .ok_or_else(|| std::io::Error::other("expected encode lease"))?;
        assert_eq!(encode.execution_policy, local_stage_policy);
        engine
            .inner
            .scheduler
            .complete(
                encode,
                Ok(EngineOutput::EncodedInput(Box::new(
                    PipelineStageResult::new(PipelineStage::Encode, Bytes::from_static(&[1])),
                ))),
            )
            .await?;

        let prove = engine
            .inner
            .scheduler
            .next_ready("w1")
            .await?
            .ok_or_else(|| std::io::Error::other("expected prove lease"))?;
        assert_eq!(prove.id, prove_id);
        assert_eq!(prove.execution_policy, remote_stage_policy);

        let aggregate_id = engine
            .submit_aggregation_proof_from_proofs(
                aggregation_request("agg"),
                vec![Proof::default()],
            )
            .await?;
        let aggregate = engine
            .inner
            .scheduler
            .next_ready("w1")
            .await?
            .ok_or_else(|| std::io::Error::other("expected aggregate lease"))?;
        assert_eq!(aggregate.id, aggregate_id);
        assert_eq!(aggregate.execution_policy, remote_stage_policy);
        Ok(())
    }

    #[tokio::test]
    async fn failed_prove_stage_ignores_scheduler_retry_policy()
    -> Result<(), Box<dyn std::error::Error>> {
        let scheduler_config = SchedulerConfig {
            lease_duration: Duration::from_secs(60),
            task_timeout: Duration::from_secs(60),
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

        assert!(engine.run_one("w1").await?);
        assert!(engine.run_one("w1").await?);
        assert!(engine.run_one("w1").await?);
        assert!(engine.run_one("w1").await?);
        assert!(!engine.run_one("w1").await?);

        let view = engine
            .get(job_id)
            .await?
            .ok_or_else(|| std::io::Error::other("expected task view"))?;
        assert!(matches!(view.state, TaskState::Failed { .. }));
        Ok(())
    }

    #[tokio::test]
    async fn run_one_applies_scheduler_task_timeout() -> Result<(), Box<dyn std::error::Error>> {
        let scheduler_config = SchedulerConfig {
            lease_duration: Duration::from_secs(60),
            task_timeout: Duration::from_millis(100),
            retry: RetryPolicy::None,
        };
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec::new(SlowProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            scheduler_config,
        );
        let job_id = engine.submit_proposal_proof(proposal_request(1)).await?;

        assert!(engine.run_one("w1").await?);
        assert!(engine.run_one("w1").await?);
        assert!(engine.run_one("w1").await?);
        assert!(engine.run_one("w1").await?);

        let view = engine
            .get(job_id)
            .await?
            .ok_or_else(|| std::io::Error::other("expected task view"))?;
        match view.state {
            TaskState::Failed { error, .. } => {
                assert!(error.contains("task timed out after 100ms"));
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
    async fn run_one_stops_running_task_after_cancel() -> Result<(), Box<dyn std::error::Error>> {
        let scheduler_config = SchedulerConfig {
            lease_duration: Duration::from_secs(60),
            task_timeout: Duration::from_secs(30),
            retry: RetryPolicy::None,
        };
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec::new(SlowProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            scheduler_config,
        );
        let job_id = engine.submit_proposal_proof(proposal_request(1)).await?;

        assert!(engine.run_one("w1").await?);
        assert!(engine.run_one("w1").await?);
        assert!(engine.run_one("w1").await?);

        let worker_engine = engine.clone();
        let handle = tokio::spawn(async move { worker_engine.run_one("w1").await });
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
        use crate::tasks::{EngineTask, EngineTaskId, EngineTaskKey, ProposalStage};
        use raiko2_pipeline::{PipelineStage, PipelineStageResult};
        use raiko2_queue::{NewTask, Priority};

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
            stage: ProposalStage::Preflight,
        });

        // Manually submit and complete a preflight task with WRONG stage output (e.g., Validation)
        engine
            .inner
            .scheduler
            .submit(
                preflight_id.clone(),
                NewTask {
                    priority: Priority::Low,
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
                    stage: ProposalStage::Validation,
                }),
                EngineTask::Validate {
                    request,
                    preflight_task: preflight_id,
                },
            )
            .await;

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "preflight task did not produce preflight output"
        );
        Ok(())
    }

    #[tokio::test]
    async fn encode_requires_validated_guest_input() -> Result<(), Box<dyn std::error::Error>> {
        use crate::tasks::{EngineTask, EngineTaskId, EngineTaskKey, ProposalStage};
        use raiko2_pipeline::{PipelineStage, PipelineStageResult};
        use raiko2_queue::{NewTask, Priority};

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
            stage: ProposalStage::Validation,
        });

        // Manually submit and complete a validation task with WRONG stage output (e.g., Preflight)
        engine
            .inner
            .scheduler
            .submit(
                validation_id.clone(),
                NewTask {
                    priority: Priority::Low,
                    payload: EngineTask::Validate {
                        request: request.clone(),
                        preflight_task: EngineTaskId::new(EngineTaskKey::Proposal {
                            pipeline: raiko2_pipeline::PipelineKey::ShastaNative,
                            request: request.clone(),
                            stage: ProposalStage::Preflight,
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
                    stage: ProposalStage::Encode,
                }),
                EngineTask::Encode {
                    request,
                    input_task: validation_id,
                },
            )
            .await;

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "input task did not produce validated GuestInput"
        );
        Ok(())
    }
}
