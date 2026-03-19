use alloy_primitives::Bytes;
use raiko2_pipeline::{PipelineKey, PipelineStageResult};
use raiko2_primitives::{L2BlockRange, Proof};
use raiko2_queue::TaskId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalTaskRequest {
    pub proposal_id: u64,
    pub l2_block_range: Option<L2BlockRange>,
    pub l1_inclusion_block_number: u64,
    pub last_anchor_block_number: u64,
    pub blob_proof_type: Option<String>,
    pub prover: Option<String>,
    pub graffiti: Option<String>,
    pub prover_args_json: Option<String>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProposalStage {
    Preflight,
    Validation,
    Encode,
    Prove,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregationTaskRequest {
    pub request_id: String,
    pub proposal_ids: Vec<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AggregationSource {
    ProofTasks(Vec<EngineTaskId>),
    Proofs(Vec<Proof>),
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum EngineTaskKey {
    Proposal {
        pipeline: PipelineKey,
        request: ProposalTaskRequest,
        stage: ProposalStage,
    },
    Aggregate {
        pipeline: PipelineKey,
        request: AggregationTaskRequest,
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
        request: ProposalTaskRequest,
    },
    Validate {
        request: ProposalTaskRequest,
        preflight_task: EngineTaskId,
    },
    Encode {
        request: ProposalTaskRequest,
        input_task: EngineTaskId,
    },
    ProveProposal {
        request: ProposalTaskRequest,
        input_task: EngineTaskId,
    },
    Aggregate {
        request: AggregationTaskRequest,
        source: AggregationSource,
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
        request: ProposalTaskRequest,
        stage: ProposalStage,
    ) -> EngineTaskId {
        TaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request,
            stage,
        })
    }

    fn proposal_request(proposal_id: u64) -> ProposalTaskRequest {
        ProposalTaskRequest {
            proposal_id,
            l2_block_range: None,
            l1_inclusion_block_number: 0,
            last_anchor_block_number: 0,
            blob_proof_type: None,
            prover: None,
            graffiti: None,
            prover_args_json: None,
        }
    }

    #[tokio::test]
    async fn aggregation_depends_on_proposals() -> StoreResult<()> {
        let sched: Scheduler<EngineTask, EngineOutput<GuestInput>, EngineTaskKey> =
            Scheduler::new(MemoryStore::new());

        let a1 = sched
            .submit(
                proposal_task_id(
                    PipelineKey::ShastaNative,
                    proposal_request(1),
                    ProposalStage::Prove,
                ),
                NewTask {
                    priority: Priority::Medium,
                    payload: EngineTask::ProveProposal {
                        request: proposal_request(1),
                        input_task: proposal_task_id(
                            PipelineKey::ShastaNative,
                            proposal_request(1),
                            ProposalStage::Encode,
                        ),
                    },
                },
                vec![],
            )
            .await?;
        let a2 = sched
            .submit(
                proposal_task_id(
                    PipelineKey::ShastaNative,
                    proposal_request(2),
                    ProposalStage::Prove,
                ),
                NewTask {
                    priority: Priority::Medium,
                    payload: EngineTask::ProveProposal {
                        request: proposal_request(2),
                        input_task: proposal_task_id(
                            PipelineKey::ShastaNative,
                            proposal_request(2),
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
                    request: AggregationTaskRequest {
                        request_id: "agg-1".to_string(),
                        proposal_ids: vec![1, 2],
                    },
                }),
                NewTask {
                    priority: Priority::High,
                    payload: EngineTask::Aggregate {
                        request: AggregationTaskRequest {
                            request_id: "agg-1".to_string(),
                            proposal_ids: vec![1, 2],
                        },
                        source: AggregationSource::ProofTasks(vec![a1.clone(), a2.clone()]),
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
