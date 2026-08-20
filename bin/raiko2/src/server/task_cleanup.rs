use crate::config::Config;
use crate::server::lifecycle::ProofLifecycle;
use crate::server::proof_artifact::ProofArtifactPayload;
use crate::server::state::PipelineFactory;
use crate::server::task_metadata::{
    ProofArtifactKind, TaskMetadata, proposal_proof_artifact_refs, root_proof_artifact_refs,
};
use anyhow::{Context, Result};
use raiko2_engine::{EngineTaskId, EngineTaskKey, ProposalTaskRequest};
use raiko2_runtime::{
    ExpiredTaskCursor, ProofArtifactRecord, RunnerStatus, RuntimeManager, RuntimeTaskRecord,
};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::MissedTickBehavior;
use tracing::{info, warn};

const RUNTIME_CLEANUP_BATCH_SIZE: usize = 64;
const ORPHANED_RUNTIME_TASK_TTL_SECS: u64 = 7 * 24 * 60 * 60;
const ORPHANED_RUNTIME_ERROR: &str = "runtime task orphaned: no active local execution";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RuntimeCleanupStats {
    pub scanned: usize,
    pub expired: usize,
    pub retired_roots: usize,
    pub skipped_roots: usize,
    pub removed_roots: usize,
    pub skipped_shared_children: usize,
    pub retained_failures: usize,
    pub invalidated_artifacts: usize,
    pub removed_artifacts: usize,
    pub retained_artifact_failures: usize,
    pub removed_pending_publications: usize,
    pub retained_pending_publication_failures: usize,
    pub orphaned_cancelled: usize,
}

impl RuntimeCleanupStats {
    const fn is_idle(self) -> bool {
        self.scanned == 0
            && self.expired == 0
            && self.retired_roots == 0
            && self.skipped_roots == 0
            && self.removed_roots == 0
            && self.skipped_shared_children == 0
            && self.retained_failures == 0
            && self.invalidated_artifacts == 0
            && self.removed_artifacts == 0
            && self.retained_artifact_failures == 0
            && self.removed_pending_publications == 0
            && self.retained_pending_publication_failures == 0
            && self.orphaned_cancelled == 0
    }
}

pub(crate) fn spawn_runtime_cleanup_loop(
    config: Arc<Config>,
    runtime: Arc<RuntimeManager>,
    pipelines: Arc<dyn PipelineFactory>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut orphan_cursor = None;
        let mut terminal_cursor = None;
        let interval_duration = Duration::from_millis(config.queue.maintenance_interval_ms);
        let terminal_task_ttl_secs = config.runtime.terminal_task_ttl_secs;
        log_runtime_cleanup_stats(
            run_runtime_cleanup_pass(
                Arc::clone(&runtime),
                Arc::clone(&pipelines),
                ORPHANED_RUNTIME_TASK_TTL_SECS,
                terminal_task_ttl_secs,
                &mut orphan_cursor,
                &mut terminal_cursor,
            )
            .await,
        );

        let mut interval = tokio::time::interval(interval_duration);
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        interval.tick().await;

        loop {
            interval.tick().await;
            log_runtime_cleanup_stats(
                run_runtime_cleanup_pass(
                    Arc::clone(&runtime),
                    Arc::clone(&pipelines),
                    ORPHANED_RUNTIME_TASK_TTL_SECS,
                    terminal_task_ttl_secs,
                    &mut orphan_cursor,
                    &mut terminal_cursor,
                )
                .await,
            );
        }
    })
}

pub(crate) async fn run_runtime_cleanup_pass(
    runtime: Arc<RuntimeManager>,
    pipelines: Arc<dyn PipelineFactory>,
    orphan_ttl_secs: u64,
    terminal_ttl_secs: u64,
    orphan_cursor: &mut Option<ExpiredTaskCursor>,
    terminal_cursor: &mut Option<ExpiredTaskCursor>,
) -> Result<RuntimeCleanupStats> {
    if orphan_ttl_secs == 0 && terminal_ttl_secs == 0 {
        return Ok(RuntimeCleanupStats::default());
    }
    let lifecycle = ProofLifecycle::new(Arc::clone(&runtime), Arc::clone(&pipelines));

    let orphaned_cancelled =
        cancel_orphaned_runtime_tasks(runtime.as_ref(), &lifecycle, orphan_ttl_secs, orphan_cursor)
            .await?;
    let records = runtime
        .list_expired_terminal_tasks(
            now_ts(),
            terminal_ttl_secs,
            terminal_cursor.as_ref(),
            RUNTIME_CLEANUP_BATCH_SIZE,
        )
        .await?;
    *terminal_cursor = records.last().map(|record| ExpiredTaskCursor {
        updated_at: record.updated_at,
        task_id: record.task_id.clone(),
    });
    let mut stats = RuntimeCleanupStats {
        scanned: records.len(),
        expired: records.len(),
        orphaned_cancelled,
        ..RuntimeCleanupStats::default()
    };

    let artifact_records = runtime
        .list_reclaimable_proof_artifacts(None, RUNTIME_CLEANUP_BATCH_SIZE)
        .await?;
    let artifact_cleanup = lifecycle
        .remove_artifact_retention_batch(&artifact_records)
        .await?;
    stats.invalidated_artifacts = artifact_cleanup.invalidated_artifacts;
    stats.removed_artifacts = artifact_cleanup.removed_artifacts;
    stats.retained_artifact_failures = artifact_cleanup.retained_artifact_failures;

    let pending_publications = runtime
        .list_reclaimable_pending_publications(None, RUNTIME_CLEANUP_BATCH_SIZE)
        .await?;
    let pending_cleanup = lifecycle
        .remove_pending_retention_batch(&pending_publications)
        .await?;
    stats.invalidated_artifacts = stats
        .invalidated_artifacts
        .saturating_add(pending_cleanup.invalidated_artifacts);
    stats.removed_artifacts = stats
        .removed_artifacts
        .saturating_add(pending_cleanup.removed_artifacts);
    stats.retained_artifact_failures = stats
        .retained_artifact_failures
        .saturating_add(pending_cleanup.retained_artifact_failures);
    stats.removed_pending_publications = pending_cleanup.removed_pending_publications;
    stats.retained_pending_publication_failures =
        pending_cleanup.retained_pending_publication_failures;

    let cleanup = lifecycle.remove_terminal_retention_batch(&records).await?;
    stats.retired_roots = cleanup.retired_roots;
    stats.skipped_roots = cleanup.skipped_roots;
    stats.removed_roots = cleanup.removed_roots;
    stats.skipped_shared_children = cleanup.skipped_shared_children;
    stats.retained_failures = cleanup.retained_root_failures;
    stats.invalidated_artifacts = stats
        .invalidated_artifacts
        .saturating_add(cleanup.invalidated_artifacts);
    stats.removed_artifacts = stats
        .removed_artifacts
        .saturating_add(cleanup.removed_artifacts);
    stats.retained_artifact_failures = stats
        .retained_artifact_failures
        .saturating_add(cleanup.retained_artifact_failures);
    crate::server::telemetry::record_runtime_cleanup_stats(&stats);
    crate::server::telemetry::record_runtime_state_stats(runtime.runtime_state_stats().await);

    Ok(stats)
}

async fn cancel_orphaned_runtime_tasks(
    runtime: &RuntimeManager,
    lifecycle: &ProofLifecycle,
    ttl_secs: u64,
    cursor: &mut Option<ExpiredTaskCursor>,
) -> Result<usize> {
    let records = runtime
        .list_stale_nonterminal_tasks(
            now_ts(),
            ttl_secs,
            cursor.as_ref(),
            RUNTIME_CLEANUP_BATCH_SIZE,
        )
        .await?;
    *cursor = records.last().map(|record| ExpiredTaskCursor {
        updated_at: record.updated_at,
        task_id: record.task_id.clone(),
    });
    let mut cancelled = 0usize;

    for record in records {
        let metadata = match TaskMetadata::decode_for_record(&record) {
            Ok(metadata) => metadata,
            Err(err) => {
                warn!(
                    task_id = %record.task_id,
                    error = %err,
                    "skipping orphaned runtime task check with invalid metadata"
                );
                continue;
            }
        };

        if reconcile_runtime_task_from_artifacts(runtime, &record, &metadata)
            .await?
            .is_some()
        {
            continue;
        }

        if metadata.has_remote_submission_progress() {
            continue;
        }

        let cancellation = lifecycle
            .cancel_orphaned_if_unchanged(&record, ORPHANED_RUNTIME_ERROR.to_string())
            .await
            .with_context(|| format!("failed cancel orphaned runtime task {}", record.task_id))?;
        if !matches!(
            cancellation,
            raiko2_runtime::RuntimeMutationOutcome::Applied
        ) {
            continue;
        }

        cancelled += 1;
        warn!(task_id = %record.task_id, "cancelled orphaned runtime task");
    }

    Ok(cancelled)
}

pub(crate) async fn reconcile_runtime_task_from_artifacts(
    runtime: &RuntimeManager,
    record: &RuntimeTaskRecord,
    metadata: &TaskMetadata,
) -> Result<Option<String>> {
    if !matches!(
        record.runner_status,
        RunnerStatus::Allocated | RunnerStatus::Running
    ) {
        return Ok(None);
    }

    let mut artifacts = Vec::new();
    if let Some(root_refs) = root_proof_artifact_refs(metadata, record.pipeline_key) {
        let expected_payload = match root_refs.kind {
            ProofArtifactKind::Proposal => ProofArtifactPayload::Proposal,
            ProofArtifactKind::Aggregate => ProofArtifactPayload::Final,
        };
        let Some(artifact) =
            load_first_artifact(runtime, record, &root_refs.refs, expected_payload).await?
        else {
            return Ok(None);
        };
        artifacts.push(artifact);
    } else {
        for proposal in &metadata.proposals {
            let refs = proposal_proof_artifact_refs(record.pipeline_key, proposal);
            let Some(artifact) =
                load_first_artifact(runtime, record, &refs, ProofArtifactPayload::Proposal).await?
            else {
                return Ok(None);
            };
            artifacts.push(artifact);
        }
    }
    let Some(proof_uri) = artifacts.last().map(|artifact| artifact.proof_uri.clone()) else {
        return Ok(None);
    };
    let artifact_preconditions = artifacts
        .into_iter()
        .map(|artifact| artifact.precondition())
        .collect::<Vec<_>>();
    if runtime
        .complete_nonterminal_task(
            &record.task_id,
            record.incarnation_id,
            &proof_uri,
            &artifact_preconditions,
        )
        .await?
    {
        return Ok(Some(proof_uri));
    }

    let current = runtime.get_task(&record.task_id).await?;
    Ok(current
        .filter(|current| {
            current.incarnation_id == record.incarnation_id
                && current.runner_status == RunnerStatus::Completed
        })
        .and_then(|current| current.proof_uri))
}

async fn load_first_artifact(
    runtime: &RuntimeManager,
    record: &RuntimeTaskRecord,
    proof_refs: &[String],
    expected_payload: ProofArtifactPayload,
) -> Result<Option<ProofArtifactRecord>> {
    for proof_ref in proof_refs {
        if let Some(material) = crate::server::proof_artifact::load_proof_artifact_material(
            runtime,
            &record.network_pair,
            record.pipeline_key,
            record.route,
            proof_ref,
            expected_payload,
        )
        .await?
        {
            return Ok(Some(material.record));
        }
    }
    Ok(None)
}

pub(crate) fn proposal_task_chain_ids(task_id: &EngineTaskId) -> Vec<EngineTaskId> {
    if !matches!(task_id.0, EngineTaskKey::Proposal { .. }) {
        return Vec::new();
    }

    vec![task_id.clone()]
}

fn log_runtime_cleanup_stats(result: Result<RuntimeCleanupStats>) {
    match result {
        Ok(stats) => {
            if !stats.is_idle() {
                info!(
                    scanned = stats.scanned,
                    expired = stats.expired,
                    retired_roots = stats.retired_roots,
                    skipped_roots = stats.skipped_roots,
                    removed_roots = stats.removed_roots,
                    skipped_shared_children = stats.skipped_shared_children,
                    retained_failures = stats.retained_failures,
                    invalidated_artifacts = stats.invalidated_artifacts,
                    removed_artifacts = stats.removed_artifacts,
                    retained_artifact_failures = stats.retained_artifact_failures,
                    removed_pending_publications = stats.removed_pending_publications,
                    retained_pending_publication_failures =
                        stats.retained_pending_publication_failures,
                    orphaned_cancelled = stats.orphaned_cancelled,
                    "runtime cleanup tick completed"
                );
            }
        }
        Err(err) => {
            warn!(error = %err, "runtime cleanup tick failed");
        }
    }
}

pub(crate) const fn proposal_task_id(
    pipeline_key: raiko2_pipeline::PipelineKey,
    request: ProposalTaskRequest,
) -> EngineTaskId {
    EngineTaskId::new(EngineTaskKey::Proposal {
        pipeline: pipeline_key,
        request,
    })
}

fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .cast_signed()
}

#[cfg(test)]
mod tests {
    use super::{
        ExpiredTaskCursor, RuntimeCleanupStats, proposal_task_id, run_runtime_cleanup_pass,
    };
    use crate::server::state::{
        EngineHandle, EngineQueueTaskState, EngineQueueTaskView, StaticPipelineFactory,
    };
    use crate::server::task_metadata::{
        ProposalTask, RuntimeMetadata, TaskMetadata, TaskRuntimeMetadata, proposal_task_ref,
        publication_proof_artifact_refs,
    };
    use anyhow::{Context, Result};
    use async_trait::async_trait;
    use raiko2_engine::{EngineTaskId, EngineTaskKey, ProposalTaskRequest, ProverTaskConfig};
    use raiko2_pipeline::PipelineKey;
    use raiko2_queue::{RootOwner, TaskStoreError, decode_task_id, encode_task_id};
    use raiko2_runtime::{
        ProofArtifactDeleteResult, ProofArtifactDescriptor, ProofArtifactKey, ProofArtifactObject,
        ProofArtifactPrefix, ProofArtifactPutResult, ProofArtifactRegistration, RunnerStatus,
        RuntimeManager, TaskRegistration,
        test_support::{
            ExactInvalidationResult, MemoryProofArtifactStore, ProofObjectStore,
            RuntimeStateObject, RuntimeStateStore, RuntimeStateWriteResult, RuntimeStoreScope,
        },
    };
    use std::collections::HashSet;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

    #[test]
    fn runtime_cleanup_stats_report_idle_when_zero() {
        assert!(RuntimeCleanupStats::default().is_idle());
        assert!(
            !RuntimeCleanupStats {
                scanned: 1,
                ..RuntimeCleanupStats::default()
            }
            .is_idle()
        );
    }

    #[tokio::test]
    async fn runtime_cleanup_cancels_orphaned_roots_without_remote_progress() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root("orphaned"))?);
        let orphaned_task_id = encoded_proposal_task_id(10)?;
        let remote_task_id =
            encoded_proposal_task_id_for_pipeline(11, PipelineKey::ShastaRisc0Network)?;
        let queued_task_id = encoded_proposal_task_id(13)?;
        let now = now_ts();
        let stale = now.saturating_sub(7_201);

        register_runtime_task(
            runtime.as_ref(),
            "orphaned-root",
            &orphaned_task_id,
            RunnerStatus::Running,
            stale,
        )
        .await?;
        register_runtime_task(
            runtime.as_ref(),
            "fresh-root",
            &queued_task_id,
            RunnerStatus::Running,
            now,
        )
        .await?;
        register_runtime_task(
            runtime.as_ref(),
            "queued-root",
            &queued_task_id,
            RunnerStatus::Running,
            stale,
        )
        .await?;

        let mut remote_metadata = metadata_for_task(&remote_task_id);
        remote_metadata.runtime.proposals.insert(
            remote_metadata.proposals[0].task_id.clone(),
            TaskRuntimeMetadata {
                provider_request_id: Some("0xremote".to_string()),
                image_ref: Some("0ximage".to_string()),
                deployment: Some("base".to_string()),
                offchain: Some(false),
                expires_at: Some(1_000),
                lock_expires_at: Some(900),
                submitted_at: Some(800),
                quoted_mcycles_count: Some(1),
                evaluated_mcycles_count: Some(1),
                max_price_multiplier: Some(1),
                max_price_wei: Some("1000".to_string()),
                rebid_attempt: Some(1),
                ..TaskRuntimeMetadata::default()
            },
        );
        register_runtime_task_with_metadata(
            runtime.as_ref(),
            "remote-root",
            &remote_metadata,
            PipelineKey::ShastaRisc0Network,
            RunnerStatus::Running,
            stale,
        )
        .await?;

        let active_owner = runtime
            .get_task("fresh-root")
            .await?
            .map(|record| RootOwner::new(record.task_id, record.incarnation_id))
            .expect("fresh root");
        let engine = Arc::new(MockEngine::with_active_projection(
            HashSet::from([active_owner]),
            HashSet::from([queued_task_id]),
        ));
        let factory = Arc::new(build_factory(engine));

        let mut orphan_cursor = None;
        let mut terminal_cursor = None;
        let stats = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
            7_200,
            7_200,
            &mut orphan_cursor,
            &mut terminal_cursor,
        )
        .await?;

        assert_eq!(
            stats,
            RuntimeCleanupStats {
                orphaned_cancelled: 2,
                ..RuntimeCleanupStats::default()
            }
        );
        let orphaned = runtime
            .get_task("orphaned-root")
            .await?
            .expect("orphaned root");
        assert_eq!(orphaned.runner_status, RunnerStatus::Cancelled);
        assert_eq!(
            orphaned.error.as_deref(),
            Some("runtime task orphaned: no active local execution")
        );
        let remote = runtime.get_task("remote-root").await?.expect("remote root");
        assert_eq!(remote.runner_status, RunnerStatus::Running);
        let fresh = runtime.get_task("fresh-root").await?.expect("fresh root");
        assert_eq!(fresh.runner_status, RunnerStatus::Running);
        let queued = runtime.get_task("queued-root").await?.expect("queued root");
        assert_eq!(queued.runner_status, RunnerStatus::Cancelled);
        Ok(())
    }

    struct MockEngine {
        failing_owners: HashSet<String>,
        active_owners: HashSet<RootOwner>,
        queue_task_ids: HashSet<String>,
    }

    impl Default for MockEngine {
        fn default() -> Self {
            Self {
                failing_owners: HashSet::new(),
                active_owners: HashSet::new(),
                queue_task_ids: HashSet::new(),
            }
        }
    }

    impl MockEngine {
        fn with_failing_owners(failing_owners: HashSet<String>) -> Self {
            Self {
                failing_owners,
                active_owners: HashSet::new(),
                queue_task_ids: HashSet::new(),
            }
        }

        fn with_active_projection(
            active_owners: HashSet<RootOwner>,
            queue_task_ids: HashSet<String>,
        ) -> Self {
            Self {
                active_owners,
                queue_task_ids,
                ..Self::default()
            }
        }
    }

    impl EngineHandle for MockEngine {
        fn get_status(
            &self,
            _id: EngineTaskId,
        ) -> BoxFuture<'_, Result<Option<crate::server::state::EngineStatusView>, TaskStoreError>>
        {
            Box::pin(async { Ok(None) })
        }

        fn get_task_state(
            &self,
            id: EngineTaskId,
        ) -> BoxFuture<'_, Result<Option<crate::server::state::EngineQueueTaskView>, TaskStoreError>>
        {
            let present = self
                .queue_task_ids
                .contains(&encode_task_id(&id).expect("encode task id"));
            Box::pin(async move {
                Ok(present.then_some(EngineQueueTaskView {
                    state: EngineQueueTaskState::Running,
                }))
            })
        }

        fn has_active_execution(
            &self,
            owner: RootOwner,
        ) -> BoxFuture<'_, Result<bool, TaskStoreError>> {
            let active = self.active_owners.contains(&owner);
            Box::pin(async move { Ok(active) })
        }

        fn attach_execution_plan(
            &self,
            _owner: raiko2_queue::RootOwner,
            _plan: raiko2_engine::EngineExecutionPlan,
        ) -> BoxFuture<'_, Result<raiko2_queue::AttachOutcome, TaskStoreError>> {
            Box::pin(async { Ok(raiko2_queue::AttachOutcome::Attached) })
        }

        fn detach_execution(
            &self,
            owner: raiko2_queue::RootOwner,
            mode: raiko2_queue::DetachMode,
        ) -> BoxFuture<'_, Result<raiko2_queue::DetachOutcome<EngineTaskKey>, TaskStoreError>>
        {
            let should_fail = self.failing_owners.contains(&owner.task_id);
            Box::pin(async move {
                if should_fail {
                    Err(TaskStoreError::backend(std::io::Error::other(
                        "mock detach failure",
                    )))
                } else {
                    Ok(raiko2_queue::DetachOutcome::not_attached(mode))
                }
            })
        }
    }

    #[derive(Debug)]
    struct FailOnceInvalidationStore {
        inner: MemoryProofArtifactStore,
        runtime_state_writes: AtomicUsize,
        fail_next_invalidation: AtomicBool,
        fail_next_delete: AtomicBool,
        block_next_invalidation: AtomicBool,
        invalidation_started: tokio::sync::Notify,
        allow_invalidation: tokio::sync::Notify,
    }

    impl FailOnceInvalidationStore {
        fn new() -> Result<Self> {
            Ok(Self {
                inner: MemoryProofArtifactStore::new(
                    "test".into(),
                    "retention-invalidation-retry".into(),
                )?,
                runtime_state_writes: AtomicUsize::new(0),
                fail_next_invalidation: AtomicBool::new(true),
                fail_next_delete: AtomicBool::new(false),
                block_next_invalidation: AtomicBool::new(false),
                invalidation_started: tokio::sync::Notify::new(),
                allow_invalidation: tokio::sync::Notify::new(),
            })
        }

        fn blocking() -> Result<Self> {
            let mut store = Self::new()?;
            store.fail_next_invalidation = AtomicBool::new(false);
            store.block_next_invalidation = AtomicBool::new(true);
            Ok(store)
        }

        fn counting() -> Result<Self> {
            let mut store = Self::new()?;
            store.fail_next_invalidation = AtomicBool::new(false);
            Ok(store)
        }

        fn pending_delete_failure() -> Result<Self> {
            let mut store = Self::new()?;
            store.fail_next_invalidation = AtomicBool::new(false);
            store.fail_next_delete = AtomicBool::new(true);
            Ok(store)
        }
    }

    impl RuntimeStoreScope for FailOnceInvalidationStore {
        fn environment(&self) -> &str {
            self.inner.environment()
        }

        fn namespace(&self) -> &str {
            self.inner.namespace()
        }

        fn backend_name(&self) -> &'static str {
            self.inner.backend_name()
        }
    }

    #[async_trait]
    impl ProofObjectStore for FailOnceInvalidationStore {
        async fn put_if_absent(
            &self,
            key: &ProofArtifactKey,
            bytes: &[u8],
        ) -> Result<ProofArtifactPutResult> {
            self.inner.put_if_absent(key, bytes).await
        }

        async fn get(&self, key: &ProofArtifactKey) -> Result<Option<ProofArtifactObject>> {
            self.inner.get(key).await
        }

        async fn get_descriptor(
            &self,
            key: &ProofArtifactKey,
        ) -> Result<Option<ProofArtifactDescriptor>> {
            self.inner.get_descriptor(key).await
        }

        async fn get_prefix(
            &self,
            key: &ProofArtifactKey,
            max_bytes: usize,
        ) -> Result<Option<ProofArtifactPrefix>> {
            self.inner.get_prefix(key, max_bytes).await
        }

        async fn invalidate_exact(
            &self,
            key: &ProofArtifactKey,
            descriptor: &ProofArtifactDescriptor,
        ) -> Result<ExactInvalidationResult> {
            if self.fail_next_invalidation.swap(false, Ordering::AcqRel) {
                anyhow::bail!("injected proof artifact invalidation failure");
            }
            if self.block_next_invalidation.swap(false, Ordering::AcqRel) {
                self.invalidation_started.notify_one();
                self.allow_invalidation.notified().await;
            }
            self.inner.invalidate_exact(key, descriptor).await
        }

        async fn is_invalidated(
            &self,
            key: &ProofArtifactKey,
            descriptor: &ProofArtifactDescriptor,
        ) -> Result<bool> {
            self.inner.is_invalidated(key, descriptor).await
        }

        async fn delete_exact(
            &self,
            key: &ProofArtifactKey,
            descriptor: &ProofArtifactDescriptor,
        ) -> Result<ProofArtifactDeleteResult> {
            if self.fail_next_delete.swap(false, Ordering::AcqRel) {
                anyhow::bail!("injected pending proof publication deletion failure");
            }
            self.inner.delete_exact(key, descriptor).await
        }
    }

    #[async_trait]
    impl RuntimeStateStore for FailOnceInvalidationStore {
        async fn load_runtime_state(&self) -> Result<Option<RuntimeStateObject>> {
            self.inner.load_runtime_state().await
        }

        async fn store_runtime_state(
            &self,
            bytes: &[u8],
            expected_generation: Option<i64>,
        ) -> Result<RuntimeStateWriteResult> {
            self.runtime_state_writes.fetch_add(1, Ordering::AcqRel);
            self.inner
                .store_runtime_state(bytes, expected_generation)
                .await
        }
    }

    #[tokio::test]
    async fn runtime_cleanup_keeps_stale_root_with_active_queue_task() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root("orphaned-active"))?);
        let active_task_id = encoded_proposal_task_id(14)?;
        let stale = now_ts().saturating_sub(7_201);

        register_runtime_task(
            runtime.as_ref(),
            "active-root",
            &active_task_id,
            RunnerStatus::Running,
            stale,
        )
        .await?;

        let active_owner = runtime
            .get_task("active-root")
            .await?
            .map(|record| RootOwner::new(record.task_id, record.incarnation_id))
            .expect("active root");
        let engine = Arc::new(MockEngine::with_active_projection(
            HashSet::from([active_owner]),
            HashSet::from([active_task_id]),
        ));
        let factory = Arc::new(build_factory(engine));

        let mut orphan_cursor = None;
        let mut terminal_cursor = None;
        let stats = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
            7_200,
            7_200,
            &mut orphan_cursor,
            &mut terminal_cursor,
        )
        .await?;

        assert_eq!(stats, RuntimeCleanupStats::default());
        let active = runtime.get_task("active-root").await?.expect("active root");
        assert_eq!(active.runner_status, RunnerStatus::Running);
        Ok(())
    }

    #[tokio::test]
    async fn runtime_cleanup_persists_artifact_completion_before_orphan_cancellation() -> Result<()>
    {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "orphaned-artifact-complete",
        ))?);
        let encoded_proposal_ref = encoded_proposal_task_id(15)?;
        let engine = Arc::new(MockEngine::default());
        let factory = Arc::new(build_factory(engine));
        let stale = now_ts().saturating_sub(7_201);

        register_runtime_task(
            runtime.as_ref(),
            "artifact-complete-root",
            &encoded_proposal_ref,
            RunnerStatus::Running,
            stale,
        )
        .await?;
        let metadata = metadata_for_task(&encoded_proposal_ref);
        let proof_ref = proposal_task_ref(PipelineKey::ShastaRisc0, &metadata.proposals[0].request);
        let artifact = runtime
            .publish_proof_artifact_bytes(
                "taiko_dev/ethereum",
                PipelineKey::ShastaRisc0,
                PipelineKey::ShastaRisc0.route(),
                &proof_ref,
                br#"{"proof":"0xcomplete"}"#,
            )
            .await?
            .try_object()
            .expect("proof publication should materialize content")
            .clone();
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: "taiko_dev/ethereum".to_string(),
                proof_ref: proof_ref.clone(),
                pipeline_key: PipelineKey::ShastaRisc0,
                route: PipelineKey::ShastaRisc0.route(),
                proof_uri: artifact.proof_uri,
                content_hash: artifact.content_hash,
                generation: artifact.generation,
            })
            .await?;

        let mut orphan_cursor = None;
        let mut terminal_cursor = None;
        let stats = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
            7_200,
            7_200,
            &mut orphan_cursor,
            &mut terminal_cursor,
        )
        .await?;

        assert_eq!(stats.orphaned_cancelled, 0);
        let record = runtime
            .get_task("artifact-complete-root")
            .await?
            .expect("artifact-complete root");
        assert_eq!(record.runner_status, RunnerStatus::Completed);
        assert!(record.proof_uri.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn restart_reconciliation_completes_compressed_sp1_proposal_root() -> Result<()> {
        let runtime = RuntimeManager::new(unique_runtime_root("sp1-artifact-reconcile"))?;
        let encoded_task = encoded_proposal_task_id_for_pipeline(16, PipelineKey::ShastaSp1)?;
        let metadata = metadata_for_task(&encoded_task);
        register_runtime_task_with_metadata(
            &runtime,
            "sp1-root",
            &metadata,
            PipelineKey::ShastaSp1,
            RunnerStatus::Running,
            1,
        )
        .await?;
        let record = runtime.get_task("sp1-root").await?.expect("SP1 root");
        let proof_ref = proposal_task_ref(PipelineKey::ShastaSp1, &metadata.proposals[0].request);
        let proof = raiko2_primitives::Proof {
            input: Some(alloy_primitives::B256::ZERO),
            quote: Some(r#"{"Compressed":{}}"#.to_string()),
            uuid: Some("sp1-verifying-key".to_string()),
            extra_data: Some(serde_json::json!({ "shasta": {} })),
            ..raiko2_primitives::Proof::default()
        };
        let artifact = runtime
            .publish_proof_artifact_bytes(
                &record.network_pair,
                record.pipeline_key,
                record.route,
                &proof_ref,
                &serde_json::to_vec(&proof)?,
            )
            .await?
            .try_object()
            .expect("proof publication should materialize content")
            .clone();
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: record.network_pair.clone(),
                proof_ref,
                pipeline_key: record.pipeline_key,
                route: record.route,
                proof_uri: artifact.proof_uri,
                content_hash: artifact.content_hash,
                generation: artifact.generation,
            })
            .await?;

        let proof_uri = super::reconcile_runtime_task_from_artifacts(&runtime, &record, &metadata)
            .await?
            .expect("compressed proposal should reconcile root completion");
        let completed = runtime
            .get_task("sp1-root")
            .await?
            .expect("completed SP1 root");
        assert_eq!(completed.runner_status, RunnerStatus::Completed);
        assert_eq!(completed.proof_uri.as_deref(), Some(proof_uri.as_str()));
        Ok(())
    }

    #[tokio::test]
    async fn runtime_cleanup_removes_expired_root_but_keeps_shared_child_tasks() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root("ttl-shared"))?);
        let engine = Arc::new(MockEngine::default());
        let factory = Arc::new(build_factory(engine.clone()));
        let proposal_task_id = encoded_proposal_task_id(1)?;

        register_runtime_task(
            runtime.as_ref(),
            "expired-root",
            &proposal_task_id,
            RunnerStatus::Completed,
            1,
        )
        .await?;
        register_runtime_task(
            runtime.as_ref(),
            "live-root",
            &proposal_task_id,
            RunnerStatus::Completed,
            now_ts(),
        )
        .await?;
        let expired = runtime
            .get_task("expired-root")
            .await?
            .context("expired runtime root")?;
        let shared_proof_ref = expired
            .artifact_refs
            .first()
            .context("shared proof reference")?;
        register_runtime_proof_artifact(runtime.as_ref(), &expired, shared_proof_ref).await?;

        let mut orphan_cursor = None;
        let mut terminal_cursor = None;
        let stats = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
            7_200,
            7_200,
            &mut orphan_cursor,
            &mut terminal_cursor,
        )
        .await?;

        assert_eq!(
            stats,
            RuntimeCleanupStats {
                scanned: 1,
                expired: 1,
                retired_roots: 1,
                skipped_roots: 0,
                removed_roots: 1,
                skipped_shared_children: 0,
                retained_failures: 0,
                invalidated_artifacts: 0,
                removed_artifacts: 0,
                retained_artifact_failures: 0,
                removed_pending_publications: 0,
                retained_pending_publication_failures: 0,
                orphaned_cancelled: 0,
            }
        );
        assert!(runtime.get_task("expired-root").await?.is_none());
        assert!(runtime.get_task("live-root").await?.is_some());
        assert!(
            runtime
                .get_proof_artifact(
                    &expired.network_pair,
                    expired.pipeline_key,
                    expired.route,
                    shared_proof_ref,
                )
                .await?
                .is_some()
        );
        Ok(())
    }

    #[tokio::test]
    async fn runtime_cleanup_invalidates_and_removes_unowned_artifact() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root("ttl-artifact"))?);
        let factory = Arc::new(build_factory(Arc::new(MockEngine::default())));
        register_runtime_task(
            runtime.as_ref(),
            "expired-root",
            &encoded_proposal_task_id(19)?,
            RunnerStatus::Completed,
            1,
        )
        .await?;
        let expired = runtime
            .get_task("expired-root")
            .await?
            .context("expired runtime root")?;
        let proof_ref = expired
            .artifact_refs
            .first()
            .context("expired proof reference")?;
        let artifact =
            register_runtime_proof_artifact(runtime.as_ref(), &expired, proof_ref).await?;

        let mut orphan_cursor = None;
        let mut terminal_cursor = None;
        let stats = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
            7_200,
            7_200,
            &mut orphan_cursor,
            &mut terminal_cursor,
        )
        .await?;

        assert_eq!(stats.removed_roots, 1);
        assert!(
            runtime
                .get_proof_artifact_including_invalidated(
                    &expired.network_pair,
                    expired.pipeline_key,
                    expired.route,
                    proof_ref,
                )
                .await?
                .is_none()
        );
        assert!(
            runtime
                .proof_artifact_descriptor_is_invalidated(
                    &expired.network_pair,
                    expired.pipeline_key,
                    expired.route,
                    proof_ref,
                    &artifact.descriptor(),
                )
                .await?
        );
        Ok(())
    }

    #[tokio::test]
    async fn legacy_orphan_artifact_is_reclaimed_without_terminal_roots() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root("legacy-orphan"))?);
        let factory = Arc::new(build_factory(Arc::new(MockEngine::default())));
        register_runtime_task(
            runtime.as_ref(),
            "legacy-root",
            &encoded_proposal_task_id(47)?,
            RunnerStatus::Completed,
            1,
        )
        .await?;
        let root = runtime
            .get_task("legacy-root")
            .await?
            .context("legacy root")?;
        let proof_ref = root
            .artifact_refs
            .first()
            .context("legacy proof reference")?;
        let artifact = register_runtime_proof_artifact(runtime.as_ref(), &root, proof_ref).await?;
        assert!(matches!(
            runtime.retire_task_if_unchanged(&root, None).await?,
            raiko2_runtime::RuntimeMutationOutcome::Applied
        ));
        let retired = runtime
            .get_task("legacy-root")
            .await?
            .context("retired legacy root")?;
        assert!(matches!(
            runtime.remove_task_if_current(&retired.lifetime()).await?,
            raiko2_runtime::RuntimeMutationOutcome::Applied
        ));

        let mut orphan_cursor = None;
        let mut terminal_cursor = None;
        let stats = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
            7_200,
            7_200,
            &mut orphan_cursor,
            &mut terminal_cursor,
        )
        .await?;

        assert_eq!(stats.scanned, 0);
        assert_eq!(stats.invalidated_artifacts, 1);
        assert_eq!(stats.removed_artifacts, 1);
        assert!(
            runtime
                .get_proof_artifact_including_invalidated(
                    &root.network_pair,
                    root.pipeline_key,
                    root.route,
                    proof_ref,
                )
                .await?
                .is_none()
        );
        assert!(
            runtime
                .proof_artifact_descriptor_is_invalidated(
                    &root.network_pair,
                    root.pipeline_key,
                    root.route,
                    proof_ref,
                    &artifact.descriptor(),
                )
                .await?
        );
        Ok(())
    }

    #[tokio::test]
    async fn runtime_cleanup_retains_failed_artifact_without_retaining_root() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::with_store(Arc::new(
            FailOnceInvalidationStore::new()?,
        )));
        let factory = Arc::new(build_factory(Arc::new(MockEngine::default())));
        register_runtime_task(
            runtime.as_ref(),
            "expired-root",
            &encoded_proposal_task_id(18)?,
            RunnerStatus::Completed,
            1,
        )
        .await?;
        let expired = runtime
            .get_task("expired-root")
            .await?
            .context("expired runtime root")?;
        let proof_ref = expired
            .artifact_refs
            .first()
            .context("expired proof reference")?;
        register_runtime_proof_artifact(runtime.as_ref(), &expired, proof_ref).await?;

        let mut orphan_cursor = None;
        let mut terminal_cursor = None;
        let first = run_runtime_cleanup_pass(
            runtime.clone(),
            factory.clone(),
            7_200,
            7_200,
            &mut orphan_cursor,
            &mut terminal_cursor,
        )
        .await?;
        assert_eq!(first.removed_roots, 1);
        assert_eq!(first.retained_failures, 0);
        assert_eq!(first.retained_artifact_failures, 1);
        assert!(runtime.get_task("expired-root").await?.is_none());
        assert!(
            runtime
                .get_proof_artifact_including_invalidated(
                    &expired.network_pair,
                    expired.pipeline_key,
                    expired.route,
                    proof_ref,
                )
                .await?
                .is_some()
        );
        Ok(())
    }

    #[tokio::test]
    async fn artifact_failure_does_not_retain_detached_roots() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::with_store(Arc::new(
            FailOnceInvalidationStore::new()?,
        )));
        let factory = Arc::new(build_factory(Arc::new(MockEngine::default())));
        register_runtime_task(
            runtime.as_ref(),
            "artifact-root",
            &encoded_proposal_task_id(45)?,
            RunnerStatus::Completed,
            1,
        )
        .await?;
        register_runtime_task(
            runtime.as_ref(),
            "plain-root",
            &encoded_proposal_task_id(46)?,
            RunnerStatus::Completed,
            2,
        )
        .await?;
        let artifact_root = runtime
            .get_task("artifact-root")
            .await?
            .context("artifact root")?;
        let plain_root = runtime
            .get_task("plain-root")
            .await?
            .context("plain root")?;
        let original_incarnation = artifact_root.incarnation_id;
        let proof_ref = artifact_root
            .artifact_refs
            .first()
            .context("artifact proof reference")?;
        register_runtime_proof_artifact(runtime.as_ref(), &artifact_root, proof_ref).await?;

        let cleanup = crate::server::lifecycle::ProofLifecycle::new(Arc::clone(&runtime), factory)
            .remove_terminal_retention_batch(&[artifact_root.clone(), plain_root])
            .await?;

        assert_eq!(cleanup.removed_roots, 2);
        assert_eq!(cleanup.retained_root_failures, 0);
        assert_eq!(cleanup.retained_artifact_failures, 1);
        assert!(runtime.get_task("artifact-root").await?.is_none());
        assert!(runtime.get_task("plain-root").await?.is_none());
        assert!(
            runtime
                .get_proof_artifact_including_invalidated(
                    &artifact_root.network_pair,
                    artifact_root.pipeline_key,
                    artifact_root.route,
                    proof_ref,
                )
                .await?
                .is_some()
        );

        register_runtime_task(
            runtime.as_ref(),
            "artifact-root",
            &encoded_proposal_task_id(45)?,
            RunnerStatus::Running,
            now_ts(),
        )
        .await?;
        assert_ne!(
            runtime
                .get_task("artifact-root")
                .await?
                .context("replacement artifact root")?
                .incarnation_id,
            original_incarnation
        );
        Ok(())
    }

    #[tokio::test]
    async fn artifact_invalidation_does_not_hold_the_execution_lifecycle_gate() -> Result<()> {
        let store = Arc::new(FailOnceInvalidationStore::blocking()?);
        let runtime = Arc::new(RuntimeManager::with_store(store.clone()));
        register_runtime_task(
            runtime.as_ref(),
            "expired-root",
            &encoded_proposal_task_id(19)?,
            RunnerStatus::Completed,
            1,
        )
        .await?;
        let expired = runtime
            .get_task("expired-root")
            .await?
            .context("expired runtime root")?;
        let proof_ref = expired
            .artifact_refs
            .first()
            .context("expired proof reference")?;
        register_runtime_proof_artifact(runtime.as_ref(), &expired, proof_ref).await?;

        let lifecycle = crate::server::lifecycle::ProofLifecycle::new(
            Arc::clone(&runtime),
            Arc::new(build_factory(Arc::new(MockEngine::default()))),
        );
        let cleanup = tokio::spawn({
            let lifecycle = lifecycle.clone();
            let expired = expired.clone();
            async move { lifecycle.remove_terminal_retention_batch(&[expired]).await }
        });
        store.invalidation_started.notified().await;

        let lifecycle_gate = runtime.execution_lifecycle_gate();
        let guard = tokio::time::timeout(std::time::Duration::from_secs(1), lifecycle_gate.lock())
            .await
            .expect("external artifact invalidation must not hold the lifecycle gate");
        drop(guard);
        store.allow_invalidation.notify_one();

        assert_eq!(cleanup.await??.removed_roots, 1);
        Ok(())
    }

    #[tokio::test]
    async fn pending_publication_failure_does_not_block_artifact_or_root_cleanup() -> Result<()> {
        let store = Arc::new(FailOnceInvalidationStore::pending_delete_failure()?);
        let runtime = Arc::new(RuntimeManager::with_store(store));
        let factory = Arc::new(build_factory(Arc::new(MockEngine::default())));
        register_runtime_task(
            runtime.as_ref(),
            "pending-root",
            &encoded_proposal_task_id(20)?,
            RunnerStatus::Running,
            1,
        )
        .await?;
        let mut terminal = runtime
            .get_task("pending-root")
            .await?
            .context("pending runtime root")?;
        let proof_ref = terminal
            .artifact_refs
            .first()
            .context("pending proof reference")?
            .clone();
        assert!(
            runtime
                .checkpoint_pending_proof_publication(
                    &terminal.network_pair,
                    terminal.pipeline_key,
                    terminal.route,
                    &proof_ref,
                    &[terminal.incarnation_id],
                    b"pending-proof",
                )
                .await?
        );
        terminal.runner_status = RunnerStatus::Completed;
        terminal.proof_uri = Some("memory://proofs/pending-root".into());
        terminal.updated_at = 1;
        runtime.upsert_task(&terminal).await?;

        let mut orphan_cursor = None;
        let mut terminal_cursor = None;
        let first = run_runtime_cleanup_pass(
            runtime.clone(),
            factory.clone(),
            7_200,
            7_200,
            &mut orphan_cursor,
            &mut terminal_cursor,
        )
        .await?;
        assert_eq!(first.removed_roots, 1);
        assert_eq!(first.retained_failures, 0);
        assert_eq!(first.retained_pending_publication_failures, 0);
        assert!(
            runtime
                .get_pending_proof_publication(
                    &terminal.network_pair,
                    terminal.pipeline_key,
                    terminal.route,
                    &proof_ref,
                )
                .await?
                .is_some()
        );
        assert!(runtime.get_task("pending-root").await?.is_none());

        register_runtime_task(
            runtime.as_ref(),
            "legacy-artifact-root",
            &encoded_proposal_task_id(48)?,
            RunnerStatus::Completed,
            1,
        )
        .await?;
        let legacy_root = runtime
            .get_task("legacy-artifact-root")
            .await?
            .context("legacy artifact root")?;
        let legacy_ref = legacy_root
            .artifact_refs
            .first()
            .context("legacy artifact proof reference")?;
        register_runtime_proof_artifact(runtime.as_ref(), &legacy_root, legacy_ref).await?;
        assert!(matches!(
            runtime.retire_task_if_unchanged(&legacy_root, None).await?,
            raiko2_runtime::RuntimeMutationOutcome::Applied
        ));
        let retired_legacy = runtime
            .get_task("legacy-artifact-root")
            .await?
            .context("retired legacy artifact root")?;
        assert!(matches!(
            runtime
                .remove_task_if_current(&retired_legacy.lifetime())
                .await?,
            raiko2_runtime::RuntimeMutationOutcome::Applied
        ));

        let failed_pending = run_runtime_cleanup_pass(
            runtime.clone(),
            factory.clone(),
            7_200,
            7_200,
            &mut orphan_cursor,
            &mut terminal_cursor,
        )
        .await?;
        assert_eq!(failed_pending.removed_artifacts, 1);
        assert_eq!(failed_pending.retained_pending_publication_failures, 1);
        assert!(
            runtime
                .get_pending_proof_publication(
                    &terminal.network_pair,
                    terminal.pipeline_key,
                    terminal.route,
                    &proof_ref,
                )
                .await?
                .is_some()
        );

        let retry = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
            7_200,
            7_200,
            &mut orphan_cursor,
            &mut terminal_cursor,
        )
        .await?;
        assert_eq!(retry.removed_pending_publications, 1);
        assert!(
            runtime
                .get_pending_proof_publication(
                    &terminal.network_pair,
                    terminal.pipeline_key,
                    terminal.route,
                    &proof_ref,
                )
                .await?
                .is_none()
        );
        Ok(())
    }

    #[tokio::test]
    async fn runtime_cleanup_batches_terminal_state_writes() -> Result<()> {
        let store = Arc::new(FailOnceInvalidationStore::counting()?);
        let runtime = Arc::new(RuntimeManager::with_store(store.clone()));
        let engine = Arc::new(MockEngine::default());
        let factory = Arc::new(build_factory(engine));
        for proposal_id in 20..23 {
            register_runtime_task(
                runtime.as_ref(),
                &format!("expired-{proposal_id}"),
                &encoded_proposal_task_id(proposal_id)?,
                RunnerStatus::Completed,
                1,
            )
            .await?;
        }
        let writes_before = store.runtime_state_writes.load(Ordering::Acquire);

        let mut orphan_cursor = None;
        let mut terminal_cursor = None;
        let stats = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
            7_200,
            7_200,
            &mut orphan_cursor,
            &mut terminal_cursor,
        )
        .await?;

        let writes_after = store.runtime_state_writes.load(Ordering::Acquire);
        assert_eq!(stats.removed_roots, 3);
        assert_eq!(writes_after - writes_before, 2);
        Ok(())
    }

    #[tokio::test]
    async fn runtime_cleanup_retries_a_durable_retirement_after_projection_failure() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root("ttl-failure"))?);
        let engine = Arc::new(MockEngine::with_failing_owners(HashSet::from([
            "expired-root".to_string(),
        ])));
        let factory = Arc::new(build_factory(engine));
        let proposal_task_id = encoded_proposal_task_id(9)?;

        register_runtime_task(
            runtime.as_ref(),
            "expired-root",
            &proposal_task_id,
            RunnerStatus::Failed,
            1,
        )
        .await?;

        let mut orphan_cursor = None;
        let mut terminal_cursor = None;
        let stats = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
            7_200,
            7_200,
            &mut orphan_cursor,
            &mut terminal_cursor,
        )
        .await?;

        assert_eq!(
            stats,
            RuntimeCleanupStats {
                scanned: 1,
                expired: 1,
                retired_roots: 1,
                skipped_roots: 0,
                removed_roots: 0,
                skipped_shared_children: 0,
                retained_failures: 1,
                invalidated_artifacts: 0,
                removed_artifacts: 0,
                retained_artifact_failures: 0,
                removed_pending_publications: 0,
                retained_pending_publication_failures: 0,
                orphaned_cancelled: 0,
            }
        );
        let retired = runtime
            .get_task("expired-root")
            .await?
            .expect("retired root remains recoverable");
        assert_eq!(retired.runner_status, RunnerStatus::Cancelled);
        assert_eq!(retired.updated_at, 1);

        let healthy_factory = Arc::new(build_factory(Arc::new(MockEngine::default())));
        let empty_page = run_runtime_cleanup_pass(
            runtime.clone(),
            healthy_factory.clone(),
            7_200,
            7_200,
            &mut orphan_cursor,
            &mut terminal_cursor,
        )
        .await?;
        assert_eq!(empty_page, RuntimeCleanupStats::default());
        assert!(terminal_cursor.is_none());

        let retry = run_runtime_cleanup_pass(
            runtime.clone(),
            healthy_factory,
            7_200,
            7_200,
            &mut orphan_cursor,
            &mut terminal_cursor,
        )
        .await?;
        assert_eq!(
            retry,
            RuntimeCleanupStats {
                scanned: 1,
                expired: 1,
                retired_roots: 1,
                skipped_roots: 0,
                removed_roots: 1,
                skipped_shared_children: 0,
                retained_failures: 0,
                invalidated_artifacts: 0,
                removed_artifacts: 0,
                retained_artifact_failures: 0,
                removed_pending_publications: 0,
                retained_pending_publication_failures: 0,
                orphaned_cancelled: 0,
            }
        );
        assert!(runtime.get_task("expired-root").await?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn runtime_cleanup_cursor_advances_past_failed_old_records() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root("ttl-cursor"))?);
        let engine = Arc::new(MockEngine::with_failing_owners(HashSet::from([
            "expired-a".to_string(),
        ])));
        let factory = Arc::new(build_factory(engine));
        let mut orphan_cursor = None;
        let mut terminal_cursor = None;

        register_runtime_task(
            runtime.as_ref(),
            "expired-a",
            &encoded_proposal_task_id(1)?,
            RunnerStatus::Failed,
            1,
        )
        .await?;
        register_runtime_task(
            runtime.as_ref(),
            "expired-b",
            &encoded_proposal_task_id(2)?,
            RunnerStatus::Completed,
            2,
        )
        .await?;

        let first = run_runtime_cleanup_pass(
            runtime.clone(),
            factory.clone(),
            7_200,
            7_200,
            &mut orphan_cursor,
            &mut terminal_cursor,
        )
        .await?;
        assert_eq!(
            first,
            RuntimeCleanupStats {
                scanned: 2,
                expired: 2,
                retired_roots: 2,
                skipped_roots: 0,
                removed_roots: 1,
                skipped_shared_children: 0,
                retained_failures: 1,
                invalidated_artifacts: 0,
                removed_artifacts: 0,
                retained_artifact_failures: 0,
                removed_pending_publications: 0,
                retained_pending_publication_failures: 0,
                orphaned_cancelled: 0,
            }
        );
        assert_eq!(
            terminal_cursor,
            Some(ExpiredTaskCursor {
                updated_at: 2,
                task_id: "expired-b".to_string()
            })
        );

        register_runtime_task(
            runtime.as_ref(),
            "expired-c",
            &encoded_proposal_task_id(3)?,
            RunnerStatus::Completed,
            3,
        )
        .await?;

        let second = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
            7_200,
            7_200,
            &mut orphan_cursor,
            &mut terminal_cursor,
        )
        .await?;
        assert_eq!(
            second,
            RuntimeCleanupStats {
                scanned: 1,
                expired: 1,
                retired_roots: 1,
                skipped_roots: 0,
                removed_roots: 1,
                skipped_shared_children: 0,
                retained_failures: 0,
                invalidated_artifacts: 0,
                removed_artifacts: 0,
                retained_artifact_failures: 0,
                removed_pending_publications: 0,
                retained_pending_publication_failures: 0,
                orphaned_cancelled: 0,
            }
        );
        assert!(runtime.get_task("expired-c").await?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn expired_snapshot_cannot_remove_replacement_queue_children() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "cleanup-incarnation",
        ))?);
        let engine = Arc::new(MockEngine::default());
        let factory = Arc::new(build_factory(engine.clone()));
        let proposal_task_id = encoded_proposal_task_id(44)?;
        register_runtime_task(
            runtime.as_ref(),
            "root",
            &proposal_task_id,
            RunnerStatus::Completed,
            now_ts().saturating_sub(100),
        )
        .await?;
        let stale = runtime.get_task("root").await?.expect("stale root");
        assert!(matches!(
            runtime.retire_task_if_unchanged(&stale, None).await?,
            raiko2_runtime::RuntimeMutationOutcome::Applied
        ));
        assert!(matches!(
            runtime.remove_task_if_current(&stale.lifetime()).await?,
            raiko2_runtime::RuntimeMutationOutcome::Applied
        ));
        register_runtime_task(
            runtime.as_ref(),
            "root",
            &proposal_task_id,
            RunnerStatus::Running,
            now_ts(),
        )
        .await?;

        let cleanup = crate::server::lifecycle::ProofLifecycle::new(Arc::clone(&runtime), factory)
            .remove_terminal_retention_batch(&[stale])
            .await?;
        assert_eq!(cleanup.removed_roots, 0);
        assert_eq!(
            runtime
                .get_task("root")
                .await?
                .expect("replacement")
                .runner_status,
            RunnerStatus::Running
        );
        Ok(())
    }

    fn build_factory(engine: Arc<dyn EngineHandle>) -> StaticPipelineFactory {
        let mut factory = StaticPipelineFactory::default();
        factory.insert("taiko_dev/ethereum", PipelineKey::ShastaRisc0, engine);
        factory
    }

    async fn register_runtime_task(
        runtime: &RuntimeManager,
        task_id: &str,
        proposal_task_id: &str,
        status: RunnerStatus,
        updated_at: i64,
    ) -> Result<()> {
        register_runtime_task_with_pair(
            runtime,
            task_id,
            proposal_task_id,
            status,
            updated_at,
            "taiko_dev/ethereum",
        )
        .await
    }

    async fn register_runtime_task_with_pair(
        runtime: &RuntimeManager,
        task_id: &str,
        proposal_task_id: &str,
        status: RunnerStatus,
        updated_at: i64,
        network_pair: &str,
    ) -> Result<()> {
        let metadata = metadata_for_task_with_pair(proposal_task_id, network_pair);
        let artifact_refs = publication_proof_artifact_refs(&metadata, PipelineKey::ShastaRisc0);
        runtime
            .register_task(TaskRegistration {
                task_id: task_id.to_string(),
                pipeline_key: PipelineKey::ShastaRisc0,
                route: "risc0/local".parse().expect("parse route"),
                task_kind: "hoodi_batch".to_string(),
                network_pair: network_pair.to_string(),
                artifact_refs,
                metadata: serde_json::to_value(metadata)?,
                request_fingerprint: format!("request-{task_id}"),
            })
            .await?;
        let mut record = runtime.get_task(task_id).await?.expect("runtime task");
        record.runner_status = status;
        record.updated_at = updated_at;
        runtime.upsert_task(&record).await?;
        Ok(())
    }

    async fn register_runtime_task_with_metadata(
        runtime: &RuntimeManager,
        task_id: &str,
        metadata: &TaskMetadata,
        pipeline_key: PipelineKey,
        status: RunnerStatus,
        updated_at: i64,
    ) -> Result<()> {
        let artifact_refs = publication_proof_artifact_refs(metadata, pipeline_key);
        runtime
            .register_task(TaskRegistration {
                task_id: task_id.to_string(),
                pipeline_key,
                route: pipeline_key.route(),
                task_kind: "hoodi_batch".to_string(),
                network_pair: metadata.network_pair.clone(),
                artifact_refs,
                metadata: serde_json::to_value(metadata)?,
                request_fingerprint: format!("request-{task_id}"),
            })
            .await?;
        let mut record = runtime.get_task(task_id).await?.expect("runtime task");
        record.runner_status = status;
        record.updated_at = updated_at;
        runtime.upsert_task(&record).await?;
        Ok(())
    }

    async fn register_runtime_proof_artifact(
        runtime: &RuntimeManager,
        record: &raiko2_runtime::RuntimeTaskRecord,
        proof_ref: &str,
    ) -> Result<ProofArtifactRegistration> {
        let object = runtime
            .publish_proof_artifact_bytes(
                &record.network_pair,
                record.pipeline_key,
                record.route,
                proof_ref,
                proof_ref.as_bytes(),
            )
            .await?
            .try_object()
            .context("runtime cleanup proof artifact")?
            .clone();
        let registration = ProofArtifactRegistration {
            network_pair: record.network_pair.clone(),
            proof_ref: proof_ref.to_string(),
            pipeline_key: record.pipeline_key,
            route: record.route,
            proof_uri: object.proof_uri,
            content_hash: object.content_hash,
            generation: object.generation,
        };
        runtime.upsert_proof_artifact(registration.clone()).await?;
        Ok(registration)
    }

    fn metadata_for_task(proposal_task_id: &str) -> TaskMetadata {
        metadata_for_task_with_pair(proposal_task_id, "taiko_dev/ethereum")
    }

    fn metadata_for_task_with_pair(proposal_task_id: &str, network_pair: &str) -> TaskMetadata {
        let (pipeline_key, request) = match decode_task_id::<EngineTaskKey>(proposal_task_id)
            .expect("decode proposal task id")
            .0
        {
            EngineTaskKey::Proposal { pipeline, request } => (pipeline, request),
            EngineTaskKey::Aggregate { .. } => unreachable!("expected proposal task id"),
        };
        let (network, l1_network) = network_pair
            .split_once('/')
            .unwrap_or((network_pair, "ethereum"));
        let task_ref = proposal_task_ref(pipeline_key, &request);
        let l2_block_numbers = request
            .l2_block_range
            .map(|range| (range.start..=range.end).collect())
            .unwrap_or_default();
        TaskMetadata {
            network_pair: network_pair.to_string(),
            network: network.to_string(),
            l1_network: l1_network.to_string(),
            proof_type: pipeline_key.proof_type(),
            requested_proof_type: None,
            prover_type: None,
            execution_mode: None,
            aggregate_requested: false,
            proposals: vec![ProposalTask {
                proposal_id: request.proposal_id,
                checkpoint: request.checkpoint,
                l1_inclusion_block_number: request.l1_inclusion_block_number,
                l2_block_numbers,
                last_anchor_block_number: request.last_anchor_block_number,
                task_id: task_ref,
                request,
            }],
            aggregate_task_id: None,
            aggregate_request: None,
            aggregate_input_artifacts: Vec::new(),
            runtime: RuntimeMetadata::default(),
        }
    }

    fn encoded_proposal_task_id(proposal_id: u64) -> Result<String> {
        encoded_proposal_task_id_for_pipeline(proposal_id, PipelineKey::ShastaRisc0)
    }

    fn encoded_proposal_task_id_for_pipeline(
        proposal_id: u64,
        pipeline_key: PipelineKey,
    ) -> Result<String> {
        let request = ProposalTaskRequest {
            proposal_id,
            l2_block_range: Some(raiko2_primitives::L2BlockRange {
                start: proposal_id,
                end: proposal_id,
            }),
            l1_inclusion_block_number: 0,
            last_anchor_block_number: 0,
            checkpoint: None,
            blob_proof_type: None,
            prover: None,
            graffiti: None,
            prover_config: ProverTaskConfig::default(),
        };
        let task_id = proposal_task_id(pipeline_key, request);
        encode_task_id(&task_id).context("encode proposal task id")
    }

    fn unique_runtime_root(prefix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "{prefix}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ))
    }

    fn now_ts() -> i64 {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        i64::try_from(secs).unwrap_or(i64::MAX)
    }
}
