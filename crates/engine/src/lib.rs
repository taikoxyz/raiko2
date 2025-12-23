//! Raiko2 Engine - queue-driven proving orchestration.
//!
//! ## Module Structure
//!
//! - `queue` - Internal worker supervision helpers
//! - `worker` - Supervised worker management with auto-restart
//! - `tasks` - Task types and outputs
//!
//! Prover integration is provided via `raiko2-prover` and wired through
//! `Engine` / `tasks::EngineTask`.

#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

mod queue;
pub mod tasks;
pub mod worker;

pub use tasks::{EncodedGuestInput, EngineTaskId, EngineTaskKey, ProposalStage};

use std::sync::Arc;
use std::time::Duration;

use raiko2_pipeline::{Pipeline, PipelineSpec, PipelineStage, PipelineStageResult, ProverBackend};
use raiko2_primitives::{AggregationGuestInput, ProofContext};
use raiko2_prover::Prover;
use raiko2_provider::Provider;
use raiko2_queue::{
    MemoryStore, NewTask, Priority, RetryPolicy, Scheduler, SchedulerConfig, TaskState,
    TaskStoreError, TaskView,
};

use crate::queue::{spawn_maintenance_supervised, spawn_worker_supervised};
use crate::tasks::{EngineOutput, EngineTask};

pub struct Engine<S, B, P>
where
    S: PipelineSpec,
    B: ProverBackend,
    P: Provider,
{
    inner: Arc<Inner<S, B, P>>,
}

struct Inner<S, B, P>
where
    S: PipelineSpec,
    B: ProverBackend,
    P: Provider,
{
    spec: S,
    backend: B,
    provider: P,
    scheduler: Scheduler<EngineTask, EngineOutput<S::GuestInput>, EngineTaskKey>,
    prover: Arc<dyn Prover<B, GuestInput = S::GuestInput>>,
    context: ProofContext,
}

impl<S, B, P> Clone for Engine<S, B, P>
where
    S: PipelineSpec,
    B: ProverBackend,
    P: Provider,
{
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<S, B, P> Engine<S, B, P>
where
    S: PipelineSpec,
    B: ProverBackend,
    P: Provider,
{
    const fn default_scheduler_config() -> SchedulerConfig {
        SchedulerConfig {
            lease_duration: Duration::from_secs(60),
            retry: RetryPolicy::Exponential {
                max_attempts: 3,
                base_delay: Duration::from_secs(1),
                max_delay: Duration::from_secs(30),
            },
        }
    }

    pub fn new(
        spec: S,
        backend: B,
        provider: P,
        prover: Arc<dyn Prover<B, GuestInput = S::GuestInput>>,
        context: ProofContext,
    ) -> Self {
        Self::with_store_and_scheduler_config(
            spec,
            backend,
            provider,
            prover,
            context,
            MemoryStore::new(),
            Self::default_scheduler_config(),
        )
    }

    pub fn with_store<Store>(
        spec: S,
        backend: B,
        provider: P,
        prover: Arc<dyn Prover<B, GuestInput = S::GuestInput>>,
        context: ProofContext,
        store: Store,
    ) -> Self
    where
        Store: raiko2_queue::TaskStore<EngineTask, EngineOutput<S::GuestInput>, EngineTaskKey>
            + 'static,
    {
        Self::with_store_and_scheduler_config(
            spec,
            backend,
            provider,
            prover,
            context,
            store,
            Self::default_scheduler_config(),
        )
    }

    pub fn with_store_and_scheduler_config<Store>(
        spec: S,
        backend: B,
        provider: P,
        prover: Arc<dyn Prover<B, GuestInput = S::GuestInput>>,
        context: ProofContext,
        store: Store,
        scheduler_config: SchedulerConfig,
    ) -> Self
    where
        Store: raiko2_queue::TaskStore<EngineTask, EngineOutput<S::GuestInput>, EngineTaskKey>
            + 'static,
    {
        Self {
            inner: Arc::new(Inner {
                spec,
                backend,
                provider,
                scheduler: Scheduler::with_config(store, scheduler_config),
                prover,
                context,
            }),
        }
    }

    fn context_for_proposal(&self, proposal_id: u64) -> ProofContext {
        let mut ctx = self.inner.context.clone();
        ctx.request.proposal_id = proposal_id;
        ctx
    }

    const fn proposal_task_id(&self, proposal_id: u64, stage: ProposalStage) -> EngineTaskId {
        EngineTaskId::new(EngineTaskKey::Proposal { proposal_id, stage })
    }

    pub async fn submit_proposal_proof(
        &self,
        proposal_id: u64,
    ) -> Result<EngineTaskId, TaskStoreError> {
        let preflight_id = self.proposal_task_id(proposal_id, ProposalStage::Preflight);
        let preflight_task = self
            .inner
            .scheduler
            .submit(
                preflight_id,
                NewTask {
                    priority: Priority::Low,
                    payload: EngineTask::Preflight { proposal_id },
                },
                vec![],
            )
            .await?;

        let validation_id = self.proposal_task_id(proposal_id, ProposalStage::Validation);
        let validation_task = self
            .inner
            .scheduler
            .submit(
                validation_id,
                NewTask {
                    priority: Priority::Low,
                    payload: EngineTask::Validate {
                        proposal_id,
                        preflight_task: preflight_task.clone(),
                    },
                },
                vec![preflight_task],
            )
            .await?;

        let encode_id = self.proposal_task_id(proposal_id, ProposalStage::Encode);
        let encode_task = self
            .inner
            .scheduler
            .submit(
                encode_id,
                NewTask {
                    priority: Priority::Low,
                    payload: EngineTask::Encode {
                        proposal_id,
                        input_task: validation_task.clone(),
                    },
                },
                vec![validation_task],
            )
            .await?;

        let prove_id = self.proposal_task_id(proposal_id, ProposalStage::Prove);
        self.inner
            .scheduler
            .submit(
                prove_id,
                NewTask {
                    priority: Priority::Medium,
                    payload: EngineTask::ProveProposal {
                        proposal_id,
                        input_task: encode_task.clone(),
                    },
                },
                vec![encode_task],
            )
            .await
    }

    pub async fn get(
        &self,
        id: EngineTaskId,
    ) -> Result<Option<TaskView<EngineOutput<S::GuestInput>, EngineTaskKey>>, TaskStoreError> {
        self.inner.scheduler.get(id).await
    }

    pub async fn cancel(&self, id: EngineTaskId) -> Result<(), TaskStoreError> {
        self.inner.scheduler.cancel(id).await
    }

    pub async fn run_one(&self, worker: &str) -> Result<bool, TaskStoreError> {
        let Some(lease) = self.inner.scheduler.next_ready(worker).await? else {
            return Ok(false);
        };

        let result = self.execute(lease.payload.clone()).await;
        self.inner.scheduler.complete(lease, result).await?;
        Ok(true)
    }

    pub fn start_workers(&self, concurrency: usize)
    where
        S: 'static,
        B: 'static,
        P: 'static,
    {
        self.start_workers_with_maintenance_interval(concurrency, Duration::from_millis(200));
    }

    pub fn start_workers_with_maintenance_interval(
        &self,
        concurrency: usize,
        maintenance_interval: Duration,
    ) where
        S: 'static,
        B: 'static,
        P: 'static,
    {
        let notify = self.inner.scheduler.notifier();
        for i in 0..concurrency {
            spawn_worker_supervised(self.clone(), notify.clone(), format!("engine-{i}"));
        }

        spawn_maintenance_supervised(self.clone(), maintenance_interval);
    }

    async fn execute(&self, task: EngineTask) -> Result<EngineOutput<S::GuestInput>, String> {
        match task {
            EngineTask::Preflight { proposal_id } => {
                let ctx = self.context_for_proposal(proposal_id);
                let pipeline = Pipeline::new(&self.inner.spec);
                pipeline
                    .preflight(&ctx, &self.inner.provider)
                    .await
                    .map(|input| EngineOutput::GuestInput(Box::new(input)))
                    .map_err(|e| e.to_string())
            }
            EngineTask::Validate {
                proposal_id,
                preflight_task,
            } => {
                let input_view = self
                    .inner
                    .scheduler
                    .get(preflight_task)
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| "missing preflight task".to_string())?;

                let preflight_input = match input_view.state {
                    TaskState::Succeeded {
                        output: EngineOutput::GuestInput(input),
                    } => match input.stage {
                        PipelineStage::Preflight => input.output,
                        _ => {
                            return Err(
                                "preflight task did not produce preflight output".to_string()
                            );
                        }
                    },
                    TaskState::Succeeded { .. } => {
                        return Err("preflight task did not produce GuestInput".to_string());
                    }
                    _ => return Err("preflight task not completed".to_string()),
                };

                let ctx = self.context_for_proposal(proposal_id);
                let pipeline = Pipeline::new(&self.inner.spec);
                let validated = pipeline
                    .validate(&ctx, preflight_input)
                    .map_err(|e| e.to_string())?;
                Ok(EngineOutput::GuestInput(Box::new(validated)))
            }
            EngineTask::Encode {
                proposal_id,
                input_task,
            } => {
                let input_view = self
                    .inner
                    .scheduler
                    .get(input_task)
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| "missing input task".to_string())?;

                let guest_input = match input_view.state {
                    TaskState::Succeeded {
                        output: EngineOutput::GuestInput(input),
                    } => match input.stage {
                        PipelineStage::Validation => input.output,
                        _ => {
                            return Err(
                                "input task did not produce validated GuestInput".to_string()
                            );
                        }
                    },
                    TaskState::Succeeded { .. } => {
                        return Err("input task did not produce GuestInput".to_string());
                    }
                    _ => return Err("input task not completed".to_string()),
                };

                let ctx = self.context_for_proposal(proposal_id);
                let encoded = self
                    .inner
                    .prover
                    .encode(&guest_input, &ctx.config)
                    .map_err(|e| e.to_string())?;

                Ok(EngineOutput::EncodedInput(Box::new(
                    PipelineStageResult::new(PipelineStage::Encode, encoded),
                )))
            }
            EngineTask::ProveProposal {
                proposal_id: _,
                input_task,
            } => {
                // NOTE: This relies on the store retaining task outputs until dependents
                // have finished. Current stores do not garbage-collect outputs; any future
                // GC/TTL must ensure dependency outputs remain available.
                let input_view = self
                    .inner
                    .scheduler
                    .get(input_task)
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| "missing input task".to_string())?;

                let encoded = match input_view.state {
                    TaskState::Succeeded {
                        output: EngineOutput::EncodedInput(input),
                    } => match input.stage {
                        PipelineStage::Encode => input.output,
                        _ => {
                            return Err("input task did not produce encoded GuestInput".to_string());
                        }
                    },
                    TaskState::Succeeded { .. } => {
                        return Err("input task did not produce encoded input".to_string());
                    }
                    _ => return Err("input task not completed".to_string()),
                };

                let proof = self
                    .inner
                    .prover
                    .prove_encoded(encoded, &self.inner.context.config, &self.inner.backend)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(EngineOutput::Proof(Box::new(PipelineStageResult::new(
                    PipelineStage::Prove,
                    proof,
                ))))
            }
            EngineTask::Aggregate {
                proposal_ids: _,
                proof_tasks,
            } => {
                let mut proofs = Vec::with_capacity(proof_tasks.len());
                for proof_task in proof_tasks {
                    let view = self
                        .inner
                        .scheduler
                        .get(proof_task)
                        .await
                        .map_err(|e| e.to_string())?
                        .ok_or_else(|| "missing dependency proof task".to_string())?;
                    match view.state {
                        TaskState::Succeeded {
                            output: EngineOutput::Proof(proof),
                        } => match proof.stage {
                            PipelineStage::Prove => proofs.push(proof.output),
                            _ => {
                                return Err(
                                    "dependency task did not produce proposal proof".to_string()
                                );
                            }
                        },
                        TaskState::Succeeded { .. } => {
                            return Err("dependency task did not produce Proof".to_string());
                        }
                        _ => return Err("dependency task not completed".to_string()),
                    }
                }

                let proof = self
                    .inner
                    .prover
                    .aggregate(
                        AggregationGuestInput { proofs },
                        &self.inner.context.config,
                        &self.inner.backend,
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
