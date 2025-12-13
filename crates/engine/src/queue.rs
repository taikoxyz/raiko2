use std::sync::Arc;
use std::time::Duration;

use raiko2_primitives::{AggregationGuestInput, ProverConfig};
use raiko2_prover::Prover;
use raiko2_queue::{
    MemoryStore, NewTask, Priority, RetryPolicy, Scheduler, SchedulerConfig, TaskId, TaskKind,
    TaskState, TaskStoreError, TaskView,
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
    fn default_scheduler_config() -> SchedulerConfig {
        SchedulerConfig {
            lease_duration: Duration::from_secs(60),
            retry: RetryPolicy::Exponential {
                max_attempts: 3,
                base_delay: Duration::from_secs(1),
                max_delay: Duration::from_secs(30),
            },
        }
    }

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
        Self::with_store_and_builder_and_scheduler_config(
            prover,
            store,
            guest_input_builder,
            Self::default_scheduler_config(),
        )
    }

    pub fn with_store_and_builder_and_scheduler_config<S>(
        prover: Arc<dyn Prover>,
        store: S,
        guest_input_builder: Arc<dyn GuestInputBuilder>,
        scheduler_config: SchedulerConfig,
    ) -> Self
    where
        S: raiko2_queue::TaskStore<EngineTask, EngineOutput> + 'static,
    {
        Self {
            inner: Arc::new(Inner {
                scheduler: Scheduler::with_config(store, scheduler_config),
                prover,
                config: ProverConfig::default(),
                guest_input_builder,
            }),
        }
    }

    pub async fn submit_batch_proof(&self, batch_id: u64) -> Result<TaskId, TaskStoreError> {
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
            .await?;

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

    pub async fn get(&self, id: TaskId) -> Result<Option<TaskView<EngineOutput>>, TaskStoreError> {
        self.inner.scheduler.get(id).await
    }

    pub async fn cancel(&self, id: TaskId) -> Result<(), TaskStoreError> {
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

    pub fn start_workers(&self, concurrency: usize) {
        self.start_workers_with_maintenance_interval(concurrency, Duration::from_millis(200));
    }

    pub fn start_workers_with_maintenance_interval(
        &self,
        concurrency: usize,
        maintenance_interval: Duration,
    ) {
        let notify = self.inner.scheduler.notifier();
        for i in 0..concurrency {
            spawn_worker_supervised(self.clone(), notify.clone(), format!("engine-{i}"));
        }

        spawn_maintenance_supervised(self.clone(), maintenance_interval);
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
                        .map_err(|e| e.to_string())?
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

fn spawn_worker_supervised(engine: EngineQueue, notify: Arc<tokio::sync::Notify>, worker: String) {
    tokio::spawn(async move {
        let restart_backoff = Duration::from_secs(1);
        loop {
            let engine = engine.clone();
            let notify = notify.clone();
            let worker_inner = worker.clone();
            let handle = tokio::spawn(async move {
                loop {
                    match engine.run_one(&worker_inner).await {
                        Ok(true) => continue,
                        Ok(false) => notify.notified().await,
                        Err(err) => {
                            tracing::warn!(worker = %worker_inner, error = %err, "engine worker tick failed");
                            tokio::time::sleep(Duration::from_millis(200)).await;
                        }
                    }
                }
            });

            match handle.await {
                Ok(()) => {
                    tracing::warn!(worker = %worker, "engine worker exited unexpectedly");
                }
                Err(err) => {
                    if err.is_panic() {
                        tracing::error!(worker = %worker, "engine worker panicked; restarting");
                    } else {
                        tracing::warn!(worker = %worker, "engine worker aborted; restarting");
                    }
                }
            }

            tokio::time::sleep(restart_backoff).await;
        }
    });
}

fn spawn_maintenance_supervised(engine: EngineQueue, maintenance_interval: Duration) {
    tokio::spawn(async move {
        let restart_backoff = Duration::from_secs(1);
        loop {
            let engine = engine.clone();
            let handle = tokio::spawn(async move {
                let mut interval = tokio::time::interval(maintenance_interval);
                loop {
                    interval.tick().await;
                    if let Err(err) = engine.inner.scheduler.maintenance_tick().await {
                        tracing::warn!(error = %err, "scheduler maintenance tick failed");
                    }
                }
            });

            match handle.await {
                Ok(()) => {
                    tracing::warn!("scheduler maintenance task exited unexpectedly");
                }
                Err(err) => {
                    if err.is_panic() {
                        tracing::error!("scheduler maintenance task panicked; restarting");
                    } else {
                        tracing::warn!("scheduler maintenance task aborted; restarting");
                    }
                }
            }

            tokio::time::sleep(restart_backoff).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use raiko2_primitives::{AggregationGuestInput, GuestInput, Proof, ProverConfig, ProverResult};
    use raiko2_prover::Prover;
    use raiko2_queue::{RetryPolicy, SchedulerConfig, TaskState};

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

    struct FailingProver;

    #[async_trait::async_trait]
    impl Prover for FailingProver {
        async fn prove(&self, _input: GuestInput, _config: &ProverConfig) -> ProverResult<Proof> {
            Err("boom".to_string().into())
        }

        async fn aggregate(
            &self,
            _input: AggregationGuestInput,
            _config: &ProverConfig,
        ) -> ProverResult<Proof> {
            Ok(Proof::default())
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

        let job_id = engine.submit_batch_proof(1).await.unwrap();

        assert!(engine.run_one("w1").await.unwrap());
        assert!(engine.run_one("w1").await.unwrap());
        assert!(!engine.run_one("w1").await.unwrap());

        let view = engine.get(job_id).await.unwrap().unwrap();
        match view.state {
            TaskState::Succeeded {
                output: EngineOutput::Proof(proof),
            } => {
                assert_eq!(proof.proof.as_deref(), Some("mock-proof"));
            }
            other => panic!("unexpected task state: {other:?}"),
        }
    }

    #[tokio::test]
    async fn retry_policy_none_fails_task_immediately() {
        let scheduler_config = SchedulerConfig {
            lease_duration: Duration::from_secs(60),
            retry: RetryPolicy::None,
        };
        let engine = EngineQueue::with_store_and_builder_and_scheduler_config(
            Arc::new(FailingProver),
            raiko2_queue::MemoryStore::new(),
            Arc::new(MockGuestInputBuilder),
            scheduler_config,
        );

        let job_id = engine.submit_batch_proof(1).await.unwrap();

        assert!(engine.run_one("w1").await.unwrap());
        assert!(engine.run_one("w1").await.unwrap());

        let view = engine.get(job_id).await.unwrap().unwrap();
        assert!(matches!(view.state, TaskState::Failed { .. }));
    }
}
