use raiko2_engine::{Engine, EngineTaskId, EngineTaskKey};
use raiko2_queue::{TaskState, TaskStoreError, TaskView};
use std::future::Future;
use std::pin::Pin;

use super::types::{EngineStatusView, ProofStatus};

type EngineOutput<I> = raiko2_engine::tasks::EngineOutput<I>;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Engine abstraction used by the HTTP server.
pub trait EngineHandle: Send + Sync {
    fn submit_proposal_proof(
        &self,
        proposal_id: u64,
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
        },
        TaskState::Retrying { error, .. } => EngineStatusView {
            status: ProofStatus::Pending,
            proof: None,
            error: Some(error),
        },
        TaskState::Running { .. } => EngineStatusView {
            status: ProofStatus::Proving,
            proof: None,
            error: None,
        },
        TaskState::Succeeded { output } => {
            let proof = match output {
                EngineOutput::Proof(proof) => proof.output.proof,
                _ => None,
            };
            EngineStatusView {
                status: ProofStatus::Completed,
                proof,
                error: None,
            }
        }
        TaskState::Failed { error, .. } => EngineStatusView {
            status: ProofStatus::Failed,
            proof: None,
            error: Some(error),
        },
        TaskState::Cancelled => EngineStatusView {
            status: ProofStatus::Cancelled,
            proof: None,
            error: None,
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
    fn submit_proposal_proof(
        &self,
        proposal_id: u64,
    ) -> BoxFuture<'_, Result<EngineTaskId, TaskStoreError>> {
        Box::pin(async move { self.submit_proposal_proof(proposal_id).await })
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
