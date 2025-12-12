use std::sync::Arc;

use raiko2_primitives::{AggregationGuestInput, ProverConfig};
use raiko2_prover::Prover;
use raiko2_queue::{
    MemoryStore, NewTask, Priority, RetryPolicy, Scheduler, SchedulerConfig, TaskId, TaskKind,
    TaskState, TaskView,
};

use crate::input_builder::{DefaultGuestInputBuilder, GuestInputBuilder};
use crate::tasks::{EngineOutput, EngineTask};

#[derive(Clone)]
pub struct EngineQueue {
    inner: Arc<Inner>,
}

struct Inner {
    scheduler: Scheduler<EngineTask, EngineOutput>,
    prover: Arc<dyn Prover>,
    config: ProverConfig,
    guest_input_builder: Arc<dyn GuestInputBuilder>,
}

impl EngineQueue {
    pub fn new(prover: Arc<dyn Prover>) -> Self {
        Self::with_store_and_builder(
            prover,
            MemoryStore::new(),
            Arc::new(DefaultGuestInputBuilder),
        )
    }

    pub fn with_store<S>(prover: Arc<dyn Prover>, store: S) -> Self
    where
        S: raiko2_queue::TaskStore<EngineTask, EngineOutput> + 'static,
    {
        Self::with_store_and_builder(prover, store, Arc::new(DefaultGuestInputBuilder))
    }

    pub fn with_store_and_builder<S>(
        prover: Arc<dyn Prover>,
        store: S,
        guest_input_builder: Arc<dyn GuestInputBuilder>,
    ) -> Self
    where
        S: raiko2_queue::TaskStore<EngineTask, EngineOutput> + 'static,
    {
        let scheduler_config = SchedulerConfig {
            lease_duration: std::time::Duration::from_secs(60),
            retry: RetryPolicy::Exponential {
                max_attempts: 3,
                base_delay: std::time::Duration::from_secs(1),
                max_delay: std::time::Duration::from_secs(30),
            },
        };

        Self {
            inner: Arc::new(Inner {
                scheduler: Scheduler::with_config(store, scheduler_config),
                prover,
                config: ProverConfig::default(),
                guest_input_builder,
            }),
        }
    }

    pub async fn submit_batch_proof(&self, batch_id: u64) -> TaskId {
        let input_task = self
            .inner
            .scheduler
            .submit(
                NewTask {
                    kind: TaskKind::BuildGuestInput,
                    priority: Priority::Low,
                    payload: EngineTask::BuildGuestInput { batch_id },
                },
                vec![],
            )
            .await;

        self.inner
            .scheduler
            .submit(
                NewTask {
                    kind: TaskKind::BatchProof,
                    priority: Priority::Medium,
                    payload: EngineTask::ProveBatch {
                        batch_id,
                        input_task,
                    },
                },
                vec![input_task],
            )
            .await
    }

    pub async fn get(&self, id: TaskId) -> Option<TaskView<EngineOutput>> {
        self.inner.scheduler.get(id).await
    }

    pub async fn cancel(&self, id: TaskId) {
        self.inner.scheduler.cancel(id).await;
    }

    pub async fn run_one(&self, worker: &str) -> bool {
        let Some(lease) = self.inner.scheduler.next_ready(worker).await else {
            return false;
        };

        let result = self.execute(lease.payload.clone()).await;
        self.inner.scheduler.complete(lease, result).await;
        true
    }

    pub fn start_workers(&self, concurrency: usize) {
        let notify = self.inner.scheduler.notifier();
        for i in 0..concurrency {
            let engine = self.clone();
            let notify = notify.clone();
            let worker = format!("engine-{i}");
            tokio::spawn(async move {
                loop {
                    if engine.run_one(&worker).await {
                        continue;
                    }
                    notify.notified().await;
                }
            });
        }

        let engine = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(200));
            loop {
                interval.tick().await;
                engine.inner.scheduler.maintenance_tick().await;
            }
        });
    }

    async fn execute(&self, task: EngineTask) -> Result<EngineOutput, String> {
        match task {
            EngineTask::BuildGuestInput { batch_id } => self
                .inner
                .guest_input_builder
                .build_guest_input(batch_id)
                .await
                .map(|input| EngineOutput::GuestInput(Box::new(input))),
            EngineTask::ProveBatch {
                batch_id: _,
                input_task,
            } => {
                let input_view = self
                    .inner
                    .scheduler
                    .get(input_task)
                    .await
                    .ok_or_else(|| "missing input task".to_string())?;

                let guest_input = match input_view.state {
                    TaskState::Succeeded {
                        output: EngineOutput::GuestInput(input),
                    } => *input,
                    TaskState::Succeeded { .. } => {
                        return Err("input task did not produce GuestInput".to_string());
                    }
                    _ => return Err("input task not completed".to_string()),
                };

                let proof = self
                    .inner
                    .prover
                    .prove(guest_input, &self.inner.config)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(EngineOutput::Proof(proof))
            }
            EngineTask::Aggregate {
                batch_ids: _,
                proof_tasks,
            } => {
                let mut proofs = Vec::with_capacity(proof_tasks.len());
                for proof_task in proof_tasks {
                    let view = self
                        .inner
                        .scheduler
                        .get(proof_task)
                        .await
                        .ok_or_else(|| "missing dependency proof task".to_string())?;
                    match view.state {
                        TaskState::Succeeded {
                            output: EngineOutput::Proof(proof),
                        } => proofs.push(proof),
                        TaskState::Succeeded { .. } => {
                            return Err("dependency task did not produce Proof".to_string());
                        }
                        _ => return Err("dependency task not completed".to_string()),
                    }
                }

                let proof = self
                    .inner
                    .prover
                    .aggregate(AggregationGuestInput { proofs }, &self.inner.config)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(EngineOutput::Proof(proof))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use raiko2_primitives::{AggregationGuestInput, GuestInput, Proof, ProverConfig, ProverResult};
    use raiko2_prover::Prover;
    use raiko2_queue::TaskState;

    use crate::input_builder::GuestInputBuilder;
    use crate::queue::EngineQueue;
    use crate::tasks::EngineOutput;

    struct MockProver;

    #[async_trait::async_trait]
    impl Prover for MockProver {
        async fn prove(&self, input: GuestInput, _config: &ProverConfig) -> ProverResult<Proof> {
            assert_eq!(input.taiko.batch_id, 1);
            Ok(Proof {
                proof: Some("mock-proof".to_string()),
                ..Default::default()
            })
        }

        async fn aggregate(
            &self,
            _input: AggregationGuestInput,
            _config: &ProverConfig,
        ) -> ProverResult<Proof> {
            Ok(Proof {
                proof: Some("mock-agg-proof".to_string()),
                ..Default::default()
            })
        }
    }

    struct MockGuestInputBuilder;

    #[async_trait::async_trait]
    impl GuestInputBuilder for MockGuestInputBuilder {
        async fn build_guest_input(&self, batch_id: u64) -> Result<GuestInput, String> {
            Ok(GuestInput {
                taiko: raiko2_primitives::TaikoManifest {
                    batch_id,
                    ..Default::default()
                },
                ..Default::default()
            })
        }
    }

    #[tokio::test]
    async fn submit_batch_proof_runs_dependency_pipeline() {
        let engine = EngineQueue::with_store_and_builder(
            Arc::new(MockProver),
            raiko2_queue::MemoryStore::new(),
            Arc::new(MockGuestInputBuilder),
        );

        let job_id = engine.submit_batch_proof(1).await;

        assert!(engine.run_one("w1").await);
        assert!(engine.run_one("w1").await);
        assert!(!engine.run_one("w1").await);

        let view = engine.get(job_id).await.unwrap();
        match view.state {
            TaskState::Succeeded {
                output: EngineOutput::Proof(proof),
            } => {
                assert_eq!(proof.proof.as_deref(), Some("mock-proof"));
            }
            other => panic!("unexpected task state: {other:?}"),
        }
    }
}
