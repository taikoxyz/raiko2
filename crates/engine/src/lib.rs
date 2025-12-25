//! Raiko2 Engine - queue-driven proving orchestration.
//!
//! ## Module Structure
//!
//! - `worker` - Supervised worker management with auto-restart
//! - `tasks` - Task types and outputs
//!
//! Prover integration is provided via `raiko2-prover` and wired through
//! `Engine` / `tasks::EngineTask`.

#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

pub mod tasks;
pub mod worker;

pub use tasks::{EncodedGuestInput, EngineTaskId, EngineTaskKey, ProposalStage};

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

use crate::tasks::{EngineOutput, EngineTask};

pub struct Engine<S>
where
    S: PipelineSpec,
    S::Prover: Prover<S::Backend, GuestInput = S::GuestInput>,
    S::Backend: ProverBackend,
    S::Provider: Provider,
{
    inner: Arc<Inner<S>>,
}

struct Inner<S>
where
    S: PipelineSpec,
{
    spec: S,
    scheduler: Scheduler<EngineTask, EngineOutput<S::GuestInput>, EngineTaskKey>,
    context: ProofContext,
}

impl<S> Clone for Engine<S>
where
    S: PipelineSpec,
    S::Prover: Prover<S::Backend, GuestInput = S::GuestInput>,
    S::Backend: ProverBackend,
    S::Provider: Provider,
{
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<S> Engine<S>
where
    S: PipelineSpec,
    S::Prover: Prover<S::Backend, GuestInput = S::GuestInput>,
    S::Backend: ProverBackend,
    S::Provider: Provider,
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

    pub fn new(spec: S, context: ProofContext) -> Self {
        Self::with_store_and_scheduler_config(
            spec,
            context,
            MemoryStore::new(),
            Self::default_scheduler_config(),
        )
    }

    pub fn with_store<Store>(spec: S, context: ProofContext, store: Store) -> Self
    where
        Store: raiko2_queue::TaskStore<EngineTask, EngineOutput<S::GuestInput>, EngineTaskKey>
            + 'static,
    {
        Self::with_store_and_scheduler_config(
            spec,
            context,
            store,
            Self::default_scheduler_config(),
        )
    }

    pub fn with_store_and_scheduler_config<Store>(
        spec: S,
        context: ProofContext,
        store: Store,
        scheduler_config: SchedulerConfig,
    ) -> Self
    where
        Store: raiko2_queue::TaskStore<EngineTask, EngineOutput<S::GuestInput>, EngineTaskKey>
            + 'static,
    {
        Self {
            inner: Arc::new(Inner {
                spec,
                scheduler: Scheduler::with_config(store, scheduler_config),
                context,
            }),
        }
    }

    fn context_for_proposal(&self, proposal_id: u64) -> ProofContext {
        let mut ctx = self.inner.context.clone();
        ctx.request.proposal_id = proposal_id;
        ctx
    }

    fn proposal_task_id(&self, proposal_id: u64, stage: ProposalStage) -> EngineTaskId {
        EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: self.inner.spec.pipeline_key(),
            proposal_id,
            stage,
        })
    }

    pub async fn submit_proposal_proof(
        &self,
        proposal_id: u64,
    ) -> Result<EngineTaskId, TaskStoreError> {
        let preflight_id = self.proposal_task_id(proposal_id, ProposalStage::Preflight);
        let preflight_task = self
            .inner
            .scheduler
            .submit(
                preflight_id,
                NewTask {
                    priority: Priority::Low,
                    payload: EngineTask::Preflight { proposal_id },
                },
                vec![],
            )
            .await?;

        let validation_id = self.proposal_task_id(proposal_id, ProposalStage::Validation);
        let validation_task = self
            .inner
            .scheduler
            .submit(
                validation_id,
                NewTask {
                    priority: Priority::Low,
                    payload: EngineTask::Validate {
                        proposal_id,
                        preflight_task: preflight_task.clone(),
                    },
                },
                vec![preflight_task],
            )
            .await?;

        let encode_id = self.proposal_task_id(proposal_id, ProposalStage::Encode);
        let encode_task = self
            .inner
            .scheduler
            .submit(
                encode_id,
                NewTask {
                    priority: Priority::Low,
                    payload: EngineTask::Encode {
                        proposal_id,
                        input_task: validation_task.clone(),
                    },
                },
                vec![validation_task],
            )
            .await?;

        let prove_id = self.proposal_task_id(proposal_id, ProposalStage::Prove);
        self.inner
            .scheduler
            .submit(
                prove_id,
                NewTask {
                    priority: Priority::Medium,
                    payload: EngineTask::ProveProposal {
                        proposal_id,
                        input_task: encode_task.clone(),
                    },
                },
                vec![encode_task],
            )
            .await
    }

    pub async fn get(
        &self,
        id: EngineTaskId,
    ) -> Result<Option<TaskView<EngineOutput<S::GuestInput>, EngineTaskKey>>, TaskStoreError> {
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
        S::Prover: Prover<S::Backend, GuestInput = S::GuestInput> + 'static,
        S::Backend: ProverBackend + 'static,
        S::Provider: Provider + 'static,
    {
        self.start_workers_with_maintenance_interval(concurrency, Duration::from_millis(200));
    }

    pub fn start_workers_with_maintenance_interval(
        &self,
        concurrency: usize,
        maintenance_interval: Duration,
    ) where
        S: 'static,
        S::Prover: Prover<S::Backend, GuestInput = S::GuestInput> + 'static,
        S::Backend: ProverBackend + 'static,
        S::Provider: Provider + 'static,
    {
        let notify = self.inner.scheduler.notifier();
        for i in 0..concurrency {
            spawn_worker_supervised(self.clone(), notify.clone(), format!("engine-{i}"));
        }

        spawn_maintenance_supervised(self.clone(), maintenance_interval);
    }

    async fn execute(&self, task: EngineTask) -> Result<EngineOutput<S::GuestInput>, String> {
        match task {
            EngineTask::Preflight { proposal_id } => {
                let ctx = self.context_for_proposal(proposal_id);
                let pipeline = Pipeline::new(&self.inner.spec);
                pipeline
                    .preflight(&ctx)
                    .await
                    .map(|input| EngineOutput::GuestInput(Box::new(input)))
                    .map_err(|e| e.to_string())
            }
            EngineTask::Validate {
                proposal_id,
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

                let ctx = self.context_for_proposal(proposal_id);
                let pipeline = Pipeline::new(&self.inner.spec);
                let validated = pipeline
                    .validate(&ctx, preflight_input)
                    .map_err(|e| e.to_string())?;
                Ok(EngineOutput::GuestInput(Box::new(validated)))
            }
            EngineTask::Encode {
                proposal_id,
                input_task,
            } => {
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

                let ctx = self.context_for_proposal(proposal_id);
                let encoded = self
                    .inner
                    .spec
                    .prover()
                    .encode(&guest_input, &ctx.config)
                    .map_err(|e| e.to_string())?;

                Ok(EngineOutput::EncodedInput(Box::new(
                    PipelineStageResult::new(PipelineStage::Encode, encoded),
                )))
            }
            EngineTask::ProveProposal {
                proposal_id: _,
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

                let encoded = match input_view.state {
                    TaskState::Succeeded {
                        output: EngineOutput::EncodedInput(input),
                    } => match input.stage {
                        PipelineStage::Encode => input.output,
                        _ => {
                            return Err("input task did not produce encoded GuestInput".to_string());
                        }
                    },
                    TaskState::Succeeded { .. } => {
                        return Err("input task did not produce encoded input".to_string());
                    }
                    _ => return Err("input task not completed".to_string()),
                };

                let proof = self
                    .inner
                    .spec
                    .prover()
                    .prove_encoded(
                        encoded,
                        &self.inner.context.config,
                        self.inner.spec.backend(),
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(EngineOutput::Proof(Box::new(PipelineStageResult::new(
                    PipelineStage::Prove,
                    proof,
                ))))
            }
            EngineTask::Aggregate {
                proposal_ids: _,
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
                                    "dependency task did not produce proposal proof".to_string()
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
                    .spec
                    .prover()
                    .aggregate(
                        AggregationGuestInput { proofs },
                        &self.inner.context.config,
                        self.inner.spec.backend(),
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

fn spawn_worker_supervised<S>(engine: Engine<S>, notify: Arc<tokio::sync::Notify>, worker: String)
where
    S: PipelineSpec + 'static,
    S::Prover: Prover<S::Backend, GuestInput = S::GuestInput> + 'static,
    S::Backend: ProverBackend + 'static,
    S::Provider: Provider + 'static,
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

fn spawn_maintenance_supervised<S>(engine: Engine<S>, maintenance_interval: Duration)
where
    S: PipelineSpec + 'static,
    S::Prover: Prover<S::Backend, GuestInput = S::GuestInput> + 'static,
    S::Backend: ProverBackend + 'static,
    S::Provider: Provider + 'static,
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
    use std::time::Duration;

    use alloy_primitives::Bytes;
    use raiko2_pipeline::{
        NoopManifestBuilder, NoopValidation, PipelineKey, PipelineSpec, Preflight, ProofStage,
        ProverBackend,
    };
    use raiko2_primitives::{
        AggregationGuestInput, GuestInput, Proof, ProofContext, ProofRequest, ProverConfig,
        RaikoError, RaikoResult,
    };
    use raiko2_prover::{GuestInputCodec, Prover};
    use raiko2_provider::Provider;
    use raiko2_queue::{RetryPolicy, SchedulerConfig, TaskState};

    use crate::Engine;
    use crate::tasks::EngineOutput;

    struct MockProver;

    impl GuestInputCodec<GuestInput> for MockProver {
        fn encode(&self, input: &GuestInput, _config: &ProverConfig) -> RaikoResult<Bytes> {
            Ok(Bytes::from(input.taiko.proposal_id.to_le_bytes().to_vec()))
        }
    }

    #[async_trait::async_trait]
    impl Prover<TestBackend> for MockProver {
        type GuestInput = GuestInput;

        fn encode(&self, input: &Self::GuestInput, config: &ProverConfig) -> RaikoResult<Bytes> {
            GuestInputCodec::encode(self, input, config)
        }

        async fn prove_encoded(
            &self,
            input: Bytes,
            _config: &ProverConfig,
            _backend: &TestBackend,
        ) -> RaikoResult<Proof> {
            let raw = input.as_ref();
            if raw.len() != 8 {
                return Err(RaikoError::Guest("Encoded input missing proposal id".to_string()));
            }
            let mut buf = [0u8; 8];
            buf.copy_from_slice(raw);
            let proposal_id = u64::from_le_bytes(buf);
            assert_eq!(proposal_id, 1);
            Ok(Proof {
                proof: Some("mock-proof".to_string()),
                ..Default::default()
            })
        }

        async fn aggregate(
            &self,
            _input: AggregationGuestInput,
            _config: &ProverConfig,
            _backend: &TestBackend,
        ) -> RaikoResult<Proof> {
            Ok(Proof {
                proof: Some("mock-agg-proof".to_string()),
                ..Default::default()
            })
        }
    }

    struct FailingProver;

    impl GuestInputCodec<GuestInput> for FailingProver {
        fn encode(&self, input: &GuestInput, _config: &ProverConfig) -> RaikoResult<Bytes> {
            Ok(Bytes::from(input.taiko.proposal_id.to_le_bytes().to_vec()))
        }
    }

    #[async_trait::async_trait]
    impl Prover<TestBackend> for FailingProver {
        type GuestInput = GuestInput;

        fn encode(&self, input: &Self::GuestInput, config: &ProverConfig) -> RaikoResult<Bytes> {
            GuestInputCodec::encode(self, input, config)
        }

        async fn prove_encoded(
            &self,
            _input: Bytes,
            _config: &ProverConfig,
            _backend: &TestBackend,
        ) -> RaikoResult<Proof> {
            Err(RaikoError::Guest("boom".to_string()))
        }

        async fn aggregate(
            &self,
            _input: AggregationGuestInput,
            _config: &ProverConfig,
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

    struct TestSpec<Pv> {
        prover: Pv,
        backend: TestBackend,
        provider: MockProvider,
    }

    impl<Pv> TestSpec<Pv> {
        fn new(prover: Pv) -> Self {
            Self {
                prover,
                backend: TestBackend,
                provider: MockProvider,
            }
        }
    }
    const NOOP_VALIDATION: NoopValidation<GuestInput> = NoopValidation::new();
    const NOOP_MANIFEST: NoopManifestBuilder = NoopManifestBuilder;

    #[async_trait::async_trait]
    impl<Pv> Preflight for TestSpec<Pv>
    where
        Pv: Send + Sync,
    {
        type Input = GuestInput;

        async fn preflight<P: Provider>(
            &self,
            ctx: &ProofContext,
            _provider: &P,
        ) -> RaikoResult<GuestInput> {
            let mut input = GuestInput::default();
            input.taiko.proposal_id = ctx.request.proposal_id;
            Ok(input)
        }
    }

    impl<Pv> PipelineSpec for TestSpec<Pv>
    where
        Pv: Send + Sync,
    {
        type GuestInput = GuestInput;
        type Preflight = Self;
        type Validation = NoopValidation<GuestInput>;
        type ManifestBuilder = NoopManifestBuilder;
        type Prover = Pv;
        type Backend = TestBackend;
        type Provider = MockProvider;

        fn pipeline_key(&self) -> PipelineKey {
            PipelineKey::ShastaNative
        }

        fn prover(&self) -> &Self::Prover {
            &self.prover
        }

        fn backend(&self) -> &Self::Backend {
            &self.backend
        }

        fn provider(&self) -> &Self::Provider {
            &self.provider
        }

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
    async fn submit_proposal_proof_runs_dependency_pipeline() {
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec::new(MockProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            Engine::<TestSpec<MockProver>>::default_scheduler_config(),
        );

        let job_id = engine.submit_proposal_proof(1).await.unwrap();

        assert!(engine.run_one("w1").await.unwrap());
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
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec::new(FailingProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            scheduler_config,
        );

        let job_id = engine.submit_proposal_proof(1).await.unwrap();

        assert!(engine.run_one("w1").await.unwrap());
        assert!(engine.run_one("w1").await.unwrap());
        assert!(engine.run_one("w1").await.unwrap());
        assert!(engine.run_one("w1").await.unwrap());

        let view = engine.get(job_id).await.unwrap().unwrap();
        assert!(matches!(view.state, TaskState::Failed { .. }));
    }
}
