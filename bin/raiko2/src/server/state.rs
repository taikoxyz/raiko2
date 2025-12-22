//! Application state for the HTTP server.

use crate::config::{Config, ProverType, QueueBackend, RetryStrategy};
use anyhow::Result;
use raiko2_engine::input_builder::{GuestInputBuilder, NetworkGuestInputBuilder};
use raiko2_engine::queue::EngineQueue;
use raiko2_engine::tasks::EngineOutput;
use raiko2_pipeline::{
    Risc0ShastaBackend, Sp1ShastaBackend,
    forks::shasta::{RISC0_SHASTA_BACKEND, SP1_SHASTA_BACKEND, ShastaSpec},
};
use raiko2_prover::Prover;
use raiko2_queue::{RetryPolicy, SchedulerConfig, TaskId, TaskStoreError, TaskView};
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
    Risc0(EngineQueue<ShastaSpec, Risc0ShastaBackend>),
    Sp1(EngineQueue<ShastaSpec, Sp1ShastaBackend>),
}

impl EngineHandle {
    pub async fn submit_batch_proof(&self, batch_id: u64) -> Result<TaskId, TaskStoreError> {
        match self {
            EngineHandle::Risc0(engine) => engine.submit_batch_proof(batch_id).await,
            EngineHandle::Sp1(engine) => engine.submit_batch_proof(batch_id).await,
        }
    }

    pub async fn get(&self, id: TaskId) -> Result<Option<TaskView<EngineOutput>>, TaskStoreError> {
        match self {
            EngineHandle::Risc0(engine) => engine.get(id).await,
            EngineHandle::Sp1(engine) => engine.get(id).await,
        }
    }

    pub async fn cancel(&self, id: TaskId) -> Result<(), TaskStoreError> {
        match self {
            EngineHandle::Risc0(engine) => engine.cancel(id).await,
            EngineHandle::Sp1(engine) => engine.cancel(id).await,
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
                let prover: Arc<dyn Prover<ShastaSpec, Risc0ShastaBackend>> =
                    Arc::new(raiko2_prover::risc0::Risc0Prover::new(risc0_config));
                let backend = RISC0_SHASTA_BACKEND;
                let guest_input_builder: Arc<
                    dyn GuestInputBuilder<ShastaSpec, Risc0ShastaBackend>,
                > = Arc::new(
                    NetworkGuestInputBuilder::new(
                        spec.clone(),
                        backend,
                        &config.rpc.l2_rpc,
                        config.rpc.l1_chain_id,
                        config.rpc.l2_chain_id,
                        "risc0".to_string(),
                        raiko2_primitives::ProverConfig::default(),
                    )
                    .map_err(|e| anyhow::anyhow!(e))?,
                );
                let engine = match config.queue.backend {
                    QueueBackend::Memory => {
                        EngineQueue::with_store_and_builder_and_scheduler_config(
                            spec.clone(),
                            backend,
                            prover,
                            raiko2_queue::MemoryStore::new(),
                            guest_input_builder,
                            scheduler_config.clone(),
                        )
                    }
                    QueueBackend::Redis => {
                        #[cfg(feature = "redis-queue")]
                        {
                            let url = config.queue.redis_url.clone().unwrap_or_default();
                            let store =
                                raiko2_queue::RedisStore::<EngineTask, EngineOutput>::connect(
                                    &url,
                                    &config.queue.namespace,
                                    Duration::from_secs(60),
                                )
                                .await?;
                            EngineQueue::with_store_and_builder_and_scheduler_config(
                                spec.clone(),
                                backend,
                                prover,
                                store,
                                guest_input_builder,
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
                let prover: Arc<dyn Prover<ShastaSpec, Sp1ShastaBackend>> =
                    Arc::new(raiko2_prover::sp1::Sp1Prover::new(sp1_config));
                let backend = SP1_SHASTA_BACKEND;
                let guest_input_builder: Arc<dyn GuestInputBuilder<ShastaSpec, Sp1ShastaBackend>> =
                    Arc::new(
                        NetworkGuestInputBuilder::new(
                            spec.clone(),
                            backend,
                            &config.rpc.l2_rpc,
                            config.rpc.l1_chain_id,
                            config.rpc.l2_chain_id,
                            "sp1".to_string(),
                            raiko2_primitives::ProverConfig::default(),
                        )
                        .map_err(|e| anyhow::anyhow!(e))?,
                    );
                let engine = match config.queue.backend {
                    QueueBackend::Memory => {
                        EngineQueue::with_store_and_builder_and_scheduler_config(
                            spec.clone(),
                            backend,
                            prover,
                            raiko2_queue::MemoryStore::new(),
                            guest_input_builder,
                            scheduler_config.clone(),
                        )
                    }
                    QueueBackend::Redis => {
                        #[cfg(feature = "redis-queue")]
                        {
                            let url = config.queue.redis_url.clone().unwrap_or_default();
                            let store =
                                raiko2_queue::RedisStore::<EngineTask, EngineOutput>::connect(
                                    &url,
                                    &config.queue.namespace,
                                    Duration::from_secs(60),
                                )
                                .await?;
                            EngineQueue::with_store_and_builder_and_scheduler_config(
                                spec.clone(),
                                backend,
                                prover,
                                store,
                                guest_input_builder,
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
