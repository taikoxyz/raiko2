use std::sync::Arc;
use std::time::Duration;

use raiko2_pipeline::{Pipeline, PipelineSpec, PipelineStage, PipelineStageResult, ProverBackend};
use raiko2_primitives::{AggregationGuestInput, ProofContext};
use raiko2_prover::Prover;
use raiko2_provider::Provider;
use raiko2_queue::{
    MemoryStore, NewTask, Priority, RetryPolicy, Scheduler, SchedulerConfig, TaskState,
    TaskStoreError, TaskView,
};

use crate::tasks::{BatchStage, EngineOutput, EngineTask, EngineTaskId, EngineTaskKey};

pub struct Engine<S, B, P>
where
    S: PipelineSpec<B>,
    B: ProverBackend,
    P: Provider,
{
    inner: Arc<Inner<S, B, P>>,
}

struct Inner<S, B, P>
where
    S: PipelineSpec<B>,
    B: ProverBackend,
    P: Provider,
{
    spec: S,
    backend: B,
    provider: P,
    scheduler: Scheduler<EngineTask, EngineOutput, EngineTaskKey>,
    prover: Arc<dyn Prover<S, B>>,
    context: ProofContext,
}

impl<S, B, P> Clone for Engine<S, B, P>
where
    S: PipelineSpec<B>,
    B: ProverBackend,
    P: Provider,
{
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<S, B, P> Engine<S, B, P>
where
    S: PipelineSpec<B>,
    B: ProverBackend,
    P: Provider,
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

    pub fn new(
        spec: S,
        backend: B,
        provider: P,
        prover: Arc<dyn Prover<S, B>>,
        context: ProofContext,
    ) -> Self {
        Self::with_store_and_scheduler_config(
            spec,
            backend,
            provider,
            prover,
            context,
            MemoryStore::new(),
            Self::default_scheduler_config(),
        )
    }

    pub fn with_store<Store>(
        spec: S,
        backend: B,
        provider: P,
        prover: Arc<dyn Prover<S, B>>,
        context: ProofContext,
        store: Store,
    ) -> Self
    where
        Store: raiko2_queue::TaskStore<EngineTask, EngineOutput, EngineTaskKey> + 'static,
    {
        Self::with_store_and_scheduler_config(
            spec,
            backend,
            provider,
            prover,
            context,
            store,
            Self::default_scheduler_config(),
        )
    }

    pub fn with_store_and_scheduler_config<Store>(
        spec: S,
        backend: B,
        provider: P,
        prover: Arc<dyn Prover<S, B>>,
        context: ProofContext,
        store: Store,
        scheduler_config: SchedulerConfig,
    ) -> Self
    where
        Store: raiko2_queue::TaskStore<EngineTask, EngineOutput, EngineTaskKey> + 'static,
    {
        Self {
            inner: Arc::new(Inner {
                spec,
                backend,
                provider,
                scheduler: Scheduler::with_config(store, scheduler_config),
                prover,
                context,
            }),
        }
    }

    fn context_for_batch(&self, batch_id: u64) -> ProofContext {
        let mut ctx = self.inner.context.clone();
        ctx.request.batch_id = batch_id;
        ctx
    }

    const fn batch_task_id(&self, batch_id: u64, stage: BatchStage) -> EngineTaskId {
        EngineTaskId::new(EngineTaskKey::Batch { batch_id, stage })
    }

    pub async fn submit_batch_proof(&self, batch_id: u64) -> Result<EngineTaskId, TaskStoreError> {
        let preflight_id = self.batch_task_id(batch_id, BatchStage::Preflight);
        let preflight_task = self
            .inner
            .scheduler
            .submit(
                preflight_id,
                NewTask {
                    priority: Priority::Low,
                    payload: EngineTask::Preflight { batch_id },
                },
                vec![],
            )
            .await?;

        let validation_id = self.batch_task_id(batch_id, BatchStage::Validation);
        let validation_task = self
            .inner
            .scheduler
            .submit(
                validation_id,
                NewTask {
                    priority: Priority::Low,
                    payload: EngineTask::Validate {
                        batch_id,
                        preflight_task: preflight_task.clone(),
                    },
                },
                vec![preflight_task],
            )
            .await?;

        let prove_id = self.batch_task_id(batch_id, BatchStage::Prove);
        self.inner
            .scheduler
            .submit(
                prove_id,
                NewTask {
                    priority: Priority::Medium,
                    payload: EngineTask::ProveBatch {
                        batch_id,
                        input_task: validation_task.clone(),
                    },
                },
                vec![validation_task],
            )
            .await
    }

    pub async fn get(
        &self,
        id: EngineTaskId,
    ) -> Result<Option<TaskView<EngineOutput, EngineTaskKey>>, TaskStoreError> {
        self.inner.scheduler.get(id).await
    }

    pub async fn cancel(&self, id: EngineTaskId) -> Result<(), TaskStoreError> {
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
        P: 'static,
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
        P: 'static,
    {
        let notify = self.inner.scheduler.notifier();
        for i in 0..concurrency {
            spawn_worker_supervised(self.clone(), notify.clone(), format!("engine-{i}"));
        }

        spawn_maintenance_supervised(self.clone(), maintenance_interval);
    }

    async fn execute(&self, task: EngineTask) -> Result<EngineOutput, String> {
        match task {
            EngineTask::Preflight { batch_id } => {
                let ctx = self.context_for_batch(batch_id);
                let pipeline = Pipeline::new(&self.inner.spec, &self.inner.backend);
                pipeline
                    .preflight(&ctx, &self.inner.provider)
                    .await
                    .map(|input| EngineOutput::GuestInput(Box::new(input)))
                    .map_err(|e| e.to_string())
            }
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

                let ctx = self.context_for_batch(batch_id);
                let pipeline = Pipeline::new(&self.inner.spec, &self.inner.backend);
                let validated = pipeline
                    .validate(&ctx, preflight_input)
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
                    .prove(
                        guest_input,
                        &self.inner.context.config,
                        &self.inner.spec,
                        &self.inner.backend,
                    )
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
                        &self.inner.context.config,
                        &self.inner.spec,
                        &self.inner.backend,
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

fn spawn_worker_supervised<S, B, P>(
    engine: Engine<S, B, P>,
    notify: Arc<tokio::sync::Notify>,
    worker: String,
) where
    S: PipelineSpec<B> + 'static,
    B: ProverBackend + 'static,
    P: Provider + 'static,
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

fn spawn_maintenance_supervised<S, B, P>(engine: Engine<S, B, P>, maintenance_interval: Duration)
where
    S: PipelineSpec<B> + 'static,
    B: ProverBackend + 'static,
    P: Provider + 'static,
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
        NoopManifestBuilder, NoopValidation, PipelineSpec, Preflight, ProofStage, ProverBackend,
    };
    use raiko2_primitives::{
        AggregationGuestInput, GuestInput, Proof, ProofContext, ProofRequest, ProverConfig,
        RaikoError, RaikoResult,
    };
    use raiko2_prover::Prover;
    use raiko2_provider::Provider;
    use raiko2_queue::{RetryPolicy, SchedulerConfig, TaskState};

    use crate::queue::Engine;
    use crate::tasks::EngineOutput;

    struct MockProver;

    #[async_trait::async_trait]
    impl Prover<TestSpec, TestBackend> for MockProver {
        async fn prove(
            &self,
            input: GuestInput,
            _config: &ProverConfig,
            _spec: &TestSpec,
            _backend: &TestBackend,
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
            _backend: &TestBackend,
        ) -> RaikoResult<Proof> {
            Ok(Proof {
                proof: Some("mock-agg-proof".to_string()),
                ..Default::default()
            })
        }
    }

    struct FailingProver;

    #[async_trait::async_trait]
    impl Prover<TestSpec, TestBackend> for FailingProver {
        async fn prove(
            &self,
            _input: GuestInput,
            _config: &ProverConfig,
            _spec: &TestSpec,
            _backend: &TestBackend,
        ) -> RaikoResult<Proof> {
            Err(RaikoError::Guest("boom".to_string()))
        }

        async fn aggregate(
            &self,
            _input: AggregationGuestInput,
            _config: &ProverConfig,
            _spec: &TestSpec,
            _backend: &TestBackend,
        ) -> RaikoResult<Proof> {
            Ok(Proof::default())
        }
    }

    struct TestBackend;

    impl ProverBackend for TestBackend {
        fn elf(&self, _stage: ProofStage) -> RaikoResult<&'static [u8]> {
            Ok(&[])
        }
    }

    struct TestSpec;
    const NOOP_VALIDATION: NoopValidation = NoopValidation;
    const NOOP_MANIFEST: NoopManifestBuilder = NoopManifestBuilder;

    #[async_trait::async_trait]
    impl Preflight for TestSpec {
        async fn preflight<P: Provider>(
            &self,
            ctx: &ProofContext,
            _provider: &P,
        ) -> RaikoResult<GuestInput> {
            let mut input = GuestInput::default();
            input.taiko.batch_id = ctx.request.batch_id;
            Ok(input)
        }
    }

    impl PipelineSpec<TestBackend> for TestSpec {
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

    struct MockProvider;

    #[async_trait::async_trait]
    impl Provider for MockProvider {
        async fn batch_blocks(
            &self,
            _blocks: &[u64],
        ) -> RaikoResult<Vec<reth_ethereum_primitives::Block>> {
            Ok(vec![])
        }

        async fn batch_accounts(
            &self,
            _blocks: &[u64],
            _accounts: &[Vec<alloy_primitives::Address>],
        ) -> RaikoResult<Vec<alloy_primitives::map::AddressMap<alloy_trie::TrieAccount>>> {
            Ok(vec![])
        }

        async fn batch_witnesses(
            &self,
            _blocks: &[u64],
        ) -> RaikoResult<Vec<reth_stateless::ExecutionWitness>> {
            Ok(vec![])
        }
    }

    fn test_context() -> ProofContext {
        ProofContext::new(ProofRequest::default(), ProverConfig::default())
    }

    #[tokio::test]
    async fn submit_batch_proof_runs_dependency_pipeline() {
        let backend = TestBackend;
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec,
            backend,
            MockProvider,
            Arc::new(MockProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            Engine::default_scheduler_config(),
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
        let backend = TestBackend;
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec,
            backend,
            MockProvider,
            Arc::new(FailingProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
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
