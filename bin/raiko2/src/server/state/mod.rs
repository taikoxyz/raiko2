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
use raiko2_primitives::{Proof, ProofType};
use raiko2_prover::gaiko2::Gaiko2Prover;
use raiko2_prover::validate_external_aggregate_proofs;
use raiko2_provider::NetworkProvider;
use raiko2_queue::{MemoryStore, SchedulerConfig};
use raiko2_runtime::{
    GcsProofArtifactStore, MemoryProofArtifactStore, ProofArtifactRegistration, ProofArtifactStore,
    RunnerStatus, RuntimeManager,
};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::warn;

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

use super::sampling::ZkAnySampler;
use super::task_cleanup::spawn_runtime_cleanup_loop;
use super::task_metadata::{
    ProofArtifactKind, TaskMetadata, aggregate_task_ref, proposal_task_ref,
    root_proof_artifact_refs,
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
    pub zk_any_sampler: Arc<Mutex<ZkAnySampler>>,
    pub(crate) acl_rate_limiter: Arc<AclRateLimiter>,
    namespace_owner_heartbeat: Option<Arc<NamespaceOwnerHeartbeat>>,
    background_tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

#[derive(Debug)]
struct NamespaceOwnerHeartbeat {
    handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl NamespaceOwnerHeartbeat {
    const fn new(handle: tokio::task::JoinHandle<()>) -> Self {
        Self {
            handle: Mutex::new(Some(handle)),
        }
    }

    async fn stop(&self) {
        let handle = self.handle.lock().ok().and_then(|mut handle| handle.take());
        if let Some(handle) = handle {
            handle.abort();
            let _ = handle.await;
        }
    }
}

impl Drop for NamespaceOwnerHeartbeat {
    fn drop(&mut self) {
        if let Ok(mut handle) = self.handle.lock()
            && let Some(handle) = handle.take()
        {
            handle.abort();
        }
    }
}

async fn build_artifact_store(config: &Config) -> Result<Arc<dyn ProofArtifactStore>> {
    match config.runtime.store.backend {
        RuntimeStoreBackend::Memory => Ok(Arc::new(MemoryProofArtifactStore::new(
            config.runtime.environment.clone(),
            config.runtime.namespace.clone(),
        )?)),
        RuntimeStoreBackend::Gcs => Ok(Arc::new(
            GcsProofArtifactStore::new(
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
            .await?,
        )),
    }
}

impl AppState {
    /// Create new application state.
    pub async fn new(mut config: Config) -> Result<Self> {
        config.normalize();
        config.validate()?;
        let artifact_store = build_artifact_store(&config).await?;
        let runtime = Arc::new(RuntimeManager::with_store(artifact_store)?);
        let scheduler_config = setup::scheduler_config(&config);
        let workers = config.queue.workers;
        let maintenance_interval = Duration::from_millis(config.queue.maintenance_interval_ms);
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

        runtime
            .acquire_namespace_owner(config.runtime.store.owner_lease_secs)
            .await?;
        let heartbeat = Arc::new(spawn_namespace_owner_heartbeat(
            Arc::clone(&runtime),
            config.runtime.store.owner_heartbeat_secs,
            config.runtime.store.owner_lease_secs,
        ));
        let startup = async {
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
                    workers,
                    maintenance_interval,
                };
                register_pair_pipelines(&mut factory, &registration)?;
            }
            runtime.initialize().await?;
            runtime.fence_namespace_owner().await?;
            restore_proof_artifacts_from_runtime_tasks(&runtime).await?;

            let config = Arc::new(config);
            let pipelines: Arc<dyn PipelineFactory> = Arc::new(factory);
            let mut state = Self::from_parts(config, pipelines, Arc::clone(&runtime));
            state.namespace_owner_heartbeat = Some(Arc::clone(&heartbeat));
            state.finish_initialization().await
        }
        .await;
        if startup.is_err() {
            heartbeat.stop().await;
            let _ = runtime.release_namespace_owner().await;
        }
        startup
    }

    async fn finish_initialization(self) -> Result<Self> {
        let recovered = crate::server::handlers::recover_pending_runtime_tasks(&self).await?;
        if recovered > 0 {
            tracing::info!(
                recovered,
                "recovered pending runtime tasks into the memory queue"
            );
        }
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
        Self {
            config,
            pipelines,
            runtime,
            zk_any_sampler,
            acl_rate_limiter: Arc::new(AclRateLimiter::default()),
            namespace_owner_heartbeat: None,
            background_tasks: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) async fn shutdown(&self) {
        self.pipelines.shutdown().await;
        let handles = self
            .background_tasks
            .lock()
            .map(|mut handles| handles.drain(..).collect::<Vec<_>>())
            .unwrap_or_default();
        for handle in handles {
            handle.abort();
            let _ = handle.await;
        }
        if let Some(heartbeat) = &self.namespace_owner_heartbeat {
            heartbeat.stop().await;
        }
        self.runtime.begin_draining();
        if let Err(error) = self.runtime.release_namespace_owner().await {
            tracing::warn!(%error, "failed to release runtime namespace ownership");
        }
    }
}

fn spawn_namespace_owner_heartbeat(
    runtime: Arc<RuntimeManager>,
    heartbeat_secs: u64,
    lease_secs: u64,
) -> NamespaceOwnerHeartbeat {
    NamespaceOwnerHeartbeat::new(tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(heartbeat_secs));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        interval.tick().await;
        loop {
            interval.tick().await;
            match runtime.renew_namespace_owner(lease_secs).await {
                Ok(true) => {}
                Ok(false) => {
                    tracing::error!(
                        environment = runtime.environment(),
                        namespace = runtime.namespace(),
                        "runtime namespace ownership was superseded; stopping heartbeat"
                    );
                    break;
                }
                Err(error) => {
                    tracing::warn!(
                        environment = runtime.environment(),
                        namespace = runtime.namespace(),
                        %error,
                        "runtime namespace owner renewal failed; admissions are frozen"
                    );
                }
            }
        }
    }))
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
    workers: usize,
    maintenance_interval: Duration,
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
            boundless_engine.start_workers_with_maintenance_interval(
                registration.workers,
                registration.maintenance_interval,
            );
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
            sp1_engine.start_workers_with_maintenance_interval(
                registration.workers,
                registration.maintenance_interval,
            );
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
        risc0_engine.start_workers_with_maintenance_interval(
            registration.workers,
            registration.maintenance_interval,
        );
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
        boundless_engine.start_workers_with_maintenance_interval(
            registration.workers,
            registration.maintenance_interval,
        );
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
        sp1_engine.start_workers_with_maintenance_interval(
            registration.workers,
            registration.maintenance_interval,
        );
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
        native_engine.start_workers_with_maintenance_interval(
            registration.workers,
            registration.maintenance_interval,
        );
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
            workers: registration.workers,
            maintenance_interval: registration.maintenance_interval,
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
            workers: registration.workers,
            maintenance_interval: registration.maintenance_interval,
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
    workers: usize,
    maintenance_interval: Duration,
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
    engine.start_workers_with_maintenance_interval(
        registration.workers,
        registration.maintenance_interval,
    );
    factory.insert(
        registration.pair.key.clone(),
        registration.pipeline_key,
        Arc::new(engine),
    );
    Ok(())
}

async fn restore_proof_artifacts_from_runtime_tasks(runtime: &Arc<RuntimeManager>) -> Result<()> {
    for record in runtime.list_tasks().await? {
        if let Err(err) = restore_proof_artifacts_from_runtime_task(runtime, &record).await {
            warn!(
                task_id = record.task_id,
                error = %err,
                "failed to restore proof artifact from runtime task; skipping"
            );
        }
    }
    Ok(())
}

async fn restore_proof_artifacts_from_runtime_task(
    runtime: &RuntimeManager,
    record: &raiko2_runtime::RuntimeTaskRecord,
) -> Result<()> {
    let metadata: TaskMetadata = serde_json::from_value(record.metadata.clone())
        .context("failed to parse runtime task metadata for proof artifact restore")?;
    restore_cached_proof_artifacts_from_metadata(runtime, record, &metadata).await;

    if record.runner_status != RunnerStatus::Completed {
        return Ok(());
    }
    let Some(restored_refs) = root_proof_artifact_refs(&metadata, record.pipeline_key) else {
        return Ok(());
    };
    let mut missing_refs = Vec::new();
    for proof_ref in &restored_refs.refs {
        if runtime
            .get_proof_artifact(
                &metadata.network_pair,
                record.pipeline_key,
                record.route,
                proof_ref,
            )
            .await?
            .is_none()
        {
            missing_refs.push(proof_ref.clone());
        }
    }
    if missing_refs.is_empty() {
        return Ok(());
    }
    let mut proof_bytes = None;
    for proof_ref in &restored_refs.refs {
        if let Some(object) = runtime
            .read_proof_artifact_bytes(
                &metadata.network_pair,
                record.pipeline_key,
                record.route,
                proof_ref,
            )
            .await?
        {
            proof_bytes = Some(object.bytes);
            break;
        }
    }
    let Some(proof_bytes) = proof_bytes else {
        return Ok(());
    };
    let proof: Proof =
        serde_json::from_slice(&proof_bytes).context("failed to parse restored proof artifact")?;

    if restored_refs.kind == ProofArtifactKind::Proposal
        && let Err(err) = validate_external_aggregate_proofs(record.route, &[proof])
    {
        warn!(
            task_id = record.task_id,
            proof_uri = record.proof_uri.as_deref().unwrap_or_default(),
            error = %err,
            "completed proposal proof is not aggregatable; skipping artifact restore"
        );
        return Ok(());
    }

    for proof_ref in missing_refs {
        let publication = runtime
            .publish_proof_artifact_bytes(
                &metadata.network_pair,
                record.pipeline_key,
                record.route,
                &proof_ref,
                &proof_bytes,
            )
            .await
            .with_context(|| format!("failed to publish proof artifact for {proof_ref}"))?;
        let artifact = publication.object();
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: metadata.network_pair.clone(),
                proof_ref,
                pipeline_key: record.pipeline_key,
                route: record.route,
                proof_uri: artifact.proof_uri.clone(),
                content_hash: artifact.content_hash.clone(),
                generation: artifact.generation,
            })
            .await?;
    }
    Ok(())
}

async fn restore_cached_proof_artifacts_from_metadata(
    runtime: &RuntimeManager,
    record: &raiko2_runtime::RuntimeTaskRecord,
    metadata: &TaskMetadata,
) {
    for (proof_ref, kind) in persisted_child_proof_refs(metadata, record.pipeline_key) {
        if let Err(err) =
            restore_cached_proof_artifact(runtime, record, metadata, &proof_ref, kind).await
        {
            warn!(
                task_id = record.task_id,
                proof_ref,
                error = %err,
                "failed to restore cached proof artifact; skipping"
            );
        }
    }
}

async fn restore_cached_proof_artifact(
    runtime: &RuntimeManager,
    record: &raiko2_runtime::RuntimeTaskRecord,
    metadata: &TaskMetadata,
    proof_ref: &str,
    kind: ProofArtifactKind,
) -> Result<()> {
    if runtime
        .get_proof_artifact(
            &metadata.network_pair,
            record.pipeline_key,
            record.route,
            proof_ref,
        )
        .await?
        .is_some()
    {
        return Ok(());
    }
    let Some(artifact) = runtime
        .read_proof_artifact_bytes(
            &metadata.network_pair,
            record.pipeline_key,
            record.route,
            proof_ref,
        )
        .await?
    else {
        return Ok(());
    };
    let proof: Proof = serde_json::from_slice(&artifact.bytes)
        .with_context(|| format!("failed to parse proof artifact for {proof_ref}"))?;
    if kind == ProofArtifactKind::Proposal
        && let Err(err) = validate_external_aggregate_proofs(record.route, &[proof])
    {
        warn!(
            task_id = record.task_id,
            proof_ref,
            error = %err,
            "cached proposal proof is not aggregatable; skipping artifact restore"
        );
        return Ok(());
    }

    runtime
        .upsert_proof_artifact(ProofArtifactRegistration {
            network_pair: metadata.network_pair.clone(),
            proof_ref: proof_ref.to_string(),
            pipeline_key: record.pipeline_key,
            route: record.route,
            proof_uri: artifact.proof_uri,
            content_hash: artifact.content_hash,
            generation: artifact.generation,
        })
        .await?;
    Ok(())
}

fn persisted_child_proof_refs(
    metadata: &TaskMetadata,
    pipeline_key: PipelineKey,
) -> Vec<(String, ProofArtifactKind)> {
    let mut seen = BTreeSet::new();
    let mut refs = Vec::new();

    for proposal in &metadata.proposals {
        if let Some(request) = proposal.request.as_ref() {
            push_restored_ref(
                &mut refs,
                &mut seen,
                proposal_task_ref(pipeline_key, request),
                ProofArtifactKind::Proposal,
            );
        }
        push_restored_ref(
            &mut refs,
            &mut seen,
            proposal.task_id.clone(),
            ProofArtifactKind::Proposal,
        );
    }

    if let Some(request) = metadata.aggregate_request.as_ref() {
        push_restored_ref(
            &mut refs,
            &mut seen,
            aggregate_task_ref(pipeline_key, request),
            ProofArtifactKind::Aggregate,
        );
    }
    if let Some(task_id) = metadata.aggregate_task_id.as_ref() {
        push_restored_ref(
            &mut refs,
            &mut seen,
            task_id.clone(),
            ProofArtifactKind::Aggregate,
        );
    }

    refs
}

fn push_restored_ref(
    refs: &mut Vec<(String, ProofArtifactKind)>,
    seen: &mut BTreeSet<String>,
    proof_ref: String,
    kind: ProofArtifactKind,
) {
    if proof_ref.is_empty() || !seen.insert(proof_ref.clone()) {
        return;
    }
    refs.push((proof_ref, kind));
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
    use crate::server::task_metadata::{ProposalTask, RuntimeMetadata, proposal_task_ref};
    use raiko2_engine::{ProposalTaskRequest, ProverTaskConfig};
    use raiko2_pipeline::{GuestSystem, PipelineRoute, RunnerKind};
    use raiko2_runtime::RuntimeTaskRecord;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct CancellationMarker(Arc<AtomicBool>);

    impl Drop for CancellationMarker {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    #[tokio::test]
    async fn heartbeat_guard_keeps_task_alive_during_slow_startup() {
        let ticks = Arc::new(AtomicUsize::new(0));
        let task_ticks = Arc::clone(&ticks);
        let heartbeat = NamespaceOwnerHeartbeat::new(tokio::spawn(async move {
            loop {
                task_ticks.fetch_add(1, Ordering::Release);
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }));

        tokio::time::sleep(Duration::from_millis(20)).await;

        assert!(ticks.load(Ordering::Acquire) > 1);
        heartbeat.stop().await;
    }

    #[tokio::test]
    async fn heartbeat_guard_stop_awaits_task_cancellation() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let task_cancelled = Arc::clone(&cancelled);
        let heartbeat = NamespaceOwnerHeartbeat::new(tokio::spawn(async move {
            let _marker = CancellationMarker(task_cancelled);
            std::future::pending::<()>().await;
        }));
        tokio::task::yield_now().await;

        heartbeat.stop().await;

        assert!(cancelled.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn restore_proof_artifacts_registers_cached_child_proposal_refs() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "restore-child-proposal",
        ))?);
        let request = ProposalTaskRequest {
            proposal_id: 9,
            l2_block_range: Some(raiko2_primitives::L2BlockRange { start: 9, end: 9 }),
            l1_inclusion_block_number: 20,
            last_anchor_block_number: 8,
            checkpoint: None,
            blob_proof_type: None,
            prover: None,
            graffiti: None,
            prover_config: ProverTaskConfig::default(),
        };
        let proposal_ref = proposal_task_ref(PipelineKey::ShastaNative, &request);
        runtime
            .publish_proof_artifact_bytes(
                "taiko_dev/ethereum",
                PipelineKey::ShastaNative,
                PipelineKey::ShastaNative.route(),
                &proposal_ref,
                &serde_json::to_vec_pretty(&valid_native_proof())?,
            )
            .await?;

        let metadata = TaskMetadata {
            network_pair: "taiko_dev/ethereum".to_string(),
            network: "taiko_dev".to_string(),
            l1_network: "ethereum".to_string(),
            proof_type: ProofType::Native,
            requested_proof_type: None,
            prover_type: None,
            execution_mode: None,
            aggregate_requested: true,
            proposals: vec![ProposalTask {
                proposal_id: 9,
                checkpoint: None,
                l1_inclusion_block_number: 20,
                l2_block_numbers: vec![9],
                last_anchor_block_number: 8,
                task_id: proposal_ref.clone(),
                request: Some(request),
            }],
            aggregate_task_id: None,
            aggregate_request: None,
            aggregate_input_artifacts: Vec::new(),
            runtime: RuntimeMetadata::default(),
        };
        runtime
            .upsert_task(&runtime_record(
                "aggregate-root",
                PipelineKey::ShastaNative,
                PipelineRoute::new(GuestSystem::Native, RunnerKind::Local),
                RunnerStatus::Allocated,
                None,
                serde_json::to_value(metadata)?,
            ))
            .await?;

        restore_proof_artifacts_from_runtime_tasks(&runtime).await?;

        runtime
            .get_proof_artifact(
                "taiko_dev/ethereum",
                PipelineKey::ShastaNative,
                PipelineKey::ShastaNative.route(),
                &proposal_ref,
            )
            .await?
            .expect("proposal proof artifact");
        assert!(
            runtime
                .read_proof_artifact_bytes(
                    "taiko_dev/ethereum",
                    PipelineKey::ShastaNative,
                    PipelineKey::ShastaNative.route(),
                    &proposal_ref,
                )
                .await?
                .is_some()
        );
        Ok(())
    }

    #[tokio::test]
    async fn restore_proof_artifacts_skips_bad_metadata() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "restore-bad-metadata",
        ))?);
        runtime
            .upsert_task(&runtime_record(
                "bad-root",
                PipelineKey::ShastaNative,
                PipelineRoute::new(GuestSystem::Native, RunnerKind::Local),
                RunnerStatus::Completed,
                Some("/tmp/nonexistent-proof.json".to_string()),
                serde_json::json!({ "bad": true }),
            ))
            .await?;

        restore_proof_artifacts_from_runtime_tasks(&runtime).await?;
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

    fn valid_native_proof() -> Proof {
        Proof {
            proof: Some("0xproof".to_string()),
            input: Some(alloy_primitives::B256::ZERO),
            extra_data: Some(serde_json::json!({ "native": true })),
            ..Proof::default()
        }
    }

    fn runtime_record(
        task_id: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        runner_status: RunnerStatus,
        proof_uri: Option<String>,
        metadata: serde_json::Value,
    ) -> RuntimeTaskRecord {
        RuntimeTaskRecord {
            task_id: task_id.to_string(),
            pipeline_key,
            route,
            task_kind: "hoodi_batch".to_string(),
            proposal_id: None,
            proof_ids: vec![],
            runner_status,
            image_ref: None,
            provider_request_id: None,
            remote_tx_hash: None,
            proof_uri,
            error: None,
            metadata,
            request_fingerprint: None,
            updated_at: 1,
        }
    }

    fn unique_test_runtime_root(prefix: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("raiko2-state-{prefix}-{unique}"))
    }
}
