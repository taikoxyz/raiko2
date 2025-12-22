use raiko2_pipeline::PipelineStageResult;
use raiko2_queue::TaskId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProposalStage {
    Preflight,
    Validation,
    Prove,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum EngineTaskKey {
    Proposal {
        proposal_id: u64,
        stage: ProposalStage,
    },
    Aggregate {
        proposal_ids: Vec<u64>,
    },
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
    ProveProposal {
        proposal_id: u64,
        input_task: EngineTaskId,
    },
    Aggregate {
        proposal_ids: Vec<u64>,
        proof_tasks: Vec<EngineTaskId>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum EngineOutput {
    GuestInput(Box<PipelineStageResult<raiko2_primitives::GuestInput>>),
    Proof(Box<PipelineStageResult<raiko2_primitives::Proof>>),
}

pub struct EngineJob {
    pub id: EngineTaskId,
}

#[cfg(test)]
mod tests {
    use super::*;
    use raiko2_pipeline::{PipelineStage, PipelineStageResult};
    use raiko2_primitives::Proof;
    use raiko2_queue::{MemoryStore, NewTask, Priority, Scheduler};

    fn proposal_task_id(proposal_id: u64, stage: ProposalStage) -> EngineTaskId {
        TaskId::new(EngineTaskKey::Proposal { proposal_id, stage })
    }

    #[tokio::test]
    async fn aggregation_depends_on_proposals() {
        let sched: Scheduler<EngineTask, EngineOutput, EngineTaskKey> =
            Scheduler::new(MemoryStore::new());

        let a1 = sched
            .submit(
                proposal_task_id(1, ProposalStage::Prove),
                NewTask {
                    priority: Priority::Medium,
                    payload: EngineTask::ProveProposal {
                        proposal_id: 1,
                        input_task: proposal_task_id(1, ProposalStage::Validation),
                    },
                },
                vec![],
            )
            .await
            .unwrap();
        let a2 = sched
            .submit(
                proposal_task_id(2, ProposalStage::Prove),
                NewTask {
                    priority: Priority::Medium,
                    payload: EngineTask::ProveProposal {
                        proposal_id: 2,
                        input_task: proposal_task_id(2, ProposalStage::Validation),
                    },
                },
                vec![],
            )
            .await
            .unwrap();
        let b = sched
            .submit(
                TaskId::new(EngineTaskKey::Aggregate {
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
            .await
            .unwrap();

        let t1 = sched.next_ready("w").await.unwrap().unwrap();
        let t2 = sched.next_ready("w").await.unwrap().unwrap();
        assert!(sched.next_ready("w").await.unwrap().is_none());

        let proof = PipelineStageResult::new(PipelineStage::Prove, Proof::default());
        sched
            .complete(t1, Ok(EngineOutput::Proof(Box::new(proof.clone()))))
            .await
            .unwrap();
        sched
            .complete(t2, Ok(EngineOutput::Proof(Box::new(proof))))
            .await
            .unwrap();

        assert_eq!(sched.next_ready("w").await.unwrap().unwrap().id, b);
    }
}
