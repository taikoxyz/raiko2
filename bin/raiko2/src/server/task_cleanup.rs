use crate::config::Config;
use crate::server::lifecycle::ProofLifecycle;
use crate::server::state::PipelineFactory;
use crate::server::state::{EngineHandle, EngineQueueTaskState};
use crate::server::task_metadata::{
    TaskMetadata, proposal_proof_artifact_refs, root_proof_artifact_refs,
};
use anyhow::{Context, Result, anyhow};
use raiko2_engine::{EngineTaskId, EngineTaskKey, ProposalTaskRequest};
use raiko2_pipeline::PipelineKey;
use raiko2_queue::DetachMode;
use raiko2_runtime::{
    ExpiredTaskCursor, ProofArtifactRecord, RunnerStatus, RuntimeManager, RuntimeTaskRecord,
};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::MissedTickBehavior;
use tracing::{info, warn};

const RUNTIME_CLEANUP_BATCH_SIZE: usize = 64;
const ORPHANED_RUNTIME_ERROR: &str = "runtime task orphaned: no active local execution";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RuntimeCleanupStats {
    pub scanned: usize,
    pub expired: usize,
    pub removed_roots: usize,
    pub skipped_shared_children: usize,
    pub retained_failures: usize,
    pub orphaned_cancelled: usize,
}

impl RuntimeCleanupStats {
    const fn is_idle(self) -> bool {
        self.scanned == 0
            && self.expired == 0
            && self.removed_roots == 0
            && self.skipped_shared_children == 0
            && self.retained_failures == 0
            && self.orphaned_cancelled == 0
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChildCleanupOutcome {
    pub skipped_shared_children: usize,
}

pub(crate) fn spawn_runtime_cleanup_loop(
    config: Arc<Config>,
    runtime: Arc<RuntimeManager>,
    pipelines: Arc<dyn PipelineFactory>,
) -> tokio::task::JoinHandle<()> {
    const TERMINAL_TASK_TTL_SECS: u64 = 7 * 24 * 60 * 60;
    tokio::spawn(async move {
        let mut orphan_cursor = None;
        let mut terminal_cursor = None;
        let interval_duration = Duration::from_millis(config.queue.maintenance_interval_ms);
        log_runtime_cleanup_stats(
            run_runtime_cleanup_pass(
                Arc::clone(&runtime),
                Arc::clone(&pipelines),
                TERMINAL_TASK_TTL_SECS,
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
                    TERMINAL_TASK_TTL_SECS,
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
    ttl_secs: u64,
    orphan_cursor: &mut Option<ExpiredTaskCursor>,
    terminal_cursor: &mut Option<ExpiredTaskCursor>,
) -> Result<RuntimeCleanupStats> {
    if ttl_secs == 0 {
        return Ok(RuntimeCleanupStats::default());
    }
    let lifecycle = ProofLifecycle::new(Arc::clone(&runtime), Arc::clone(&pipelines));

    let orphaned_cancelled = cancel_orphaned_runtime_tasks(
        runtime.as_ref(),
        pipelines.as_ref(),
        &lifecycle,
        ttl_secs,
        orphan_cursor,
    )
    .await?;
    let records = runtime
        .list_expired_terminal_tasks(
            now_ts(),
            ttl_secs,
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

    for record in records {
        match cleanup_expired_root_task(runtime.as_ref(), &lifecycle, &record).await {
            Ok(Some(outcome)) => {
                stats.removed_roots += 1;
                stats.skipped_shared_children += outcome.skipped_shared_children;
            }
            Ok(None) => {}
            Err(err) => {
                stats.retained_failures += 1;
                warn!(task_id = %record.task_id, error = %err, "failed to cleanup expired runtime task");
            }
        }
    }

    Ok(stats)
}

async fn cancel_orphaned_runtime_tasks(
    runtime: &RuntimeManager,
    pipelines: &dyn PipelineFactory,
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
        if runtime
            .get_task(&record.task_id)
            .await?
            .is_none_or(|current| current.incarnation_id != record.incarnation_id)
        {
            continue;
        }
        let metadata: TaskMetadata = match serde_json::from_value(record.metadata.clone()) {
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

        if let Some(engine) = pipelines.get(&metadata.network_pair, record.pipeline_key)
            && has_active_queue_task(
                engine.as_ref(),
                &metadata_queue_task_ids(&metadata, record.pipeline_key),
            )
            .await?
        {
            continue;
        }

        let cancellation = lifecycle
            .cancel(&record, &metadata, Some(ORPHANED_RUNTIME_ERROR.to_string()))
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

async fn has_active_queue_task(
    engine: &dyn EngineHandle,
    task_ids: &HashSet<EngineTaskId>,
) -> Result<bool> {
    for task_id in task_ids {
        let Some(view) = engine
            .get_task_state(task_id.clone())
            .await
            .map_err(|err| anyhow!("failed load queue task: {err}"))?
        else {
            continue;
        };
        if matches!(
            view.state,
            EngineQueueTaskState::Pending
                | EngineQueueTaskState::Ready
                | EngineQueueTaskState::Retrying
                | EngineQueueTaskState::Running
        ) {
            return Ok(true);
        }
    }

    Ok(false)
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
        let Some(artifact) =
            load_first_artifact(runtime, record, metadata, &root_refs.refs).await?
        else {
            return Ok(None);
        };
        artifacts.push(artifact);
    } else {
        for proposal in &metadata.proposals {
            let refs = proposal_proof_artifact_refs(record.pipeline_key, proposal);
            let Some(artifact) = load_first_artifact(runtime, record, metadata, &refs).await?
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
    metadata: &TaskMetadata,
    proof_refs: &[String],
) -> Result<Option<ProofArtifactRecord>> {
    for proof_ref in proof_refs {
        if let Some(material) = crate::server::proof_artifact::load_proof_artifact_material(
            runtime,
            &metadata.network_pair,
            record.pipeline_key,
            record.route,
            proof_ref,
        )
        .await?
        {
            return Ok(Some(material.record));
        }
    }
    Ok(None)
}

fn metadata_queue_task_ids(
    metadata: &TaskMetadata,
    pipeline_key: PipelineKey,
) -> HashSet<EngineTaskId> {
    let mut task_ids = HashSet::new();
    for proposal in &metadata.proposals {
        let Some(task_id) = proposal.engine_task_id(pipeline_key) else {
            continue;
        };
        task_ids.extend(proposal_task_chain_ids(&task_id));
    }
    if let Some(task_id) = metadata.aggregate_engine_task_id(pipeline_key) {
        task_ids.insert(task_id);
    }
    task_ids
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
                    removed_roots = stats.removed_roots,
                    skipped_shared_children = stats.skipped_shared_children,
                    retained_failures = stats.retained_failures,
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

async fn cleanup_expired_root_task(
    runtime: &RuntimeManager,
    lifecycle: &ProofLifecycle,
    record: &RuntimeTaskRecord,
) -> Result<Option<ChildCleanupOutcome>> {
    if runtime
        .get_task(&record.task_id)
        .await?
        .is_none_or(|current| current.incarnation_id != record.incarnation_id)
    {
        return Ok(None);
    }
    let metadata: TaskMetadata =
        serde_json::from_value(record.metadata.clone()).context("failed to parse task metadata")?;
    let (retirement, detached) = lifecycle
        .retire(record, &metadata, DetachMode::Remove)
        .await?;
    if !matches!(
        retirement,
        raiko2_runtime::RuntimeMutationOutcome::Applied
            | raiko2_runtime::RuntimeMutationOutcome::AlreadyApplied
    ) {
        return Ok(None);
    }
    let outcome = ChildCleanupOutcome {
        skipped_shared_children: detached.retained.len(),
    };
    runtime
        .remove_task_if_current(&record.lifetime())
        .await
        .with_context(|| format!("failed to remove runtime task {}", record.task_id))?;
    Ok(Some(outcome))
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
        ExpiredTaskCursor, RuntimeCleanupStats, cleanup_expired_root_task, proposal_task_id,
        run_runtime_cleanup_pass,
    };
    use crate::server::state::{
        EngineHandle, EngineQueueTaskState, EngineQueueTaskView, StaticPipelineFactory,
    };
    use crate::server::task_metadata::{
        ProposalTask, RuntimeMetadata, TaskMetadata, TaskRuntimeMetadata, proposal_task_ref,
    };
    use anyhow::{Context, Result};
    use raiko2_engine::{EngineTaskId, EngineTaskKey, ProposalTaskRequest, ProverTaskConfig};
    use raiko2_pipeline::PipelineKey;
    use raiko2_primitives::ProofType;
    use raiko2_queue::{TaskStoreError, decode_task_id, encode_task_id};
    use raiko2_runtime::{
        ProofArtifactRegistration, RunnerStatus, RuntimeManager, TaskRegistration,
    };
    use std::collections::HashSet;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
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
        let remote_task_id = encoded_proposal_task_id(11)?;
        let queued_task_id = encoded_proposal_task_id(13)?;
        let engine = Arc::new(MockEngine::with_queue_tasks(HashSet::from([
            queued_task_id.clone(),
        ])));
        let factory = Arc::new(build_factory(engine));
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
            &encoded_proposal_task_id(12)?,
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
            remote_task_id.clone(),
            TaskRuntimeMetadata {
                provider_request_id: Some("0xremote".to_string()),
                ..TaskRuntimeMetadata::default()
            },
        );
        register_runtime_task_with_metadata(
            runtime.as_ref(),
            "remote-root",
            &remote_metadata,
            RunnerStatus::Running,
            stale,
        )
        .await?;

        let mut orphan_cursor = None;
        let mut terminal_cursor = None;
        let stats = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
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
        queue_task_ids: HashSet<String>,
        queue_task_state: EngineQueueTaskState,
    }

    impl Default for MockEngine {
        fn default() -> Self {
            Self {
                failing_owners: HashSet::new(),
                queue_task_ids: HashSet::new(),
                queue_task_state: EngineQueueTaskState::Succeeded,
            }
        }
    }

    impl MockEngine {
        fn with_failing_owners(failing_owners: HashSet<String>) -> Self {
            Self {
                failing_owners,
                queue_task_ids: HashSet::new(),
                queue_task_state: EngineQueueTaskState::Succeeded,
            }
        }

        fn with_queue_tasks(queue_task_ids: HashSet<String>) -> Self {
            Self {
                queue_task_ids,
                ..Self::default()
            }
        }

        fn with_queue_task_state(
            queue_task_ids: HashSet<String>,
            queue_task_state: EngineQueueTaskState,
        ) -> Self {
            Self {
                queue_task_ids,
                queue_task_state,
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
            let state = self.queue_task_state;
            Box::pin(async move { Ok(present.then_some(EngineQueueTaskView { id, state })) })
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

    #[tokio::test]
    async fn runtime_cleanup_keeps_stale_root_with_active_queue_task() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root("orphaned-active"))?);
        let active_task_id = encoded_proposal_task_id(14)?;
        let engine = Arc::new(MockEngine::with_queue_task_state(
            HashSet::from([active_task_id.clone()]),
            EngineQueueTaskState::Running,
        ));
        let factory = Arc::new(build_factory(engine.clone()));
        let stale = now_ts().saturating_sub(7_201);

        register_runtime_task(
            runtime.as_ref(),
            "active-root",
            &active_task_id,
            RunnerStatus::Running,
            stale,
        )
        .await?;

        let mut orphan_cursor = None;
        let mut terminal_cursor = None;
        let stats = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
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
        let engine = Arc::new(MockEngine::with_queue_tasks(HashSet::from([
            encoded_proposal_ref.clone(),
        ])));
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
        let proof_ref = proposal_task_ref(
            PipelineKey::ShastaRisc0,
            metadata.proposals[0]
                .request
                .as_ref()
                .expect("proposal request"),
        );
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

        let mut orphan_cursor = None;
        let mut terminal_cursor = None;
        let stats = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
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
                removed_roots: 1,
                skipped_shared_children: 0,
                retained_failures: 0,
                orphaned_cancelled: 0,
            }
        );
        assert!(runtime.get_task("expired-root").await?.is_none());
        assert!(runtime.get_task("live-root").await?.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn runtime_cleanup_retains_root_when_projection_detach_fails() -> Result<()> {
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
            &mut orphan_cursor,
            &mut terminal_cursor,
        )
        .await?;

        assert_eq!(
            stats,
            RuntimeCleanupStats {
                scanned: 1,
                expired: 1,
                removed_roots: 0,
                skipped_shared_children: 0,
                retained_failures: 1,
                orphaned_cancelled: 0,
            }
        );
        assert!(runtime.get_task("expired-root").await?.is_some());
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
            &mut orphan_cursor,
            &mut terminal_cursor,
        )
        .await?;
        assert_eq!(
            first,
            RuntimeCleanupStats {
                scanned: 2,
                expired: 2,
                removed_roots: 1,
                skipped_shared_children: 0,
                retained_failures: 1,
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
            &mut orphan_cursor,
            &mut terminal_cursor,
        )
        .await?;
        assert_eq!(
            second,
            RuntimeCleanupStats {
                scanned: 1,
                expired: 1,
                removed_roots: 1,
                skipped_shared_children: 0,
                retained_failures: 0,
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

        assert!(
            cleanup_expired_root_task(
                runtime.as_ref(),
                &crate::server::lifecycle::ProofLifecycle::new(Arc::clone(&runtime), factory),
                &stale,
            )
            .await?
            .is_none()
        );
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
        runtime
            .register_task(TaskRegistration {
                task_id: task_id.to_string(),
                pipeline_key: None,
                route: "risc0/local".parse().expect("parse route"),
                task_kind: "hoodi_batch".to_string(),
                proposal_id: Some(1),
                proof_ids: vec![proposal_task_id.to_string()],
                metadata: serde_json::to_value(metadata_for_task_with_pair(
                    proposal_task_id,
                    network_pair,
                ))?,
                request_fingerprint: None,
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
        status: RunnerStatus,
        updated_at: i64,
    ) -> Result<()> {
        runtime
            .register_task(TaskRegistration {
                task_id: task_id.to_string(),
                pipeline_key: None,
                route: "risc0/local".parse().expect("parse route"),
                task_kind: "hoodi_batch".to_string(),
                proposal_id: Some(1),
                proof_ids: metadata
                    .proposals
                    .iter()
                    .map(|proposal| proposal.task_id.clone())
                    .collect(),
                metadata: serde_json::to_value(metadata)?,
                request_fingerprint: None,
            })
            .await?;
        let mut record = runtime.get_task(task_id).await?.expect("runtime task");
        record.runner_status = status;
        record.updated_at = updated_at;
        runtime.upsert_task(&record).await?;
        Ok(())
    }

    fn metadata_for_task(proposal_task_id: &str) -> TaskMetadata {
        metadata_for_task_with_pair(proposal_task_id, "taiko_dev/ethereum")
    }

    fn metadata_for_task_with_pair(proposal_task_id: &str, network_pair: &str) -> TaskMetadata {
        let request = match decode_task_id::<EngineTaskKey>(proposal_task_id)
            .expect("decode proposal task id")
            .0
        {
            EngineTaskKey::Proposal { request, .. } => request,
            EngineTaskKey::Aggregate { .. } => unreachable!("expected proposal task id"),
        };
        let (network, l1_network) = network_pair
            .split_once('/')
            .unwrap_or((network_pair, "ethereum"));
        TaskMetadata {
            network_pair: network_pair.to_string(),
            network: network.to_string(),
            l1_network: l1_network.to_string(),
            proof_type: ProofType::Risc0,
            requested_proof_type: None,
            prover_type: None,
            execution_mode: None,
            aggregate_requested: false,
            proposals: vec![ProposalTask {
                proposal_id: 1,
                checkpoint: None,
                l1_inclusion_block_number: 0,
                l2_block_numbers: vec![1],
                last_anchor_block_number: 0,
                task_id: proposal_task_id.to_string(),
                request: Some(request),
            }],
            aggregate_task_id: None,
            aggregate_request: None,
            aggregate_input_artifacts: Vec::new(),
            runtime: RuntimeMetadata::default(),
        }
    }

    fn encoded_proposal_task_id(proposal_id: u64) -> Result<String> {
        let request = ProposalTaskRequest {
            proposal_id,
            l2_block_range: None,
            l1_inclusion_block_number: 0,
            last_anchor_block_number: 0,
            checkpoint: None,
            blob_proof_type: None,
            prover: None,
            graffiti: None,
            prover_config: ProverTaskConfig::default(),
        };
        let task_id = proposal_task_id(PipelineKey::ShastaRisc0, request);
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
