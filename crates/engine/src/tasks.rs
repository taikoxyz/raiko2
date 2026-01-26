use alloy_primitives::Bytes;
use raiko2_pipeline::{PipelineKey, PipelineStageResult};
use raiko2_primitives::Proof;
use raiko2_queue::TaskId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProposalStage {
    Preflight,
    Validation,
    Encode,
    Prove,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum EngineTaskKey {
    Proposal {
        pipeline: PipelineKey,
        proposal_id: u64,
        stage: ProposalStage,
    },
    Aggregate {
        pipeline: PipelineKey,
        proposal_ids: Vec<u64>,
    },
}

impl EngineTaskKey {
    #[must_use]
    pub const fn pipeline_key(&self) -> PipelineKey {
        match self {
            EngineTaskKey::Proposal { pipeline, .. }
            | EngineTaskKey::Aggregate { pipeline, .. } => *pipeline,
        }
    }
}

pub type EngineTaskId = TaskId<EngineTaskKey>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum EngineTask {
    Preflight {
        proposal_id: u64,
    },
    Validate {
        proposal_id: u64,
        preflight_task: EngineTaskId,
    },
    Encode {
        proposal_id: u64,
        input_task: EngineTaskId,
    },
    ProveProposal {
        proposal_id: u64,
        input_task: EngineTaskId,
    },
    Aggregate {
        proposal_ids: Vec<u64>,
        proof_tasks: Vec<EngineTaskId>,
    },
}

pub type EncodedGuestInput = Bytes;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum EngineOutput<I> {
    GuestInput(Box<PipelineStageResult<I>>),
    EncodedInput(Box<PipelineStageResult<EncodedGuestInput>>),
    Proof(Box<PipelineStageResult<Proof>>),
}

pub struct EngineJob {
    pub id: EngineTaskId,
}

#[cfg(test)]
mod tests {
    use super::*;
    use raiko2_pipeline::{PipelineKey, PipelineStage, PipelineStageResult};
    use raiko2_primitives::Proof;
    use raiko2_primitives_shasta::GuestInput;
    use raiko2_queue::{MemoryStore, NewTask, Priority, Scheduler, StoreResult, TaskStoreError};

    fn proposal_task_id(
        pipeline: PipelineKey,
        proposal_id: u64,
        stage: ProposalStage,
    ) -> EngineTaskId {
        TaskId::new(EngineTaskKey::Proposal {
            pipeline,
            proposal_id,
            stage,
        })
    }

    #[tokio::test]
    async fn aggregation_depends_on_proposals() -> StoreResult<()> {
        let sched: Scheduler<EngineTask, EngineOutput<GuestInput>, EngineTaskKey> =
            Scheduler::new(MemoryStore::new());

        let a1 = sched
            .submit(
                proposal_task_id(PipelineKey::ShastaNative, 1, ProposalStage::Prove),
                NewTask {
                    priority: Priority::Medium,
                    payload: EngineTask::ProveProposal {
                        proposal_id: 1,
                        input_task: proposal_task_id(
                            PipelineKey::ShastaNative,
                            1,
                            ProposalStage::Encode,
                        ),
                    },
                },
                vec![],
            )
            .await?;
        let a2 = sched
            .submit(
                proposal_task_id(PipelineKey::ShastaNative, 2, ProposalStage::Prove),
                NewTask {
                    priority: Priority::Medium,
                    payload: EngineTask::ProveProposal {
                        proposal_id: 2,
                        input_task: proposal_task_id(
                            PipelineKey::ShastaNative,
                            2,
                            ProposalStage::Encode,
                        ),
                    },
                },
                vec![],
            )
            .await?;
        let b = sched
            .submit(
                TaskId::new(EngineTaskKey::Aggregate {
                    pipeline: PipelineKey::ShastaNative,
                    proposal_ids: vec![1, 2],
                }),
                NewTask {
                    priority: Priority::High,
                    payload: EngineTask::Aggregate {
                        proposal_ids: vec![1, 2],
                        proof_tasks: vec![a1.clone(), a2.clone()],
                    },
                },
                vec![a1, a2],
            )
            .await?;

        let t1 = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected first ready task in queue"))?;
        let t2 = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected second ready task in queue"))?;
        assert!(sched.next_ready("w").await?.is_none());

        let proof = PipelineStageResult::new(PipelineStage::Prove, Proof::default());
        sched
            .complete(t1, Ok(EngineOutput::Proof(Box::new(proof.clone()))))
            .await?;
        sched
            .complete(t2, Ok(EngineOutput::Proof(Box::new(proof))))
            .await?;

        let next = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected aggregate task to be ready"))?;
        assert_eq!(next.id, b);
        Ok(())
    }
}
