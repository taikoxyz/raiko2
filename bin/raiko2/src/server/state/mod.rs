//! Application state for the HTTP server.

mod engine;
mod factory;
mod runtime_observer;
mod setup;
mod types;

pub(crate) use engine::EngineHandle;
pub use factory::{PipelineFactory, StaticPipelineFactory};
pub(crate) use runtime_observer::RuntimeObserver;
pub use types::{EngineStatusView, ProofStatus};

use crate::config::{Config, QueueBackend, ResolvedNetworkPair};
use anyhow::Result;
use raiko2_engine::{Engine, EngineObserver};
use raiko2_pipeline::{
    NativeBackend, PipelineKey, Risc0ShastaBackend, Sp1ShastaBackend,
    forks::shasta::{RISC0_SHASTA_BACKEND, SP1_SHASTA_BACKEND, ShastaSpec},
};
use raiko2_prover::{
    boundless::BoundlessProver, native::NativeProver, risc0::Risc0Prover, sp1::Sp1Prover,
};
use raiko2_provider::NetworkProvider;
use raiko2_queue::{MemoryStore, SchedulerConfig};
use raiko2_runtime::RuntimeManager;
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
type BoundlessSpec = ShastaSpec<BoundlessProver, Risc0ShastaBackend, NetworkProvider>;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub pipelines: Arc<dyn PipelineFactory>,
    pub runtime: Arc<RuntimeManager>,
}

impl AppState {
    /// Create new application state.
    pub async fn new(config: Config) -> Result<Self> {
        let runtime = Arc::new(RuntimeManager::new(config.runtime.root.clone())?);
        let runtime_observer: Arc<dyn EngineObserver> =
            Arc::new(RuntimeObserver::new(Arc::clone(&runtime)));
        let scheduler_config = setup::scheduler_config(&config);
        let boundless_scheduler_config = setup::boundless_scheduler_config(&config);
        let workers = config.queue.workers;
        let maintenance_interval = Duration::from_millis(config.queue.maintenance_interval_ms);
        let resolved_pairs = config.rpc.resolved_pairs()?;

        let mut factory = StaticPipelineFactory::default();

        for pair in &resolved_pairs {
            let risc0_engine = build_risc0_engine(
                &config,
                pair,
                scheduler_config.clone(),
                Arc::clone(&runtime_observer),
            )
            .await?;
            risc0_engine.start_workers_with_maintenance_interval(workers, maintenance_interval);
            factory.insert(
                pair.key.clone(),
                PipelineKey::ShastaRisc0,
                Arc::new(risc0_engine),
            );

            let boundless_engine = build_boundless_engine(
                &config,
                pair,
                boundless_scheduler_config.clone(),
                Arc::clone(&runtime_observer),
            )
            .await?;
            boundless_engine.start_workers_with_maintenance_interval(workers, maintenance_interval);
            factory.insert(
                pair.key.clone(),
                PipelineKey::ShastaRisc0Boundless,
                Arc::new(boundless_engine),
            );

            let sp1_engine = build_sp1_engine(
                &config,
                pair,
                scheduler_config.clone(),
                Arc::clone(&runtime_observer),
            )
            .await?;
            sp1_engine.start_workers_with_maintenance_interval(workers, maintenance_interval);
            factory.insert(
                pair.key.clone(),
                PipelineKey::ShastaSp1,
                Arc::new(sp1_engine),
            );

            let native_engine = build_native_engine(
                &config,
                pair,
                scheduler_config.clone(),
                Arc::clone(&runtime_observer),
            )
            .await?;
            native_engine.start_workers_with_maintenance_interval(workers, maintenance_interval);
            factory.insert(
                pair.key.clone(),
                PipelineKey::ShastaNative,
                Arc::new(native_engine),
            );
        }

        Ok(Self {
            config: Arc::new(config),
            pipelines: Arc::new(factory),
            runtime,
        })
    }
}

#[cfg_attr(not(feature = "redis-queue"), allow(clippy::unused_async))]
async fn build_risc0_engine(
    config: &Config,
    pair: &ResolvedNetworkPair,
    scheduler_config: SchedulerConfig,
    observer: Arc<dyn EngineObserver>,
) -> Result<Engine<Risc0Spec>> {
    let risc0_config = setup::risc0_prover_config(config);

    let engine = match config.queue.backend {
        QueueBackend::Memory => {
            let provider = setup::build_provider(config, pair)?;
            let context = setup::build_context(config, pair, "risc0")?;
            let spec = ShastaSpec::new(
                PipelineKey::ShastaRisc0,
                Risc0Prover::new(risc0_config),
                RISC0_SHASTA_BACKEND,
                provider,
            );
            Engine::with_store_scheduler_config_and_observer(
                spec,
                context,
                MemoryStore::with_lease(scheduler_config.lease_duration),
                scheduler_config,
                Some(Arc::clone(&observer)),
            )
        }
        QueueBackend::Redis => {
            #[cfg(feature = "redis-queue")]
            {
                type Risc0Output =
                    EngineOutput<<Risc0Spec as raiko2_pipeline::PipelineSpec>::GuestInput>;
                let provider = setup::build_provider(config, pair)?;
                let context = setup::build_context(config, pair, "risc0")?;
                let url = config.queue.redis_url.clone().unwrap_or_default();
                let namespace =
                    setup::queue_namespace(&config.queue.namespace, pair, PipelineKey::ShastaRisc0);
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
                Engine::with_store_scheduler_config_and_observer(
                    spec,
                    context,
                    store,
                    scheduler_config,
                    Some(Arc::clone(&observer)),
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

    Ok(engine)
}

#[cfg_attr(not(feature = "redis-queue"), allow(clippy::unused_async))]
async fn build_sp1_engine(
    config: &Config,
    pair: &ResolvedNetworkPair,
    scheduler_config: SchedulerConfig,
    observer: Arc<dyn EngineObserver>,
) -> Result<Engine<Sp1Spec>> {
    let sp1_config = setup::sp1_prover_config(config);

    let engine = match config.queue.backend {
        QueueBackend::Memory => {
            let provider = setup::build_provider(config, pair)?;
            let context = setup::build_context(config, pair, "sp1")?;
            let spec = ShastaSpec::new(
                PipelineKey::ShastaSp1,
                Sp1Prover::new(sp1_config),
                SP1_SHASTA_BACKEND,
                provider,
            );
            Engine::with_store_scheduler_config_and_observer(
                spec,
                context,
                MemoryStore::with_lease(scheduler_config.lease_duration),
                scheduler_config,
                Some(Arc::clone(&observer)),
            )
        }
        QueueBackend::Redis => {
            #[cfg(feature = "redis-queue")]
            {
                type Sp1Output =
                    EngineOutput<<Sp1Spec as raiko2_pipeline::PipelineSpec>::GuestInput>;
                let provider = setup::build_provider(config, pair)?;
                let context = setup::build_context(config, pair, "sp1")?;
                let url = config.queue.redis_url.clone().unwrap_or_default();
                let namespace =
                    setup::queue_namespace(&config.queue.namespace, pair, PipelineKey::ShastaSp1);
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
                Engine::with_store_scheduler_config_and_observer(
                    spec,
                    context,
                    store,
                    scheduler_config,
                    Some(Arc::clone(&observer)),
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

    Ok(engine)
}

#[cfg_attr(not(feature = "redis-queue"), allow(clippy::unused_async))]
async fn build_native_engine(
    config: &Config,
    pair: &ResolvedNetworkPair,
    scheduler_config: SchedulerConfig,
    observer: Arc<dyn EngineObserver>,
) -> Result<Engine<NativeSpec>> {
    let engine = match config.queue.backend {
        QueueBackend::Memory => {
            let provider = setup::build_provider(config, pair)?;
            let context = setup::build_context(config, pair, "native")?;
            let spec = ShastaSpec::new(
                PipelineKey::ShastaNative,
                NativeProver,
                NativeBackend,
                provider,
            );
            Engine::with_store_scheduler_config_and_observer(
                spec,
                context,
                MemoryStore::with_lease(scheduler_config.lease_duration),
                scheduler_config,
                Some(Arc::clone(&observer)),
            )
        }
        QueueBackend::Redis => {
            #[cfg(feature = "redis-queue")]
            {
                type NativeOutput =
                    EngineOutput<<NativeSpec as raiko2_pipeline::PipelineSpec>::GuestInput>;
                let provider = setup::build_provider(config, pair)?;
                let context = setup::build_context(config, pair, "native")?;
                let url = config.queue.redis_url.clone().unwrap_or_default();
                let namespace = setup::queue_namespace(
                    &config.queue.namespace,
                    pair,
                    PipelineKey::ShastaNative,
                );
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
                Engine::with_store_scheduler_config_and_observer(
                    spec,
                    context,
                    store,
                    scheduler_config,
                    Some(Arc::clone(&observer)),
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

    Ok(engine)
}

#[cfg_attr(not(feature = "redis-queue"), allow(clippy::unused_async))]
async fn build_boundless_engine(
    config: &Config,
    pair: &ResolvedNetworkPair,
    scheduler_config: SchedulerConfig,
    observer: Arc<dyn EngineObserver>,
) -> Result<Engine<BoundlessSpec>> {
    let agent_config = setup::boundless_prover_config(config);

    let engine = match config.queue.backend {
        QueueBackend::Memory => {
            let provider = setup::build_provider(config, pair)?;
            let context = setup::build_context(config, pair, "risc0")?;
            let spec = ShastaSpec::new(
                PipelineKey::ShastaRisc0Boundless,
                BoundlessProver::new(agent_config),
                RISC0_SHASTA_BACKEND,
                provider,
            );
            Engine::with_store_scheduler_config_and_observer(
                spec,
                context,
                MemoryStore::with_lease(scheduler_config.lease_duration),
                scheduler_config,
                Some(Arc::clone(&observer)),
            )
        }
        QueueBackend::Redis => {
            #[cfg(feature = "redis-queue")]
            {
                type BoundlessOutput =
                    EngineOutput<<BoundlessSpec as raiko2_pipeline::PipelineSpec>::GuestInput>;
                let provider = setup::build_provider(config, pair)?;
                let context = setup::build_context(config, pair, "risc0")?;
                let url = config.queue.redis_url.clone().unwrap_or_default();
                let namespace = setup::queue_namespace(
                    &config.queue.namespace,
                    pair,
                    PipelineKey::ShastaRisc0Boundless,
                );
                let store =
                    raiko2_queue::RedisStore::<EngineTask, BoundlessOutput, EngineTaskKey>::connect(
                        &url,
                        &namespace,
                        scheduler_config.lease_duration,
                    )
                    .await?;
                let spec = ShastaSpec::new(
                    PipelineKey::ShastaRisc0Boundless,
                    BoundlessProver::new(agent_config),
                    RISC0_SHASTA_BACKEND,
                    provider,
                );
                Engine::with_store_scheduler_config_and_observer(
                    spec,
                    context,
                    store,
                    scheduler_config,
                    Some(observer),
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

    Ok(engine)
}
