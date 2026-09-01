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

use crate::config::{
    Config, GuestSystem, PreflightCacheMode, ResolvedNetworkPair, RunnerKind, RuntimeStoreBackend,
};
use anyhow::{Context, Result};
use raiko2_engine::{Engine, EngineObserver};
use raiko2_pipeline::{
    NativeBackend, PipelineKey, PipelineRoute,
    forks::shasta::{ShastaSpec, preflight_cache::PreflightCoordinator},
};
use raiko2_primitives::ProofType;
use raiko2_prover::{
    boundless::{BoundlessAccountBlocker, QuoteSizing, validate_estimation_model},
    gaiko2::Gaiko2Prover,
    native::NativeProver,
};
use raiko2_provider::NetworkProvider;
use raiko2_queue::{MemoryStore, SchedulerConfig};
use raiko2_runtime::{RuntimeManager, StartupCleanupMask};
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
#[cfg(feature = "local-provers")]
use raiko2_prover::risc0::Risc0Prover;
#[cfg(feature = "host")]
use raiko2_prover::{
    boundless::{BoundlessBalanceGate, BoundlessProver},
    sp1::Sp1Prover,
};

#[cfg(feature = "local-provers")]
type Risc0Spec = ShastaSpec<Risc0Prover, Risc0ShastaBackend, NetworkProvider>;
#[cfg(feature = "host")]
type Sp1Spec = ShastaSpec<Sp1Prover, Sp1ShastaBackend, NetworkProvider>;
type NativeSpec = ShastaSpec<NativeProver, NativeBackend, NetworkProvider>;
type Gaiko2Spec = ShastaSpec<Gaiko2Prover, NativeBackend, NetworkProvider>;
#[cfg(feature = "host")]
type BoundlessSpec = ShastaSpec<BoundlessProver, Risc0ShastaBackend, NetworkProvider>;

use super::lifecycle::ProofLifecycle;
use super::sampling::ZkAnySampler;
use super::task_cleanup::spawn_runtime_cleanup_loop;
use super::task_metadata::TaskMetadata;
use super::telemetry::{
    PreflightCacheMetricsObserver, record_startup_cleanup_failure, record_startup_cleanup_report,
    record_startup_reconciliation, runtime_lifecycle_observer,
};

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct PipelineRegistration {
    pipeline_key: PipelineKey,
    proof_type: ProofType,
    runner: RunnerKind,
    remote_url: Option<String>,
}

impl PipelineRegistration {
    const fn route(&self) -> PipelineRoute {
        let guest_system = match self.proof_type {
            ProofType::Risc0 => GuestSystem::Risc0,
            ProofType::Sp1 => GuestSystem::Sp1,
            ProofType::Native => GuestSystem::Native,
            ProofType::Sgx => GuestSystem::Sgx,
            ProofType::SgxGeth => GuestSystem::SgxGeth,
        };
        PipelineRoute::new(guest_system, self.runner)
    }
}

fn enabled_pipeline_registrations(config: &Config) -> Result<Vec<PipelineRegistration>> {
    #[cfg(not(feature = "local-provers"))]
    if config.prover.runner(ProofType::Risc0) == Some(RunnerKind::Local) {
        anyhow::bail!("prover route risc0/local requires building raiko2 with `local-provers`");
    }

    #[cfg(not(feature = "host"))]
    if let Some((proof_type, runner)) = config.prover.iter_routes().find(|(proof_type, runner)| {
        *proof_type == ProofType::Sp1 || *runner == RunnerKind::Network
    }) {
        anyhow::bail!("prover route {proof_type}/{runner} requires building raiko2 with `host`");
    }

    config
        .prover
        .iter_routes()
        .map(|(proof_type, runner)| {
            let pipeline_key = match (proof_type, runner) {
                (ProofType::Risc0, RunnerKind::Local) => PipelineKey::ShastaRisc0,
                (ProofType::Risc0, RunnerKind::Network) => PipelineKey::ShastaRisc0Network,
                (ProofType::Sp1, RunnerKind::Local | RunnerKind::Network) => PipelineKey::ShastaSp1,
                (ProofType::Native, RunnerKind::Local) => PipelineKey::ShastaNative,
                (ProofType::Sgx, RunnerKind::Remote) => PipelineKey::ShastaSgx,
                (ProofType::SgxGeth, RunnerKind::Remote) => PipelineKey::ShastaSgxGeth,
                _ => unreachable!("prover routes are validated before pipeline registration"),
            };
            let remote_url = match proof_type {
                ProofType::Sgx => Some(config.prover.sgx.base_url.clone()),
                ProofType::SgxGeth => Some(config.prover.sgxgeth.base_url.clone()),
                ProofType::Risc0 | ProofType::Sp1 | ProofType::Native => None,
            };
            Ok(PipelineRegistration {
                pipeline_key,
                proof_type,
                runner,
                remote_url,
            })
        })
        .collect()
}

fn validate_enabled_boundless_estimation_models(
    config: &Config,
    pairs: &[ResolvedNetworkPair],
    pipelines: &[PipelineRegistration],
) -> Result<()> {
    validate_enabled_boundless_estimation_models_with(config, pairs, pipelines, || {
        validate_estimation_model()
    })
}

fn validate_enabled_boundless_estimation_models_with<F>(
    config: &Config,
    pairs: &[ResolvedNetworkPair],
    pipelines: &[PipelineRegistration],
    mut validate_model: F,
) -> Result<()>
where
    F: FnMut() -> Result<(), String>,
{
    let boundless_enabled = pipelines
        .iter()
        .any(|registration| registration.pipeline_key == PipelineKey::ShastaRisc0Network);
    if !boundless_enabled {
        return Ok(());
    }

    for pair in pairs {
        let effective = config
            .prover
            .risc0
            .boundless
            .apply_pair_override(&pair.boundless)
            .with_context(|| format!("Boundless configuration error for rpc pair {}", pair.key))?;
        if matches!(&effective.batch_quote, QuoteSizing::Estimated { .. })
            || matches!(&effective.aggregation_quote, QuoteSizing::Estimated { .. })
        {
            validate_model()
                .map_err(anyhow::Error::msg)
                .with_context(|| {
                    format!(
                        "Boundless estimation model validation failed for rpc pair {}",
                        pair.key
                    )
                })?;
        }
    }

    Ok(())
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

async fn initialize_runtime(config: &Config, runtime: &RuntimeManager) -> Result<()> {
    let cleanup = config.runtime.startup_cleanup_mask()?;
    for scope in cleanup.ordered_scopes() {
        let scope_mask = StartupCleanupMask::from(scope);
        warn!(
            backend = runtime.backend_name(),
            environment = runtime.environment(),
            namespace = runtime.namespace(),
            store_prefix = %config.runtime.store.prefix,
            scope = scope.as_str(),
            "about to run scoped startup cleanup"
        );
        let report = match runtime.cleanup_before_start(scope_mask).await {
            Ok(report) => report,
            Err(error) => {
                record_startup_cleanup_failure(scope);
                return Err(error).with_context(|| {
                    format!("failed {scope:?} startup cleanup before runtime initialization")
                });
            }
        };
        record_startup_cleanup_report(&report);
        for entry in &report.scopes {
            tracing::info!(
                backend = runtime.backend_name(),
                environment = runtime.environment(),
                namespace = runtime.namespace(),
                store_prefix = %config.runtime.store.prefix,
                scope = entry.scope.as_str(),
                matched = entry.matched,
                removed = entry.removed,
                failed = entry.failed,
                elapsed_ms = entry.duration.as_millis(),
                "completed scoped startup cleanup"
            );
        }
    }
    runtime.initialize().await?;
    let reconciliation_started = Instant::now();
    let reconciled = match runtime.reconcile_invalidated_proof_artifacts().await {
        Ok(reconciled) => {
            record_startup_reconciliation("success", reconciled, reconciliation_started.elapsed());
            reconciled
        }
        Err(error) => {
            tracing::error!(
                elapsed_ms = reconciliation_started.elapsed().as_millis(),
                error = %error,
                "startup invalidated-proof reconciliation failed"
            );
            return Err(error)
                .context("failed to reconcile invalidated proof artifacts during startup");
        }
    };
    if reconciled > 0 {
        tracing::info!(
            reconciled,
            "reconciled invalidated proof artifacts during startup"
        );
    }
    Ok(())
}

async fn restored_boundless_account_blockers(
    runtime: &RuntimeManager,
) -> Result<Vec<BoundlessAccountBlocker>> {
    let mut blockers = HashMap::<_, u64>::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("system clock is before the UNIX epoch")?
        .as_secs();
    for record in runtime.list_tasks().await? {
        let metadata = TaskMetadata::decode_for_record(&record).with_context(|| {
            format!(
                "failed to restore Boundless signer checkpoint from runtime task {}",
                record.task_id
            )
        })?;
        for blocker in metadata.boundless_account_blockers(record.runner_status, now)? {
            blockers
                .entry(blocker.checkpoint_key)
                .and_modify(|deadline| *deadline = (*deadline).max(blocker.lock_expires_at))
                .or_insert(blocker.lock_expires_at);
        }
    }
    Ok(blockers
        .into_iter()
        .map(
            |(checkpoint_key, lock_expires_at)| BoundlessAccountBlocker {
                checkpoint_key,
                lock_expires_at,
            },
        )
        .collect())
}

struct PipelineResources {
    #[cfg(feature = "local-provers")]
    shasta_backends: Option<ShastaBackends>,
    #[cfg(all(feature = "host", not(feature = "local-provers")))]
    boundless_backend: Option<Risc0ShastaBackend>,
    #[cfg(all(feature = "host", not(feature = "local-provers")))]
    sp1_backend: Option<Sp1ShastaBackend>,
    #[cfg(feature = "host")]
    sp1_prover: Option<Sp1Prover>,
    #[cfg(feature = "host")]
    boundless_balance_gate: Option<BoundlessBalanceGate>,
}

#[cfg(any(feature = "host", feature = "local-provers"))]
impl PipelineResources {
    fn prepare(
        config: &Config,
        pipelines: &[PipelineRegistration],
        boundless_account_blockers: Vec<BoundlessAccountBlocker>,
    ) -> Result<Self> {
        #[cfg(not(feature = "host"))]
        let _ = (config, boundless_account_blockers);

        #[cfg(feature = "local-provers")]
        let shasta_backends = if pipelines.iter().any(|registration| {
            matches!(
                registration.pipeline_key,
                PipelineKey::ShastaRisc0 | PipelineKey::ShastaRisc0Network | PipelineKey::ShastaSp1
            )
        }) {
            Some(load_shasta_backends().map_err(anyhow::Error::msg)?)
        } else {
            None
        };
        #[cfg(all(feature = "host", not(feature = "local-provers")))]
        let boundless_backend = if pipelines
            .iter()
            .any(|registration| registration.pipeline_key == PipelineKey::ShastaRisc0Network)
        {
            Some(load_risc0_boundless_shasta_backend().map_err(anyhow::Error::msg)?)
        } else {
            None
        };
        #[cfg(all(feature = "host", not(feature = "local-provers")))]
        let sp1_backend = if pipelines
            .iter()
            .any(|registration| registration.pipeline_key == PipelineKey::ShastaSp1)
        {
            Some(load_sp1_shasta_backend().map_err(anyhow::Error::msg)?)
        } else {
            None
        };
        #[cfg(feature = "host")]
        let sp1_prover = if pipelines
            .iter()
            .any(|registration| registration.pipeline_key == PipelineKey::ShastaSp1)
        {
            let sp1_config = setup::sp1_prover_config(config);
            #[cfg(feature = "local-provers")]
            {
                Some(
                    Sp1Prover::new_with_backend(
                        sp1_config,
                        &shasta_backends
                            .as_ref()
                            .expect("SP1 route requires Shasta backends")
                            .sp1,
                    )
                    .map_err(anyhow::Error::msg)?,
                )
            }
            #[cfg(all(feature = "host", not(feature = "local-provers")))]
            {
                Some(
                    Sp1Prover::new_with_backend(
                        sp1_config,
                        sp1_backend
                            .as_ref()
                            .expect("SP1 route requires SP1 backend"),
                    )
                    .map_err(anyhow::Error::msg)?,
                )
            }
        } else {
            None
        };

        // Every pair uses the same market account, so reservations share one balance gate.
        #[cfg(feature = "host")]
        let boundless_enabled = pipelines
            .iter()
            .any(|registration| registration.pipeline_key == PipelineKey::ShastaRisc0Network);
        #[cfg(feature = "host")]
        if boundless_enabled {
            BoundlessProver::validate_storage_configuration().map_err(anyhow::Error::msg)?;
        }
        #[cfg(feature = "host")]
        let boundless_balance_gate = boundless_enabled
            .then(|| BoundlessBalanceGate::with_durable_blockers(boundless_account_blockers));

        Ok(Self {
            #[cfg(feature = "local-provers")]
            shasta_backends,
            #[cfg(all(feature = "host", not(feature = "local-provers")))]
            boundless_backend,
            #[cfg(all(feature = "host", not(feature = "local-provers")))]
            sp1_backend,
            #[cfg(feature = "host")]
            sp1_prover,
            #[cfg(feature = "host")]
            boundless_balance_gate,
        })
    }
}

fn build_pipeline_factory(
    config: &Config,
    pairs: &[ResolvedNetworkPair],
    pipelines: &[PipelineRegistration],
    runtime: &Arc<RuntimeManager>,
    scheduler_config: &SchedulerConfig,
    #[cfg(any(feature = "host", feature = "local-provers"))] resources: &PipelineResources,
    #[cfg(not(any(feature = "host", feature = "local-provers")))] _resources: &PipelineResources,
) -> Result<StaticPipelineFactory> {
    let mut factory = StaticPipelineFactory::default();
    for pair in pairs {
        let preflight_coordinator = preflight_coordinator(config, runtime, pair);
        let registration = PairPipelineRegistration {
            config,
            pair,
            pipelines,
            runtime: Arc::clone(runtime),
            preflight_coordinator,
            #[cfg(feature = "host")]
            boundless_balance_gate: resources.boundless_balance_gate.clone(),
            #[cfg(feature = "local-provers")]
            shasta_backends: resources.shasta_backends.as_ref(),
            #[cfg(all(feature = "host", not(feature = "local-provers")))]
            boundless_backend: resources.boundless_backend.as_ref(),
            #[cfg(all(feature = "host", not(feature = "local-provers")))]
            sp1_backend: resources.sp1_backend.as_ref(),
            #[cfg(feature = "host")]
            sp1_prover: resources.sp1_prover.clone(),
            scheduler_config: scheduler_config.clone(),
        };
        register_pair_pipelines(&mut factory, &registration)?;
    }
    Ok(factory)
}

fn preflight_coordinator(
    config: &Config,
    runtime: &Arc<RuntimeManager>,
    pair: &ResolvedNetworkPair,
) -> Option<Arc<PreflightCoordinator>> {
    match config.runtime.preflight_cache {
        PreflightCacheMode::Shared => Some(Arc::new(PreflightCoordinator::with_observer(
            runtime.canonical_preflight_store(),
            Arc::new(PreflightCacheMetricsObserver::new(pair.key.clone())),
        ))),
        PreflightCacheMode::Off => None,
    }
}

impl AppState {
    /// Create new application state.
    pub async fn new(config: Config) -> Result<Self> {
        config.validate()?;
        let pipeline_registrations = enabled_pipeline_registrations(&config)?;
        let resolved_pairs = config.rpc.resolved_pairs()?;
        validate_enabled_boundless_estimation_models(
            &config,
            &resolved_pairs,
            &pipeline_registrations,
        )?;
        let runtime = Arc::new(build_runtime(&config).await?);
        runtime.set_lifecycle_observer(runtime_lifecycle_observer());
        initialize_runtime(&config, &runtime).await?;
        let scheduler_config = setup::scheduler_config(&config);
        let boundless_enabled = pipeline_registrations
            .iter()
            .any(|registration| registration.pipeline_key == PipelineKey::ShastaRisc0Network);
        let boundless_account_blockers = if boundless_enabled {
            restored_boundless_account_blockers(&runtime).await?
        } else {
            Vec::new()
        };
        if !boundless_account_blockers.is_empty() {
            tracing::info!(
                blocker_count = boundless_account_blockers.len(),
                "restored unresolved Boundless signer checkpoints before worker startup"
            );
        }
        #[cfg(any(feature = "host", feature = "local-provers"))]
        let resources = PipelineResources::prepare(
            &config,
            &pipeline_registrations,
            boundless_account_blockers,
        )?;
        #[cfg(not(any(feature = "host", feature = "local-provers")))]
        let resources = {
            let _ = boundless_account_blockers;
            PipelineResources {}
        };

        let factory = build_pipeline_factory(
            &config,
            &resolved_pairs,
            &pipeline_registrations,
            &runtime,
            &scheduler_config,
            &resources,
        )?;
        let config = Arc::new(config);
        let pipelines: Arc<dyn PipelineFactory> = Arc::new(factory);
        let state = Self::from_parts(config, pipelines, Arc::clone(&runtime));
        state.finish_initialization().await
    }

    async fn finish_initialization(self) -> Result<Self> {
        crate::server::handlers::validate_persisted_runtime_task_metadata(&self).await?;
        let restored = crate::server::handlers::restore_pending_runtime_state(&self).await?;
        if restored > 0 {
            tracing::info!(
                restored,
                "restored pending runtime state without execution attachment"
            );
        }
        crate::server::telemetry::record_runtime_state_stats(
            self.runtime.runtime_state_stats().await,
        );
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
        runtime.set_lifecycle_observer(runtime_lifecycle_observer());
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
    pipelines: &'a [PipelineRegistration],
    runtime: Arc<RuntimeManager>,
    preflight_coordinator: Option<Arc<PreflightCoordinator>>,
    /// Balance gate shared across all pairs by `PipelineResources`.
    #[cfg(feature = "host")]
    boundless_balance_gate: Option<BoundlessBalanceGate>,
    #[cfg(feature = "local-provers")]
    shasta_backends: Option<&'a ShastaBackends>,
    #[cfg(all(feature = "host", not(feature = "local-provers")))]
    boundless_backend: Option<&'a Risc0ShastaBackend>,
    #[cfg(all(feature = "host", not(feature = "local-provers")))]
    sp1_backend: Option<&'a Sp1ShastaBackend>,
    #[cfg(feature = "host")]
    sp1_prover: Option<Sp1Prover>,
    scheduler_config: SchedulerConfig,
}

#[allow(clippy::too_many_lines)]
fn register_pair_pipelines(
    factory: &mut StaticPipelineFactory,
    registration: &PairPipelineRegistration<'_>,
) -> Result<()> {
    for pipeline in registration.pipelines {
        let observer: Arc<dyn EngineObserver> = Arc::new(RuntimeObserver::new(
            Arc::clone(&registration.runtime),
            registration.pair.key.clone(),
            pipeline.route(),
        ));
        match (pipeline.proof_type, pipeline.runner) {
            (ProofType::Risc0, RunnerKind::Local) => {
                #[cfg(feature = "local-provers")]
                {
                    let engine = build_risc0_engine(
                        registration.config,
                        registration.pair,
                        registration
                            .shasta_backends
                            .expect("RISC0 local route requires Shasta backends")
                            .risc0
                            .clone(),
                        registration.scheduler_config.clone(),
                        observer,
                        registration.preflight_coordinator.clone(),
                    )?;
                    factory.insert(
                        registration.pair.key.clone(),
                        pipeline.pipeline_key,
                        Arc::new(engine),
                    );
                }
                #[cfg(not(feature = "local-provers"))]
                unreachable!("local routes are rejected before pipeline construction");
            }
            (ProofType::Risc0, RunnerKind::Network) => {
                #[cfg(feature = "host")]
                {
                    #[cfg(feature = "local-provers")]
                    let backend = registration
                        .shasta_backends
                        .expect("RISC0 network route requires Shasta backends")
                        .risc0_boundless
                        .clone();
                    #[cfg(all(feature = "host", not(feature = "local-provers")))]
                    let backend = registration
                        .boundless_backend
                        .expect("RISC0 network route requires Boundless backend")
                        .clone();
                    let engine = build_boundless_engine(
                        registration.config,
                        registration.pair,
                        backend,
                        setup::boundless_scheduler_config(registration.config),
                        observer,
                        registration
                            .boundless_balance_gate
                            .clone()
                            .expect("RISC0 network route requires Boundless balance gate"),
                        registration.preflight_coordinator.clone(),
                    )?;
                    factory.insert(
                        registration.pair.key.clone(),
                        pipeline.pipeline_key,
                        Arc::new(engine),
                    );
                }
                #[cfg(not(feature = "host"))]
                unreachable!("network routes are rejected before pipeline construction");
            }
            (ProofType::Sp1, RunnerKind::Local | RunnerKind::Network) => {
                #[cfg(feature = "host")]
                {
                    #[cfg(feature = "local-provers")]
                    let backend = registration
                        .shasta_backends
                        .expect("SP1 route requires Shasta backends")
                        .sp1
                        .clone();
                    #[cfg(all(feature = "host", not(feature = "local-provers")))]
                    let backend = registration
                        .sp1_backend
                        .expect("SP1 route requires SP1 backend")
                        .clone();
                    let engine = build_sp1_engine(
                        registration.config,
                        registration.pair,
                        registration
                            .sp1_prover
                            .clone()
                            .expect("SP1 route requires SP1 prover"),
                        backend,
                        setup::sp1_scheduler_config(registration.config),
                        observer,
                        registration.preflight_coordinator.clone(),
                    )?;
                    factory.insert(
                        registration.pair.key.clone(),
                        pipeline.pipeline_key,
                        Arc::new(engine),
                    );
                }
                #[cfg(not(feature = "host"))]
                unreachable!("SP1 routes are rejected before pipeline construction");
            }
            (ProofType::Native, RunnerKind::Local) => {
                let engine = build_native_engine(
                    registration.config,
                    registration.pair,
                    registration.scheduler_config.clone(),
                    observer,
                    registration.preflight_coordinator.clone(),
                )?;
                factory.insert(
                    registration.pair.key.clone(),
                    pipeline.pipeline_key,
                    Arc::new(engine),
                );
            }
            (ProofType::Sgx | ProofType::SgxGeth, RunnerKind::Remote) => {
                let engine = build_remote_sgx_engine(
                    registration.config,
                    registration.pair,
                    registration.scheduler_config.clone(),
                    observer,
                    RemoteSgxLane {
                        pipeline_key: pipeline.pipeline_key,
                        proof_type: pipeline.proof_type,
                        base_url: pipeline
                            .remote_url
                            .clone()
                            .expect("remote SGX route requires a selected URL"),
                    },
                    registration.preflight_coordinator.clone(),
                )?;
                factory.insert(
                    registration.pair.key.clone(),
                    pipeline.pipeline_key,
                    Arc::new(engine),
                );
            }
            _ => unreachable!("prover routes are validated before pipeline registration"),
        }
    }
    Ok(())
}

#[cfg(feature = "local-provers")]
fn build_risc0_engine(
    config: &Config,
    pair: &ResolvedNetworkPair,
    backend: Risc0ShastaBackend,
    scheduler_config: SchedulerConfig,
    observer: Arc<dyn EngineObserver>,
    preflight_coordinator: Option<Arc<PreflightCoordinator>>,
) -> Result<Engine<Risc0Spec>> {
    let risc0_config = setup::risc0_prover_config(config);

    let provider = setup::build_provider(config, pair)?;
    let context = setup::build_context(config, pair, ProofType::Risc0)?;
    let spec = ShastaSpec::new(
        PipelineKey::ShastaRisc0,
        Risc0Prover::new(risc0_config),
        backend,
        provider,
    )
    .with_optional_preflight_coordinator(preflight_coordinator);
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
    preflight_coordinator: Option<Arc<PreflightCoordinator>>,
) -> Result<Engine<Sp1Spec>> {
    let provider = setup::build_provider(config, pair)?;
    let context = setup::build_context(config, pair, ProofType::Sp1)?;
    let spec = ShastaSpec::new(PipelineKey::ShastaSp1, prover, backend, provider)
        .with_optional_preflight_coordinator(preflight_coordinator);
    let engine = Engine::with_store_scheduler_config_and_observer(
        spec,
        context,
        MemoryStore::with_lease(scheduler_config.lease_duration),
        scheduler_config,
        Some(observer),
    );

    Ok(engine)
}

fn build_native_engine(
    config: &Config,
    pair: &ResolvedNetworkPair,
    scheduler_config: SchedulerConfig,
    observer: Arc<dyn EngineObserver>,
    preflight_coordinator: Option<Arc<PreflightCoordinator>>,
) -> Result<Engine<NativeSpec>> {
    let provider = setup::build_provider(config, pair)?;
    let context = setup::build_context(config, pair, ProofType::Native)?;
    let spec = ShastaSpec::new(
        PipelineKey::ShastaNative,
        NativeProver,
        NativeBackend,
        provider,
    )
    .with_optional_preflight_coordinator(preflight_coordinator);
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
    preflight_coordinator: Option<Arc<PreflightCoordinator>>,
) -> Result<Engine<BoundlessSpec>> {
    let agent_config = setup::boundless_prover_config(config, pair);

    let provider = setup::build_provider(config, pair)?;
    let context = setup::build_context(config, pair, ProofType::Risc0)?;
    let spec = ShastaSpec::new(
        PipelineKey::ShastaRisc0Network,
        BoundlessProver::with_balance_gate(agent_config, balance_gate),
        backend,
        provider,
    )
    .with_optional_preflight_coordinator(preflight_coordinator);
    let engine = Engine::with_store_scheduler_config_and_observer(
        spec,
        context,
        MemoryStore::with_lease(scheduler_config.lease_duration),
        scheduler_config,
        Some(observer),
    );

    Ok(engine)
}

struct RemoteSgxLane {
    pipeline_key: PipelineKey,
    proof_type: ProofType,
    base_url: String,
}

fn build_remote_sgx_engine(
    config: &Config,
    pair: &ResolvedNetworkPair,
    scheduler_config: SchedulerConfig,
    observer: Arc<dyn EngineObserver>,
    lane: RemoteSgxLane,
    preflight_coordinator: Option<Arc<PreflightCoordinator>>,
) -> Result<Engine<Gaiko2Spec>> {
    let timeout_ms = match lane.proof_type {
        ProofType::Sgx => config.prover.sgx.timeout_ms,
        ProofType::SgxGeth => config.prover.sgxgeth.timeout_ms,
        ProofType::Risc0 | ProofType::Sp1 | ProofType::Native => {
            unreachable!("remote SGX engine requires an SGX proof type")
        }
    };
    let gaiko2_config = setup::remote_sgx_prover_config(lane.base_url, timeout_ms);
    let gaiko2_prover = Gaiko2Prover::new_for_proof_type(&gaiko2_config, lane.proof_type)?;

    let provider = setup::build_provider(config, pair)?;
    let context = setup::build_context(config, pair, lane.proof_type)?;
    let spec = ShastaSpec::new(lane.pipeline_key, gaiko2_prover, NativeBackend, provider)
        .with_optional_preflight_coordinator(preflight_coordinator);
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
    use crate::config::BoundlessPairConfig;
    use raiko2_primitives::ChainSpec;
    use raiko2_provider::L2ProviderKind;
    use raiko2_runtime::{
        ExactDeleteResult, ProofArtifactKey, ProofArtifactRegistration, TaskRegistration,
        test_support::{MemoryProofArtifactStore, ProofObjectStore, RuntimeStore},
    };
    use std::sync::atomic::{AtomicBool, Ordering};

    struct ShutdownProbeFactory {
        shutdown_called: Arc<AtomicBool>,
    }

    fn preflight_cache_test_pair() -> ResolvedNetworkPair {
        ResolvedNetworkPair {
            key: "sample_l2/sample_l1".to_string(),
            network: "sample_l2".to_string(),
            l1_network: "sample_l1".to_string(),
            l1_rpc: "http://l1.example.invalid".to_string(),
            l2_rpc: "http://l2.example.invalid".to_string(),
            l2_provider: L2ProviderKind::Reth,
            l2_witness_rpc: "http://l2.example.invalid".to_string(),
            sp1_verifier_rpc_url: None,
            sp1_verifier_address: None,
            boundless: BoundlessPairConfig::default(),
            l1_spec: ChainSpec::default(),
            l2_spec: ChainSpec::default(),
        }
    }

    #[test]
    fn preflight_cache_mode_controls_coordinator_wiring() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "preflight-cache-mode",
        ))?);
        let pair = preflight_cache_test_pair();
        let mut config = Config::default();

        assert!(preflight_coordinator(&config, &runtime, &pair).is_some());
        config.runtime.preflight_cache = PreflightCacheMode::Off;
        assert!(preflight_coordinator(&config, &runtime, &pair).is_none());
        Ok(())
    }

    #[test]
    fn estimated_pair_override_validates_the_estimation_model_at_startup() {
        let mut config = Config::default();
        config.prover.risc0.boundless.offchain = true;
        let mut pair = preflight_cache_test_pair();
        pair.boundless.batch_quote = Some(QuoteSizing::Estimated { mcycles_offset: 0 });
        let pipelines = [PipelineRegistration {
            pipeline_key: PipelineKey::ShastaRisc0Network,
            proof_type: ProofType::Risc0,
            runner: RunnerKind::Network,
            remote_url: None,
        }];
        let mut validation_calls = 0;

        let error =
            validate_enabled_boundless_estimation_models_with(&config, &[pair], &pipelines, || {
                validation_calls += 1;
                Err("model artifact rejected".to_string())
            })
            .expect_err("an effective estimated pair must validate the model");

        assert_eq!(validation_calls, 1);
        assert!(
            format!("{error:#}").contains("model artifact rejected"),
            "{error:#}"
        );
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

    #[tokio::test]
    async fn proof_startup_cleanup_discards_persisted_tasks_before_initialization() -> Result<()> {
        let mut config = Config::default();
        config.runtime.environment = "test".into();
        config.runtime.namespace = "startup-proof-cleanup".into();
        config.runtime.startup_cleanup = vec![crate::config::StartupCleanupScope::Proof];
        let store: Arc<dyn RuntimeStore> = Arc::new(MemoryProofArtifactStore::new(
            config.runtime.environment.clone(),
            config.runtime.namespace.clone(),
        )?);
        let previous_runtime = RuntimeManager::with_store(Arc::clone(&store));
        previous_runtime.initialize().await?;
        previous_runtime
            .register_task(TaskRegistration {
                task_id: "stale-task".into(),
                pipeline_key: PipelineKey::ShastaNative,
                route: PipelineKey::ShastaNative.route(),
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: Vec::new(),
                metadata: serde_json::json!({}),
                request_fingerprint: "stale-task-request".into(),
            })
            .await?;

        let runtime = RuntimeManager::with_store(store);

        initialize_runtime(&config, &runtime).await?;

        assert!(runtime.list_tasks().await?.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn startup_cleanup_is_opt_in() -> Result<()> {
        let mut config = Config::default();
        config.runtime.environment = "test".into();
        config.runtime.namespace = "startup-no-cleanup".into();
        let store: Arc<dyn RuntimeStore> = Arc::new(MemoryProofArtifactStore::new(
            config.runtime.environment.clone(),
            config.runtime.namespace.clone(),
        )?);
        let previous_runtime = RuntimeManager::with_store(Arc::clone(&store));
        previous_runtime.initialize().await?;
        previous_runtime
            .register_task(TaskRegistration {
                task_id: "preserved-task".into(),
                pipeline_key: PipelineKey::ShastaNative,
                route: PipelineKey::ShastaNative.route(),
                task_kind: "proposal".into(),
                network_pair: "l1-l2".into(),
                artifact_refs: Vec::new(),
                metadata: serde_json::json!({}),
                request_fingerprint: "preserved-task-request".into(),
            })
            .await?;

        let runtime = RuntimeManager::with_store(store);

        initialize_runtime(&config, &runtime).await?;

        assert_eq!(runtime.list_tasks().await?.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn startup_reconciles_durable_invalidated_proof_before_workers() -> Result<()> {
        let mut config = Config::default();
        config.runtime.environment = "test".into();
        config.runtime.namespace = "startup-invalidated-proof".into();
        let store: Arc<dyn RuntimeStore> = Arc::new(MemoryProofArtifactStore::new(
            config.runtime.environment.clone(),
            config.runtime.namespace.clone(),
        )?);
        let previous = RuntimeManager::with_store(Arc::clone(&store));
        previous.initialize().await?;
        let pipeline_key = PipelineKey::ShastaNative;
        let route = pipeline_key.route();
        let proof_ref = "startup-invalidated-proof";
        let object = previous
            .publish_proof_artifact_bytes(
                "l1-l2",
                pipeline_key,
                route,
                proof_ref,
                br#"{"proof":"0x01"}"#,
            )
            .await?
            .try_object()
            .context("published proof")?
            .clone();
        previous
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: "l1-l2".into(),
                proof_ref: proof_ref.into(),
                pipeline_key,
                route,
                proof_uri: object.proof_uri,
                content_hash: object.content_hash,
                generation: object.generation,
            })
            .await?;
        let record = previous
            .get_proof_artifact("l1-l2", pipeline_key, route, proof_ref)
            .await?
            .context("active proof artifact")?;
        let prepared = previous.prepare_artifact_retention_batch(&[record]).await?;
        assert_eq!(prepared.newly_invalidated_artifacts, 1);

        let runtime = RuntimeManager::with_store(store);
        initialize_runtime(&config, &runtime).await?;

        assert!(
            runtime
                .get_proof_artifact_including_invalidated("l1-l2", pipeline_key, route, proof_ref,)
                .await?
                .is_none()
        );
        assert!(
            runtime
                .read_proof_artifact_bytes("l1-l2", pipeline_key, route, proof_ref)
                .await?
                .is_none()
        );
        Ok(())
    }

    #[tokio::test]
    async fn startup_retention_does_not_fail_on_stale_invalidated_descriptor() -> Result<()> {
        let store = Arc::new(MemoryProofArtifactStore::new(
            "test".into(),
            "startup-retention-stale-descriptor".into(),
        )?);
        let runtime_store: Arc<dyn RuntimeStore> = store.clone();
        let previous = RuntimeManager::with_store(runtime_store);
        previous.initialize().await?;
        let pipeline_key = PipelineKey::ShastaNative;
        let route = pipeline_key.route();
        let proof_ref = "stale-proof";
        let first = previous
            .publish_proof_artifact_bytes(
                "l1-l2",
                pipeline_key,
                route,
                proof_ref,
                b"identical-proof-bytes",
            )
            .await?
            .try_object()
            .context("first canonical proof")?
            .clone();
        previous
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: "l1-l2".into(),
                proof_ref: proof_ref.into(),
                pipeline_key,
                route,
                proof_uri: first.proof_uri.clone(),
                content_hash: first.content_hash.clone(),
                generation: first.generation,
            })
            .await?;
        let record = previous
            .get_proof_artifact_including_invalidated("l1-l2", pipeline_key, route, proof_ref)
            .await?
            .context("first artifact record")?;
        previous.prepare_artifact_retention_batch(&[record]).await?;
        let key = ProofArtifactKey {
            network_pair: "l1-l2".into(),
            pipeline_key,
            route,
            proof_ref: proof_ref.into(),
        };
        assert_eq!(
            store.delete_exact(&key, &first.descriptor()).await?,
            ExactDeleteResult::Removed
        );
        let second = previous
            .publish_proof_artifact_bytes(
                "l1-l2",
                pipeline_key,
                route,
                proof_ref,
                b"identical-proof-bytes",
            )
            .await?
            .try_object()
            .context("republished canonical proof")?
            .clone();
        assert_ne!(first.generation, second.generation);

        let restarted_store: Arc<dyn RuntimeStore> = store;
        let restarted = Arc::new(RuntimeManager::with_store(restarted_store));
        restarted.initialize().await?;
        let state = AppState::from_parts(
            Arc::new(Config::default()),
            Arc::new(StaticPipelineFactory::default()),
            Arc::clone(&restarted),
        )
        .finish_initialization()
        .await?;

        assert!(
            restarted
                .proof_artifact_descriptor_is_current(
                    "l1-l2",
                    pipeline_key,
                    route,
                    proof_ref,
                    &second.descriptor(),
                )
                .await?
        );
        assert!(
            restarted
                .get_reclaimable_proof_artifact(&key)
                .await?
                .is_some(),
            "failed maintenance must keep the invalidated record retryable"
        );
        state.shutdown().await;
        Ok(())
    }

    fn config_with_routes(routes: &str) -> Config {
        let mut config = Config::default();
        config
            .prover
            .apply_routes_override(&routes.parse().expect("valid prover routes"));
        config
    }

    #[cfg(feature = "host")]
    fn selected_pipeline_keys(config: &Config) -> Result<Vec<PipelineKey>> {
        Ok(enabled_pipeline_registrations(config)?
            .into_iter()
            .map(|registration| registration.pipeline_key)
            .collect())
    }

    #[cfg(all(feature = "host", not(feature = "local-provers")))]
    #[test]
    fn host_only_risc0_network_selects_only_boundless() -> Result<()> {
        let config = config_with_routes("risc0/network");

        assert_eq!(
            selected_pipeline_keys(&config)?,
            vec![PipelineKey::ShastaRisc0Network]
        );
        Ok(())
    }

    #[cfg(all(feature = "host", not(feature = "local-provers")))]
    #[test]
    fn host_only_sp1_network_selects_only_sp1() -> Result<()> {
        let config = config_with_routes("sp1/network");

        assert_eq!(
            selected_pipeline_keys(&config)?,
            vec![PipelineKey::ShastaSp1]
        );
        Ok(())
    }

    #[cfg(feature = "host")]
    #[test]
    fn combined_routes_select_every_configured_pipeline() -> Result<()> {
        let config = config_with_routes("risc0/network,sp1/network,sgx/remote,sgxgeth/remote");

        assert_eq!(
            selected_pipeline_keys(&config)?,
            vec![
                PipelineKey::ShastaRisc0Network,
                PipelineKey::ShastaSp1,
                PipelineKey::ShastaSgx,
                PipelineKey::ShastaSgxGeth,
            ]
        );
        Ok(())
    }

    #[test]
    fn sgx_route_does_not_select_sgxgeth_or_use_its_url() -> Result<()> {
        let mut config = config_with_routes("sgx/remote");
        config.prover.sgx.base_url = "http://sgx.example".to_string();
        config.prover.sgxgeth.base_url = "http://unused-sgxgeth.example".to_string();

        let registrations = enabled_pipeline_registrations(&config)?;

        assert_eq!(registrations.len(), 1);
        assert_eq!(registrations[0].pipeline_key, PipelineKey::ShastaSgx);
        assert_eq!(
            registrations[0].route(),
            "sgx/remote".parse().expect("parse SGX route")
        );
        assert_eq!(
            registrations[0].remote_url.as_deref(),
            Some("http://sgx.example")
        );
        Ok(())
    }

    #[test]
    fn sgxgeth_route_does_not_select_sgx_or_use_its_url() -> Result<()> {
        let mut config = config_with_routes("sgxgeth/remote");
        config.prover.sgx.base_url = "http://unused-sgx.example".to_string();
        config.prover.sgxgeth.base_url = "http://sgxgeth.example".to_string();

        let registrations = enabled_pipeline_registrations(&config)?;

        assert_eq!(registrations.len(), 1);
        assert_eq!(registrations[0].pipeline_key, PipelineKey::ShastaSgxGeth);
        assert_eq!(
            registrations[0].route(),
            "sgxgeth/remote".parse().expect("parse SGXGETH route")
        );
        assert_eq!(
            registrations[0].remote_url.as_deref(),
            Some("http://sgxgeth.example")
        );
        Ok(())
    }

    #[cfg(not(feature = "local-provers"))]
    #[test]
    fn risc0_local_requires_local_provers() {
        let config = config_with_routes("risc0/local");
        let error = enabled_pipeline_registrations(&config)
            .expect_err("RISC0 local route must require local-provers");

        assert!(
            error
                .to_string()
                .contains("risc0/local requires building raiko2 with `local-provers`"),
            "unexpected error: {error}"
        );
    }

    #[cfg(not(feature = "host"))]
    #[test]
    fn sp1_local_requires_host_feature() {
        let config = config_with_routes("sp1/local");
        let error = enabled_pipeline_registrations(&config)
            .expect_err("SP1 local route must require host support");

        assert!(
            error
                .to_string()
                .contains("sp1/local requires building raiko2 with `host`"),
            "unexpected error: {error}"
        );
    }

    #[cfg(not(feature = "local-provers"))]
    #[test]
    fn native_local_is_available_without_local_provers() -> Result<()> {
        let config = config_with_routes("native/local");
        let registrations = enabled_pipeline_registrations(&config)?;

        assert_eq!(registrations.len(), 1);
        assert_eq!(registrations[0].pipeline_key, PipelineKey::ShastaNative);
        assert_eq!(registrations[0].runner, RunnerKind::Local);
        Ok(())
    }

    #[cfg(feature = "local-provers")]
    #[test]
    fn local_provers_select_only_explicit_local_pipelines() -> Result<()> {
        let config = config_with_routes("sp1/local,native/local");

        assert_eq!(
            selected_pipeline_keys(&config)?,
            vec![PipelineKey::ShastaSp1, PipelineKey::ShastaNative]
        );
        assert_eq!(
            enabled_pipeline_registrations(&config)?[0].runner,
            RunnerKind::Local
        );
        Ok(())
    }

    fn unique_test_runtime_root(prefix: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("raiko2-state-{prefix}-{unique}"))
    }
}
