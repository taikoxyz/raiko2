use raiko2_engine::{
    AggregationTaskRequest, Engine, EngineTaskId, EngineTaskKey, ProposalTaskRequest,
};
use raiko2_queue::{TaskState, TaskStoreError, TaskView};
use std::future::Future;
use std::pin::Pin;

use super::types::{EngineStatusView, ProofStatus};

type EngineOutput<I> = raiko2_engine::tasks::EngineOutput<I>;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Engine abstraction used by the HTTP server.
pub trait EngineHandle: Send + Sync {
    fn submit_proposal_proof_with_dependencies(
        &self,
        request: ProposalTaskRequest,
        dependencies: Vec<EngineTaskId>,
    ) -> BoxFuture<'_, Result<EngineTaskId, TaskStoreError>>;
    fn submit_aggregation_proof_from_tasks(
        &self,
        request: AggregationTaskRequest,
        proof_tasks: Vec<EngineTaskId>,
    ) -> BoxFuture<'_, Result<EngineTaskId, TaskStoreError>>;
    fn submit_aggregation_proof_from_proofs(
        &self,
        request: AggregationTaskRequest,
        proofs: Vec<raiko2_primitives::Proof>,
    ) -> BoxFuture<'_, Result<EngineTaskId, TaskStoreError>>;
    fn get_status(
        &self,
        id: EngineTaskId,
    ) -> BoxFuture<'_, Result<Option<EngineStatusView>, TaskStoreError>>;
    fn cancel(&self, id: EngineTaskId) -> BoxFuture<'_, Result<(), TaskStoreError>>;
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
        TaskState::Succeeded { output } => {
            let (proof, extra_data) = match output {
                EngineOutput::Proof(proof) => (proof.output.proof, proof.output.extra_data),
                _ => (None, None),
            };
            EngineStatusView {
                status: ProofStatus::Completed,
                proof,
                error: None,
                extra_data,
            }
        }
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
    fn submit_proposal_proof_with_dependencies(
        &self,
        request: ProposalTaskRequest,
        dependencies: Vec<EngineTaskId>,
    ) -> BoxFuture<'_, Result<EngineTaskId, TaskStoreError>> {
        Box::pin(async move {
            self.submit_proposal_proof_with_dependencies(request, dependencies)
                .await
        })
    }

    fn submit_aggregation_proof_from_tasks(
        &self,
        request: AggregationTaskRequest,
        proof_tasks: Vec<EngineTaskId>,
    ) -> BoxFuture<'_, Result<EngineTaskId, TaskStoreError>> {
        Box::pin(async move {
            self.submit_aggregation_proof_from_tasks(request, proof_tasks)
                .await
        })
    }

    fn submit_aggregation_proof_from_proofs(
        &self,
        request: AggregationTaskRequest,
        proofs: Vec<raiko2_primitives::Proof>,
    ) -> BoxFuture<'_, Result<EngineTaskId, TaskStoreError>> {
        Box::pin(async move {
            self.submit_aggregation_proof_from_proofs(request, proofs)
                .await
        })
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

    fn cancel(&self, id: EngineTaskId) -> BoxFuture<'_, Result<(), TaskStoreError>> {
        Box::pin(async move { self.cancel(id).await })
    }
}

#[cfg(test)]
mod tests {
    use super::summarize_task;
    use crate::server::state::types::ProofStatus;
    use raiko2_engine::tasks::EngineOutput;
    use raiko2_engine::{EngineTaskId, EngineTaskKey, ProposalStage, ProposalTaskRequest};
    use raiko2_pipeline::{PipelineKey, PipelineStage, PipelineStageResult};
    use raiko2_primitives::Proof;
    use raiko2_primitives_shasta::GuestInput;
    use raiko2_queue::{Priority, TaskState, TaskView};
    use serde_json::json;

    #[test]
    fn summarize_task_keeps_extra_data_for_completed_proof() {
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
                pipeline: PipelineKey::ShastaRisc0,
                request: ProposalTaskRequest {
                    proposal_id: 1,
                    l2_block_range: None,
                    l1_inclusion_block_number: 0,
                    last_anchor_block_number: 0,
                    blob_proof_type: None,
                    prover: None,
                    graffiti: None,
                    prover_args_json: None,
                },
                stage: ProposalStage::Prove,
            }),
            state: TaskState::Succeeded { output },
            priority: Priority::Medium,
        };

        let summary = summarize_task(task);
        assert!(matches!(summary.status, ProofStatus::Completed));
        assert_eq!(summary.proof.as_deref(), Some("0xdeadbeef"));
        assert_eq!(
            summary.extra_data,
            Some(json!({
                "zkvm": "risc0",
                "mode": "mock",
                "total_cycles": 42
            }))
        );
    }
}
