use raiko2_queue::TaskId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum EngineTask {
    BuildGuestInput {
        batch_id: u64,
    },
    ProveBatch {
        batch_id: u64,
        input_task: TaskId,
    },
    Aggregate {
        batch_ids: Vec<u64>,
        proof_tasks: Vec<TaskId>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum EngineOutput {
    GuestInput(Box<raiko2_primitives::GuestInput>),
    Proof(Box<raiko2_primitives::Proof>),
}

pub struct EngineJob {
    pub id: TaskId,
}

#[cfg(test)]
mod tests {
    use super::*;
    use raiko2_queue::{MemoryStore, NewTask, Priority, Scheduler, TaskKind};

    #[tokio::test]
    async fn aggregation_depends_on_batches() {
        let sched: Scheduler<EngineTask, EngineOutput> = Scheduler::new(MemoryStore::new());

        let a1 = sched
            .submit(
                NewTask {
                    kind: TaskKind::BatchProof,
                    priority: Priority::Medium,
                    payload: EngineTask::ProveBatch {
                        batch_id: 1,
                        input_task: TaskId::new(),
                    },
                },
                vec![],
            )
            .await
            .unwrap();
        let a2 = sched
            .submit(
                NewTask {
                    kind: TaskKind::BatchProof,
                    priority: Priority::Medium,
                    payload: EngineTask::ProveBatch {
                        batch_id: 2,
                        input_task: TaskId::new(),
                    },
                },
                vec![],
            )
            .await
            .unwrap();
        let b = sched
            .submit(
                NewTask {
                    kind: TaskKind::Aggregation,
                    priority: Priority::High,
                    payload: EngineTask::Aggregate {
                        batch_ids: vec![1, 2],
                        proof_tasks: vec![a1, a2],
                    },
                },
                vec![a1, a2],
            )
            .await
            .unwrap();

        let t1 = sched.next_ready("w").await.unwrap().unwrap();
        let t2 = sched.next_ready("w").await.unwrap().unwrap();
        assert!(sched.next_ready("w").await.unwrap().is_none());

        sched
            .complete(t1, Ok(EngineOutput::Proof(Box::default())))
            .await
            .unwrap();
        sched
            .complete(t2, Ok(EngineOutput::Proof(Box::default())))
            .await
            .unwrap();

        assert_eq!(sched.next_ready("w").await.unwrap().unwrap().id, b);
    }
}
