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
use anyhow::{Context, Result};
use raiko2_engine::{Engine, EngineObserver};
use raiko2_pipeline::{
    NativeBackend, PipelineKey, Risc0ShastaBackend, Sp1ShastaBackend,
    forks::shasta::{ShastaSpec, load_shasta_backends},
};
use raiko2_primitives::{Proof, ProofType};
#[cfg(feature = "boundless")]
use raiko2_prover::boundless::BoundlessProver;
use raiko2_prover::validate_external_aggregate_proofs;
use raiko2_prover::{native::NativeProver, risc0::Risc0Prover, sp1::Sp1Prover};
use raiko2_provider::NetworkProvider;
use raiko2_queue::{MemoryStore, SchedulerConfig};
use raiko2_runtime::{ProofArtifactRegistration, RunnerStatus, RuntimeManager};
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

type Risc0Spec = ShastaSpec<Risc0Prover, Risc0ShastaBackend, NetworkProvider>;
type Sp1Spec = ShastaSpec<Sp1Prover, Sp1ShastaBackend, NetworkProvider>;
type NativeSpec = ShastaSpec<NativeProver, NativeBackend, NetworkProvider>;
#[cfg(feature = "boundless")]
type BoundlessSpec = ShastaSpec<BoundlessProver, Risc0ShastaBackend, NetworkProvider>;

use super::sampling::ZkAnySampler;
use super::task_cleanup::spawn_runtime_cleanup_loop;
use super::task_metadata::{ProofArtifactKind, TaskMetadata, root_proof_artifact_refs};

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
    pub async fn new(config: Config) -> Result<Self> {
        let runtime = Arc::new(RuntimeManager::new(config.runtime.root.clone())?);
        restore_proof_artifacts_from_runtime_tasks(&runtime).await?;
        let scheduler_config = setup::scheduler_config(&config);
        let workers = config.queue.workers;
        let maintenance_interval = Duration::from_millis(config.queue.maintenance_interval_ms);
        let resolved_pairs = config.rpc.resolved_pairs()?;
        let shasta_backends = load_shasta_backends().map_err(anyhow::Error::msg)?;

        let mut factory = StaticPipelineFactory::default();

        for pair in &resolved_pairs {
            let runtime_observer: Arc<dyn EngineObserver> =
                Arc::new(RuntimeObserver::new(Arc::clone(&runtime), pair.key.clone()));
            let risc0_engine = build_risc0_engine(
                &config,
                pair,
                shasta_backends.risc0.clone(),
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

            #[cfg(feature = "boundless")]
            {
                let boundless_scheduler_config = setup::boundless_scheduler_config(&config);
                let boundless_engine = build_boundless_engine(
                    &config,
                    pair,
                    shasta_backends.risc0_boundless.clone(),
                    boundless_scheduler_config,
                    Arc::clone(&runtime_observer),
                )
                .await?;
                boundless_engine
                    .start_workers_with_maintenance_interval(workers, maintenance_interval);
                factory.insert(
                    pair.key.clone(),
                    PipelineKey::ShastaRisc0Boundless,
                    Arc::new(boundless_engine),
                );
            }

            let sp1_engine = build_sp1_engine(
                &config,
                pair,
                shasta_backends.sp1.clone(),
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
    if record.runner_status != RunnerStatus::Completed {
        return Ok(());
    }
    let Some(proof_path) = record.proof_path.as_deref() else {
        return Ok(());
    };
    let metadata: TaskMetadata = serde_json::from_value(record.metadata.clone())
        .context("failed to parse runtime task metadata for proof artifact restore")?;
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

#[cfg_attr(not(feature = "redis-queue"), allow(clippy::unused_async))]
async fn build_sp1_engine(
    config: &Config,
    pair: &ResolvedNetworkPair,
    backend: Sp1ShastaBackend,
    scheduler_config: SchedulerConfig,
    observer: Arc<dyn EngineObserver>,
) -> Result<Engine<Sp1Spec>> {
    let sp1_config = setup::sp1_prover_config(config);

    let engine = match config.queue.backend {
        QueueBackend::Memory => {
            let provider = setup::build_provider(config, pair)?;
            let context = setup::build_context(config, pair, ProofType::Sp1)?;
            let spec = ShastaSpec::new(
                PipelineKey::ShastaSp1,
                Sp1Prover::new(sp1_config),
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
                let spec = ShastaSpec::new(
                    PipelineKey::ShastaSp1,
                    Sp1Prover::new(sp1_config),
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

#[cfg(feature = "boundless")]
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
                PipelineKey::ShastaRisc0Boundless,
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
