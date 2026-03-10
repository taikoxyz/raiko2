//! Application state for the HTTP server.

mod engine;
mod factory;
mod setup;
mod types;

pub use factory::{PipelineFactory, StaticPipelineFactory};
pub use types::ProofStatus;

use crate::config::{Config, QueueBackend};
use anyhow::Result;
use raiko2_engine::Engine;
use raiko2_pipeline::{
    NativeBackend, PipelineKey, Risc0ShastaBackend, Sp1ShastaBackend,
    forks::shasta::{RISC0_SHASTA_BACKEND, SP1_SHASTA_BACKEND, ShastaSpec},
};
use raiko2_prover::{agent::AgentProver, native::NativeProver, risc0::Risc0Prover, sp1::Sp1Prover};
use raiko2_provider::NetworkProvider;
use raiko2_queue::{MemoryStore, SchedulerConfig};
use std::sync::Arc;
use std::time::Duration;

#[cfg(feature = "redis-queue")]
use raiko2_engine::EngineTaskKey;
#[cfg(feature = "redis-queue")]
use raiko2_engine::tasks::EngineTask;

#[cfg(feature = "redis-queue")]
type EngineOutput<I> = raiko2_engine::tasks::EngineOutput<I>;

type Risc0Spec = ShastaSpec<Risc0Prover, Risc0ShastaBackend, NetworkProvider>;
type Sp1Spec = ShastaSpec<Sp1Prover, Sp1ShastaBackend, NetworkProvider>;
type NativeSpec = ShastaSpec<NativeProver, NativeBackend, NetworkProvider>;
type AgentSpec = ShastaSpec<AgentProver, Risc0ShastaBackend, NetworkProvider>;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub pipelines: Arc<dyn PipelineFactory>,
}

impl AppState {
    /// Create new application state.
    pub async fn new(config: Config) -> Result<Self> {
        let scheduler_config = setup::scheduler_config(&config);
        let agent_scheduler_config = setup::agent_scheduler_config(&config);
        let workers = config.queue.workers;
        let maintenance_interval = Duration::from_millis(config.queue.maintenance_interval_ms);

        let mut factory = StaticPipelineFactory::default();

        let risc0_engine = build_risc0_engine(&config, scheduler_config.clone()).await?;
        risc0_engine.start_workers_with_maintenance_interval(workers, maintenance_interval);
        factory.insert(PipelineKey::ShastaRisc0, Arc::new(risc0_engine));

        let agent_engine = build_agent_engine(&config, agent_scheduler_config).await?;
        agent_engine.start_workers_with_maintenance_interval(workers, maintenance_interval);
        factory.insert(PipelineKey::ShastaAgentRisc0, Arc::new(agent_engine));

        let sp1_engine = build_sp1_engine(&config, scheduler_config.clone()).await?;
        sp1_engine.start_workers_with_maintenance_interval(workers, maintenance_interval);
        factory.insert(PipelineKey::ShastaSp1, Arc::new(sp1_engine));

        let native_engine = build_native_engine(&config, scheduler_config).await?;
        native_engine.start_workers_with_maintenance_interval(workers, maintenance_interval);
        factory.insert(PipelineKey::ShastaNative, Arc::new(native_engine));

        Ok(Self {
            config: Arc::new(config),
            pipelines: Arc::new(factory),
        })
    }
}

#[cfg_attr(not(feature = "redis-queue"), allow(clippy::unused_async))]
async fn build_risc0_engine(
    config: &Config,
    scheduler_config: SchedulerConfig,
) -> Result<Engine<Risc0Spec>> {
    let risc0_config = setup::risc0_prover_config(config);

    let engine = match config.queue.backend {
        QueueBackend::Memory => {
            let provider = setup::build_provider(config)?;
            let context = setup::build_context(config, "risc0");
            let spec = ShastaSpec::new(
                PipelineKey::ShastaRisc0,
                Risc0Prover::new(risc0_config),
                RISC0_SHASTA_BACKEND,
                provider,
            );
            Engine::with_store_and_scheduler_config(
                spec,
                context,
                MemoryStore::with_lease(scheduler_config.lease_duration),
                scheduler_config,
            )
        }
        QueueBackend::Redis => {
            #[cfg(feature = "redis-queue")]
            {
                type Risc0Output =
                    EngineOutput<<Risc0Spec as raiko2_pipeline::PipelineSpec>::GuestInput>;
                let provider = setup::build_provider(config)?;
                let context = setup::build_context(config, "risc0");
                let url = config.queue.redis_url.clone().unwrap_or_default();
                let namespace =
                    setup::queue_namespace(&config.queue.namespace, PipelineKey::ShastaRisc0);
                let store =
                    raiko2_queue::RedisStore::<EngineTask, Risc0Output, EngineTaskKey>::connect(
                        &url,
                        &namespace,
                        scheduler_config.lease_duration,
                    )
                    .await?;
                let spec = ShastaSpec::new(
                    PipelineKey::ShastaRisc0,
                    Risc0Prover::new(risc0_config),
                    RISC0_SHASTA_BACKEND,
                    provider,
                );
                Engine::with_store_and_scheduler_config(spec, context, store, scheduler_config)
            }

            #[cfg(not(feature = "redis-queue"))]
            {
                anyhow::bail!(
                    "queue backend redis requires building raiko2 with `--features redis-queue`"
                );
            }
        }
    };

    Ok(engine)
}

#[cfg_attr(not(feature = "redis-queue"), allow(clippy::unused_async))]
async fn build_sp1_engine(
    config: &Config,
    scheduler_config: SchedulerConfig,
) -> Result<Engine<Sp1Spec>> {
    let sp1_config = setup::sp1_prover_config(config);

    let engine = match config.queue.backend {
        QueueBackend::Memory => {
            let provider = setup::build_provider(config)?;
            let context = setup::build_context(config, "sp1");
            let spec = ShastaSpec::new(
                PipelineKey::ShastaSp1,
                Sp1Prover::new(sp1_config),
                SP1_SHASTA_BACKEND,
                provider,
            );
            Engine::with_store_and_scheduler_config(
                spec,
                context,
                MemoryStore::with_lease(scheduler_config.lease_duration),
                scheduler_config,
            )
        }
        QueueBackend::Redis => {
            #[cfg(feature = "redis-queue")]
            {
                type Sp1Output =
                    EngineOutput<<Sp1Spec as raiko2_pipeline::PipelineSpec>::GuestInput>;
                let provider = setup::build_provider(config)?;
                let context = setup::build_context(config, "sp1");
                let url = config.queue.redis_url.clone().unwrap_or_default();
                let namespace =
                    setup::queue_namespace(&config.queue.namespace, PipelineKey::ShastaSp1);
                let store =
                    raiko2_queue::RedisStore::<EngineTask, Sp1Output, EngineTaskKey>::connect(
                        &url,
                        &namespace,
                        scheduler_config.lease_duration,
                    )
                    .await?;
                let spec = ShastaSpec::new(
                    PipelineKey::ShastaSp1,
                    Sp1Prover::new(sp1_config),
                    SP1_SHASTA_BACKEND,
                    provider,
                );
                Engine::with_store_and_scheduler_config(spec, context, store, scheduler_config)
            }

            #[cfg(not(feature = "redis-queue"))]
            {
                anyhow::bail!(
                    "queue backend redis requires building raiko2 with `--features redis-queue`"
                );
            }
        }
    };

    Ok(engine)
}

#[cfg_attr(not(feature = "redis-queue"), allow(clippy::unused_async))]
async fn build_native_engine(
    config: &Config,
    scheduler_config: SchedulerConfig,
) -> Result<Engine<NativeSpec>> {
    let engine = match config.queue.backend {
        QueueBackend::Memory => {
            let provider = setup::build_provider(config)?;
            let context = setup::build_context(config, "native");
            let spec = ShastaSpec::new(
                PipelineKey::ShastaNative,
                NativeProver,
                NativeBackend,
                provider,
            );
            Engine::with_store_and_scheduler_config(
                spec,
                context,
                MemoryStore::with_lease(scheduler_config.lease_duration),
                scheduler_config,
            )
        }
        QueueBackend::Redis => {
            #[cfg(feature = "redis-queue")]
            {
                type NativeOutput =
                    EngineOutput<<NativeSpec as raiko2_pipeline::PipelineSpec>::GuestInput>;
                let provider = setup::build_provider(config)?;
                let context = setup::build_context(config, "native");
                let url = config.queue.redis_url.clone().unwrap_or_default();
                let namespace =
                    setup::queue_namespace(&config.queue.namespace, PipelineKey::ShastaNative);
                let store =
                    raiko2_queue::RedisStore::<EngineTask, NativeOutput, EngineTaskKey>::connect(
                        &url,
                        &namespace,
                        scheduler_config.lease_duration,
                    )
                    .await?;
                let spec = ShastaSpec::new(
                    PipelineKey::ShastaNative,
                    NativeProver,
                    NativeBackend,
                    provider,
                );
                Engine::with_store_and_scheduler_config(spec, context, store, scheduler_config)
            }

            #[cfg(not(feature = "redis-queue"))]
            {
                anyhow::bail!(
                    "queue backend redis requires building raiko2 with `--features redis-queue`"
                );
            }
        }
    };

    Ok(engine)
}

#[cfg_attr(not(feature = "redis-queue"), allow(clippy::unused_async))]
async fn build_agent_engine(
    config: &Config,
    scheduler_config: SchedulerConfig,
) -> Result<Engine<AgentSpec>> {
    let agent_config = setup::agent_prover_config(config);

    let engine = match config.queue.backend {
        QueueBackend::Memory => {
            let provider = setup::build_provider(config)?;
            let context = setup::build_context(config, "risc0");
            let spec = ShastaSpec::new(
                PipelineKey::ShastaAgentRisc0,
                AgentProver::new(agent_config),
                RISC0_SHASTA_BACKEND,
                provider,
            );
            Engine::with_store_and_scheduler_config(
                spec,
                context,
                MemoryStore::with_lease(scheduler_config.lease_duration),
                scheduler_config,
            )
        }
        QueueBackend::Redis => {
            #[cfg(feature = "redis-queue")]
            {
                type AgentOutput =
                    EngineOutput<<AgentSpec as raiko2_pipeline::PipelineSpec>::GuestInput>;
                let provider = setup::build_provider(config)?;
                let context = setup::build_context(config, "risc0");
                let url = config.queue.redis_url.clone().unwrap_or_default();
                let namespace =
                    setup::queue_namespace(&config.queue.namespace, PipelineKey::ShastaAgentRisc0);
                let store =
                    raiko2_queue::RedisStore::<EngineTask, AgentOutput, EngineTaskKey>::connect(
                        &url,
                        &namespace,
                        scheduler_config.lease_duration,
                    )
                    .await?;
                let spec = ShastaSpec::new(
                    PipelineKey::ShastaAgentRisc0,
                    AgentProver::new(agent_config),
                    RISC0_SHASTA_BACKEND,
                    provider,
                );
                Engine::with_store_and_scheduler_config(spec, context, store, scheduler_config)
            }

            #[cfg(not(feature = "redis-queue"))]
            {
                anyhow::bail!(
                    "queue backend redis requires building raiko2 with `--features redis-queue`"
                );
            }
        }
    };

    Ok(engine)
}
