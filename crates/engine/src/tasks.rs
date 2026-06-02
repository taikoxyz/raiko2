use alloy_primitives::Bytes;
use raiko2_pipeline::{PipelineKey, PipelineStageResult};
use raiko2_primitives::{L2BlockRange, Proof, ShastaCheckpoint};
use raiko2_prover::sp1_config::{Sp1ConfigOverrides, Sp1SystemConfig};
use raiko2_queue::{ReadyQueueSort, TaskId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProverTaskConfig {
    #[serde(default)]
    pub sp1: Option<Sp1ConfigOverrides>,
    #[serde(default)]
    pub sp1_system: Option<Sp1SystemConfig>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalTaskRequest {
    pub proposal_id: u64,
    pub l2_block_range: Option<L2BlockRange>,
    pub l1_inclusion_block_number: u64,
    pub last_anchor_block_number: u64,
    pub checkpoint: Option<ShastaCheckpoint>,
    pub blob_proof_type: Option<String>,
    pub prover: Option<String>,
    pub graffiti: Option<String>,
    #[serde(default)]
    pub prover_config: ProverTaskConfig,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
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
    #[serde(default)]
    pub prover_config: ProverTaskConfig,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AggregationSource {
    Inputs(Vec<AggregateProofInput>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AggregateProofInput {
    ProofArtifact(ProofArtifactRef),
    PendingProofArtifact {
        artifact: ProofArtifactRef,
        dependency: Box<EngineTaskId>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofArtifactRef {
    pub network_pair: String,
    pub proof_ref: String,
    pub proof_path: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum EngineTaskKey {
    Proposal {
        pipeline: PipelineKey,
        request: ProposalTaskRequest,
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

fn proposal_ready_sort_prefix(proposal_id: u64) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[0..8].copy_from_slice(&proposal_id.to_be_bytes());
    b
}

fn aggregate_ready_sort_prefix(proposal_ids: &[u64]) -> [u8; 16] {
    let proposal_id = proposal_ids.iter().copied().min().unwrap_or(u64::MAX);
    let mut b = [0u8; 16];
    b[0..8].copy_from_slice(&proposal_id.to_be_bytes());
    b
}

impl ReadyQueueSort for EngineTaskKey {
    fn ready_queue_sort_prefix(&self) -> [u8; 16] {
        match self {
            EngineTaskKey::Proposal { request, .. } => {
                proposal_ready_sort_prefix(request.proposal_id)
            }
            EngineTaskKey::Aggregate { request, .. } => {
                aggregate_ready_sort_prefix(&request.proposal_ids)
            }
        }
    }
}

pub type EngineTaskId = TaskId<EngineTaskKey>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum EngineTask {
    Proposal {
        request: ProposalTaskRequest,
    },
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

    fn proposal_task_id(pipeline: PipelineKey, request: ProposalTaskRequest) -> EngineTaskId {
        TaskId::new(EngineTaskKey::Proposal { pipeline, request })
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

    fn proof_artifact(proof_ref: &str) -> ProofArtifactRef {
        ProofArtifactRef {
            network_pair: "taiko_dev/ethereum".to_string(),
            proof_ref: proof_ref.to_string(),
            proof_path: format!("/tmp/{proof_ref}.json"),
        }
    }

    #[test]
    fn proposal_ready_sort_orders_by_proposal_id_only() {
        let p2 = EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaNative,
            request: proposal_request(2),
        };
        let p1_native = EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaNative,
            request: proposal_request(1),
        };
        let p1_sp1 = EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaSp1,
            request: proposal_request(1),
        };
        let p1_risc0 = EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaRisc0,
            request: proposal_request(1),
        };

        assert_eq!(
            p1_native.ready_queue_sort_prefix(),
            p1_sp1.ready_queue_sort_prefix()
        );
        assert_eq!(
            p1_native.ready_queue_sort_prefix(),
            p1_risc0.ready_queue_sort_prefix()
        );

        let mut keys = [p2, p1_native, p1_sp1, p1_risc0];
        keys.sort_by(raiko2_queue::ReadyQueueSort::cmp_for_ready_queue);

        assert!(matches!(
            keys[0],
            EngineTaskKey::Proposal {
                request: ProposalTaskRequest { proposal_id: 1, .. },
                pipeline: PipelineKey::ShastaNative
            }
        ));
        assert!(matches!(
            keys[1],
            EngineTaskKey::Proposal {
                request: ProposalTaskRequest { proposal_id: 1, .. },
                ..
            }
        ));
        assert!(matches!(
            keys[2],
            EngineTaskKey::Proposal {
                request: ProposalTaskRequest { proposal_id: 1, .. },
                ..
            }
        ));
        assert!(matches!(
            keys[3],
            EngineTaskKey::Proposal {
                request: ProposalTaskRequest { proposal_id: 2, .. },
                pipeline: PipelineKey::ShastaNative
            }
        ));
    }

    #[test]
    fn aggregate_ready_sort_stays_with_its_proposal_range() {
        let p2 = EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaNative,
            request: proposal_request(2),
        };
        let p1 = EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaNative,
            request: proposal_request(1),
        };
        let aggregate = EngineTaskKey::Aggregate {
            pipeline: PipelineKey::ShastaNative,
            request: AggregationTaskRequest {
                request_id: "1-192".to_string(),
                proposal_ids: vec![1, 192],
                prover_config: ProverTaskConfig::default(),
            },
        };

        let mut keys = [p2, p1, aggregate];
        keys.sort_by(raiko2_queue::ReadyQueueSort::cmp_for_ready_queue);

        assert!(matches!(
            keys[0],
            EngineTaskKey::Proposal {
                request: ProposalTaskRequest { proposal_id: 1, .. },
                ..
            }
        ));
        assert!(matches!(keys[1], EngineTaskKey::Aggregate { .. }));
        assert!(matches!(
            keys[2],
            EngineTaskKey::Proposal {
                request: ProposalTaskRequest { proposal_id: 2, .. },
                ..
            }
        ));
    }

    #[tokio::test]
    async fn aggregation_depends_on_proposals() -> StoreResult<()> {
        let sched: Scheduler<EngineTask, EngineOutput<GuestInput>, EngineTaskKey> =
            Scheduler::new(MemoryStore::new());

        let a1 = sched
            .submit(
                proposal_task_id(PipelineKey::ShastaNative, proposal_request(1)),
                NewTask {
                    priority: Priority::Medium,
                    payload: EngineTask::Proposal {
                        request: proposal_request(1),
                    },
                },
                vec![],
            )
            .await?;
        let a2 = sched
            .submit(
                proposal_task_id(PipelineKey::ShastaNative, proposal_request(2)),
                NewTask {
                    priority: Priority::Medium,
                    payload: EngineTask::Proposal {
                        request: proposal_request(2),
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
                        prover_config: ProverTaskConfig::default(),
                    },
                }),
                NewTask {
                    priority: Priority::High,
                    payload: EngineTask::Aggregate {
                        request: AggregationTaskRequest {
                            request_id: "agg-1".to_string(),
                            proposal_ids: vec![1, 2],
                            prover_config: ProverTaskConfig::default(),
                        },
                        source: AggregationSource::Inputs(vec![
                            AggregateProofInput::PendingProofArtifact {
                                artifact: proof_artifact("a1"),
                                dependency: Box::new(a1.clone()),
                            },
                            AggregateProofInput::PendingProofArtifact {
                                artifact: proof_artifact("a2"),
                                dependency: Box::new(a2.clone()),
                            },
                        ]),
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

    #[tokio::test]
    async fn aggregate_priority_wins_before_proposal_id_sort() -> StoreResult<()> {
        let sched: Scheduler<EngineTask, EngineOutput<GuestInput>, EngineTaskKey> =
            Scheduler::new(MemoryStore::new());

        let proposal = sched
            .submit(
                proposal_task_id(PipelineKey::ShastaNative, proposal_request(1)),
                NewTask {
                    priority: Priority::Medium,
                    payload: EngineTask::Proposal {
                        request: proposal_request(1),
                    },
                },
                vec![],
            )
            .await?;
        let aggregate = sched
            .submit(
                TaskId::new(EngineTaskKey::Aggregate {
                    pipeline: PipelineKey::ShastaNative,
                    request: AggregationTaskRequest {
                        request_id: "agg-2".to_string(),
                        proposal_ids: vec![2],
                        prover_config: ProverTaskConfig::default(),
                    },
                }),
                NewTask {
                    priority: Priority::High,
                    payload: EngineTask::Aggregate {
                        request: AggregationTaskRequest {
                            request_id: "agg-2".to_string(),
                            proposal_ids: vec![2],
                            prover_config: ProverTaskConfig::default(),
                        },
                        source: AggregationSource::Inputs(vec![
                            AggregateProofInput::ProofArtifact(proof_artifact("agg-2-input")),
                        ]),
                    },
                },
                vec![],
            )
            .await?;

        let first = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected first ready task"))?;
        assert_eq!(first.id, aggregate);
        let second = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected second ready task"))?;
        assert_eq!(second.id, proposal);
        Ok(())
    }
}
