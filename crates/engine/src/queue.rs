use std::sync::Arc;
use std::time::Duration;

use raiko2_pipeline::{PipelineSpec, ProverBackend};
use raiko2_provider::Provider;

use crate::Engine;

pub(crate) fn spawn_worker_supervised<S, B, P>(
    engine: Engine<S, B, P>,
    notify: Arc<tokio::sync::Notify>,
    worker: String,
) where
    S: PipelineSpec + 'static,
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

pub(crate) fn spawn_maintenance_supervised<S, B, P>(
    engine: Engine<S, B, P>,
    maintenance_interval: Duration,
) where
    S: PipelineSpec + 'static,
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

    use alloy_primitives::Bytes;
    use raiko2_pipeline::{
        NoopManifestBuilder, NoopValidation, PipelineSpec, Preflight, ProofStage, ProverBackend,
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
            let bytes = bincode::serialize(input)
                .map_err(|e| RaikoError::Guest(format!("Failed to serialize input: {}", e)))?;
            Ok(Bytes::from(bytes))
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
            let input: GuestInput = bincode::deserialize(input.as_ref())
                .map_err(|e| RaikoError::Guest(format!("Failed to deserialize input: {}", e)))?;
            assert_eq!(input.taiko.proposal_id, 1);
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
            let bytes = bincode::serialize(input)
                .map_err(|e| RaikoError::Guest(format!("Failed to serialize input: {}", e)))?;
            Ok(Bytes::from(bytes))
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

    struct TestSpec;
    const NOOP_VALIDATION: NoopValidation<GuestInput> = NoopValidation(std::marker::PhantomData);
    const NOOP_MANIFEST: NoopManifestBuilder = NoopManifestBuilder;

    #[async_trait::async_trait]
    impl Preflight for TestSpec {
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

    impl PipelineSpec for TestSpec {
        type GuestInput = GuestInput;
        type Preflight = Self;
        type Validation = NoopValidation<GuestInput>;
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
    async fn submit_proposal_proof_runs_dependency_pipeline() {
        let backend = TestBackend;
        let engine = Engine::with_store_and_scheduler_config(
            TestSpec,
            backend,
            MockProvider,
            Arc::new(MockProver),
            test_context(),
            raiko2_queue::MemoryStore::new(),
            Engine::<TestSpec, TestBackend, MockProvider>::default_scheduler_config(),
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

        let job_id = engine.submit_proposal_proof(1).await.unwrap();

        assert!(engine.run_one("w1").await.unwrap());
        assert!(engine.run_one("w1").await.unwrap());
        assert!(engine.run_one("w1").await.unwrap());
        assert!(engine.run_one("w1").await.unwrap());

        let view = engine.get(job_id).await.unwrap().unwrap();
        assert!(matches!(view.state, TaskState::Failed { .. }));
    }
}
