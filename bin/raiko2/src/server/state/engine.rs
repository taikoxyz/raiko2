use raiko2_engine::{Engine, EngineExecutionPlan, EngineTaskId, EngineTaskKey};
use raiko2_queue::{
    AttachOutcome, DetachMode, DetachOutcome, RootOwner, TaskState, TaskStateKind, TaskStoreError,
    TaskView,
};
use std::future::Future;
use std::pin::Pin;

use super::types::{EngineStatusView, ProofStatus};

type EngineOutput<I> = raiko2_engine::tasks::EngineOutput<I>;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Engine abstraction used by the HTTP server.
pub trait EngineHandle: Send + Sync {
    fn start_workers(&self, _workers: usize, _maintenance_interval: std::time::Duration) {}

    fn queue_maintenance_ready(&self, _max_age: std::time::Duration) -> bool {
        true
    }
    fn get_status(
        &self,
        id: EngineTaskId,
    ) -> BoxFuture<'_, Result<Option<EngineStatusView>, TaskStoreError>>;
    fn get_task_state(
        &self,
        id: EngineTaskId,
    ) -> BoxFuture<'_, Result<Option<EngineQueueTaskView>, TaskStoreError>>;
    fn has_active_execution(&self, owner: RootOwner)
    -> BoxFuture<'_, Result<bool, TaskStoreError>>;
    fn attach_execution_plan(
        &self,
        owner: RootOwner,
        plan: EngineExecutionPlan,
    ) -> BoxFuture<'_, Result<AttachOutcome, TaskStoreError>>;
    fn detach_execution(
        &self,
        owner: RootOwner,
        mode: DetachMode,
    ) -> BoxFuture<'_, Result<DetachOutcome<EngineTaskKey>, TaskStoreError>>;
    fn shutdown(&self) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EngineQueueTaskState {
    Pending,
    Ready,
    Retrying,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug)]
pub(crate) struct EngineQueueTaskView {
    pub(crate) state: EngineQueueTaskState,
}

const fn queue_task_state(state: TaskStateKind) -> EngineQueueTaskState {
    match state {
        TaskStateKind::Pending => EngineQueueTaskState::Pending,
        TaskStateKind::Ready => EngineQueueTaskState::Ready,
        TaskStateKind::Retrying => EngineQueueTaskState::Retrying,
        TaskStateKind::Running => EngineQueueTaskState::Running,
        TaskStateKind::Succeeded => EngineQueueTaskState::Succeeded,
        TaskStateKind::Failed => EngineQueueTaskState::Failed,
        TaskStateKind::Cancelled => EngineQueueTaskState::Cancelled,
    }
}

fn summarize_task<I>(view: TaskView<EngineOutput<I>, EngineTaskKey>) -> EngineStatusView {
    match view.state {
        TaskState::Pending { .. } | TaskState::Ready => EngineStatusView {
            status: ProofStatus::Pending,
            proof: None,
            error: None,
            extra_data: None,
        },
        TaskState::Retrying { error, .. } => EngineStatusView {
            status: ProofStatus::Pending,
            proof: None,
            error: Some(error),
            extra_data: None,
        },
        TaskState::Running { .. } => EngineStatusView {
            status: ProofStatus::Proving,
            proof: None,
            error: None,
            extra_data: None,
        },
        TaskState::Succeeded { .. } => EngineStatusView {
            status: ProofStatus::Completed,
            proof: None,
            error: None,
            extra_data: None,
        },
        TaskState::Failed { error, .. } => EngineStatusView {
            status: ProofStatus::Failed,
            proof: None,
            error: Some(error),
            extra_data: None,
        },
        TaskState::Cancelled => EngineStatusView {
            status: ProofStatus::Cancelled,
            proof: None,
            error: None,
            extra_data: None,
        },
    }
}

impl<S> EngineHandle for Engine<S>
where
    S: raiko2_pipeline::PipelineSpec + Send + Sync + 'static,
    S::Prover: raiko2_prover::Prover<S::Backend, GuestInput = S::GuestInput> + 'static,
    S::Backend: raiko2_pipeline::ProverBackend + 'static,
    S::Provider: raiko2_provider::Provider + 'static,
{
    fn start_workers(&self, workers: usize, maintenance_interval: std::time::Duration) {
        self.start_workers_with_maintenance_interval(workers, maintenance_interval);
    }

    fn queue_maintenance_ready(&self, max_age: std::time::Duration) -> bool {
        Engine::queue_maintenance_ready(self, max_age)
    }

    fn get_status(
        &self,
        id: EngineTaskId,
    ) -> BoxFuture<'_, Result<Option<EngineStatusView>, TaskStoreError>> {
        Box::pin(async move {
            let view = self.get(id).await?;
            Ok(view.map(summarize_task))
        })
    }

    fn get_task_state(
        &self,
        id: EngineTaskId,
    ) -> BoxFuture<'_, Result<Option<EngineQueueTaskView>, TaskStoreError>> {
        Box::pin(async move {
            Engine::get_task_state(self, id).await.map(|view| {
                view.map(|view| EngineQueueTaskView {
                    state: queue_task_state(view.state),
                })
            })
        })
    }

    fn has_active_execution(
        &self,
        owner: RootOwner,
    ) -> BoxFuture<'_, Result<bool, TaskStoreError>> {
        Box::pin(async move {
            Ok(self
                .inspect_execution(&owner)
                .await?
                .is_some_and(|projection| {
                    projection.tasks.iter().any(|task| {
                        matches!(
                            task.state,
                            TaskStateKind::Pending
                                | TaskStateKind::Ready
                                | TaskStateKind::Retrying
                                | TaskStateKind::Running
                        )
                    })
                }))
        })
    }

    fn attach_execution_plan(
        &self,
        owner: RootOwner,
        plan: EngineExecutionPlan,
    ) -> BoxFuture<'_, Result<AttachOutcome, TaskStoreError>> {
        Box::pin(async move { self.attach_execution_plan(owner, plan).await })
    }

    fn detach_execution(
        &self,
        owner: RootOwner,
        mode: DetachMode,
    ) -> BoxFuture<'_, Result<DetachOutcome<EngineTaskKey>, TaskStoreError>> {
        Box::pin(async move { self.detach_execution(&owner, mode).await })
    }

    fn shutdown(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move { self.shutdown_workers().await })
    }
}

#[cfg(test)]
mod tests {
    use super::summarize_task;
    use crate::server::state::types::ProofStatus;
    use raiko2_engine::tasks::EngineOutput;
    use raiko2_engine::{EngineTaskId, EngineTaskKey, ProposalTaskRequest, ProverTaskConfig};
    use raiko2_pipeline::{PipelineKey, PipelineStage, PipelineStageResult};
    use raiko2_primitives::Proof;
    use raiko2_primitives_shasta::GuestInput;
    use raiko2_queue::{Priority, TaskState, TaskView};
    use serde_json::json;

    #[test]
    fn summarize_task_does_not_expose_completed_queue_proof() {
        let proof = Proof {
            proof: Some("0xdeadbeef".to_string()),
            extra_data: Some(json!({
                "zkvm": "risc0",
                "mode": "mock",
                "total_cycles": 42
            })),
            ..Proof::default()
        };
        let output: EngineOutput<GuestInput> = EngineOutput::Proof(Box::new(
            PipelineStageResult::new(PipelineStage::Prove, proof),
        ));
        let task = TaskView {
            id: EngineTaskId::new(EngineTaskKey::Proposal {
                pipeline: PipelineKey::Risc0Local,
                request: ProposalTaskRequest {
                    proposal_id: 1,
                    l2_block_range: None,
                    l1_inclusion_block_number: 0,
                    last_anchor_block_number: 0,
                    checkpoint: None,
                    blob_proof_type: None,
                    prover: None,
                    graffiti: None,
                    prover_config: ProverTaskConfig::default(),
                },
            }),
            state: TaskState::Succeeded { output },
            priority: Priority::Medium,
        };

        let summary = summarize_task(task);
        assert!(matches!(summary.status, ProofStatus::Completed));
        assert!(summary.proof.is_none());
        assert!(summary.extra_data.is_none());
    }
}
