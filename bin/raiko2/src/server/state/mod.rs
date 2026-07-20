//! Application state for the HTTP server.

mod engine;
mod factory;
mod runtime_observer;
mod setup;
mod types;

pub(crate) use engine::{EngineHandle, EngineQueueTaskState, EngineQueueTaskView};
pub use factory::{PipelineFactory, StaticPipelineFactory};
pub(crate) use runtime_observer::RuntimeObserver;
pub use types::{EngineStatusView, ProofStatus};

#[cfg(feature = "host")]
use crate::config::GuestSystem;
#[cfg(all(feature = "host", not(feature = "local-provers")))]
use crate::config::PipelineRoute;
#[cfg(feature = "host")]
use crate::config::RunnerKind;
use crate::config::{Config, ResolvedNetworkPair, RuntimeStoreBackend};
use anyhow::{Context, Result};
use raiko2_engine::{Engine, EngineObserver};
use raiko2_pipeline::{NativeBackend, PipelineKey, forks::shasta::ShastaSpec};
use raiko2_primitives::ProofType;
use raiko2_prover::gaiko2::Gaiko2Prover;
use raiko2_provider::NetworkProvider;
use raiko2_queue::{MemoryStore, SchedulerConfig};
use raiko2_runtime::RuntimeManager;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::warn;

const SUBMISSION_CHECKPOINT_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

#[cfg(feature = "local-provers")]
use raiko2_pipeline::forks::shasta::{ShastaBackends, load_shasta_backends};
#[cfg(all(feature = "host", not(feature = "local-provers")))]
use raiko2_pipeline::forks::shasta::{
    load_risc0_boundless_shasta_backend, load_sp1_shasta_backend,
};
#[cfg(feature = "host")]
use raiko2_pipeline::{Risc0ShastaBackend, Sp1ShastaBackend};
#[cfg(feature = "host")]
use raiko2_prover::{
    boundless::{BoundlessBalanceGate, BoundlessProver},
    sp1::Sp1Prover,
};
#[cfg(feature = "local-provers")]
use raiko2_prover::{native::NativeProver, risc0::Risc0Prover};

#[cfg(feature = "local-provers")]
type Risc0Spec = ShastaSpec<Risc0Prover, Risc0ShastaBackend, NetworkProvider>;
#[cfg(feature = "host")]
type Sp1Spec = ShastaSpec<Sp1Prover, Sp1ShastaBackend, NetworkProvider>;
#[cfg(feature = "local-provers")]
type NativeSpec = ShastaSpec<NativeProver, NativeBackend, NetworkProvider>;
type Gaiko2Spec = ShastaSpec<Gaiko2Prover, NativeBackend, NetworkProvider>;
#[cfg(feature = "host")]
type BoundlessSpec = ShastaSpec<BoundlessProver, Risc0ShastaBackend, NetworkProvider>;

use super::lifecycle::ProofLifecycle;
use super::sampling::ZkAnySampler;
use super::task_cleanup::spawn_runtime_cleanup_loop;

/// In-memory sliding-window limiter for ACL-protected endpoints.
/// Buckets use config indexes so each configured ACL entry gets an independent quota.
#[derive(Default)]
pub(crate) struct AclRateLimiter {
    requests: Mutex<HashMap<usize, VecDeque<Instant>>>,
}

impl AclRateLimiter {
    pub(crate) fn allow_request(
        &self,
        key_index: usize,
        limit: u32,
        window: Duration,
    ) -> Result<bool, ()> {
        let now = Instant::now();
        let mut requests = self.requests.lock().map_err(|_| ())?;
        let key_requests = requests.entry(key_index).or_default();
        key_requests.retain(|requested_at| now.duration_since(*requested_at) < window);
        if key_requests.len() >= limit as usize {
            return Ok(false);
        }
        key_requests.push_back(now);
        Ok(true)
    }
}

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub pipelines: Arc<dyn PipelineFactory>,
    pub runtime: Arc<RuntimeManager>,
    pub(crate) lifecycle: ProofLifecycle,
    pub zk_any_sampler: Arc<Mutex<ZkAnySampler>>,
    pub(crate) acl_rate_limiter: Arc<AclRateLimiter>,
    background_tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

async fn build_runtime(config: &Config) -> Result<RuntimeManager> {
    match config.runtime.store.backend {
        RuntimeStoreBackend::Memory => RuntimeManager::new_memory(
            config.runtime.environment.clone(),
            config.runtime.namespace.clone(),
        ),
        RuntimeStoreBackend::Gcs => {
            RuntimeManager::new_gcs(
                config.runtime.environment.clone(),
                config.runtime.namespace.clone(),
                config
                    .runtime
                    .store
                    .bucket
                    .clone()
                    .context("runtime.store.bucket is required for GCS")?,
                config.runtime.store.prefix.clone(),
            )
            .await
        }
    }
}

impl AppState {
    /// Create new application state.
    pub async fn new(mut config: Config) -> Result<Self> {
        config.normalize();
        config.validate()?;
        let runtime = Arc::new(build_runtime(&config).await?);
        let scheduler_config = setup::scheduler_config(&config);
        let resolved_pairs = config.rpc.resolved_pairs()?;
        #[cfg(feature = "local-provers")]
        let shasta_backends = load_shasta_backends().map_err(anyhow::Error::msg)?;
        #[cfg(all(feature = "host", not(feature = "local-provers")))]
        let boundless_backend =
            load_risc0_boundless_shasta_backend().map_err(anyhow::Error::msg)?;
        #[cfg(all(feature = "host", not(feature = "local-provers")))]
        let sp1_backend = load_sp1_shasta_backend().map_err(anyhow::Error::msg)?;
        #[cfg(feature = "host")]
        let sp1_prover = if should_create_sp1_prover(&config) {
            let sp1_config = setup::sp1_prover_config(&config);
            #[cfg(feature = "local-provers")]
            {
                Some(
                    Sp1Prover::new_with_backend(sp1_config, &shasta_backends.sp1)
                        .map_err(anyhow::Error::msg)?,
                )
            }
            #[cfg(all(feature = "host", not(feature = "local-provers")))]
            {
                Some(
                    Sp1Prover::new_with_backend(sp1_config, &sp1_backend)
                        .map_err(anyhow::Error::msg)?,
                )
            }
        } else {
            None
        };

        // One balance gate shared by every pair's Boundless prover: all pairs fund the same market
        // account (one global signer/rpc/deployment), so concurrent submissions across pairs must
        // deposit against a single combined reserved total, not one per pair.
        #[cfg(feature = "host")]
        let boundless_balance_gate = BoundlessBalanceGate::new();

        async {
            runtime.initialize().await?;
            let mut factory = StaticPipelineFactory::default();
            for pair in &resolved_pairs {
                let registration = PairPipelineRegistration {
                    config: &config,
                    pair,
                    runtime: Arc::clone(&runtime),
                    #[cfg(feature = "host")]
                    boundless_balance_gate: boundless_balance_gate.clone(),
                    #[cfg(feature = "local-provers")]
                    shasta_backends: &shasta_backends,
                    #[cfg(all(feature = "host", not(feature = "local-provers")))]
                    boundless_backend: &boundless_backend,
                    #[cfg(all(feature = "host", not(feature = "local-provers")))]
                    sp1_backend: &sp1_backend,
                    #[cfg(feature = "host")]
                    sp1_prover: sp1_prover.clone(),
                    scheduler_config: scheduler_config.clone(),
                };
                register_pair_pipelines(&mut factory, &registration)?;
            }
            let config = Arc::new(config);
            let pipelines: Arc<dyn PipelineFactory> = Arc::new(factory);
            let state = Self::from_parts(config, pipelines, Arc::clone(&runtime));
            state.finish_initialization().await
        }
        .await
    }

    async fn finish_initialization(self) -> Result<Self> {
        crate::server::handlers::validate_persisted_runtime_task_metadata(&self).await?;
        let reconciled = self.runtime.reconcile_invalidated_proof_artifacts().await?;
        if reconciled > 0 {
            tracing::info!(
                reconciled,
                "reconciled invalidated proof artifacts after runtime restart"
            );
        }
        let removed_pending = self
            .runtime
            .reconcile_unowned_pending_proof_publications()
            .await?;
        if removed_pending > 0 {
            tracing::info!(
                removed_pending,
                "removed unowned pending proof publications after runtime restart"
            );
        }
        let recovered = crate::server::handlers::recover_pending_runtime_tasks(&self).await?;
        if recovered > 0 {
            tracing::info!(
                recovered,
                "recovered pending runtime tasks into the memory queue"
            );
        }
        self.pipelines.start_workers(
            self.config.queue.workers,
            Duration::from_millis(self.config.queue.maintenance_interval_ms),
        );
        let cleanup = spawn_runtime_cleanup_loop(
            Arc::clone(&self.config),
            Arc::clone(&self.runtime),
            Arc::clone(&self.pipelines),
        );
        self.background_tasks
            .lock()
            .expect("background task lock poisoned")
            .push(cleanup);

        Ok(self)
    }

    pub(crate) fn from_parts(
        config: Arc<Config>,
        pipelines: Arc<dyn PipelineFactory>,
        runtime: Arc<RuntimeManager>,
    ) -> Self {
        let zk_any_sampler = Arc::new(Mutex::new(ZkAnySampler::from_config(&config.prover.zk_any)));
        let lifecycle = ProofLifecycle::new(Arc::clone(&runtime), Arc::clone(&pipelines));
        Self {
            config,
            pipelines,
            runtime,
            lifecycle,
            zk_any_sampler,
            acl_rate_limiter: Arc::new(AclRateLimiter::default()),
            background_tasks: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) async fn shutdown(&self) {
        self.begin_shutdown().await;
        let checkpoint_deadline = Instant::now() + SUBMISSION_CHECKPOINT_DRAIN_TIMEOUT;
        if !self
            .runtime
            .begin_draining_with_deadline(checkpoint_deadline)
            .await
        {
            warn!(
                timeout_secs = SUBMISSION_CHECKPOINT_DRAIN_TIMEOUT.as_secs(),
                "timed out waiting for accepted provider submissions to checkpoint"
            );
        }
        self.pipelines.shutdown().await;
        let handles = match self.background_tasks.lock() {
            Ok(mut handles) => handles.drain(..).collect::<Vec<_>>(),
            Err(poisoned) => {
                warn!("background task lock poisoned during shutdown");
                poisoned.into_inner().drain(..).collect()
            }
        };
        for handle in handles {
            handle.abort();
            if let Err(error) = handle.await
                && !error.is_cancelled()
            {
                warn!(%error, "background task failed before shutdown completed");
            }
        }
    }

    pub(crate) async fn begin_shutdown(&self) {
        self.lifecycle.begin_shutdown().await;
    }
}

struct PairPipelineRegistration<'a> {
    config: &'a Config,
    pair: &'a ResolvedNetworkPair,
    runtime: Arc<RuntimeManager>,
    /// Balance gate shared across all pairs (see the construction site in `ServerState::new`).
    #[cfg(feature = "host")]
    boundless_balance_gate: BoundlessBalanceGate,
    #[cfg(feature = "local-provers")]
    shasta_backends: &'a ShastaBackends,
    #[cfg(all(feature = "host", not(feature = "local-provers")))]
    boundless_backend: &'a Risc0ShastaBackend,
    #[cfg(all(feature = "host", not(feature = "local-provers")))]
    sp1_backend: &'a Sp1ShastaBackend,
    #[cfg(feature = "host")]
    sp1_prover: Option<Sp1Prover>,
    scheduler_config: SchedulerConfig,
}

#[cfg(feature = "host")]
const fn should_create_sp1_prover(config: &Config) -> bool {
    matches!(
        config.prover.route(),
        raiko2_pipeline::PipelineRoute {
            guest_system: GuestSystem::Sp1,
            ..
        } | raiko2_pipeline::PipelineRoute {
            guest_system: GuestSystem::Risc0,
            runner: RunnerKind::Network,
        }
    ) || (cfg!(feature = "local-provers") && !config.prover.is_remote_sgx_route())
}

#[allow(clippy::too_many_lines)]
fn register_pair_pipelines(
    factory: &mut StaticPipelineFactory,
    registration: &PairPipelineRegistration<'_>,
) -> Result<()> {
    if registration.config.prover.is_remote_sgx_route() {
        let runtime_observer: Arc<dyn EngineObserver> = Arc::new(RuntimeObserver::new(
            Arc::clone(&registration.runtime),
            registration.pair.key.clone(),
            PipelineKey::ShastaSgx.route(),
        ));
        register_remote_sgx_pipelines(factory, registration, runtime_observer)?;
        return Ok(());
    }

    #[cfg(all(feature = "host", not(feature = "local-provers")))]
    {
        let route = registration.config.prover.route();
        let register_risc0_network = matches!(
            route,
            PipelineRoute {
                guest_system: GuestSystem::Risc0,
                runner: RunnerKind::Network,
            }
        );
        let register_sp1_network = matches!(
            route,
            PipelineRoute {
                guest_system: GuestSystem::Risc0 | GuestSystem::Sp1,
                runner: RunnerKind::Network,
            }
        );
        if !register_risc0_network && !register_sp1_network {
            anyhow::bail!("local prover routes require building raiko2 with `local-provers`");
        }

        if register_risc0_network {
            let boundless_engine = build_boundless_engine(
                registration.config,
                registration.pair,
                registration.boundless_backend.clone(),
                setup::boundless_scheduler_config(registration.config),
                Arc::new(RuntimeObserver::new(
                    Arc::clone(&registration.runtime),
                    registration.pair.key.clone(),
                    PipelineKey::ShastaRisc0Network.route(),
                )),
                registration.boundless_balance_gate.clone(),
            )?;
            factory.insert(
                registration.pair.key.clone(),
                PipelineKey::ShastaRisc0Network,
                Arc::new(boundless_engine),
            );
        }

        if register_sp1_network {
            let sp1_engine = build_sp1_engine(
                registration.config,
                registration.pair,
                registration
                    .sp1_prover
                    .clone()
                    .expect("sp1 prover must be initialized for network hosts"),
                registration.sp1_backend.clone(),
                setup::sp1_scheduler_config(registration.config),
                Arc::new(RuntimeObserver::new(
                    Arc::clone(&registration.runtime),
                    registration.pair.key.clone(),
                    PipelineRoute::new(GuestSystem::Sp1, RunnerKind::Network),
                )),
            )?;
            factory.insert(
                registration.pair.key.clone(),
                PipelineKey::ShastaSp1,
                Arc::new(sp1_engine),
            );
        }

        Ok(())
    }

    #[cfg(all(not(feature = "host"), not(feature = "local-provers")))]
    {
        anyhow::bail!("local prover routes require building raiko2 with `local-provers`");
    }

    #[cfg(feature = "local-provers")]
    {
        let risc0_engine = build_risc0_engine(
            registration.config,
            registration.pair,
            registration.shasta_backends.risc0.clone(),
            registration.scheduler_config.clone(),
            Arc::new(RuntimeObserver::new(
                Arc::clone(&registration.runtime),
                registration.pair.key.clone(),
                PipelineKey::ShastaRisc0.route(),
            )),
        )?;
        factory.insert(
            registration.pair.key.clone(),
            PipelineKey::ShastaRisc0,
            Arc::new(risc0_engine),
        );

        let boundless_engine = build_boundless_engine(
            registration.config,
            registration.pair,
            registration.shasta_backends.risc0_boundless.clone(),
            setup::boundless_scheduler_config(registration.config),
            Arc::new(RuntimeObserver::new(
                Arc::clone(&registration.runtime),
                registration.pair.key.clone(),
                PipelineKey::ShastaRisc0Network.route(),
            )),
            registration.boundless_balance_gate.clone(),
        )?;
        factory.insert(
            registration.pair.key.clone(),
            PipelineKey::ShastaRisc0Network,
            Arc::new(boundless_engine),
        );

        let sp1_engine = build_sp1_engine(
            registration.config,
            registration.pair,
            registration
                .sp1_prover
                .clone()
                .expect("sp1 prover must be initialized for local prover hosts"),
            registration.shasta_backends.sp1.clone(),
            setup::sp1_scheduler_config(registration.config),
            Arc::new(RuntimeObserver::new(
                Arc::clone(&registration.runtime),
                registration.pair.key.clone(),
                registration.config.prover.sp1_route(),
            )),
        )?;
        factory.insert(
            registration.pair.key.clone(),
            PipelineKey::ShastaSp1,
            Arc::new(sp1_engine),
        );

        let native_engine = build_native_engine(
            registration.config,
            registration.pair,
            registration.scheduler_config.clone(),
            Arc::new(RuntimeObserver::new(
                Arc::clone(&registration.runtime),
                registration.pair.key.clone(),
                PipelineKey::ShastaNative.route(),
            )),
        )?;
        factory.insert(
            registration.pair.key.clone(),
            PipelineKey::ShastaNative,
            Arc::new(native_engine),
        );

        let remote_observer: Arc<dyn EngineObserver> = Arc::new(RuntimeObserver::new(
            Arc::clone(&registration.runtime),
            registration.pair.key.clone(),
            PipelineKey::ShastaSgx.route(),
        ));
        register_remote_sgx_pipelines(factory, registration, remote_observer)?;
        Ok(())
    }
}

fn register_remote_sgx_pipelines(
    factory: &mut StaticPipelineFactory,
    registration: &PairPipelineRegistration<'_>,
    runtime_observer: Arc<dyn EngineObserver>,
) -> Result<()> {
    register_remote_sgx_engine(
        factory,
        RemoteSgxRegistration {
            config: registration.config,
            pair: registration.pair,
            scheduler_config: registration.scheduler_config.clone(),
            observer: Arc::clone(&runtime_observer),
            pipeline_key: PipelineKey::ShastaSgx,
            proof_type: ProofType::Sgx,
            base_url: &registration.config.prover.remote_sgx.base_url,
        },
    )?;
    register_remote_sgx_engine(
        factory,
        RemoteSgxRegistration {
            config: registration.config,
            pair: registration.pair,
            scheduler_config: registration.scheduler_config.clone(),
            observer: runtime_observer,
            pipeline_key: PipelineKey::ShastaSgxGeth,
            proof_type: ProofType::SgxGeth,
            base_url: &registration.config.prover.remote_sgx.sgxgeth_base_url,
        },
    )?;
    Ok(())
}

struct RemoteSgxRegistration<'a> {
    config: &'a Config,
    pair: &'a ResolvedNetworkPair,
    scheduler_config: SchedulerConfig,
    observer: Arc<dyn EngineObserver>,
    pipeline_key: PipelineKey,
    proof_type: ProofType,
    base_url: &'a str,
}

fn register_remote_sgx_engine(
    factory: &mut StaticPipelineFactory,
    registration: RemoteSgxRegistration<'_>,
) -> Result<()> {
    if registration.base_url.trim().is_empty() {
        return Ok(());
    }

    let engine = build_remote_sgx_engine(
        registration.config,
        registration.pair,
        registration.scheduler_config,
        registration.observer,
        registration.pipeline_key,
        registration.proof_type,
        registration.base_url.to_string(),
    )?;
    factory.insert(
        registration.pair.key.clone(),
        registration.pipeline_key,
        Arc::new(engine),
    );
    Ok(())
}

#[cfg(feature = "local-provers")]
fn build_risc0_engine(
    config: &Config,
    pair: &ResolvedNetworkPair,
    backend: Risc0ShastaBackend,
    scheduler_config: SchedulerConfig,
    observer: Arc<dyn EngineObserver>,
) -> Result<Engine<Risc0Spec>> {
    let risc0_config = setup::risc0_prover_config(config);

    let provider = setup::build_provider(config, pair)?;
    let context = setup::build_context(config, pair, ProofType::Risc0)?;
    let spec = ShastaSpec::new(
        PipelineKey::ShastaRisc0,
        Risc0Prover::new(risc0_config),
        backend,
        provider,
    );
    let engine = Engine::with_store_scheduler_config_and_observer(
        spec,
        context,
        MemoryStore::with_lease(scheduler_config.lease_duration),
        scheduler_config,
        Some(observer),
    );

    Ok(engine)
}

#[cfg(feature = "host")]
fn build_sp1_engine(
    config: &Config,
    pair: &ResolvedNetworkPair,
    prover: Sp1Prover,
    backend: Sp1ShastaBackend,
    scheduler_config: SchedulerConfig,
    observer: Arc<dyn EngineObserver>,
) -> Result<Engine<Sp1Spec>> {
    let provider = setup::build_provider(config, pair)?;
    let context = setup::build_context(config, pair, ProofType::Sp1)?;
    let spec = ShastaSpec::new(PipelineKey::ShastaSp1, prover, backend, provider);
    let engine = Engine::with_store_scheduler_config_and_observer(
        spec,
        context,
        MemoryStore::with_lease(scheduler_config.lease_duration),
        scheduler_config,
        Some(observer),
    );

    Ok(engine)
}

#[cfg(feature = "local-provers")]
fn build_native_engine(
    config: &Config,
    pair: &ResolvedNetworkPair,
    scheduler_config: SchedulerConfig,
    observer: Arc<dyn EngineObserver>,
) -> Result<Engine<NativeSpec>> {
    let provider = setup::build_provider(config, pair)?;
    let context = setup::build_context(config, pair, ProofType::Native)?;
    let spec = ShastaSpec::new(
        PipelineKey::ShastaNative,
        NativeProver,
        NativeBackend,
        provider,
    );
    let engine = Engine::with_store_scheduler_config_and_observer(
        spec,
        context,
        MemoryStore::with_lease(scheduler_config.lease_duration),
        scheduler_config,
        Some(observer),
    );

    Ok(engine)
}

#[cfg(feature = "host")]
fn build_boundless_engine(
    config: &Config,
    pair: &ResolvedNetworkPair,
    backend: Risc0ShastaBackend,
    scheduler_config: SchedulerConfig,
    observer: Arc<dyn EngineObserver>,
    balance_gate: BoundlessBalanceGate,
) -> Result<Engine<BoundlessSpec>> {
    let agent_config = setup::boundless_prover_config(config, pair);

    let provider = setup::build_provider(config, pair)?;
    let context = setup::build_context(config, pair, ProofType::Risc0)?;
    let spec = ShastaSpec::new(
        PipelineKey::ShastaRisc0Network,
        BoundlessProver::with_balance_gate(agent_config, balance_gate),
        backend,
        provider,
    );
    let engine = Engine::with_store_scheduler_config_and_observer(
        spec,
        context,
        MemoryStore::with_lease(scheduler_config.lease_duration),
        scheduler_config,
        Some(observer),
    );

    Ok(engine)
}

fn build_remote_sgx_engine(
    config: &Config,
    pair: &ResolvedNetworkPair,
    scheduler_config: SchedulerConfig,
    observer: Arc<dyn EngineObserver>,
    pipeline_key: PipelineKey,
    proof_type: ProofType,
    base_url: String,
) -> Result<Engine<Gaiko2Spec>> {
    let gaiko2_config =
        setup::remote_sgx_prover_config(base_url, config.prover.remote_sgx.timeout_ms);
    let gaiko2_prover = Gaiko2Prover::new_for_proof_type(&gaiko2_config, proof_type)?;

    let provider = setup::build_provider(config, pair)?;
    let context = setup::build_context(config, pair, proof_type)?;
    let spec = ShastaSpec::new(pipeline_key, gaiko2_prover, NativeBackend, provider);
    let engine = Engine::with_store_scheduler_config_and_observer(
        spec,
        context,
        MemoryStore::with_lease(scheduler_config.lease_duration),
        scheduler_config,
        Some(observer),
    );

    Ok(engine)
}

#[cfg(test)]
mod tests {
    use super::*;
    use raiko2_pipeline::{GuestSystem, RunnerKind};
    use std::sync::atomic::{AtomicBool, Ordering};

    struct ShutdownProbeFactory {
        shutdown_called: Arc<AtomicBool>,
    }

    impl PipelineFactory for ShutdownProbeFactory {
        fn get(&self, _network_pair: &str, _key: PipelineKey) -> Option<Arc<dyn EngineHandle>> {
            None
        }

        fn shutdown(&self) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            Box::pin(async move {
                self.shutdown_called.store(true, Ordering::Release);
            })
        }
    }

    #[tokio::test]
    async fn multiple_engine_observers_share_runtime_checkpoint_drain() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "shutdown-checkpoint-order",
        ))?);
        let first_observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            PipelineKey::ShastaSp1.route(),
        );
        let second_observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            PipelineKey::ShastaRisc0Network.route(),
        );
        let first_permit = first_observer
            .acquire_submission_checkpoint_permit()
            .await?;
        let second_permit = second_observer
            .acquire_submission_checkpoint_permit()
            .await?;
        let shutdown_called = Arc::new(AtomicBool::new(false));
        let factory = ShutdownProbeFactory {
            shutdown_called: Arc::clone(&shutdown_called),
        };
        let state = Arc::new(AppState::from_parts(
            Arc::new(Config::default()),
            Arc::new(factory),
            Arc::clone(&runtime),
        ));

        let mut shutdown = tokio::spawn({
            let state = Arc::clone(&state);
            async move { state.shutdown().await }
        });
        let mut admission_closed = false;
        for _ in 0..100 {
            if let Ok(permit) = runtime.acquire_submission_checkpoint_permit() {
                drop(permit);
            } else {
                admission_closed = true;
                break;
            }
            tokio::task::yield_now().await;
        }

        assert!(admission_closed);
        assert!(!runtime.accepts_mutations());
        assert!(!shutdown_called.load(Ordering::Acquire));
        drop(first_permit);
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut shutdown)
                .await
                .is_err(),
            "shutdown must wait for every engine observer's checkpoint permit"
        );

        drop(second_permit);
        shutdown.await?;
        assert!(!runtime.accepts_mutations());
        assert!(shutdown_called.load(Ordering::Acquire));
        Ok(())
    }

    #[test]
    fn remote_sgx_route_does_not_create_sp1_prover() {
        let mut config = Config::default();
        config.prover.guest_system = GuestSystem::Sgx;
        config.prover.runner = RunnerKind::Remote;

        assert!(!should_create_sp1_prover(&config));
    }

    #[test]
    fn sp1_route_still_creates_sp1_prover() {
        let mut config = Config::default();
        config.prover.guest_system = GuestSystem::Sp1;
        config.prover.runner = RunnerKind::Local;

        assert!(should_create_sp1_prover(&config));
    }

    #[test]
    fn risc0_network_route_creates_sp1_handle_for_explicit_sp1_requests() {
        let mut config = Config::default();
        config.prover.guest_system = GuestSystem::Risc0;
        config.prover.runner = RunnerKind::Network;

        assert!(should_create_sp1_prover(&config));
    }

    fn unique_test_runtime_root(prefix: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("raiko2-state-{prefix}-{unique}"))
    }
}
