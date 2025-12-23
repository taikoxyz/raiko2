//! Application state for the HTTP server.

use crate::config::{Config, ProverType, QueueBackend, RetryStrategy};
use anyhow::Result;
use raiko2_engine::tasks::EngineOutput;
use raiko2_engine::{Engine, EngineTaskId, EngineTaskKey};
use raiko2_pipeline::{
    NativeBackend, Risc0ShastaBackend, Sp1ShastaBackend,
    forks::shasta::{RISC0_SHASTA_BACKEND, SP1_SHASTA_BACKEND, ShastaSpec},
};
use raiko2_primitives::{ProofContext, ProofRequest};
use raiko2_prover::{Prover, native::NativeProver};
use raiko2_provider::NetworkProvider;
use raiko2_queue::{RetryPolicy, SchedulerConfig, TaskStoreError, TaskView};
use std::sync::Arc;
use std::time::Duration;

#[cfg(feature = "redis-queue")]
use raiko2_engine::tasks::EngineTask;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub engine: EngineHandle,
}

#[derive(Clone)]
pub enum EngineHandle {
    Risc0(Engine<ShastaSpec, Risc0ShastaBackend, NetworkProvider>),
    Sp1(Engine<ShastaSpec, Sp1ShastaBackend, NetworkProvider>),
    Native(Engine<ShastaSpec, NativeBackend, NetworkProvider>),
}

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

impl EngineHandle {
    pub async fn submit_proposal_proof(
        &self,
        proposal_id: u64,
    ) -> Result<EngineTaskId, TaskStoreError> {
        match self {
            EngineHandle::Risc0(engine) => engine.submit_proposal_proof(proposal_id).await,
            EngineHandle::Sp1(engine) => engine.submit_proposal_proof(proposal_id).await,
            EngineHandle::Native(engine) => engine.submit_proposal_proof(proposal_id).await,
        }
    }

    pub async fn get(
        &self,
        id: EngineTaskId,
    ) -> Result<Option<TaskView<EngineOutput, EngineTaskKey>>, TaskStoreError> {
        match self {
            EngineHandle::Risc0(engine) => engine.get(id).await,
            EngineHandle::Sp1(engine) => engine.get(id).await,
            EngineHandle::Native(engine) => engine.get(id).await,
        }
    }

    pub async fn cancel(&self, id: EngineTaskId) -> Result<(), TaskStoreError> {
        match self {
            EngineHandle::Risc0(engine) => engine.cancel(id).await,
            EngineHandle::Sp1(engine) => engine.cancel(id).await,
            EngineHandle::Native(engine) => engine.cancel(id).await,
        }
    }

    pub fn start_workers_with_maintenance_interval(
        &self,
        concurrency: usize,
        maintenance_interval: Duration,
    ) {
        match self {
            EngineHandle::Risc0(engine) => {
                engine.start_workers_with_maintenance_interval(concurrency, maintenance_interval);
            }
            EngineHandle::Sp1(engine) => {
                engine.start_workers_with_maintenance_interval(concurrency, maintenance_interval);
            }
            EngineHandle::Native(engine) => {
                engine.start_workers_with_maintenance_interval(concurrency, maintenance_interval);
            }
        }
    }
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

        let spec = ShastaSpec::default();
        let engine = match config.prover.prover_type {
            ProverType::Risc0 => {
                let risc0_config = raiko2_prover::risc0::Risc0Config {
                    bonsai: config.prover.risc0.bonsai,
                    snark: config.prover.risc0.snark,
                    profile: false,
                    execution_po2: 20,
                    verify: true,
                };
                let prover: Arc<dyn Prover<Risc0ShastaBackend>> =
                    Arc::new(raiko2_prover::risc0::Risc0Prover::new(risc0_config));
                let backend = RISC0_SHASTA_BACKEND;
                let engine = match config.queue.backend {
                    QueueBackend::Memory => {
                        let provider = build_provider(&config)?;
                        let context = build_context(&config, "risc0");
                        Engine::with_store_and_scheduler_config(
                            spec.clone(),
                            backend,
                            provider,
                            prover,
                            context,
                            raiko2_queue::MemoryStore::new(),
                            scheduler_config.clone(),
                        )
                    }
                    QueueBackend::Redis => {
                        #[cfg(feature = "redis-queue")]
                        {
                            let provider = build_provider(&config)?;
                            let context = build_context(&config, "risc0");
                            let url = config.queue.redis_url.clone().unwrap_or_default();
                            let store = raiko2_queue::RedisStore::<
                                EngineTask,
                                EngineOutput,
                                EngineTaskKey,
                            >::connect(
                                &url, &config.queue.namespace, Duration::from_secs(60)
                            )
                            .await?;
                            Engine::with_store_and_scheduler_config(
                                spec.clone(),
                                backend,
                                provider,
                                prover,
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
                EngineHandle::Risc0(engine)
            }
            ProverType::Sp1 => {
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
                let prover: Arc<dyn Prover<Sp1ShastaBackend>> =
                    Arc::new(raiko2_prover::sp1::Sp1Prover::new(sp1_config));
                let backend = SP1_SHASTA_BACKEND;
                let engine = match config.queue.backend {
                    QueueBackend::Memory => {
                        let provider = build_provider(&config)?;
                        let context = build_context(&config, "sp1");
                        Engine::with_store_and_scheduler_config(
                            spec.clone(),
                            backend,
                            provider,
                            prover,
                            context,
                            raiko2_queue::MemoryStore::new(),
                            scheduler_config.clone(),
                        )
                    }
                    QueueBackend::Redis => {
                        #[cfg(feature = "redis-queue")]
                        {
                            let provider = build_provider(&config)?;
                            let context = build_context(&config, "sp1");
                            let url = config.queue.redis_url.clone().unwrap_or_default();
                            let store = raiko2_queue::RedisStore::<
                                EngineTask,
                                EngineOutput,
                                EngineTaskKey,
                            >::connect(
                                &url, &config.queue.namespace, Duration::from_secs(60)
                            )
                            .await?;
                            Engine::with_store_and_scheduler_config(
                                spec.clone(),
                                backend,
                                provider,
                                prover,
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
                EngineHandle::Sp1(engine)
            }
            ProverType::Native => {
                let prover: Arc<dyn Prover<NativeBackend>> = Arc::new(NativeProver);
                let backend = NativeBackend;
                let engine = match config.queue.backend {
                    QueueBackend::Memory => {
                        let provider = build_provider(&config)?;
                        let context = build_context(&config, "native");
                        Engine::with_store_and_scheduler_config(
                            spec.clone(),
                            backend,
                            provider,
                            prover,
                            context,
                            raiko2_queue::MemoryStore::new(),
                            scheduler_config.clone(),
                        )
                    }
                    QueueBackend::Redis => {
                        #[cfg(feature = "redis-queue")]
                        {
                            let provider = build_provider(&config)?;
                            let context = build_context(&config, "native");
                            let url = config.queue.redis_url.clone().unwrap_or_default();
                            let store = raiko2_queue::RedisStore::<
                                EngineTask,
                                EngineOutput,
                                EngineTaskKey,
                            >::connect(
                                &url, &config.queue.namespace, Duration::from_secs(60)
                            )
                            .await?;
                            Engine::with_store_and_scheduler_config(
                                spec.clone(),
                                backend,
                                provider,
                                prover,
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
                EngineHandle::Native(engine)
            }
        };

        engine.start_workers_with_maintenance_interval(
            config.queue.workers,
            Duration::from_millis(config.queue.maintenance_interval_ms),
        );

        Ok(Self {
            config: Arc::new(config),
            engine,
        })
    }
}
