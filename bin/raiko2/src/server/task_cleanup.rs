use crate::config::Config;
use crate::server::state::PipelineFactory;
use crate::server::state::{EngineHandle, EngineQueueTaskState};
use crate::server::task_metadata::{TaskMetadata, referenced_proof_artifact_refs};
use crate::server::telemetry;
use anyhow::{Context, Result, anyhow};
use raiko2_engine::{EngineTaskId, EngineTaskKey, ProposalTaskRequest};
use raiko2_pipeline::PipelineKey;
use raiko2_queue::{TaskStoreError, encode_task_id};
use raiko2_runtime::{
    ExpiredTaskCursor, ProofArtifactCursor, RunnerStatus, RuntimeManager, RuntimeTaskRecord,
};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::fs;
use tokio::sync::RwLock;
use tokio::time::MissedTickBehavior;
use tracing::{info, warn};

const RUNTIME_CLEANUP_BATCH_SIZE: usize = 64;
const PROOF_ARTIFACT_CLEANUP_BATCH_SIZE: usize = 64;
const PROOF_ARTIFACT_FILE_DELETION_BATCH_SIZE: usize = 64;
const ORPHANED_RUNTIME_ERROR: &str = "runtime task orphaned: no active local execution";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProofArtifactIdentity {
    network_pair: String,
    proof_ref: String,
}

#[derive(Debug, Default)]
struct LiveProofArtifactRefs {
    refs: HashSet<ProofArtifactIdentity>,
    invalid_metadata_records: usize,
}

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
pub(crate) struct ProofArtifactCleanupStats {
    pub scanned: usize,
    pub removed_rows: usize,
    pub retained_active: usize,
    pub retained_changed: usize,
    pub files_removed: usize,
    pub files_missing: usize,
    pub file_delete_failures: usize,
    pub record_delete_failures: usize,
    pub invalid_metadata_records: usize,
    pub retained_invalid_metadata: usize,
}

impl ProofArtifactCleanupStats {
    const fn is_idle(self) -> bool {
        self.scanned == 0
            && self.removed_rows == 0
            && self.retained_active == 0
            && self.retained_changed == 0
            && self.files_removed == 0
            && self.files_missing == 0
            && self.file_delete_failures == 0
            && self.record_delete_failures == 0
            && self.invalid_metadata_records == 0
            && self.retained_invalid_metadata == 0
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProofArtifactFileDeletionStats {
    pub scanned: usize,
    pub files_removed: usize,
    pub files_missing: usize,
    pub skipped_republished: usize,
    pub failures: usize,
}

impl ProofArtifactFileDeletionStats {
    const fn is_idle(self) -> bool {
        self.scanned == 0
            && self.files_removed == 0
            && self.files_missing == 0
            && self.skipped_republished == 0
            && self.failures == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProofArtifactFileDeletionOutcome {
    Removed,
    Missing,
    SkippedRepublished,
    Failed,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChildCleanupOutcome {
    pub skipped_shared_children: usize,
}

pub(crate) fn spawn_runtime_cleanup_loop(
    config: Arc<Config>,
    runtime: Arc<RuntimeManager>,
    pipelines: Arc<dyn PipelineFactory>,
    artifact_cleanup_guard: Arc<RwLock<()>>,
) {
    tokio::spawn(async move {
        let mut artifact_cursor = None;
        let mut orphan_cursor = None;
        let mut terminal_cursor = None;
        let interval_duration = Duration::from_millis(config.queue.maintenance_interval_ms);
        run_runtime_maintenance_tick(
            config.as_ref(),
            Arc::clone(&runtime),
            Arc::clone(&pipelines),
            Arc::clone(&artifact_cleanup_guard),
            now_ts(),
            &mut artifact_cursor,
            &mut orphan_cursor,
            &mut terminal_cursor,
        )
        .await;

        let mut interval = tokio::time::interval(interval_duration);
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        interval.tick().await;

        loop {
            interval.tick().await;
            run_runtime_maintenance_tick(
                config.as_ref(),
                Arc::clone(&runtime),
                Arc::clone(&pipelines),
                Arc::clone(&artifact_cleanup_guard),
                now_ts(),
                &mut artifact_cursor,
                &mut orphan_cursor,
                &mut terminal_cursor,
            )
            .await;
        }
    });
}

#[allow(clippy::too_many_arguments)]
async fn run_runtime_maintenance_tick(
    config: &Config,
    runtime: Arc<RuntimeManager>,
    pipelines: Arc<dyn PipelineFactory>,
    artifact_cleanup_guard: Arc<RwLock<()>>,
    artifact_now_ts: i64,
    artifact_cursor: &mut Option<ProofArtifactCursor>,
    orphan_cursor: &mut Option<ExpiredTaskCursor>,
    terminal_cursor: &mut Option<ExpiredTaskCursor>,
) {
    if config.runtime.inactive_ttl_secs != 0 {
        log_runtime_cleanup_stats(
            run_runtime_cleanup_pass(
                Arc::clone(&runtime),
                pipelines,
                config.runtime.inactive_ttl_secs,
                orphan_cursor,
                terminal_cursor,
            )
            .await,
        );
    }
    if config.runtime.proof_artifact_ttl_secs != 0 {
        log_proof_artifact_cleanup_stats(
            run_proof_artifact_cleanup_pass(
                Arc::clone(&runtime),
                Arc::clone(&artifact_cleanup_guard),
                artifact_now_ts,
                config.runtime.proof_artifact_ttl_secs,
                artifact_cursor,
            )
            .await,
        );
    }
    log_proof_artifact_file_deletion_stats(
        run_proof_artifact_file_deletion_pass(
            runtime,
            artifact_cleanup_guard,
            PROOF_ARTIFACT_FILE_DELETION_BATCH_SIZE,
        )
        .await,
    );
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

    let orphaned_cancelled = cancel_orphaned_runtime_tasks(
        runtime.as_ref(),
        pipelines.as_ref(),
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
        match cleanup_expired_root_task(runtime.as_ref(), pipelines.as_ref(), &record).await {
            Ok(outcome) => {
                stats.removed_roots += 1;
                stats.skipped_shared_children += outcome.skipped_shared_children;
            }
            Err(err) => {
                stats.retained_failures += 1;
                warn!(task_id = %record.task_id, error = %err, "failed to cleanup expired runtime task");
            }
        }
    }

    Ok(stats)
}

pub(crate) async fn run_proof_artifact_cleanup_pass(
    runtime: Arc<RuntimeManager>,
    artifact_cleanup_guard: Arc<RwLock<()>>,
    now_ts: i64,
    ttl_secs: u64,
    cursor: &mut Option<ProofArtifactCursor>,
) -> Result<ProofArtifactCleanupStats> {
    if ttl_secs == 0 {
        return Ok(ProofArtifactCleanupStats::default());
    }

    let artifacts = runtime
        .list_expired_proof_artifacts(
            now_ts,
            ttl_secs,
            cursor.as_ref(),
            PROOF_ARTIFACT_CLEANUP_BATCH_SIZE,
        )
        .await?;
    if artifacts.is_empty() {
        *cursor = None;
        return Ok(ProofArtifactCleanupStats::default());
    }
    let _guard = artifact_cleanup_guard.write().await;
    let live = live_proof_artifact_refs(runtime.as_ref()).await?;
    if live.invalid_metadata_records != 0 {
        telemetry::record_artifact_cleanup_invalid_metadata(live.invalid_metadata_records as u64);
        return Ok(ProofArtifactCleanupStats {
            scanned: artifacts.len(),
            invalid_metadata_records: live.invalid_metadata_records,
            retained_invalid_metadata: artifacts.len(),
            ..ProofArtifactCleanupStats::default()
        });
    }
    *cursor = artifacts.last().map(ProofArtifactCursor::from);
    let mut stats = ProofArtifactCleanupStats {
        scanned: artifacts.len(),
        ..ProofArtifactCleanupStats::default()
    };

    for artifact in artifacts {
        let identity = ProofArtifactIdentity {
            network_pair: artifact.network_pair.clone(),
            proof_ref: artifact.proof_ref.clone(),
        };
        if live.refs.contains(&identity) {
            stats.retained_active += 1;
            continue;
        }
        let proof_path = match runtime
            .queue_proof_artifact_file_deletion_if_unchanged(
                &artifact.network_pair,
                &artifact.proof_ref,
                artifact.updated_at,
            )
            .await
        {
            Ok(None) => {
                stats.retained_changed += 1;
                continue;
            }
            Ok(Some(proof_path)) => {
                stats.removed_rows += 1;
                proof_path
            }
            Err(err) => {
                stats.record_delete_failures += 1;
                warn!(
                    network_pair = artifact.network_pair,
                    proof_ref = artifact.proof_ref,
                    error = %err,
                    "failed to queue expired proof artifact file deletion"
                );
                continue;
            }
        };
        match process_proof_artifact_file_deletion(runtime.as_ref(), &proof_path).await? {
            ProofArtifactFileDeletionOutcome::Removed => stats.files_removed += 1,
            ProofArtifactFileDeletionOutcome::Missing => stats.files_missing += 1,
            ProofArtifactFileDeletionOutcome::SkippedRepublished => stats.retained_changed += 1,
            ProofArtifactFileDeletionOutcome::Failed => {
                stats.file_delete_failures += 1;
            }
        }
    }

    Ok(stats)
}

pub(crate) async fn run_proof_artifact_file_deletion_pass(
    runtime: Arc<RuntimeManager>,
    artifact_cleanup_guard: Arc<RwLock<()>>,
    limit: usize,
) -> Result<ProofArtifactFileDeletionStats> {
    let _guard = artifact_cleanup_guard.write().await;
    let deletions = runtime.list_proof_artifact_file_deletions(limit).await?;
    let mut stats = ProofArtifactFileDeletionStats {
        scanned: deletions.len(),
        ..ProofArtifactFileDeletionStats::default()
    };
    for deletion in deletions {
        match process_proof_artifact_file_deletion(runtime.as_ref(), &deletion.proof_path).await? {
            ProofArtifactFileDeletionOutcome::Removed => stats.files_removed += 1,
            ProofArtifactFileDeletionOutcome::Missing => stats.files_missing += 1,
            ProofArtifactFileDeletionOutcome::SkippedRepublished => {
                stats.skipped_republished += 1;
            }
            ProofArtifactFileDeletionOutcome::Failed => stats.failures += 1,
        }
    }
    Ok(stats)
}

async fn process_proof_artifact_file_deletion(
    runtime: &RuntimeManager,
    proof_path: &str,
) -> Result<ProofArtifactFileDeletionOutcome> {
    if runtime
        .proof_artifact_path_is_registered(proof_path)
        .await?
    {
        runtime
            .remove_proof_artifact_file_deletion(proof_path)
            .await?;
        return Ok(ProofArtifactFileDeletionOutcome::SkippedRepublished);
    }
    match fs::remove_file(proof_path).await {
        Ok(()) => {
            runtime
                .remove_proof_artifact_file_deletion(proof_path)
                .await?;
            Ok(ProofArtifactFileDeletionOutcome::Removed)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            runtime
                .remove_proof_artifact_file_deletion(proof_path)
                .await?;
            Ok(ProofArtifactFileDeletionOutcome::Missing)
        }
        Err(err) => {
            runtime
                .record_proof_artifact_file_deletion_failure(proof_path, &err.to_string())
                .await?;
            warn!(proof_path, error = %err, "failed to remove queued proof artifact file");
            Ok(ProofArtifactFileDeletionOutcome::Failed)
        }
    }
}

async fn live_proof_artifact_refs(runtime: &RuntimeManager) -> Result<LiveProofArtifactRefs> {
    let mut live = LiveProofArtifactRefs::default();
    for record in runtime.list_active_tasks().await? {
        let metadata: TaskMetadata = match serde_json::from_value(record.metadata.clone()) {
            Ok(metadata) => metadata,
            Err(err) => {
                live.invalid_metadata_records += 1;
                warn!(
                    task_id = %record.task_id,
                    error = %err,
                    "failed to parse active task metadata during proof artifact cleanup"
                );
                continue;
            }
        };
        live.refs.extend(
            referenced_proof_artifact_refs(&metadata, record.pipeline_key)
                .into_iter()
                .map(|proof_ref| ProofArtifactIdentity {
                    network_pair: metadata.network_pair.clone(),
                    proof_ref,
                }),
        );
    }
    Ok(live)
}

async fn cancel_orphaned_runtime_tasks(
    runtime: &RuntimeManager,
    pipelines: &dyn PipelineFactory,
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

        if metadata.has_remote_submission_progress() {
            continue;
        }

        let Some(engine) = pipelines.get(&metadata.network_pair, record.pipeline_key) else {
            continue;
        };
        if has_active_queue_task(
            engine.as_ref(),
            &metadata_queue_task_ids(&metadata, record.pipeline_key),
        )
        .await?
        {
            continue;
        }

        let cancelled_stale = runtime
            .cancel_nonterminal_task_if_stale(
                &record.task_id,
                record.updated_at,
                Some(ORPHANED_RUNTIME_ERROR.to_string()),
            )
            .await
            .with_context(|| format!("failed cancel orphaned runtime task {}", record.task_id))?;
        if !cancelled_stale {
            continue;
        }

        cancelled += 1;
        warn!(task_id = %record.task_id, "cancelled orphaned runtime task");

        if let Err(err) = cancel_registered_tasks(
            runtime,
            &engine,
            &record.task_id,
            record.pipeline_key,
            &metadata,
        )
        .await
        {
            warn!(
                task_id = %record.task_id,
                error = %err,
                "failed best-effort queue cleanup for orphaned runtime task"
            );
        }
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

pub(crate) async fn cancel_registered_tasks(
    runtime: &RuntimeManager,
    engine: &Arc<dyn EngineHandle>,
    public_task_id: &str,
    pipeline_key: PipelineKey,
    metadata: &TaskMetadata,
) -> Result<()> {
    let mut errors = Vec::new();

    for proposal in &metadata.proposals {
        if has_other_live_task_reference(
            runtime,
            public_task_id,
            &proposal.task_id,
            &metadata.network_pair,
        )
        .await?
        {
            continue;
        }
        let Some(task_id) = proposal.engine_task_id(pipeline_key) else {
            continue;
        };
        for stage_task_id in proposal_task_chain_ids(&task_id) {
            if let Err(err) = engine.cancel(stage_task_id.clone()).await {
                let encoded = encode_task_id(&stage_task_id)
                    .unwrap_or_else(|_| "<invalid-task-id>".to_string());
                errors.push(format!("{encoded}: {err}"));
            }
        }
    }

    if let Some(task_id) = &metadata.aggregate_task_id
        && !has_other_live_task_reference(runtime, public_task_id, task_id, &metadata.network_pair)
            .await?
    {
        let Some(task_id) = metadata.aggregate_engine_task_id(pipeline_key) else {
            return if errors.is_empty() {
                Ok(())
            } else {
                Err(anyhow!(
                    "failed to cancel one or more child tasks: {}",
                    errors.join("; ")
                ))
            };
        };
        if let Err(err) = engine.cancel(task_id.clone()).await {
            let encoded =
                encode_task_id(&task_id).unwrap_or_else(|_| "<invalid-task-id>".to_string());
            errors.push(format!("{encoded}: {err}"));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(anyhow!(
            "failed to cancel one or more child tasks: {}",
            errors.join("; ")
        ))
    }
}

pub(crate) async fn has_other_task_reference(
    runtime: &RuntimeManager,
    public_task_id: &str,
    engine_task_id: &str,
    network_pair: &str,
) -> Result<bool> {
    let records = runtime
        .find_tasks_by_task_ref(engine_task_id)
        .await
        .with_context(|| {
            format!("failed to inspect runtime task references for {engine_task_id}")
        })?;
    for record in records {
        if record.task_id != public_task_id && record_matches_network_pair(&record, network_pair)? {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) async fn has_other_live_task_reference(
    runtime: &RuntimeManager,
    public_task_id: &str,
    engine_task_id: &str,
    network_pair: &str,
) -> Result<bool> {
    let records = runtime
        .find_tasks_by_task_ref(engine_task_id)
        .await
        .with_context(|| {
            format!("failed to inspect runtime task references for {engine_task_id}")
        })?;
    for record in records {
        if record.task_id != public_task_id
            && matches!(
                record.runner_status,
                RunnerStatus::Allocated | RunnerStatus::Running
            )
            && record_matches_network_pair(&record, network_pair)?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) async fn remove_task_children(
    engine: &Arc<dyn EngineHandle>,
    pipeline_key: PipelineKey,
    metadata: &TaskMetadata,
    removed_engine_task_ids: &mut HashSet<String>,
) -> Result<()> {
    for proposal in &metadata.proposals {
        let Some(task_id) = proposal.engine_task_id(pipeline_key) else {
            continue;
        };
        for stage_task_id in proposal_task_chain_ids(&task_id) {
            let encoded = encode_task_id(&stage_task_id)
                .context("failed to encode proposal stage task id")?;
            if removed_engine_task_ids.insert(encoded) {
                engine
                    .remove(stage_task_id)
                    .await
                    .map_err(|err| task_store_error_to_anyhow(&err))?;
            }
        }
    }

    if let Some(task_id) = metadata.aggregate_engine_task_id(pipeline_key) {
        let encoded = encode_task_id(&task_id).context("failed to encode aggregate task id")?;
        if removed_engine_task_ids.insert(encoded) {
            engine
                .remove(task_id)
                .await
                .map_err(|err| task_store_error_to_anyhow(&err))?;
        }
    }

    Ok(())
}

pub(crate) async fn remove_task_children_if_unreferenced(
    runtime: &RuntimeManager,
    engine: &Arc<dyn EngineHandle>,
    public_task_id: &str,
    pipeline_key: PipelineKey,
    metadata: &TaskMetadata,
) -> Result<ChildCleanupOutcome> {
    let mut outcome = ChildCleanupOutcome::default();

    for proposal in &metadata.proposals {
        let Some(task_id) = proposal.engine_task_id(pipeline_key) else {
            continue;
        };
        let stage_task_ids = proposal_task_chain_ids(&task_id);
        if has_other_task_reference(
            runtime,
            public_task_id,
            &proposal.task_id,
            &metadata.network_pair,
        )
        .await?
        {
            outcome.skipped_shared_children += stage_task_ids.len();
            continue;
        }
        for stage_task_id in stage_task_ids {
            engine
                .remove(stage_task_id)
                .await
                .map_err(|err| task_store_error_to_anyhow(&err))?;
        }
    }

    if let Some(task_id) = &metadata.aggregate_task_id {
        if has_other_task_reference(runtime, public_task_id, task_id, &metadata.network_pair)
            .await?
        {
            outcome.skipped_shared_children += 1;
        } else if let Some(task_id) = metadata.aggregate_engine_task_id(pipeline_key) {
            engine
                .remove(task_id)
                .await
                .map_err(|err| task_store_error_to_anyhow(&err))?;
        }
    }

    Ok(outcome)
}

fn record_matches_network_pair(record: &RuntimeTaskRecord, network_pair: &str) -> Result<bool> {
    let metadata: TaskMetadata = serde_json::from_value(record.metadata.clone())
        .context("failed to parse referenced task metadata")?;
    Ok(metadata.network_pair == network_pair)
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

fn log_proof_artifact_cleanup_stats(result: Result<ProofArtifactCleanupStats>) {
    match result {
        Ok(stats) if !stats.is_idle() => info!(
            scanned = stats.scanned,
            removed_rows = stats.removed_rows,
            retained_active = stats.retained_active,
            retained_changed = stats.retained_changed,
            files_removed = stats.files_removed,
            files_missing = stats.files_missing,
            file_delete_failures = stats.file_delete_failures,
            record_delete_failures = stats.record_delete_failures,
            invalid_metadata_records = stats.invalid_metadata_records,
            retained_invalid_metadata = stats.retained_invalid_metadata,
            "proof artifact cleanup tick completed"
        ),
        Ok(_) => {}
        Err(err) => warn!(error = %err, "proof artifact cleanup tick failed"),
    }
}

fn log_proof_artifact_file_deletion_stats(result: Result<ProofArtifactFileDeletionStats>) {
    match result {
        Ok(stats) if !stats.is_idle() => info!(
            scanned = stats.scanned,
            files_removed = stats.files_removed,
            files_missing = stats.files_missing,
            skipped_republished = stats.skipped_republished,
            failures = stats.failures,
            "proof artifact file deletion tick completed"
        ),
        Ok(_) => {}
        Err(err) => warn!(error = %err, "proof artifact file deletion tick failed"),
    }
}

async fn cleanup_expired_root_task(
    runtime: &RuntimeManager,
    pipelines: &dyn PipelineFactory,
    record: &RuntimeTaskRecord,
) -> Result<ChildCleanupOutcome> {
    let metadata: TaskMetadata =
        serde_json::from_value(record.metadata.clone()).context("failed to parse task metadata")?;
    let engine = pipelines
        .get(&metadata.network_pair, record.pipeline_key)
        .ok_or_else(|| anyhow!("pipeline not available: {}", record.pipeline_key.as_str()))?;
    let outcome = remove_task_children_if_unreferenced(
        runtime,
        &engine,
        &record.task_id,
        record.pipeline_key,
        &metadata,
    )
    .await?;
    runtime
        .remove_task(&record.task_id)
        .await
        .with_context(|| format!("failed to remove runtime task {}", record.task_id))?;
    Ok(outcome)
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

fn task_store_error_to_anyhow(err: &TaskStoreError) -> anyhow::Error {
    anyhow!("failed to remove task: {err}")
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
        ExpiredTaskCursor, PROOF_ARTIFACT_CLEANUP_BATCH_SIZE, ProofArtifactCleanupStats,
        ProofArtifactFileDeletionStats, RuntimeCleanupStats, cancel_registered_tasks,
        proposal_task_chain_ids, proposal_task_id, run_proof_artifact_cleanup_pass,
        run_runtime_cleanup_pass, run_runtime_maintenance_tick,
    };
    use crate::config::Config;
    use crate::server::state::{
        AppState, EngineHandle, EngineQueueTaskState, EngineQueueTaskView, StaticPipelineFactory,
    };
    use crate::server::task_metadata::{
        ProposalTask, RuntimeMetadata, TaskMetadata, TaskRuntimeMetadata,
    };
    use anyhow::{Context, Result};
    use raiko2_engine::{
        AggregateProofInput, AggregationTaskRequest, EngineTaskId, EngineTaskKey,
        ProposalTaskRequest, ProverTaskConfig,
    };
    use raiko2_pipeline::PipelineKey;
    use raiko2_primitives::ProofType;
    use raiko2_queue::{TaskStoreError, decode_task_id, encode_task_id};
    use raiko2_runtime::{
        ProofArtifactCursor, ProofArtifactRecord, ProofArtifactRegistration, RunnerStatus,
        RuntimeManager, TaskRegistration,
    };
    use std::collections::HashSet;
    use std::future::Future;
    use std::path::Path;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use tokio::sync::RwLock;

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

    #[test]
    fn artifact_cleanup_stats_report_idle_when_zero() {
        assert!(ProofArtifactCleanupStats::default().is_idle());
        assert!(
            !ProofArtifactCleanupStats {
                scanned: 1,
                ..ProofArtifactCleanupStats::default()
            }
            .is_idle()
        );
    }

    #[test]
    fn artifact_file_deletion_stats_report_idle_when_zero() {
        assert!(ProofArtifactFileDeletionStats::default().is_idle());
        assert!(
            !ProofArtifactFileDeletionStats {
                scanned: 1,
                ..ProofArtifactFileDeletionStats::default()
            }
            .is_idle()
        );
    }

    #[test]
    fn app_state_shares_artifact_cleanup_guard_across_clones() -> Result<()> {
        let state = AppState::from_parts(
            Arc::new(Config::default()),
            Arc::new(StaticPipelineFactory::default()),
            Arc::new(RuntimeManager::new(unique_runtime_root("artifact-guard"))?),
        );
        let cloned = state.clone();

        assert!(Arc::ptr_eq(
            &state.artifact_cleanup_guard,
            &cloned.artifact_cleanup_guard
        ));
        Ok(())
    }

    #[tokio::test]
    async fn maintenance_tick_runs_artifact_cleanup_when_root_cleanup_disabled() -> Result<()> {
        let mut config = Config::default();
        config.runtime.inactive_ttl_secs = 0;
        config.runtime.proof_artifact_ttl_secs = 10;
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "artifact-only-maintenance",
        ))?);
        let artifact = register_artifact(runtime.as_ref(), "artifact-only", true).await?;
        let mut artifact_cursor = None;
        let mut orphan_cursor = None;
        let mut terminal_cursor = None;

        run_runtime_maintenance_tick(
            &config,
            runtime.clone(),
            Arc::new(StaticPipelineFactory::default()),
            Arc::new(RwLock::new(())),
            artifact.updated_at + 10,
            &mut artifact_cursor,
            &mut orphan_cursor,
            &mut terminal_cursor,
        )
        .await;

        assert!(
            runtime
                .get_proof_artifact(&artifact.network_pair, &artifact.proof_ref)
                .await?
                .is_none()
        );
        assert!(!tokio::fs::try_exists(&artifact.proof_path).await?);
        Ok(())
    }

    #[tokio::test]
    async fn artifact_cleanup_removes_expired_row_and_file() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "artifact-expired",
        ))?);
        let artifact = register_artifact(runtime.as_ref(), "expired", true).await?;
        let mut cursor: Option<ProofArtifactCursor> = None;
        let stats = run_proof_artifact_cleanup_pass(
            runtime.clone(),
            Arc::new(RwLock::new(())),
            artifact.updated_at + 10,
            10,
            &mut cursor,
        )
        .await?;
        assert_eq!(stats.removed_rows, 1);
        assert_eq!(stats.files_removed, 1);
        assert!(
            runtime
                .get_proof_artifact(&artifact.network_pair, &artifact.proof_ref)
                .await?
                .is_none()
        );
        assert!(!tokio::fs::try_exists(Path::new(&artifact.proof_path)).await?);
        Ok(())
    }

    #[tokio::test]
    async fn artifact_cleanup_pass_is_bounded() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "artifact-cleanup-bound",
        ))?);
        let mut newest_updated_at = 0;
        for index in 0..=PROOF_ARTIFACT_CLEANUP_BATCH_SIZE {
            let artifact =
                register_artifact(runtime.as_ref(), &format!("bounded-{index}"), false).await?;
            newest_updated_at = newest_updated_at.max(artifact.updated_at);
        }
        let mut cursor = None;

        let stats = run_proof_artifact_cleanup_pass(
            runtime.clone(),
            Arc::new(RwLock::new(())),
            newest_updated_at + 10,
            10,
            &mut cursor,
        )
        .await?;

        assert_eq!(stats.scanned, PROOF_ARTIFACT_CLEANUP_BATCH_SIZE);
        assert_eq!(stats.removed_rows, PROOF_ARTIFACT_CLEANUP_BATCH_SIZE);
        assert_eq!(
            runtime
                .list_expired_proof_artifacts(newest_updated_at + 10, 10, None, 128)
                .await?
                .len(),
            1
        );
        Ok(())
    }

    #[tokio::test]
    async fn artifact_cleanup_waits_for_submission_read_guard() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root("artifact-guard"))?);
        let artifact = register_artifact(runtime.as_ref(), "guarded", true).await?;
        let guard = Arc::new(RwLock::new(()));
        let read_guard = guard.read().await;
        let mut cleanup = tokio::spawn({
            let runtime = runtime.clone();
            let guard = guard.clone();
            async move {
                let mut cursor = None;
                run_proof_artifact_cleanup_pass(
                    runtime,
                    guard,
                    artifact.updated_at + 10,
                    10,
                    &mut cursor,
                )
                .await
            }
        });

        assert!(
            tokio::time::timeout(Duration::from_millis(25), &mut cleanup)
                .await
                .is_err()
        );
        drop(read_guard);
        let stats = cleanup.await??;
        assert_eq!(stats.removed_rows, 1);
        Ok(())
    }

    #[tokio::test]
    async fn artifact_cleanup_retains_nonterminal_references() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root("artifact-active"))?);
        let proposal_ref = encoded_proposal_task_id(44)?;
        let artifact = register_artifact(runtime.as_ref(), &proposal_ref, true).await?;
        register_runtime_task(
            runtime.as_ref(),
            "active-root",
            &proposal_ref,
            RunnerStatus::Running,
            now_ts(),
        )
        .await?;
        let mut cursor = None;
        let stats = run_proof_artifact_cleanup_pass(
            runtime.clone(),
            Arc::new(RwLock::new(())),
            artifact.updated_at + 10,
            10,
            &mut cursor,
        )
        .await?;
        assert_eq!(stats.retained_active, 1);
        assert!(
            runtime
                .get_proof_artifact(&artifact.network_pair, &artifact.proof_ref)
                .await?
                .is_some()
        );
        assert!(tokio::fs::try_exists(&artifact.proof_path).await?);
        Ok(())
    }

    #[tokio::test]
    async fn artifact_cleanup_does_not_retain_terminal_references() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "artifact-terminal",
        ))?);
        let proposal_ref = encoded_proposal_task_id(45)?;
        let artifact = register_artifact(runtime.as_ref(), &proposal_ref, true).await?;
        register_runtime_task(
            runtime.as_ref(),
            "terminal-root",
            &proposal_ref,
            RunnerStatus::Completed,
            now_ts(),
        )
        .await?;
        let mut cursor = None;
        let stats = run_proof_artifact_cleanup_pass(
            runtime.clone(),
            Arc::new(RwLock::new(())),
            artifact.updated_at + 10,
            10,
            &mut cursor,
        )
        .await?;
        assert_eq!(stats.removed_rows, 1);
        assert_eq!(stats.files_removed, 1);
        assert!(runtime.get_task("terminal-root").await?.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn artifact_cleanup_removes_missing_file_rows_and_honors_zero() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "artifact-missing",
        ))?);
        let artifact = register_artifact(runtime.as_ref(), "missing", false).await?;
        let guard = Arc::new(RwLock::new(()));
        let mut cursor = None;
        assert_eq!(
            run_proof_artifact_cleanup_pass(
                runtime.clone(),
                guard.clone(),
                artifact.updated_at + 10,
                0,
                &mut cursor,
            )
            .await?,
            ProofArtifactCleanupStats::default()
        );
        let stats = run_proof_artifact_cleanup_pass(
            runtime.clone(),
            guard,
            artifact.updated_at + 10,
            10,
            &mut cursor,
        )
        .await?;
        assert_eq!(stats.removed_rows, 1);
        assert_eq!(stats.files_missing, 1);
        Ok(())
    }

    #[tokio::test]
    async fn artifact_file_deletion_is_retried_after_cleanup_failure() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "artifact-file-failure",
        ))?);
        let artifact = register_artifact(runtime.as_ref(), "delete-fails", false).await?;
        tokio::fs::create_dir_all(&artifact.proof_path).await?;
        let guard = Arc::new(RwLock::new(()));
        let mut cursor = None;
        let stats = run_proof_artifact_cleanup_pass(
            runtime.clone(),
            guard.clone(),
            artifact.updated_at + 10,
            10,
            &mut cursor,
        )
        .await?;
        assert_eq!(stats.removed_rows, 1);
        assert_eq!(stats.file_delete_failures, 1);
        assert!(
            runtime
                .get_proof_artifact(&artifact.network_pair, &artifact.proof_ref)
                .await?
                .is_none()
        );
        let pending = runtime.list_proof_artifact_file_deletions(64).await?;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].proof_path, artifact.proof_path);

        tokio::fs::remove_dir(&artifact.proof_path).await?;
        tokio::fs::write(&artifact.proof_path, b"retry").await?;
        let mut config = Config::default();
        config.runtime.inactive_ttl_secs = 0;
        config.runtime.proof_artifact_ttl_secs = 0;
        let mut orphan_cursor = None;
        let mut terminal_cursor = None;
        run_runtime_maintenance_tick(
            &config,
            runtime.clone(),
            Arc::new(StaticPipelineFactory::default()),
            guard,
            artifact.updated_at + 20,
            &mut cursor,
            &mut orphan_cursor,
            &mut terminal_cursor,
        )
        .await;

        assert!(
            runtime
                .list_proof_artifact_file_deletions(64)
                .await?
                .is_empty()
        );
        assert!(!tokio::fs::try_exists(&artifact.proof_path).await?);
        Ok(())
    }

    #[tokio::test]
    async fn artifact_file_deletion_skips_republished_path() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "artifact-file-republished",
        ))?);
        let artifact = register_artifact(runtime.as_ref(), "republished", true).await?;
        runtime
            .queue_proof_artifact_file_deletion_if_unchanged(
                &artifact.network_pair,
                &artifact.proof_ref,
                artifact.updated_at,
            )
            .await?
            .context("queued old artifact deletion")?;
        let current = register_artifact(runtime.as_ref(), "republished", true).await?;
        let mut config = Config::default();
        config.runtime.inactive_ttl_secs = 0;
        config.runtime.proof_artifact_ttl_secs = 0;
        let mut artifact_cursor = None;
        let mut orphan_cursor = None;
        let mut terminal_cursor = None;

        run_runtime_maintenance_tick(
            &config,
            runtime.clone(),
            Arc::new(StaticPipelineFactory::default()),
            Arc::new(RwLock::new(())),
            current.updated_at,
            &mut artifact_cursor,
            &mut orphan_cursor,
            &mut terminal_cursor,
        )
        .await;

        assert!(
            runtime
                .get_proof_artifact(&current.network_pair, &current.proof_ref)
                .await?
                .is_some()
        );
        assert!(tokio::fs::try_exists(&current.proof_path).await?);
        assert!(
            runtime
                .list_proof_artifact_file_deletions(64)
                .await?
                .is_empty()
        );
        Ok(())
    }

    #[tokio::test]
    async fn artifact_file_deletion_pass_is_bounded() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "artifact-file-deletion-bound",
        ))?);
        let mut paths = Vec::new();
        for index in 0..=PROOF_ARTIFACT_CLEANUP_BATCH_SIZE {
            let artifact =
                register_artifact(runtime.as_ref(), &format!("bounded-{index}"), true).await?;
            runtime
                .queue_proof_artifact_file_deletion_if_unchanged(
                    &artifact.network_pair,
                    &artifact.proof_ref,
                    artifact.updated_at,
                )
                .await?
                .context("queued bounded artifact deletion")?;
            paths.push(artifact.proof_path);
        }
        let mut config = Config::default();
        config.runtime.inactive_ttl_secs = 0;
        config.runtime.proof_artifact_ttl_secs = 0;
        let mut artifact_cursor = None;
        let mut orphan_cursor = None;
        let mut terminal_cursor = None;

        run_runtime_maintenance_tick(
            &config,
            runtime.clone(),
            Arc::new(StaticPipelineFactory::default()),
            Arc::new(RwLock::new(())),
            now_ts(),
            &mut artifact_cursor,
            &mut orphan_cursor,
            &mut terminal_cursor,
        )
        .await;

        assert_eq!(
            runtime.list_proof_artifact_file_deletions(64).await?.len(),
            1
        );
        let mut surviving_files = 0;
        for path in paths {
            surviving_files += usize::from(tokio::fs::try_exists(path).await?);
        }
        assert_eq!(surviving_files, 1);
        Ok(())
    }

    #[tokio::test]
    async fn artifact_cleanup_fails_closed_on_invalid_live_metadata() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "artifact-invalid-metadata",
        ))?);
        let artifacts = [
            register_artifact(runtime.as_ref(), "retained-first", true).await?,
            register_artifact(runtime.as_ref(), "retained-second", true).await?,
        ];
        runtime
            .register_task(TaskRegistration {
                task_id: "invalid-live-root".to_string(),
                pipeline_key: Some(PipelineKey::ShastaRisc0),
                route: "risc0/local".parse().expect("parse route"),
                task_kind: "hoodi_batch".to_string(),
                proposal_id: None,
                proof_ids: Vec::new(),
                metadata: serde_json::json!({"invalid": true}),
                request_fingerprint: None,
            })
            .await?;
        let mut cursor = None;
        for _ in 0..2 {
            let stats = run_proof_artifact_cleanup_pass(
                runtime.clone(),
                Arc::new(RwLock::new(())),
                artifacts[0].updated_at + 10,
                10,
                &mut cursor,
            )
            .await?;
            assert_eq!(stats.scanned, artifacts.len());
            assert_eq!(stats.invalid_metadata_records, 1);
            assert_eq!(stats.retained_invalid_metadata, artifacts.len());
            assert!(cursor.is_none());
            for artifact in &artifacts {
                assert!(
                    runtime
                        .get_proof_artifact(&artifact.network_pair, &artifact.proof_ref)
                        .await?
                        .is_some()
                );
                assert!(tokio::fs::try_exists(&artifact.proof_path).await?);
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn artifact_cleanup_idle_page_skips_invalid_live_metadata() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "artifact-idle-invalid-metadata",
        ))?);
        runtime
            .register_task(TaskRegistration {
                task_id: "invalid-live-root".to_string(),
                pipeline_key: Some(PipelineKey::ShastaRisc0),
                route: "risc0/local".parse().expect("parse route"),
                task_kind: "hoodi_batch".to_string(),
                proposal_id: None,
                proof_ids: Vec::new(),
                metadata: serde_json::json!({"invalid": true}),
                request_fingerprint: None,
            })
            .await?;
        let mut cursor = Some(ProofArtifactCursor {
            updated_at: 1,
            network_pair: "taiko_dev/ethereum".to_string(),
            proof_ref: "previous-page".to_string(),
        });

        let guard = Arc::new(RwLock::new(()));
        let _read_guard = guard.read().await;
        let stats = tokio::time::timeout(
            Duration::from_millis(25),
            run_proof_artifact_cleanup_pass(runtime, guard.clone(), now_ts(), 10, &mut cursor),
        )
        .await
        .context("empty artifact page should not wait for the write guard")??;

        assert_eq!(stats, ProofArtifactCleanupStats::default());
        assert!(cursor.is_none());
        Ok(())
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
        removed: Mutex<Vec<String>>,
        cancelled: Mutex<Vec<String>>,
        fail_on: HashSet<String>,
        queue_task_ids: HashSet<String>,
        queue_task_state: EngineQueueTaskState,
    }

    impl Default for MockEngine {
        fn default() -> Self {
            Self {
                removed: Mutex::new(Vec::new()),
                cancelled: Mutex::new(Vec::new()),
                fail_on: HashSet::new(),
                queue_task_ids: HashSet::new(),
                queue_task_state: EngineQueueTaskState::Succeeded,
            }
        }
    }

    impl MockEngine {
        fn new(fail_on: HashSet<String>) -> Self {
            Self {
                removed: Mutex::new(Vec::new()),
                cancelled: Mutex::new(Vec::new()),
                fail_on,
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

        fn removed(&self) -> Vec<String> {
            self.removed.lock().expect("removed lock").clone()
        }

        fn cancelled(&self) -> Vec<String> {
            self.cancelled.lock().expect("cancelled lock").clone()
        }
    }

    impl EngineHandle for MockEngine {
        fn submit_proposal_proof_with_dependencies(
            &self,
            _request: ProposalTaskRequest,
            _dependencies: Vec<EngineTaskId>,
        ) -> BoxFuture<'_, Result<EngineTaskId, TaskStoreError>> {
            Box::pin(async { panic!("unexpected proposal submission") })
        }

        fn submit_aggregation_proof_from_inputs(
            &self,
            _request: AggregationTaskRequest,
            _inputs: Vec<AggregateProofInput>,
        ) -> BoxFuture<'_, Result<EngineTaskId, TaskStoreError>> {
            Box::pin(async { panic!("unexpected aggregation submission from inputs") })
        }

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

        fn cancel(&self, id: EngineTaskId) -> BoxFuture<'_, Result<(), TaskStoreError>> {
            let encoded = encode_task_id(&id).expect("encode task id");
            let cancelled = &self.cancelled;
            Box::pin(async move {
                cancelled.lock().expect("cancelled lock").push(encoded);
                Ok(())
            })
        }

        fn remove(&self, id: EngineTaskId) -> BoxFuture<'_, Result<(), TaskStoreError>> {
            let encoded = encode_task_id(&id).expect("encode task id");
            let should_fail = self.fail_on.contains(&encoded);
            let removed = &self.removed;
            Box::pin(async move {
                if should_fail {
                    Err(TaskStoreError::backend(std::io::Error::other(
                        "mock remove failure",
                    )))
                } else {
                    removed.lock().expect("removed lock").push(encoded);
                    Ok(())
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
        assert!(engine.cancelled().is_empty());
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
                skipped_shared_children: 1,
                retained_failures: 0,
                orphaned_cancelled: 0,
            }
        );
        assert!(runtime.get_task("expired-root").await?.is_none());
        assert!(runtime.get_task("live-root").await?.is_some());
        assert!(engine.removed().is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn runtime_cleanup_retains_root_when_child_removal_fails() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root("ttl-failure"))?);
        let failing_stage = first_stage_task_id(9)?;
        let engine = Arc::new(MockEngine::new(HashSet::from([failing_stage.clone()])));
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
        let failing_stage = first_stage_task_id(1)?;
        let engine = Arc::new(MockEngine::new(HashSet::from([failing_stage])));
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
    async fn cancel_registered_tasks_ignores_terminal_roots_for_live_shared_children() -> Result<()>
    {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root("cancel-shared"))?);
        let engine = Arc::new(MockEngine::default());
        let engine_handle: Arc<dyn EngineHandle> = engine.clone();
        let proposal_task_id = encoded_proposal_task_id(4)?;

        register_runtime_task(
            runtime.as_ref(),
            "terminal-root",
            &proposal_task_id,
            RunnerStatus::Completed,
            now_ts(),
        )
        .await?;
        register_runtime_task(
            runtime.as_ref(),
            "live-root",
            &proposal_task_id,
            RunnerStatus::Running,
            now_ts(),
        )
        .await?;

        cancel_registered_tasks(
            runtime.as_ref(),
            &engine_handle,
            "live-root",
            PipelineKey::ShastaRisc0,
            &metadata_for_task(&proposal_task_id),
        )
        .await?;

        let prove_task_id =
            decode_task_id::<EngineTaskKey>(&proposal_task_id).expect("decode prove task id");
        let expected = proposal_task_chain_ids(&prove_task_id)
            .into_iter()
            .map(|task_id| encode_task_id(&task_id).expect("encode stage task id"))
            .collect::<Vec<_>>();
        assert_eq!(engine.cancelled(), expected);
        Ok(())
    }

    #[tokio::test]
    async fn cancel_registered_tasks_does_not_treat_other_pair_as_shared_child() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "cancel-cross-pair",
        ))?);
        let engine = Arc::new(MockEngine::default());
        let engine_handle: Arc<dyn EngineHandle> = engine.clone();
        let proposal_task_id = encoded_proposal_task_id(5)?;

        register_runtime_task_with_pair(
            runtime.as_ref(),
            "other-pair-root",
            &proposal_task_id,
            RunnerStatus::Running,
            now_ts(),
            "taiko_alt/ethereum",
        )
        .await?;

        cancel_registered_tasks(
            runtime.as_ref(),
            &engine_handle,
            "current-root",
            PipelineKey::ShastaRisc0,
            &metadata_for_task_with_pair(&proposal_task_id, "taiko_dev/ethereum"),
        )
        .await?;

        let prove_task_id =
            decode_task_id::<EngineTaskKey>(&proposal_task_id).expect("decode prove task id");
        let expected = proposal_task_chain_ids(&prove_task_id)
            .into_iter()
            .map(|task_id| encode_task_id(&task_id).expect("encode stage task id"))
            .collect::<Vec<_>>();
        assert_eq!(engine.cancelled(), expected);
        Ok(())
    }

    fn build_factory(engine: Arc<dyn EngineHandle>) -> StaticPipelineFactory {
        let mut factory = StaticPipelineFactory::default();
        factory.insert("taiko_dev/ethereum", PipelineKey::ShastaRisc0, engine);
        factory
    }

    async fn register_artifact(
        runtime: &RuntimeManager,
        proof_ref: &str,
        file: bool,
    ) -> Result<ProofArtifactRecord> {
        let proof_path = runtime.proof_artifact_path("taiko_dev/ethereum", proof_ref);
        if file {
            tokio::fs::create_dir_all(proof_path.parent().context("artifact parent")?).await?;
            tokio::fs::write(&proof_path, b"{}").await?;
        }
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: "taiko_dev/ethereum".to_string(),
                proof_ref: proof_ref.to_string(),
                pipeline_key: PipelineKey::ShastaRisc0,
                route: "risc0/local".parse().expect("parse route"),
                proof_path: proof_path.display().to_string(),
            })
            .await?;
        runtime
            .get_proof_artifact("taiko_dev/ethereum", proof_ref)
            .await?
            .context("registered artifact")
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

    fn first_stage_task_id(proposal_id: u64) -> Result<String> {
        let prove_task_id =
            decode_task_id::<EngineTaskKey>(&encoded_proposal_task_id(proposal_id)?)
                .expect("decode prove task id");
        let stage_task_id = proposal_task_chain_ids(&prove_task_id)
            .into_iter()
            .next()
            .expect("proposal stage task");
        encode_task_id(&stage_task_id).context("encode failing task id")
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
