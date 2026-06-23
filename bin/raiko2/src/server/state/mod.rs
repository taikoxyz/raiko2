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

#[cfg(all(feature = "host", not(feature = "local-provers")))]
use crate::config::PipelineRoute;
#[cfg(feature = "host")]
use crate::config::RunnerKind;
use crate::config::{Config, GuestSystem, QueueBackend, ResolvedNetworkPair};
use anyhow::{Context, Result};
use raiko2_engine::{Engine, EngineObserver};
use raiko2_pipeline::{NativeBackend, PipelineKey, forks::shasta::ShastaSpec};
use raiko2_primitives::{Proof, ProofType};
use raiko2_prover::gaiko2::Gaiko2Prover;
use raiko2_prover::validate_external_aggregate_proofs;
use raiko2_provider::NetworkProvider;
use raiko2_queue::{MemoryStore, SchedulerConfig};
use raiko2_runtime::{ProofArtifactRegistration, RunnerStatus, RuntimeManager};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::fs;
use tracing::warn;

#[cfg(feature = "redis-queue")]
use raiko2_engine::EngineTaskKey;
#[cfg(feature = "redis-queue")]
use raiko2_engine::tasks::EngineTask;

#[cfg(feature = "redis-queue")]
type EngineOutput<I> = raiko2_engine::tasks::EngineOutput<I>;

#[cfg(feature = "local-provers")]
use raiko2_pipeline::forks::shasta::{ShastaBackends, load_shasta_backends};
#[cfg(all(feature = "host", not(feature = "local-provers")))]
use raiko2_pipeline::forks::shasta::{
    load_risc0_boundless_shasta_backend, load_sp1_shasta_backend,
};
#[cfg(feature = "host")]
use raiko2_pipeline::{Risc0ShastaBackend, Sp1ShastaBackend};
#[cfg(feature = "host")]
use raiko2_prover::{boundless::BoundlessProver, sp1::Sp1Prover};
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

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub pipelines: Arc<dyn PipelineFactory>,
    pub runtime: Arc<RuntimeManager>,
    pub zk_any_sampler: Arc<Mutex<ZkAnySampler>>,
}

impl AppState {
    /// Create new application state.
    pub async fn new(mut config: Config) -> Result<Self> {
        config.normalize();
        config.validate()?;
        let runtime = Arc::new(RuntimeManager::new(config.runtime.root.clone())?);
        restore_proof_artifacts_from_runtime_tasks(&runtime).await?;
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

        let mut factory = StaticPipelineFactory::default();

        for pair in &resolved_pairs {
            register_pair_pipelines(
                &mut factory,
                PairPipelineRegistration {
                    config: &config,
                    pair,
                    runtime: Arc::clone(&runtime),
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
                },
            )
            .await?;
        }

        let config = Arc::new(config);
        let pipelines: Arc<dyn PipelineFactory> = Arc::new(factory);
        let zk_any_sampler = Arc::new(Mutex::new(ZkAnySampler::from_config(&config.prover.zk_any)));
        spawn_runtime_cleanup_loop(
            Arc::clone(&config),
            Arc::clone(&runtime),
            Arc::clone(&pipelines),
        );

        Ok(Self {
            config,
            pipelines,
            runtime,
            zk_any_sampler,
        })
    }
}

struct PairPipelineRegistration<'a> {
    config: &'a Config,
    pair: &'a ResolvedNetworkPair,
    runtime: Arc<RuntimeManager>,
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
async fn register_pair_pipelines(
    factory: &mut StaticPipelineFactory,
    registration: PairPipelineRegistration<'_>,
) -> Result<()> {
    if registration.config.prover.is_remote_sgx_route() {
        let runtime_observer: Arc<dyn EngineObserver> = Arc::new(RuntimeObserver::new(
            Arc::clone(&registration.runtime),
            registration.pair.key.clone(),
        ));
        register_remote_sgx_pipelines(factory, &registration, runtime_observer).await?;
        return Ok(());
    }

    #[cfg(all(feature = "host", not(feature = "local-provers")))]
    {
        let runtime_observer: Arc<dyn EngineObserver> = Arc::new(RuntimeObserver::new(
            Arc::clone(&registration.runtime),
            registration.pair.key.clone(),
        ));

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
                Arc::clone(&runtime_observer),
            )
            .await?;
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
                Arc::clone(&runtime_observer),
            )
            .await?;
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

        return Ok(());
    }

    #[cfg(all(not(feature = "host"), not(feature = "local-provers")))]
    {
        anyhow::bail!("local prover routes require building raiko2 with `local-provers`");
    }

    #[cfg(feature = "local-provers")]
    {
        let runtime_observer: Arc<dyn EngineObserver> = Arc::new(RuntimeObserver::new(
            Arc::clone(&registration.runtime),
            registration.pair.key.clone(),
        ));
        let risc0_engine = build_risc0_engine(
            registration.config,
            registration.pair,
            registration.shasta_backends.risc0.clone(),
            registration.scheduler_config.clone(),
            Arc::clone(&runtime_observer),
        )
        .await?;
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
            Arc::clone(&runtime_observer),
        )
        .await?;
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
            Arc::clone(&runtime_observer),
        )
        .await?;
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
            Arc::clone(&runtime_observer),
        )
        .await?;
        native_engine.start_workers_with_maintenance_interval(
            registration.workers,
            registration.maintenance_interval,
        );
        factory.insert(
            registration.pair.key.clone(),
            PipelineKey::ShastaNative,
            Arc::new(native_engine),
        );

        register_remote_sgx_pipelines(factory, &registration, runtime_observer).await?;
        Ok(())
    }
}

async fn register_remote_sgx_pipelines(
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
    )
    .await?;
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
    )
    .await?;
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

async fn register_remote_sgx_engine(
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
    )
    .await?;
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
    let Some(proof_path) = record.proof_path.as_deref() else {
        return Ok(());
    };
    let Some(restored_refs) = root_proof_artifact_refs(&metadata, record.pipeline_key) else {
        return Ok(());
    };
    let proof_bytes = fs::read(proof_path)
        .await
        .with_context(|| format!("failed to read proof file {proof_path}"))?;
    let proof: Proof = serde_json::from_slice(&proof_bytes)
        .with_context(|| format!("failed to parse proof file {proof_path}"))?;

    if restored_refs.kind == ProofArtifactKind::Proposal
        && let Err(err) = validate_external_aggregate_proofs(record.route, &[proof])
    {
        warn!(
            task_id = record.task_id,
            proof_path,
            error = %err,
            "completed proposal proof is not aggregatable; skipping artifact restore"
        );
        return Ok(());
    }

    for proof_ref in restored_refs.refs {
        let artifact_path = runtime
            .write_proof_artifact_bytes(&metadata.network_pair, &proof_ref, &proof_bytes)
            .await
            .with_context(|| format!("failed to write proof artifact for {proof_ref}"))?;
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: metadata.network_pair.clone(),
                proof_ref,
                pipeline_key: record.pipeline_key,
                route: record.route,
                proof_path: artifact_path.display().to_string(),
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
    let proof_path = runtime.proof_artifact_path(&metadata.network_pair, proof_ref);
    if !fs::try_exists(&proof_path)
        .await
        .with_context(|| format!("failed to inspect proof artifact for {proof_ref}"))?
    {
        return Ok(());
    }

    let proof_bytes = fs::read(&proof_path)
        .await
        .with_context(|| format!("failed to read proof artifact for {proof_ref}"))?;
    let proof: Proof = serde_json::from_slice(&proof_bytes)
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
            proof_path: proof_path.display().to_string(),
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
#[cfg_attr(not(feature = "redis-queue"), allow(clippy::unused_async))]
async fn build_risc0_engine(
    config: &Config,
    pair: &ResolvedNetworkPair,
    backend: Risc0ShastaBackend,
    scheduler_config: SchedulerConfig,
    observer: Arc<dyn EngineObserver>,
) -> Result<Engine<Risc0Spec>> {
    let risc0_config = setup::risc0_prover_config(config);

    let engine = match config.queue.backend {
        QueueBackend::Memory => {
            let provider = setup::build_provider(config, pair)?;
            let context = setup::build_context(config, pair, ProofType::Risc0)?;
            let spec = ShastaSpec::new(
                PipelineKey::ShastaRisc0,
                Risc0Prover::new(risc0_config),
                backend,
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
                let context = setup::build_context(config, pair, ProofType::Risc0)?;
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
                    backend,
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

#[cfg(feature = "host")]
#[cfg_attr(not(feature = "redis-queue"), allow(clippy::unused_async))]
async fn build_sp1_engine(
    config: &Config,
    pair: &ResolvedNetworkPair,
    prover: Sp1Prover,
    backend: Sp1ShastaBackend,
    scheduler_config: SchedulerConfig,
    observer: Arc<dyn EngineObserver>,
) -> Result<Engine<Sp1Spec>> {
    let engine = match config.queue.backend {
        QueueBackend::Memory => {
            let provider = setup::build_provider(config, pair)?;
            let context = setup::build_context(config, pair, ProofType::Sp1)?;
            let spec = ShastaSpec::new(PipelineKey::ShastaSp1, prover, backend, provider);
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
                let context = setup::build_context(config, pair, ProofType::Sp1)?;
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
                let spec = ShastaSpec::new(PipelineKey::ShastaSp1, prover, backend, provider);
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

#[cfg(feature = "local-provers")]
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
            let context = setup::build_context(config, pair, ProofType::Native)?;
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
                let context = setup::build_context(config, pair, ProofType::Native)?;
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

#[cfg(feature = "host")]
#[cfg_attr(not(feature = "redis-queue"), allow(clippy::unused_async))]
async fn build_boundless_engine(
    config: &Config,
    pair: &ResolvedNetworkPair,
    backend: Risc0ShastaBackend,
    scheduler_config: SchedulerConfig,
    observer: Arc<dyn EngineObserver>,
) -> Result<Engine<BoundlessSpec>> {
    let agent_config = setup::boundless_prover_config(config, pair);

    let engine = match config.queue.backend {
        QueueBackend::Memory => {
            let provider = setup::build_provider(config, pair)?;
            let context = setup::build_context(config, pair, ProofType::Risc0)?;
            let spec = ShastaSpec::new(
                PipelineKey::ShastaRisc0Network,
                BoundlessProver::new(agent_config),
                backend,
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
                let context = setup::build_context(config, pair, ProofType::Risc0)?;
                let url = config.queue.redis_url.clone().unwrap_or_default();
                let namespace = setup::queue_namespace(
                    &config.queue.namespace,
                    pair,
                    PipelineKey::ShastaRisc0Network,
                );
                let store =
                    raiko2_queue::RedisStore::<EngineTask, BoundlessOutput, EngineTaskKey>::connect(
                        &url,
                        &namespace,
                        scheduler_config.lease_duration,
                    )
                    .await?;
                let spec = ShastaSpec::new(
                    PipelineKey::ShastaRisc0Network,
                    BoundlessProver::new(agent_config),
                    backend,
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

#[cfg_attr(not(feature = "redis-queue"), allow(clippy::unused_async))]
async fn build_remote_sgx_engine(
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

    let engine = match config.queue.backend {
        QueueBackend::Memory => {
            let provider = setup::build_provider(config, pair)?;
            let context = setup::build_context(config, pair, proof_type)?;
            let spec = ShastaSpec::new(
                pipeline_key,
                Gaiko2Prover::new(&gaiko2_config)?,
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
                type Gaiko2Output =
                    EngineOutput<<Gaiko2Spec as raiko2_pipeline::PipelineSpec>::GuestInput>;
                let provider = setup::build_provider(config, pair)?;
                let context = setup::build_context(config, pair, proof_type)?;
                let url = config.queue.redis_url.clone().unwrap_or_default();
                let namespace = setup::queue_namespace(&config.queue.namespace, pair, pipeline_key);
                let store =
                    raiko2_queue::RedisStore::<EngineTask, Gaiko2Output, EngineTaskKey>::connect(
                        &url,
                        &namespace,
                        scheduler_config.lease_duration,
                    )
                    .await?;
                let spec = ShastaSpec::new(
                    pipeline_key,
                    Gaiko2Prover::new(&gaiko2_config)?,
                    NativeBackend,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::task_metadata::{ProposalTask, RuntimeMetadata, proposal_task_ref};
    use raiko2_engine::{ProposalStage, ProposalTaskRequest, ProverTaskConfig};
    use raiko2_pipeline::{GuestSystem, PipelineRoute, RunnerKind};
    use raiko2_queue::{TaskId, encode_task_id};
    use raiko2_runtime::RuntimeTaskRecord;
    use serde::Serialize;

    #[derive(Serialize)]
    enum LegacyEngineTaskKey {
        Proposal {
            pipeline: PipelineKey,
            request: ProposalTaskRequest,
            stage: ProposalStage,
        },
    }

    #[tokio::test]
    async fn restore_proof_artifacts_registers_canonical_and_legacy_proposal_refs() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_test_runtime_root(
            "restore-canonical",
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
        let legacy_ref = encode_task_id(&TaskId::new(LegacyEngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaNative,
            request: request.clone(),
            stage: ProposalStage::Prove,
        }))?;
        let canonical_ref = proposal_task_ref(PipelineKey::ShastaNative, &request);
        assert_ne!(legacy_ref, canonical_ref);

        let proof_path = runtime
            .task_dir(PipelineKey::ShastaNative, "legacy")
            .join("proof.json");
        if let Some(parent) = proof_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(
            &proof_path,
            serde_json::to_vec_pretty(&valid_native_proof())?,
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
            aggregate_requested: false,
            proposals: vec![ProposalTask {
                proposal_id: 9,
                checkpoint: None,
                l1_inclusion_block_number: 20,
                l2_block_numbers: vec![9],
                last_anchor_block_number: 8,
                task_id: legacy_ref.clone(),
                request: Some(request),
            }],
            aggregate_task_id: None,
            aggregate_request: None,
            aggregate_input_artifacts: Vec::new(),
            runtime: RuntimeMetadata::default(),
        };
        runtime
            .upsert_task(&runtime_record(
                "legacy-root",
                PipelineKey::ShastaNative,
                PipelineRoute::new(GuestSystem::Native, RunnerKind::Local),
                RunnerStatus::Completed,
                Some(proof_path.display().to_string()),
                serde_json::to_value(metadata)?,
            ))
            .await?;

        restore_proof_artifacts_from_runtime_tasks(&runtime).await?;

        assert!(
            runtime
                .get_proof_artifact("taiko_dev/ethereum", &canonical_ref)
                .await?
                .is_some()
        );
        assert!(
            runtime
                .get_proof_artifact("taiko_dev/ethereum", &legacy_ref)
                .await?
                .is_some()
        );
        Ok(())
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
            .write_proof_artifact_bytes(
                "taiko_dev/ethereum",
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

        let artifact = runtime
            .get_proof_artifact("taiko_dev/ethereum", &proposal_ref)
            .await?
            .expect("proposal proof artifact");
        assert!(tokio::fs::try_exists(artifact.proof_path).await?);
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
        proof_path: Option<String>,
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
            task_dir: "/tmp/task".to_string(),
            image_ref: None,
            provider_request_id: None,
            remote_tx_hash: None,
            proof_path,
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
