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
use raiko2_pipeline::{PipelineSpec, PipelineStage, PipelineStageResult, ProverBackend};

pub struct EngineQueue<S, B>
where
    S: PipelineSpec<B>,
    B: ProverBackend<S>,
{
    inner: Arc<Inner<S, B>>,
}

struct Inner<S, B>
where
    S: PipelineSpec<B>,
    B: ProverBackend<S>,
{
    spec: S,
    scheduler: Scheduler<EngineTask, EngineOutput>,
    prover: Arc<dyn Prover<S, B>>,
    config: ProverConfig,
    guest_input_builder: Arc<dyn GuestInputBuilder<S, B>>,
}

impl<S, B> Clone for EngineQueue<S, B>
where
    S: PipelineSpec<B>,
    B: ProverBackend<S>,
{
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<S, B> EngineQueue<S, B>
where
    S: PipelineSpec<B>,
    B: ProverBackend<S>,
{
    const fn default_scheduler_config() -> SchedulerConfig {
        SchedulerConfig {
            lease_duration: Duration::from_secs(60),
            retry: RetryPolicy::Exponential {
                max_attempts: 3,
                base_delay: Duration::from_secs(1),
                max_delay: Duration::from_secs(30),
            },
        }
    }

    pub fn new(spec: S, prover: Arc<dyn Prover<S, B>>) -> Self {
        Self::with_store_and_builder(
            spec,
            prover,
            MemoryStore::new(),
            Arc::new(DefaultGuestInputBuilder),
        )
    }

    pub fn with_store<Store>(spec: S, prover: Arc<dyn Prover<S, B>>, store: Store) -> Self
    where
        Store: raiko2_queue::TaskStore<EngineTask, EngineOutput> + 'static,
    {
        Self::with_store_and_builder(spec, prover, store, Arc::new(DefaultGuestInputBuilder))
    }

    pub fn with_store_and_builder<Store>(
        spec: S,
        prover: Arc<dyn Prover<S, B>>,
        store: Store,
        guest_input_builder: Arc<dyn GuestInputBuilder<S, B>>,
    ) -> Self
    where
        Store: raiko2_queue::TaskStore<EngineTask, EngineOutput> + 'static,
    {
        Self::with_store_and_builder_and_scheduler_config(
            spec,
            prover,
            store,
            guest_input_builder,
            Self::default_scheduler_config(),
        )
    }

    pub fn with_store_and_builder_and_scheduler_config<Store>(
        spec: S,
        prover: Arc<dyn Prover<S, B>>,
        store: Store,
        guest_input_builder: Arc<dyn GuestInputBuilder<S, B>>,
        scheduler_config: SchedulerConfig,
    ) -> Self
    where
        Store: raiko2_queue::TaskStore<EngineTask, EngineOutput> + 'static,
    {
        Self {
            inner: Arc::new(Inner {
                spec,
                scheduler: Scheduler::with_config(store, scheduler_config),
                prover,
                config: ProverConfig::default(),
                guest_input_builder,
            }),
        }
    }

    pub async fn submit_batch_proof(&self, batch_id: u64) -> Result<TaskId, TaskStoreError> {
        let preflight_task = self
            .inner
            .scheduler
            .submit(
                NewTask {
                    kind: TaskKind::Preflight,
                    priority: Priority::Low,
                    payload: EngineTask::Preflight { batch_id },
                },
                vec![],
            )
            .await?;

        let validation_task = self
            .inner
            .scheduler
            .submit(
                NewTask {
                    kind: TaskKind::BuildGuestInput,
                    priority: Priority::Low,
                    payload: EngineTask::Validate {
                        batch_id,
                        preflight_task,
                    },
                },
                vec![preflight_task],
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
                        input_task: validation_task,
                    },
                },
                vec![validation_task],
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

    pub fn start_workers(&self, concurrency: usize)
    where
        S: 'static,
        B: 'static,
    {
        self.start_workers_with_maintenance_interval(concurrency, Duration::from_millis(200));
    }

    pub fn start_workers_with_maintenance_interval(
        &self,
        concurrency: usize,
        maintenance_interval: Duration,
    ) where
        S: 'static,
        B: 'static,
    {
        let notify = self.inner.scheduler.notifier();
        for i in 0..concurrency {
            spawn_worker_supervised(self.clone(), notify.clone(), format!("engine-{i}"));
        }

        spawn_maintenance_supervised(self.clone(), maintenance_interval);
    }

    async fn execute(&self, task: EngineTask) -> Result<EngineOutput, String> {
        match task {
            EngineTask::Preflight { batch_id } => self
                .inner
                .guest_input_builder
                .preflight(batch_id)
                .await
                .map(|input| EngineOutput::GuestInput(Box::new(input))),
            EngineTask::Validate {
                batch_id,
                preflight_task,
            } => {
                let input_view = self
                    .inner
                    .scheduler
                    .get(preflight_task)
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| "missing preflight task".to_string())?;

                let preflight_input = match input_view.state {
                    TaskState::Succeeded {
                        output: EngineOutput::GuestInput(input),
                    } => match input.stage {
                        PipelineStage::Preflight => input.output,
                        _ => {
                            return Err(
                                "preflight task did not produce preflight output".to_string()
                            );
                        }
                    },
                    TaskState::Succeeded { .. } => {
                        return Err("preflight task did not produce GuestInput".to_string());
                    }
                    _ => return Err("preflight task not completed".to_string()),
                };

                let validated = self
                    .inner
                    .guest_input_builder
                    .validate(batch_id, preflight_input)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(EngineOutput::GuestInput(Box::new(validated)))
            }
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
                    } => match input.stage {
                        PipelineStage::Validation => input.output,
                        _ => {
                            return Err(
                                "input task did not produce validated GuestInput".to_string()
                            );
                        }
                    },
                    TaskState::Succeeded { .. } => {
                        return Err("input task did not produce GuestInput".to_string());
                    }
                    _ => return Err("input task not completed".to_string()),
                };

                let proof = self
                    .inner
                    .prover
                    .prove(guest_input, &self.inner.config, &self.inner.spec)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(EngineOutput::Proof(Box::new(PipelineStageResult::new(
                    PipelineStage::Prove,
                    proof,
                ))))
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
                        } => match proof.stage {
                            PipelineStage::Prove => proofs.push(proof.output),
                            _ => {
                                return Err(
                                    "dependency task did not produce batch proof".to_string()
                                );
                            }
                        },
                        TaskState::Succeeded { .. } => {
                            return Err("dependency task did not produce Proof".to_string());
                        }
                        _ => return Err("dependency task not completed".to_string()),
                    }
                }

                let proof = self
                    .inner
                    .prover
                    .aggregate(
                        AggregationGuestInput { proofs },
                        &self.inner.config,
                        &self.inner.spec,
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(EngineOutput::Proof(Box::new(PipelineStageResult::new(
                    PipelineStage::Aggregate,
                    proof,
                ))))
            }
        }
    }
}

fn spawn_worker_supervised<S, B>(
    engine: EngineQueue<S, B>,
    notify: Arc<tokio::sync::Notify>,
    worker: String,
) where
    S: PipelineSpec<B> + 'static,
    B: ProverBackend<S> + 'static,
{
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

fn spawn_maintenance_supervised<S, B>(engine: EngineQueue<S, B>, maintenance_interval: Duration)
where
    S: PipelineSpec<B> + 'static,
    B: ProverBackend<S> + 'static,
{
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

    use raiko2_pipeline::{
        NoopManifestBuilder, NoopValidation, PipelineSpec, PipelineStage, PipelineStageResult,
        Preflight, ProverBackend, Risc0Backend,
    };
    use raiko2_primitives::{
        AggregationGuestInput, GuestInput, Proof, ProofContext, ProverConfig, RaikoError,
        RaikoResult,
    };
    use raiko2_prover::Prover;
    use raiko2_queue::{RetryPolicy, SchedulerConfig, TaskState};

    use crate::input_builder::GuestInputBuilder;
    use crate::queue::EngineQueue;
    use crate::tasks::EngineOutput;

    struct MockProver;

    #[async_trait::async_trait]
    impl Prover<TestSpec, Risc0Backend> for MockProver {
        async fn prove(
            &self,
            input: GuestInput,
            _config: &ProverConfig,
            _spec: &TestSpec,
        ) -> RaikoResult<Proof> {
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
            _spec: &TestSpec,
        ) -> RaikoResult<Proof> {
            Ok(Proof {
                proof: Some("mock-agg-proof".to_string()),
                ..Default::default()
            })
        }
    }

    struct FailingProver;

    #[async_trait::async_trait]
    impl Prover<TestSpec, Risc0Backend> for FailingProver {
        async fn prove(
            &self,
            _input: GuestInput,
            _config: &ProverConfig,
            _spec: &TestSpec,
        ) -> RaikoResult<Proof> {
            Err(RaikoError::Guest("boom".to_string()))
        }

        async fn aggregate(
            &self,
            _input: AggregationGuestInput,
            _config: &ProverConfig,
            _spec: &TestSpec,
        ) -> RaikoResult<Proof> {
            Ok(Proof::default())
        }
    }

    struct TestSpec;
    const NOOP_VALIDATION: NoopValidation = NoopValidation;
    const NOOP_MANIFEST: NoopManifestBuilder = NoopManifestBuilder;

    #[async_trait::async_trait]
    impl Preflight for TestSpec {
        async fn preflight<P: raiko2_provider::Provider>(
            &self,
            _ctx: &ProofContext,
            _provider: &P,
        ) -> RaikoResult<GuestInput> {
            Ok(GuestInput::default())
        }
    }

    impl PipelineSpec<Risc0Backend> for TestSpec {
        type Preflight = Self;
        type Validation = NoopValidation;
        type ManifestBuilder = NoopManifestBuilder;

        fn preflight(&self) -> &Self::Preflight {
            self
        }

        fn validation(&self) -> &Self::Validation {
            &NOOP_VALIDATION
        }

        fn manifest_builder(&self) -> &Self::ManifestBuilder {
            &NOOP_MANIFEST
        }
    }

    impl ProverBackend<TestSpec> for Risc0Backend {
        fn elf(
            &self,
            _spec: &TestSpec,
            _stage: raiko2_pipeline::ProofStage,
        ) -> RaikoResult<&'static [u8]> {
            Ok(&[])
        }
    }

    struct MockGuestInputBuilder;

    #[async_trait::async_trait]
    impl GuestInputBuilder<TestSpec, Risc0Backend> for MockGuestInputBuilder {
        async fn preflight(
            &self,
            batch_id: u64,
        ) -> Result<PipelineStageResult<GuestInput>, String> {
            let input = GuestInput {
                taiko: raiko2_primitives::TaikoManifest {
                    batch_id,
                    ..Default::default()
                },
                ..Default::default()
            };
            Ok(PipelineStageResult::new(PipelineStage::Preflight, input))
        }

        async fn validate(
            &self,
            _batch_id: u64,
            input: GuestInput,
        ) -> Result<PipelineStageResult<GuestInput>, String> {
            Ok(PipelineStageResult::new(PipelineStage::Validation, input))
        }
    }

    #[tokio::test]
    async fn submit_batch_proof_runs_dependency_pipeline() {
        let engine = EngineQueue::<TestSpec, Risc0Backend>::with_store_and_builder(
            TestSpec,
            Arc::new(MockProver),
            raiko2_queue::MemoryStore::new(),
            Arc::new(MockGuestInputBuilder),
        );

        let job_id = engine.submit_batch_proof(1).await.unwrap();

        assert!(engine.run_one("w1").await.unwrap());
        assert!(engine.run_one("w1").await.unwrap());
        assert!(engine.run_one("w1").await.unwrap());
        assert!(!engine.run_one("w1").await.unwrap());

        let view = engine.get(job_id).await.unwrap().unwrap();
        match view.state {
            TaskState::Succeeded {
                output: EngineOutput::Proof(proof),
            } => {
                assert_eq!(proof.output.proof.as_deref(), Some("mock-proof"));
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
        let engine =
            EngineQueue::<TestSpec, Risc0Backend>::with_store_and_builder_and_scheduler_config(
                TestSpec,
                Arc::new(FailingProver),
                raiko2_queue::MemoryStore::new(),
                Arc::new(MockGuestInputBuilder),
                scheduler_config,
            );

        let job_id = engine.submit_batch_proof(1).await.unwrap();

        assert!(engine.run_one("w1").await.unwrap());
        assert!(engine.run_one("w1").await.unwrap());
        assert!(engine.run_one("w1").await.unwrap());

        let view = engine.get(job_id).await.unwrap().unwrap();
        assert!(matches!(view.state, TaskState::Failed { .. }));
    }
}
