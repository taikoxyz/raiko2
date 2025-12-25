//! Application state for the HTTP server.

use crate::config::{Config, QueueBackend, RetryStrategy};
use anyhow::Result;
use raiko2_engine::{Engine, EngineTaskId, EngineTaskKey};
use raiko2_pipeline::{
    NativeBackend, PipelineKey, Risc0ShastaBackend, Sp1ShastaBackend,
    forks::shasta::{RISC0_SHASTA_BACKEND, SP1_SHASTA_BACKEND, ShastaSpec},
};
use raiko2_primitives::{ProofContext, ProofRequest};
use raiko2_prover::{native::NativeProver, risc0::Risc0Prover, sp1::Sp1Prover};
use raiko2_provider::NetworkProvider;
use raiko2_queue::{RetryPolicy, SchedulerConfig, TaskState, TaskStoreError, TaskView};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

type EngineOutput<I> = raiko2_engine::tasks::EngineOutput<I>;

#[cfg(feature = "redis-queue")]
use raiko2_engine::tasks::EngineTask;

/// Proof status.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProofStatus {
    Pending,
    Proving,
    Completed,
    Failed,
    Cancelled,
}

/// Engine task status summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineStatusView {
    pub status: ProofStatus,
    pub proof: Option<String>,
    pub error: Option<String>,
}

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Engine abstraction used by the HTTP server.
pub trait EngineHandle: Send + Sync {
    fn submit_proposal_proof(
        &self,
        proposal_id: u64,
    ) -> BoxFuture<'_, Result<EngineTaskId, TaskStoreError>>;
    fn get_status(
        &self,
        id: EngineTaskId,
    ) -> BoxFuture<'_, Result<Option<EngineStatusView>, TaskStoreError>>;
    fn cancel(&self, id: EngineTaskId) -> BoxFuture<'_, Result<(), TaskStoreError>>;
}

fn summarize_task<I>(view: TaskView<EngineOutput<I>, EngineTaskKey>) -> EngineStatusView {
    match view.state {
        TaskState::Pending { .. } | TaskState::Ready => EngineStatusView {
            status: ProofStatus::Pending,
            proof: None,
            error: None,
        },
        TaskState::Retrying { error, .. } => EngineStatusView {
            status: ProofStatus::Pending,
            proof: None,
            error: Some(error),
        },
        TaskState::Running { .. } => EngineStatusView {
            status: ProofStatus::Proving,
            proof: None,
            error: None,
        },
        TaskState::Succeeded { output } => {
            let proof = match output {
                EngineOutput::Proof(proof) => proof.output.proof,
                _ => None,
            };
            EngineStatusView {
                status: ProofStatus::Completed,
                proof,
                error: None,
            }
        }
        TaskState::Failed { error, .. } => EngineStatusView {
            status: ProofStatus::Failed,
            proof: None,
            error: Some(error),
        },
        TaskState::Cancelled => EngineStatusView {
            status: ProofStatus::Cancelled,
            proof: None,
            error: None,
        },
    }
}

impl<S> EngineHandle for Engine<S>
where
    S: raiko2_pipeline::PipelineSpec + Send + Sync + 'static,
    S::Prover: raiko2_prover::Prover<S::Backend, GuestInput = S::GuestInput> + 'static,
    S::Backend: raiko2_pipeline::ProverBackend + 'static,
    S::Provider: raiko2_provider::Provider + 'static,
{
    fn submit_proposal_proof(
        &self,
        proposal_id: u64,
    ) -> BoxFuture<'_, Result<EngineTaskId, TaskStoreError>> {
        Box::pin(async move { self.submit_proposal_proof(proposal_id).await })
    }

    fn get_status(
        &self,
        id: EngineTaskId,
    ) -> BoxFuture<'_, Result<Option<EngineStatusView>, TaskStoreError>> {
        Box::pin(async move {
            let view = self.get(id).await?;
            Ok(view.map(summarize_task))
        })
    }

    fn cancel(&self, id: EngineTaskId) -> BoxFuture<'_, Result<(), TaskStoreError>> {
        Box::pin(async move { self.cancel(id).await })
    }
}

/// Pipeline factory for resolving engines.
pub trait PipelineFactory: Send + Sync {
    fn get(&self, key: PipelineKey) -> Option<Arc<dyn EngineHandle>>;
}

#[derive(Default)]
pub struct StaticPipelineFactory {
    engines: HashMap<PipelineKey, Arc<dyn EngineHandle>>,
}

impl StaticPipelineFactory {
    pub fn insert(&mut self, key: PipelineKey, engine: Arc<dyn EngineHandle>) {
        self.engines.insert(key, engine);
    }
}

impl PipelineFactory for StaticPipelineFactory {
    fn get(&self, key: PipelineKey) -> Option<Arc<dyn EngineHandle>> {
        self.engines.get(&key).cloned()
    }
}

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub pipelines: Arc<dyn PipelineFactory>,
}

type Risc0Spec = ShastaSpec<Risc0Prover, Risc0ShastaBackend, NetworkProvider>;
type Sp1Spec = ShastaSpec<Sp1Prover, Sp1ShastaBackend, NetworkProvider>;
type NativeSpec = ShastaSpec<NativeProver, NativeBackend, NetworkProvider>;

fn build_context(config: &Config, proof_type: &str) -> ProofContext {
    ProofContext::new(
        ProofRequest {
            l1_chain_id: config.rpc.l1_chain_id,
            l2_chain_id: config.rpc.l2_chain_id,
            proposal_id: 0,
            proof_type: proof_type.to_string(),
            blob_proof_type: None,
            prover: None,
            graffiti: None,
        },
        raiko2_primitives::ProverConfig::default(),
    )
}

fn build_provider(config: &Config) -> Result<NetworkProvider> {
    NetworkProvider::new(&config.rpc.l2_rpc).map_err(|e| anyhow::anyhow!(e))
}

#[cfg(feature = "redis-queue")]
fn queue_namespace(base: &str, key: PipelineKey) -> String {
    let base = base.trim_end_matches('/');
    format!("{}/{}", base, key.as_str())
}

impl AppState {
    /// Create new application state.
    pub async fn new(config: Config) -> Result<Self> {
        let retry_policy = match config.queue.retry.strategy {
            RetryStrategy::None => RetryPolicy::None,
            RetryStrategy::Fixed => RetryPolicy::Fixed {
                max_attempts: config.queue.retry.max_attempts,
                delay: Duration::from_millis(config.queue.retry.fixed_delay_ms),
            },
            RetryStrategy::Exponential => RetryPolicy::Exponential {
                max_attempts: config.queue.retry.max_attempts,
                base_delay: Duration::from_millis(config.queue.retry.base_delay_ms),
                max_delay: Duration::from_millis(config.queue.retry.max_delay_ms),
            },
        };
        let scheduler_config = SchedulerConfig {
            lease_duration: Duration::from_secs(60),
            retry: retry_policy,
        };

        let mut factory = StaticPipelineFactory::default();
        let workers = config.queue.workers;
        let maintenance_interval = Duration::from_millis(config.queue.maintenance_interval_ms);

        let risc0_config = raiko2_prover::risc0::Risc0Config {
            bonsai: config.prover.risc0.bonsai,
            snark: config.prover.risc0.snark,
            profile: false,
            execution_po2: 20,
            verify: true,
        };
        let risc0_engine: Engine<Risc0Spec> = match config.queue.backend {
            QueueBackend::Memory => {
                let provider = build_provider(&config)?;
                let context = build_context(&config, "risc0");
                let spec = ShastaSpec::new(
                    PipelineKey::ShastaRisc0,
                    Risc0Prover::new(risc0_config.clone()),
                    RISC0_SHASTA_BACKEND,
                    provider,
                );
                Engine::with_store_and_scheduler_config(
                    spec,
                    context,
                    raiko2_queue::MemoryStore::new(),
                    scheduler_config.clone(),
                )
            }
            QueueBackend::Redis => {
                #[cfg(feature = "redis-queue")]
                {
                    type Risc0Output =
                        EngineOutput<<Risc0Spec as raiko2_pipeline::PipelineSpec>::GuestInput>;
                    let provider = build_provider(&config)?;
                    let context = build_context(&config, "risc0");
                    let url = config.queue.redis_url.clone().unwrap_or_default();
                    let namespace =
                        queue_namespace(&config.queue.namespace, PipelineKey::ShastaRisc0);
                    let store = raiko2_queue::RedisStore::<EngineTask, Risc0Output, EngineTaskKey>::connect(
                        &url,
                        &namespace,
                        Duration::from_secs(60),
                    )
                    .await?;
                    let spec = ShastaSpec::new(
                        PipelineKey::ShastaRisc0,
                        Risc0Prover::new(risc0_config.clone()),
                        RISC0_SHASTA_BACKEND,
                        provider,
                    );
                    Engine::with_store_and_scheduler_config(
                        spec,
                        context,
                        store,
                        scheduler_config.clone(),
                    )
                }

                #[cfg(not(feature = "redis-queue"))]
                {
                    anyhow::bail!(
                        "queue backend redis requires building raiko2 with `--features redis-queue`"
                    );
                }
            }
        };
        risc0_engine.start_workers_with_maintenance_interval(workers, maintenance_interval);
        factory.insert(PipelineKey::ShastaRisc0, Arc::new(risc0_engine));

        let sp1_config = raiko2_prover::sp1::Sp1Config {
            recursion: if config.prover.sp1.plonk {
                raiko2_prover::sp1::RecursionMode::Plonk
            } else {
                raiko2_prover::sp1::RecursionMode::Compressed
            },
            prover: if config.prover.sp1.network {
                Some(raiko2_prover::sp1::ProverMode::Network)
            } else {
                Some(raiko2_prover::sp1::ProverMode::Local)
            },
            verify: true,
        };
        let sp1_engine: Engine<Sp1Spec> = match config.queue.backend {
            QueueBackend::Memory => {
                let provider = build_provider(&config)?;
                let context = build_context(&config, "sp1");
                let spec = ShastaSpec::new(
                    PipelineKey::ShastaSp1,
                    Sp1Prover::new(sp1_config.clone()),
                    SP1_SHASTA_BACKEND,
                    provider,
                );
                Engine::with_store_and_scheduler_config(
                    spec,
                    context,
                    raiko2_queue::MemoryStore::new(),
                    scheduler_config.clone(),
                )
            }
            QueueBackend::Redis => {
                #[cfg(feature = "redis-queue")]
                {
                    type Sp1Output =
                        EngineOutput<<Sp1Spec as raiko2_pipeline::PipelineSpec>::GuestInput>;
                    let provider = build_provider(&config)?;
                    let context = build_context(&config, "sp1");
                    let url = config.queue.redis_url.clone().unwrap_or_default();
                    let namespace =
                        queue_namespace(&config.queue.namespace, PipelineKey::ShastaSp1);
                    let store =
                        raiko2_queue::RedisStore::<EngineTask, Sp1Output, EngineTaskKey>::connect(
                            &url,
                            &namespace,
                            Duration::from_secs(60),
                        )
                        .await?;
                    let spec = ShastaSpec::new(
                        PipelineKey::ShastaSp1,
                        Sp1Prover::new(sp1_config.clone()),
                        SP1_SHASTA_BACKEND,
                        provider,
                    );
                    Engine::with_store_and_scheduler_config(
                        spec,
                        context,
                        store,
                        scheduler_config.clone(),
                    )
                }

                #[cfg(not(feature = "redis-queue"))]
                {
                    anyhow::bail!(
                        "queue backend redis requires building raiko2 with `--features redis-queue`"
                    );
                }
            }
        };
        sp1_engine.start_workers_with_maintenance_interval(workers, maintenance_interval);
        factory.insert(PipelineKey::ShastaSp1, Arc::new(sp1_engine));

        let native_engine: Engine<NativeSpec> = match config.queue.backend {
            QueueBackend::Memory => {
                let provider = build_provider(&config)?;
                let context = build_context(&config, "native");
                let spec = ShastaSpec::new(
                    PipelineKey::ShastaNative,
                    NativeProver,
                    NativeBackend,
                    provider,
                );
                Engine::with_store_and_scheduler_config(
                    spec,
                    context,
                    raiko2_queue::MemoryStore::new(),
                    scheduler_config.clone(),
                )
            }
            QueueBackend::Redis => {
                #[cfg(feature = "redis-queue")]
                {
                    type NativeOutput =
                        EngineOutput<<NativeSpec as raiko2_pipeline::PipelineSpec>::GuestInput>;
                    let provider = build_provider(&config)?;
                    let context = build_context(&config, "native");
                    let url = config.queue.redis_url.clone().unwrap_or_default();
                    let namespace =
                        queue_namespace(&config.queue.namespace, PipelineKey::ShastaNative);
                    let store = raiko2_queue::RedisStore::<EngineTask, NativeOutput, EngineTaskKey>::connect(
                        &url,
                        &namespace,
                        Duration::from_secs(60),
                    )
                    .await?;
                    let spec = ShastaSpec::new(
                        PipelineKey::ShastaNative,
                        NativeProver,
                        NativeBackend,
                        provider,
                    );
                    Engine::with_store_and_scheduler_config(
                        spec,
                        context,
                        store,
                        scheduler_config.clone(),
                    )
                }

                #[cfg(not(feature = "redis-queue"))]
                {
                    anyhow::bail!(
                        "queue backend redis requires building raiko2 with `--features redis-queue`"
                    );
                }
            }
        };
        native_engine.start_workers_with_maintenance_interval(workers, maintenance_interval);
        factory.insert(PipelineKey::ShastaNative, Arc::new(native_engine));

        Ok(Self {
            config: Arc::new(config),
            pipelines: Arc::new(factory),
        })
    }
}
