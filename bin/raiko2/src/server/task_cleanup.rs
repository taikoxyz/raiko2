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
    ArtifactRetentionCursor, ExpiredTaskCursor, PendingPublicationExpectation,
    PendingPublicationRetentionCursor, ProofArtifactKey, ProofArtifactRecord, RunnerStatus,
    RuntimeManager, RuntimeTaskRecord, TaskLifetime,
};
use std::collections::{HashSet, VecDeque};
use std::hash::Hash;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::MissedTickBehavior;
use tracing::{info, warn};

const ORPHANED_RUNTIME_TASK_TTL_SECS: u64 = 7 * 24 * 60 * 60;
const ORPHANED_RUNTIME_ERROR: &str = "runtime task orphaned: no active local execution";
const ORPHAN_CLEANUP_BATCH_SIZE: usize = 64;
const MAX_OVERDUE_ACTIVE_OBSERVATIONS: usize = 4_096;
const MAX_RETENTION_RETRY_IDENTITIES: usize = 4_096;

#[derive(Debug)]
struct BoundedObservationSet<T> {
    identities: HashSet<T>,
}

impl<T> Default for BoundedObservationSet<T> {
    fn default() -> Self {
        Self {
            identities: HashSet::new(),
        }
    }
}

impl<T> BoundedObservationSet<T>
where
    T: Eq + Hash,
{
    fn observe(&mut self, identity: T, capacity: usize) -> bool {
        if capacity == 0 || self.identities.len() >= capacity || self.identities.contains(&identity)
        {
            return false;
        }
        self.identities.insert(identity)
    }
}

#[derive(Debug)]
struct RetryQueue<T> {
    queue: VecDeque<T>,
    identities: HashSet<T>,
}

impl<T> Default for RetryQueue<T> {
    fn default() -> Self {
        Self {
            queue: VecDeque::new(),
            identities: HashSet::new(),
        }
    }
}

impl<T> RetryQueue<T>
where
    T: Clone + Eq + Hash,
{
    fn enqueue(&mut self, identity: T) -> bool {
        if self.identities.contains(&identity) || self.queue.len() >= MAX_RETENTION_RETRY_IDENTITIES
        {
            return false;
        }
        self.identities.insert(identity.clone());
        self.queue.push_back(identity);
        true
    }

    #[cfg(test)]
    fn peek(&self, limit: usize) -> Vec<T> {
        self.queue.iter().take(limit).cloned().collect()
    }

    fn peek_excluding(&self, limit: usize, excluded: &HashSet<T>) -> Vec<T> {
        self.queue
            .iter()
            .filter(|identity| !excluded.contains(*identity))
            .take(limit)
            .cloned()
            .collect()
    }

    fn acknowledge(&mut self, acknowledged: &[T]) {
        for identity in acknowledged {
            self.identities.remove(identity);
        }
        self.queue
            .retain(|identity| self.identities.contains(identity));
    }

    fn contains(&self, identity: &T) -> bool {
        self.identities.contains(identity)
    }

    fn len(&self) -> usize {
        self.queue.len()
    }

    #[cfg(test)]
    fn is_deduplicated(&self) -> bool {
        self.queue.len() == self.identities.len()
            && self.queue.iter().all(|item| self.identities.contains(item))
    }
}

#[derive(Debug)]
struct RetentionLaneState<I, C> {
    fresh_cursor: Option<C>,
    retries: RetryQueue<I>,
    prefer_retry: bool,
    last_fresh_attempts: usize,
    last_retry_attempts: usize,
}

impl<I, C> Default for RetentionLaneState<I, C> {
    fn default() -> Self {
        Self {
            fresh_cursor: None,
            retries: RetryQueue::default(),
            prefer_retry: true,
            last_fresh_attempts: 0,
            last_retry_attempts: 0,
        }
    }
}

impl<I, C> RetentionLaneState<I, C> {
    fn budgets(&mut self, total: usize) -> (usize, usize) {
        if total == 0 || self.retries.queue.is_empty() {
            return (0, total);
        }
        if total == 1 {
            let budgets = if self.prefer_retry { (1, 0) } else { (0, 1) };
            self.prefer_retry = !self.prefer_retry;
            return budgets;
        }
        let retry = total / 2;
        (retry, total - retry)
    }
}

#[derive(Debug, Default)]
pub(crate) struct RuntimeCleanupLoopState {
    orphan_cursor: Option<ExpiredTaskCursor>,
    overdue_active_cursor: Option<ExpiredTaskCursor>,
    overdue_active_observations: BoundedObservationSet<TaskLifetime>,
    roots: RetentionLaneState<TaskLifetime, ExpiredTaskCursor>,
    artifacts: RetentionLaneState<ProofArtifactKey, ArtifactRetentionCursor>,
    pending: RetentionLaneState<ProofArtifactKey, PendingPublicationRetentionCursor>,
}

#[cfg(test)]
impl RuntimeCleanupLoopState {
    fn root_retry_len(&self) -> usize {
        self.roots.retries.len()
    }

    fn artifact_retry_len(&self) -> usize {
        self.artifacts.retries.len()
    }

    fn pending_retry_len(&self) -> usize {
        self.pending.retries.len()
    }

    fn retry_queues_are_deduplicated(&self) -> bool {
        self.roots.retries.is_deduplicated()
            && self.artifacts.retries.is_deduplicated()
            && self.pending.retries.is_deduplicated()
    }
}

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
    pub overdue_active_warnings: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct RetentionLaneCleanupSummary {
    attempts: usize,
    removed: usize,
    retries: usize,
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
            && self.overdue_active_warnings == 0
    }
}

pub(crate) fn spawn_runtime_cleanup_loop(
    config: Arc<Config>,
    runtime: Arc<RuntimeManager>,
    pipelines: Arc<dyn PipelineFactory>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut cleanup_state = RuntimeCleanupLoopState::default();
        let interval_duration = Duration::from_secs(config.runtime.cleanup_interval_secs);
        let terminal_task_ttl_secs = config.runtime.terminal_task_ttl_secs;
        let cleanup_batch_size = config.runtime.cleanup_batch_size;
        log_runtime_cleanup_stats(
            run_runtime_cleanup_pass(
                Arc::clone(&runtime),
                Arc::clone(&pipelines),
                ORPHANED_RUNTIME_TASK_TTL_SECS,
                terminal_task_ttl_secs,
                cleanup_batch_size,
                &mut cleanup_state,
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
                    cleanup_batch_size,
                    &mut cleanup_state,
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
    batch_size: usize,
    cleanup_state: &mut RuntimeCleanupLoopState,
) -> Result<RuntimeCleanupStats> {
    let result = run_runtime_cleanup_pass_inner(
        Arc::clone(&runtime),
        pipelines,
        orphan_ttl_secs,
        terminal_ttl_secs,
        batch_size,
        cleanup_state,
    )
    .await;
    runtime.sweep_artifact_lifecycle_locks();
    crate::server::telemetry::record_runtime_cleanup_pass(if result.is_ok() {
        "success"
    } else {
        "failure"
    });
    if result.is_err() {
        crate::server::telemetry::record_runtime_state_stats(runtime.runtime_state_stats().await);
    }
    result
}

async fn run_runtime_cleanup_pass_inner(
    runtime: Arc<RuntimeManager>,
    pipelines: Arc<dyn PipelineFactory>,
    orphan_ttl_secs: u64,
    terminal_ttl_secs: u64,
    batch_size: usize,
    cleanup_state: &mut RuntimeCleanupLoopState,
) -> Result<RuntimeCleanupStats> {
    if batch_size == 0 {
        return Ok(RuntimeCleanupStats::default());
    }
    let lifecycle = ProofLifecycle::new(Arc::clone(&runtime), Arc::clone(&pipelines));

    let overdue_active_warnings = observe_overdue_active_tasks(
        runtime.as_ref(),
        terminal_ttl_secs,
        batch_size,
        &mut cleanup_state.overdue_active_cursor,
        &mut cleanup_state.overdue_active_observations,
    )
    .await?;
    let orphan_result = cancel_orphaned_runtime_tasks(
        runtime.as_ref(),
        &lifecycle,
        orphan_ttl_secs,
        batch_size,
        &mut cleanup_state.orphan_cursor,
    )
    .await;
    crate::server::telemetry::record_runtime_retention_blocked("orphan", orphan_result.is_err());
    let orphaned_cancelled = orphan_result?;
    let root_cursor_before = cleanup_state.roots.fresh_cursor.clone();
    let records = select_root_retention_batch(
        runtime.as_ref(),
        terminal_ttl_secs,
        batch_size,
        &mut cleanup_state.roots,
    )
    .await?;
    let mut stats = RuntimeCleanupStats {
        scanned: records.len(),
        expired: records.len(),
        orphaned_cancelled,
        overdue_active_warnings,
        ..RuntimeCleanupStats::default()
    };
    if let Err(error) = run_root_retention_lane(
        &lifecycle,
        &records,
        &mut cleanup_state.roots,
        &mut cleanup_state.artifacts,
        &mut stats,
    )
    .await
    {
        cleanup_state.roots.fresh_cursor = root_cursor_before;
        return Err(error);
    }
    let artifact_summary = run_artifact_retention_lane(
        &lifecycle,
        runtime.as_ref(),
        batch_size,
        &mut cleanup_state.artifacts,
        &mut stats,
    )
    .await?;
    run_pending_retention_lane(
        &lifecycle,
        runtime.as_ref(),
        batch_size,
        &mut cleanup_state.pending,
        &mut stats,
    )
    .await?;
    crate::server::telemetry::record_runtime_cleanup_scheduler_lane(
        "artifact",
        cleanup_state.artifacts.retries.len(),
        cleanup_state.artifacts.last_fresh_attempts,
        cleanup_state.artifacts.last_retry_attempts,
        artifact_summary.removed,
        artifact_summary.retries,
        artifact_summary
            .attempts
            .saturating_sub(artifact_summary.removed + artifact_summary.retries),
    );
    crate::server::telemetry::record_runtime_cleanup_stats(&stats);
    crate::server::telemetry::record_runtime_state_stats(runtime.runtime_state_stats().await);

    Ok(stats)
}

async fn run_artifact_retention_lane(
    lifecycle: &ProofLifecycle,
    runtime: &RuntimeManager,
    batch_size: usize,
    lane: &mut RetentionLaneState<ProofArtifactKey, ArtifactRetentionCursor>,
    stats: &mut RuntimeCleanupStats,
) -> Result<RetentionLaneCleanupSummary> {
    let cursor_before = lane.fresh_cursor.clone();
    let records = select_artifact_retention_batch(runtime, batch_size, lane).await?;
    let selected_retries = records
        .iter()
        .map(proof_artifact_key)
        .filter(|key| lane.retries.contains(key))
        .collect::<Vec<_>>();
    let cleanup = match lifecycle.remove_artifact_retention_batch(&records).await {
        Ok(cleanup) => cleanup,
        Err(error) => {
            lane.fresh_cursor = cursor_before;
            return Err(error);
        }
    };
    lane.retries.acknowledge(&selected_retries);
    for key in cleanup.retry_artifacts.iter().cloned() {
        lane.retries.enqueue(key);
    }
    stats.invalidated_artifacts = stats
        .invalidated_artifacts
        .saturating_add(cleanup.invalidated_artifacts);
    stats.removed_artifacts = stats
        .removed_artifacts
        .saturating_add(cleanup.removed_artifacts);
    stats.retained_artifact_failures = stats
        .retained_artifact_failures
        .saturating_add(cleanup.retained_artifact_failures);
    Ok(RetentionLaneCleanupSummary {
        attempts: records.len(),
        removed: cleanup.removed_artifacts,
        retries: cleanup.retry_artifacts.len(),
    })
}

async fn run_pending_retention_lane(
    lifecycle: &ProofLifecycle,
    runtime: &RuntimeManager,
    batch_size: usize,
    lane: &mut RetentionLaneState<ProofArtifactKey, PendingPublicationRetentionCursor>,
    stats: &mut RuntimeCleanupStats,
) -> Result<()> {
    let cursor_before = lane.fresh_cursor.clone();
    let records = select_pending_retention_batch(runtime, batch_size, lane).await?;
    let selected_retries = records
        .iter()
        .map(|expectation| expectation.key.clone())
        .filter(|key| lane.retries.contains(key))
        .collect::<Vec<_>>();
    let cleanup = match lifecycle.remove_pending_retention_batch(&records).await {
        Ok(cleanup) => cleanup,
        Err(error) => {
            lane.fresh_cursor = cursor_before;
            return Err(error);
        }
    };
    lane.retries.acknowledge(&selected_retries);
    for key in cleanup.retry_pending_publications.iter().cloned() {
        lane.retries.enqueue(key);
    }
    crate::server::telemetry::record_runtime_cleanup_scheduler_lane(
        "pending",
        lane.retries.len(),
        lane.last_fresh_attempts,
        lane.last_retry_attempts,
        cleanup.removed_pending_publications,
        cleanup.retry_pending_publications.len(),
        records.len().saturating_sub(
            cleanup.removed_pending_publications + cleanup.retry_pending_publications.len(),
        ),
    );
    stats.removed_pending_publications = cleanup.removed_pending_publications;
    stats.retained_pending_publication_failures = cleanup.retained_pending_publication_failures;
    Ok(())
}

async fn run_root_retention_lane(
    lifecycle: &ProofLifecycle,
    records: &[RuntimeTaskRecord],
    root_lane: &mut RetentionLaneState<TaskLifetime, ExpiredTaskCursor>,
    artifact_lane: &mut RetentionLaneState<ProofArtifactKey, ArtifactRetentionCursor>,
    stats: &mut RuntimeCleanupStats,
) -> Result<()> {
    let selected_retries = records
        .iter()
        .map(RuntimeTaskRecord::lifetime)
        .filter(|lifetime| root_lane.retries.contains(lifetime))
        .collect::<Vec<_>>();
    let cleanup = lifecycle.remove_terminal_retention_batch(records).await?;
    root_lane.retries.acknowledge(&selected_retries);
    for lifetime in cleanup.retry_roots.iter().cloned() {
        root_lane.retries.enqueue(lifetime);
    }
    for key in cleanup.retry_artifacts.iter().cloned() {
        artifact_lane.retries.enqueue(key);
    }
    crate::server::telemetry::record_runtime_cleanup_scheduler_lane(
        "root",
        root_lane.retries.len(),
        root_lane.last_fresh_attempts,
        root_lane.last_retry_attempts,
        cleanup.removed_roots,
        cleanup.retry_roots.len(),
        records
            .len()
            .saturating_sub(cleanup.removed_roots + cleanup.retry_roots.len()),
    );
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
    Ok(())
}

async fn select_root_retention_batch(
    runtime: &RuntimeManager,
    ttl_secs: u64,
    batch_size: usize,
    lane: &mut RetentionLaneState<TaskLifetime, ExpiredTaskCursor>,
) -> Result<Vec<RuntimeTaskRecord>> {
    let now = now_ts();
    let (retry_budget, fresh_budget) = lane.budgets(batch_size);
    let mut retries =
        load_root_retries(runtime, ttl_secs, now, lane, retry_budget, &HashSet::new()).await?;
    let fresh_capacity = fresh_budget.saturating_add(retry_budget.saturating_sub(retries.len()));
    let fresh_page = runtime
        .list_expired_terminal_tasks(now, ttl_secs, lane.fresh_cursor.as_ref(), fresh_capacity)
        .await?;
    let next_cursor = if let Some(last) = fresh_page.last() {
        Some(ExpiredTaskCursor {
            updated_at: last.updated_at,
            task_id: last.task_id.clone(),
        })
    } else if fresh_capacity > 0 {
        None
    } else {
        lane.fresh_cursor.clone()
    };
    let retry_identities = retries
        .iter()
        .map(RuntimeTaskRecord::lifetime)
        .collect::<HashSet<_>>();
    let mut fresh = fresh_page
        .into_iter()
        .filter(|record| {
            let lifetime = record.lifetime();
            !retry_identities.contains(&lifetime) && !lane.retries.contains(&lifetime)
        })
        .collect::<Vec<_>>();
    let extra_retry_capacity = batch_size.saturating_sub(retries.len() + fresh.len());
    let extra_retries = load_root_retries(
        runtime,
        ttl_secs,
        now,
        lane,
        extra_retry_capacity,
        &retry_identities,
    )
    .await?;
    retries.extend(extra_retries);
    lane.fresh_cursor = next_cursor;
    lane.last_retry_attempts = retries.len();
    lane.last_fresh_attempts = fresh.len();
    retries.append(&mut fresh);
    Ok(retries)
}

async fn load_root_retries(
    runtime: &RuntimeManager,
    ttl_secs: u64,
    now: i64,
    lane: &mut RetentionLaneState<TaskLifetime, ExpiredTaskCursor>,
    limit: usize,
    excluded: &HashSet<TaskLifetime>,
) -> Result<Vec<RuntimeTaskRecord>> {
    let mut records = Vec::with_capacity(limit);
    let selected = lane.retries.peek_excluding(limit, excluded);
    let mut stale = Vec::new();
    for lifetime in selected {
        if let Some(record) = runtime
            .get_expired_terminal_task(&lifetime, now, ttl_secs)
            .await?
        {
            records.push(record);
        } else {
            stale.push(lifetime);
        }
    }
    lane.retries.acknowledge(&stale);
    Ok(records)
}

async fn select_artifact_retention_batch(
    runtime: &RuntimeManager,
    batch_size: usize,
    lane: &mut RetentionLaneState<ProofArtifactKey, ArtifactRetentionCursor>,
) -> Result<Vec<ProofArtifactRecord>> {
    let (retry_budget, fresh_budget) = lane.budgets(batch_size);
    let mut retries = load_artifact_retries(runtime, lane, retry_budget, &HashSet::new()).await?;
    let fresh_capacity = fresh_budget.saturating_add(retry_budget.saturating_sub(retries.len()));
    let fresh_page = runtime
        .list_reclaimable_proof_artifacts(lane.fresh_cursor.as_ref(), fresh_capacity)
        .await?;
    let next_cursor = if let Some(last) = fresh_page.last() {
        Some(ArtifactRetentionCursor::from_record(last))
    } else if fresh_capacity > 0 {
        None
    } else {
        lane.fresh_cursor.clone()
    };
    let retry_identities = retries
        .iter()
        .map(proof_artifact_key)
        .collect::<HashSet<_>>();
    let mut fresh = fresh_page
        .into_iter()
        .filter(|record| {
            let key = proof_artifact_key(record);
            !retry_identities.contains(&key) && !lane.retries.contains(&key)
        })
        .collect::<Vec<_>>();
    let extra_retry_capacity = batch_size.saturating_sub(retries.len() + fresh.len());
    let extra_retries =
        load_artifact_retries(runtime, lane, extra_retry_capacity, &retry_identities).await?;
    retries.extend(extra_retries);
    lane.fresh_cursor = next_cursor;
    lane.last_retry_attempts = retries.len();
    lane.last_fresh_attempts = fresh.len();
    retries.append(&mut fresh);
    Ok(retries)
}

async fn load_artifact_retries(
    runtime: &RuntimeManager,
    lane: &mut RetentionLaneState<ProofArtifactKey, ArtifactRetentionCursor>,
    limit: usize,
    excluded: &HashSet<ProofArtifactKey>,
) -> Result<Vec<ProofArtifactRecord>> {
    let mut records = Vec::with_capacity(limit);
    let selected = lane.retries.peek_excluding(limit, excluded);
    let mut stale = Vec::new();
    for key in selected {
        if let Some(record) = runtime.get_reclaimable_proof_artifact(&key).await? {
            records.push(record);
        } else {
            stale.push(key);
        }
    }
    lane.retries.acknowledge(&stale);
    Ok(records)
}

async fn select_pending_retention_batch(
    runtime: &RuntimeManager,
    batch_size: usize,
    lane: &mut RetentionLaneState<ProofArtifactKey, PendingPublicationRetentionCursor>,
) -> Result<Vec<PendingPublicationExpectation>> {
    let (retry_budget, fresh_budget) = lane.budgets(batch_size);
    let mut retries = load_pending_retries(runtime, lane, retry_budget, &HashSet::new()).await?;
    let fresh_capacity = fresh_budget.saturating_add(retry_budget.saturating_sub(retries.len()));
    let fresh_page = runtime
        .list_reclaimable_pending_publications(lane.fresh_cursor.as_ref(), fresh_capacity)
        .await?;
    let next_cursor = if let Some(last) = fresh_page.last() {
        Some(PendingPublicationRetentionCursor::from_expectation(last))
    } else if fresh_capacity > 0 {
        None
    } else {
        lane.fresh_cursor.clone()
    };
    let retry_identities = retries
        .iter()
        .map(|expectation| expectation.key.clone())
        .collect::<HashSet<_>>();
    let mut fresh = fresh_page
        .into_iter()
        .filter(|expectation| {
            !retry_identities.contains(&expectation.key) && !lane.retries.contains(&expectation.key)
        })
        .collect::<Vec<_>>();
    let extra_retry_capacity = batch_size.saturating_sub(retries.len() + fresh.len());
    let extra_retries =
        load_pending_retries(runtime, lane, extra_retry_capacity, &retry_identities).await?;
    retries.extend(extra_retries);
    lane.fresh_cursor = next_cursor;
    lane.last_retry_attempts = retries.len();
    lane.last_fresh_attempts = fresh.len();
    retries.append(&mut fresh);
    Ok(retries)
}

async fn load_pending_retries(
    runtime: &RuntimeManager,
    lane: &mut RetentionLaneState<ProofArtifactKey, PendingPublicationRetentionCursor>,
    limit: usize,
    excluded: &HashSet<ProofArtifactKey>,
) -> Result<Vec<PendingPublicationExpectation>> {
    let mut pending = Vec::with_capacity(limit);
    let selected = lane.retries.peek_excluding(limit, excluded);
    let mut stale = Vec::new();
    for key in selected {
        if let Some(expectation) = runtime.get_reclaimable_pending_publication(&key).await? {
            pending.push(expectation);
        } else {
            stale.push(key);
        }
    }
    lane.retries.acknowledge(&stale);
    Ok(pending)
}

fn proof_artifact_key(record: &ProofArtifactRecord) -> ProofArtifactKey {
    ProofArtifactKey {
        network_pair: record.network_pair.clone(),
        pipeline_key: record.pipeline_key,
        route: record.route,
        proof_ref: record.proof_ref.clone(),
    }
}

async fn observe_overdue_active_tasks(
    runtime: &RuntimeManager,
    ttl_secs: u64,
    batch_size: usize,
    cursor: &mut Option<ExpiredTaskCursor>,
    observations: &mut BoundedObservationSet<TaskLifetime>,
) -> Result<usize> {
    let now = now_ts();
    let records = runtime
        .list_stale_nonterminal_tasks(now, ttl_secs, cursor.as_ref(), batch_size)
        .await?;
    *cursor = records.last().map(|record| ExpiredTaskCursor {
        updated_at: record.updated_at,
        task_id: record.task_id.clone(),
    });
    if records.is_empty() {
        *cursor = None;
    }

    let mut warnings = 0usize;
    for record in records {
        if !observations.observe(record.lifetime(), MAX_OVERDUE_ACTIVE_OBSERVATIONS) {
            continue;
        }
        warnings = warnings.saturating_add(1);
        warn!(
            task_id = %record.task_id,
            incarnation_id = %record.incarnation_id,
            status = %record.runner_status,
            age_secs = now.saturating_sub(record.updated_at),
            pipeline = record.pipeline_key.as_str(),
            route = %record.route,
            "runtime task remains active past the terminal retention window"
        );
    }
    Ok(warnings)
}

async fn cancel_orphaned_runtime_tasks(
    runtime: &RuntimeManager,
    lifecycle: &ProofLifecycle,
    ttl_secs: u64,
    batch_size: usize,
    cursor: &mut Option<ExpiredTaskCursor>,
) -> Result<usize> {
    let batch_size = batch_size.min(ORPHAN_CLEANUP_BATCH_SIZE);
    let records = runtime
        .list_stale_nonterminal_tasks(now_ts(), ttl_secs, cursor.as_ref(), batch_size)
        .await?;
    let next_cursor = records.last().map(|record| ExpiredTaskCursor {
        updated_at: record.updated_at,
        task_id: record.task_id.clone(),
    });
    if records.is_empty() {
        *cursor = None;
        return Ok(0);
    }
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
            .await
            .with_context(|| {
                format!(
                    "orphan retention blocked while reconciling task {}",
                    record.task_id
                )
            })?
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
            .with_context(|| {
                format!(
                    "orphan retention blocked while cancelling task {}",
                    record.task_id
                )
            })?;
        if !matches!(
            cancellation,
            raiko2_runtime::RuntimeMutationOutcome::Applied
        ) {
            continue;
        }

        cancelled += 1;
        warn!(task_id = %record.task_id, "cancelled orphaned runtime task");
    }

    *cursor = next_cursor;
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
                    overdue_active_warnings = stats.overdue_active_warnings,
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
        ExpiredTaskCursor, RetryQueue, RuntimeCleanupLoopState, RuntimeCleanupStats,
        proposal_task_id, run_runtime_cleanup_pass,
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
        ExactDeleteResult, ProofArtifactDescriptor, ProofArtifactKey, ProofArtifactObject,
        ProofArtifactPrefix, ProofArtifactPutResult, ProofArtifactRegistration, RunnerStatus,
        RuntimeManager, TaskRegistration, TaskRetentionState,
        test_support::{
            MemoryProofArtifactStore, ProofObjectStore, RuntimeStateObject, RuntimeStateStore,
            RuntimeStateWriteResult, RuntimeStoreScope,
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

    #[test]
    fn retry_queue_keeps_selected_items_until_acknowledged() {
        let mut queue = RetryQueue::default();
        assert!(queue.enqueue("first".to_string()));
        assert!(queue.enqueue("second".to_string()));

        let selected = queue.peek(1);

        assert_eq!(selected, vec!["first"]);
        assert_eq!(queue.len(), 2);
        queue.acknowledge(&selected);
        assert_eq!(queue.peek(2), vec!["second"]);
        assert_eq!(queue.len(), 1);

        for index in 0..=super::MAX_RETENTION_RETRY_IDENTITIES {
            queue.enqueue(format!("bounded-{index}"));
        }
        assert_eq!(queue.len(), super::MAX_RETENTION_RETRY_IDENTITIES);
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

        let mut cleanup_state = RuntimeCleanupLoopState::default();
        let stats = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
            7_200,
            7_200,
            64,
            &mut cleanup_state,
        )
        .await?;

        assert_eq!(
            stats,
            RuntimeCleanupStats {
                orphaned_cancelled: 2,
                overdue_active_warnings: 3,
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

    #[tokio::test]
    async fn overdue_active_task_is_observed_once_without_removal() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root("overdue-active"))?);
        let task_id = "overdue-active-root";
        register_runtime_task(
            runtime.as_ref(),
            task_id,
            &encoded_proposal_task_id(63)?,
            RunnerStatus::Running,
            now_ts().saturating_sub(7_201),
        )
        .await?;
        let factory = Arc::new(build_factory(Arc::new(MockEngine::default())));
        let mut cleanup_state = RuntimeCleanupLoopState::default();

        let first = run_runtime_cleanup_pass(
            runtime.clone(),
            factory.clone(),
            14_400,
            7_200,
            64,
            &mut cleanup_state,
        )
        .await?;
        let second = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
            14_400,
            7_200,
            64,
            &mut cleanup_state,
        )
        .await?;

        assert_eq!(first.overdue_active_warnings, 1);
        assert_eq!(second.overdue_active_warnings, 0);
        assert_eq!(
            runtime
                .get_task(task_id)
                .await?
                .context("overdue active task")?
                .runner_status,
            RunnerStatus::Running
        );
        Ok(())
    }

    #[tokio::test]
    async fn orphan_cleanup_has_an_independent_batch_limit() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "orphan-batch-limit",
        ))?);
        let stale = now_ts().saturating_sub(7_201);
        for index in 0..65 {
            register_runtime_task(
                runtime.as_ref(),
                &format!("orphan-root-{index:03}"),
                &encoded_proposal_task_id(100 + index)?,
                RunnerStatus::Running,
                stale,
            )
            .await?;
        }
        let factory = Arc::new(build_factory(Arc::new(MockEngine::default())));
        let mut cleanup_state = RuntimeCleanupLoopState::default();

        let stats = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
            7_200,
            14_400,
            1_024,
            &mut cleanup_state,
        )
        .await?;

        assert_eq!(stats.orphaned_cancelled, 64);
        let records = runtime.list_tasks().await?;
        assert_eq!(
            records
                .iter()
                .filter(|record| record.runner_status == RunnerStatus::Cancelled)
                .count(),
            64
        );
        assert_eq!(
            records
                .iter()
                .filter(|record| record.runner_status == RunnerStatus::Running)
                .count(),
            1
        );
        Ok(())
    }

    #[tokio::test]
    async fn overdue_active_observations_do_not_evict_at_four_batches() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "overdue-capacity",
        ))?);
        let stale = now_ts().saturating_sub(7_201);
        for index in 0..5 {
            register_runtime_task(
                runtime.as_ref(),
                &format!("overdue-root-{index}"),
                &encoded_proposal_task_id(200 + index)?,
                RunnerStatus::Running,
                stale,
            )
            .await?;
        }
        let factory = Arc::new(build_factory(Arc::new(MockEngine::default())));
        let mut cleanup_state = RuntimeCleanupLoopState::default();

        for _ in 0..5 {
            let stats = run_runtime_cleanup_pass(
                runtime.clone(),
                factory.clone(),
                14_400,
                7_200,
                1,
                &mut cleanup_state,
            )
            .await?;
            assert_eq!(stats.overdue_active_warnings, 1);
        }
        let wrapped = run_runtime_cleanup_pass(
            runtime.clone(),
            factory.clone(),
            14_400,
            7_200,
            1,
            &mut cleanup_state,
        )
        .await?;
        assert_eq!(wrapped.overdue_active_warnings, 0);
        let repeated =
            run_runtime_cleanup_pass(runtime, factory, 14_400, 7_200, 1, &mut cleanup_state)
                .await?;
        assert_eq!(repeated.overdue_active_warnings, 0);
        Ok(())
    }

    #[tokio::test]
    async fn cleanup_pass_sweeps_dead_artifact_lifecycle_locks() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "artifact-lock-sweep",
        ))?);
        let pipeline = PipelineKey::ShastaSp1;
        let route = pipeline.route();

        runtime
            .publish_proof_artifact_bytes("l1-l2", pipeline, route, "control", b"control")
            .await?;
        assert_eq!(runtime.sweep_artifact_lifecycle_locks(), 1);

        runtime
            .publish_proof_artifact_bytes("l1-l2", pipeline, route, "swept", b"swept")
            .await?;
        run_runtime_cleanup_pass(
            runtime.clone(),
            Arc::new(build_factory(Arc::new(MockEngine::default()))),
            14_400,
            7_200,
            0,
            &mut RuntimeCleanupLoopState::default(),
        )
        .await?;

        assert_eq!(runtime.sweep_artifact_lifecycle_locks(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn orphan_cursor_rolls_back_when_a_page_fails() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root("orphan-cursor"))?);
        let stale = now_ts().saturating_sub(7_201);
        for (index, task_id) in ["a-orphan", "b-orphan"].into_iter().enumerate() {
            register_runtime_task(
                runtime.as_ref(),
                task_id,
                &encoded_proposal_task_id(300 + index as u64)?,
                RunnerStatus::Running,
                stale,
            )
            .await?;
        }
        register_runtime_task(
            runtime.as_ref(),
            "terminal-behind-orphan",
            &encoded_proposal_task_id(302)?,
            RunnerStatus::Completed,
            1,
        )
        .await?;
        let poison = runtime
            .get_task("a-orphan")
            .await?
            .context("poison orphan")?;
        let poison_ref = poison
            .artifact_refs
            .first()
            .context("poison orphan proof reference")?
            .clone();
        let poison_artifact =
            register_runtime_proof_artifact(runtime.as_ref(), &poison, &poison_ref).await?;
        let failing = Arc::new(MockEngine::default());
        let mut cleanup_state = RuntimeCleanupLoopState::default();

        for _ in 0..2 {
            runtime
                .publish_proof_artifact_bytes(
                    "l1-l2",
                    PipelineKey::ShastaSp1,
                    PipelineKey::ShastaSp1.route(),
                    "failed-pass-lock",
                    b"proof",
                )
                .await?;
            let error = run_runtime_cleanup_pass(
                runtime.clone(),
                Arc::new(build_factory(failing.clone())),
                7_200,
                14_400,
                64,
                &mut cleanup_state,
            )
            .await
            .expect_err("permanent orphan failure must fail-stop retention");
            assert!(
                error
                    .to_string()
                    .contains("orphan retention blocked while reconciling task a-orphan")
            );
            assert!(cleanup_state.orphan_cursor.is_none());
            assert_eq!(runtime.sweep_artifact_lifecycle_locks(), 0);
            assert!(
                runtime.get_task("terminal-behind-orphan").await?.is_some(),
                "later retention lanes must remain frozen"
            );
        }

        assert!(
            runtime
                .remove_proof_artifact_if_descriptor(
                    &poison.network_pair,
                    poison.pipeline_key,
                    poison.route,
                    &poison_ref,
                    &poison_artifact.descriptor(),
                )
                .await?,
            "operator repair must remove the poison registration"
        );

        run_runtime_cleanup_pass(
            runtime.clone(),
            Arc::new(build_factory(Arc::new(MockEngine::default()))),
            7_200,
            14_400,
            64,
            &mut cleanup_state,
        )
        .await?;
        assert_eq!(
            runtime
                .get_task("a-orphan")
                .await?
                .context("repaired orphan")?
                .runner_status,
            RunnerStatus::Cancelled
        );
        assert_eq!(
            runtime
                .get_task("b-orphan")
                .await?
                .context("second orphan")?
                .runner_status,
            RunnerStatus::Cancelled
        );
        assert!(runtime.get_task("terminal-behind-orphan").await?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn orphan_cancellation_failure_names_task_and_keeps_cursor_uncommitted() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "orphan-cancellation-context",
        ))?);
        register_runtime_task(
            runtime.as_ref(),
            "cancel-poison",
            &encoded_proposal_task_id(303)?,
            RunnerStatus::Running,
            now_ts().saturating_sub(7_201),
        )
        .await?;
        let factory = Arc::new(build_factory(Arc::new(MockEngine::with_failing_owners(
            HashSet::from(["cancel-poison".to_string()]),
        ))));
        let mut cleanup_state = RuntimeCleanupLoopState::default();

        let error = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
            7_200,
            14_400,
            64,
            &mut cleanup_state,
        )
        .await
        .expect_err("orphan cancellation projection failure must fail-stop retention");

        assert!(
            error
                .to_string()
                .contains("orphan retention blocked while cancelling task cancel-poison")
        );
        assert!(cleanup_state.orphan_cursor.is_none());
        assert_eq!(
            runtime
                .get_task("cancel-poison")
                .await?
                .context("cancelled poison task")?
                .runner_status,
            RunnerStatus::Cancelled
        );
        Ok(())
    }

    struct MockEngine {
        failing_owners: HashSet<String>,
        active_owners: HashSet<RootOwner>,
        queue_task_ids: HashSet<String>,
        mutate_on_detach: Option<(Arc<RuntimeManager>, String)>,
    }

    impl Default for MockEngine {
        fn default() -> Self {
            Self {
                failing_owners: HashSet::new(),
                active_owners: HashSet::new(),
                queue_task_ids: HashSet::new(),
                mutate_on_detach: None,
            }
        }
    }

    impl MockEngine {
        fn with_failing_owners(failing_owners: HashSet<String>) -> Self {
            Self {
                failing_owners,
                active_owners: HashSet::new(),
                queue_task_ids: HashSet::new(),
                mutate_on_detach: None,
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

        fn with_mutating_detach(runtime: Arc<RuntimeManager>, task_id: &str) -> Self {
            Self {
                mutate_on_detach: Some((runtime, task_id.to_string())),
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
            let mutate_on_detach = self.mutate_on_detach.clone();
            Box::pin(async move {
                if should_fail {
                    Err(TaskStoreError::backend(std::io::Error::other(
                        "mock detach failure",
                    )))
                } else {
                    if let Some((runtime, task_id)) = mutate_on_detach {
                        let mut task = runtime
                            .get_task(&task_id)
                            .await
                            .map_err(|error| {
                                TaskStoreError::backend(std::io::Error::other(error.to_string()))
                            })?
                            .context("task mutated during detach")
                            .map_err(|error| {
                                TaskStoreError::backend(std::io::Error::other(error.to_string()))
                            })?;
                        task.image_ref = Some("changed-during-detach".to_string());
                        runtime.upsert_task(&task).await.map_err(|error| {
                            TaskStoreError::backend(std::io::Error::other(error.to_string()))
                        })?;
                    }
                    Ok(raiko2_queue::DetachOutcome::not_attached(mode))
                }
            })
        }
    }

    #[tokio::test]
    async fn finalize_skipped_root_is_returned_to_retry_queue() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root("finalize-skip"))?);
        register_runtime_task(
            runtime.as_ref(),
            "expired-root",
            &encoded_proposal_task_id(62)?,
            RunnerStatus::Completed,
            1,
        )
        .await?;
        let expired = runtime
            .get_task("expired-root")
            .await?
            .context("expired root")?;
        let expected_lifetime = expired.lifetime();
        let engine = Arc::new(MockEngine::with_mutating_detach(
            runtime.clone(),
            "expired-root",
        ));
        let lifecycle = crate::server::lifecycle::ProofLifecycle::new(
            runtime.clone(),
            Arc::new(build_factory(engine)),
        );

        let cleanup = lifecycle
            .remove_terminal_retention_batch(&[expired])
            .await?;

        assert_eq!(cleanup.removed_roots, 0);
        assert_eq!(cleanup.retained_root_failures, 1);
        assert_eq!(cleanup.retry_roots, vec![expected_lifetime]);
        assert!(runtime.get_task("expired-root").await?.is_some());
        Ok(())
    }

    #[derive(Debug)]
    struct FailOnceInvalidationStore {
        inner: MemoryProofArtifactStore,
        runtime_state_writes: AtomicUsize,
        fail_next_invalidation: AtomicBool,
        fail_invalidation_for: Option<String>,
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
                fail_invalidation_for: None,
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

        fn failing_invalidation_for(proof_ref: &str) -> Result<Self> {
            let mut store = Self::new()?;
            store.fail_next_invalidation = AtomicBool::new(false);
            store.fail_invalidation_for = Some(proof_ref.to_string());
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

        async fn delete_exact(
            &self,
            key: &ProofArtifactKey,
            descriptor: &ProofArtifactDescriptor,
        ) -> Result<ExactDeleteResult> {
            let is_pending = key.proof_ref.starts_with("__pending__:");
            if self
                .fail_invalidation_for
                .as_deref()
                .is_some_and(|proof_ref| !is_pending && proof_ref == key.proof_ref)
            {
                anyhow::bail!("injected persistent proof artifact deletion failure");
            }
            if !is_pending && self.fail_next_invalidation.swap(false, Ordering::AcqRel) {
                anyhow::bail!("injected proof artifact deletion failure");
            }
            if !is_pending && self.block_next_invalidation.swap(false, Ordering::AcqRel) {
                self.invalidation_started.notify_one();
                self.allow_invalidation.notified().await;
            }
            if is_pending && self.fail_next_delete.swap(false, Ordering::AcqRel) {
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

        let mut cleanup_state = RuntimeCleanupLoopState::default();
        let stats = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
            7_200,
            7_200,
            64,
            &mut cleanup_state,
        )
        .await?;

        assert_eq!(
            stats,
            RuntimeCleanupStats {
                overdue_active_warnings: 1,
                ..RuntimeCleanupStats::default()
            }
        );
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

        let mut cleanup_state = RuntimeCleanupLoopState::default();
        let stats = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
            7_200,
            7_200,
            64,
            &mut cleanup_state,
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

        let mut cleanup_state = RuntimeCleanupLoopState::default();
        let stats = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
            7_200,
            7_200,
            64,
            &mut cleanup_state,
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
                overdue_active_warnings: 0,
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
        register_runtime_proof_artifact(runtime.as_ref(), &expired, proof_ref).await?;

        let mut cleanup_state = RuntimeCleanupLoopState::default();
        let stats = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
            7_200,
            7_200,
            64,
            &mut cleanup_state,
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
        register_runtime_proof_artifact(runtime.as_ref(), &root, proof_ref).await?;
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

        let mut cleanup_state = RuntimeCleanupLoopState::default();
        let stats = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
            7_200,
            7_200,
            64,
            &mut cleanup_state,
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

        let mut cleanup_state = RuntimeCleanupLoopState::default();
        let first = run_runtime_cleanup_pass(
            runtime.clone(),
            factory.clone(),
            7_200,
            7_200,
            64,
            &mut cleanup_state,
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
        let original_incarnation = artifact_root.incarnation_id;
        let proof_ref = artifact_root
            .artifact_refs
            .first()
            .context("artifact proof reference")?;
        register_runtime_proof_artifact(runtime.as_ref(), &artifact_root, proof_ref).await?;

        let mut cleanup_state = RuntimeCleanupLoopState::default();
        let cleanup = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
            7_200,
            7_200,
            64,
            &mut cleanup_state,
        )
        .await?;

        assert_eq!(cleanup.removed_roots, 2);
        assert_eq!(cleanup.retained_failures, 0);
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
        assert!(matches!(
            runtime.retire_task_if_unchanged(&expired, None).await?,
            raiko2_runtime::RuntimeMutationOutcome::Applied
        ));
        let retired = runtime
            .get_task("expired-root")
            .await?
            .context("retired root")?;
        assert!(matches!(
            runtime.remove_task_if_current(&retired.lifetime()).await?,
            raiko2_runtime::RuntimeMutationOutcome::Applied
        ));
        let artifacts = runtime.list_reclaimable_proof_artifacts(None, 64).await?;
        assert_eq!(artifacts.len(), 1);

        let lifecycle = crate::server::lifecycle::ProofLifecycle::new(
            Arc::clone(&runtime),
            Arc::new(build_factory(Arc::new(MockEngine::default()))),
        );
        let cleanup = tokio::spawn({
            let lifecycle = lifecycle.clone();
            async move { lifecycle.remove_artifact_retention_batch(&artifacts).await }
        });
        store.invalidation_started.notified().await;

        let lifecycle_gate = runtime.execution_lifecycle_gate();
        let guard = tokio::time::timeout(std::time::Duration::from_secs(1), lifecycle_gate.lock())
            .await
            .expect("external artifact invalidation must not hold the lifecycle gate");
        drop(guard);
        store.allow_invalidation.notify_one();

        assert_eq!(cleanup.await??.removed_artifacts, 1);
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

        let mut cleanup_state = RuntimeCleanupLoopState::default();
        let first = run_runtime_cleanup_pass(
            runtime.clone(),
            factory.clone(),
            7_200,
            7_200,
            64,
            &mut cleanup_state,
        )
        .await?;
        assert_eq!(first.removed_roots, 1);
        assert_eq!(first.retained_failures, 0);
        assert_eq!(first.removed_artifacts, 1);
        assert_eq!(first.retained_pending_publication_failures, 1);
        assert_eq!(cleanup_state.pending_retry_len(), 1);
        assert!(cleanup_state.retry_queues_are_deduplicated());
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

        let retry = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
            7_200,
            7_200,
            64,
            &mut cleanup_state,
        )
        .await?;
        assert_eq!(retry.removed_pending_publications, 1);
        assert_eq!(cleanup_state.pending_retry_len(), 0);
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
    async fn runtime_cleanup_batches_roots_while_fencing_each_untracked_canonical() -> Result<()> {
        const ROOT_COUNT: usize = 3;
        let store = Arc::new(FailOnceInvalidationStore::counting()?);
        let runtime = Arc::new(RuntimeManager::with_store(store.clone()));
        let engine = Arc::new(MockEngine::default());
        let factory = Arc::new(build_factory(engine));
        for proposal_id in 20_u64..23 {
            let task_id = format!("expired-{proposal_id}");
            register_runtime_task(
                runtime.as_ref(),
                &task_id,
                &encoded_proposal_task_id(proposal_id)?,
                RunnerStatus::Running,
                1,
            )
            .await?;
            let mut record = runtime.get_task(&task_id).await?.context("pending root")?;
            let proof_ref = record
                .artifact_refs
                .first()
                .context("pending proof reference")?
                .clone();
            runtime
                .publish_proof_artifact_bytes(
                    &record.network_pair,
                    record.pipeline_key,
                    record.route,
                    &proof_ref,
                    proof_ref.as_bytes(),
                )
                .await?;
            assert!(
                runtime
                    .checkpoint_pending_proof_publication(
                        &record.network_pair,
                        record.pipeline_key,
                        record.route,
                        &proof_ref,
                        &[record.incarnation_id],
                        proof_ref.as_bytes(),
                    )
                    .await?
            );
            record.runner_status = RunnerStatus::Completed;
            record.proof_uri = Some(format!("memory://proofs/{task_id}"));
            record.updated_at = 1;
            runtime.upsert_task(&record).await?;
        }
        let writes_before = store.runtime_state_writes.load(Ordering::Acquire);

        let mut cleanup_state = RuntimeCleanupLoopState::default();
        let stats = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
            7_200,
            7_200,
            64,
            &mut cleanup_state,
        )
        .await?;

        let writes_after = store.runtime_state_writes.load(Ordering::Acquire);
        assert_eq!(stats.removed_roots, ROOT_COUNT);
        assert_eq!(stats.removed_pending_publications, ROOT_COUNT);
        // Root and pending-intent removal are each batched once. Every untracked canonical
        // recovery needs one durable Invalidated write before exact deletion and one exact
        // finalization write afterwards.
        assert_eq!(writes_after - writes_before, 2 * ROOT_COUNT + 2);
        Ok(())
    }

    #[tokio::test]
    async fn runtime_cleanup_retries_a_durable_retirement_after_projection_failure() -> Result<()> {
        let store = Arc::new(MemoryProofArtifactStore::new(
            "test".to_string(),
            format!(
                "ttl-failure-{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("time")
                    .as_nanos()
            ),
        )?);
        let runtime = Arc::new(RuntimeManager::with_store(store.clone()));
        runtime.initialize().await?;
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
        let mut failed = runtime
            .get_task("expired-root")
            .await?
            .context("failed root before retention")?;
        failed.error = Some("prover failed before retention".to_string());
        runtime.upsert_task(&failed).await?;

        let mut cleanup_state = RuntimeCleanupLoopState::default();
        let stats = run_runtime_cleanup_pass(
            runtime.clone(),
            factory,
            7_200,
            7_200,
            64,
            &mut cleanup_state,
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
                overdue_active_warnings: 0,
            }
        );
        let retired = runtime
            .get_task("expired-root")
            .await?
            .expect("retired root remains recoverable");
        assert_eq!(retired.runner_status, RunnerStatus::Failed);
        assert_eq!(
            retired.error.as_deref(),
            Some("prover failed before retention")
        );
        assert_eq!(retired.retention_state, TaskRetentionState::Removing);
        assert_eq!(retired.updated_at, 1);

        drop(runtime);
        let runtime = Arc::new(RuntimeManager::with_store(store));
        runtime.initialize().await?;
        assert_eq!(
            runtime
                .get_task("expired-root")
                .await?
                .context("restarted root")?
                .retention_state,
            TaskRetentionState::Retained
        );
        let mut cleanup_state = RuntimeCleanupLoopState::default();
        let healthy_factory = Arc::new(build_factory(Arc::new(MockEngine::default())));
        let retry = run_runtime_cleanup_pass(
            runtime.clone(),
            healthy_factory,
            7_200,
            7_200,
            64,
            &mut cleanup_state,
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
                overdue_active_warnings: 0,
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
        let mut cleanup_state = RuntimeCleanupLoopState::default();

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
            64,
            &mut cleanup_state,
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
                overdue_active_warnings: 0,
            }
        );
        assert_eq!(
            cleanup_state.roots.fresh_cursor,
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
            64,
            &mut cleanup_state,
        )
        .await?;
        assert_eq!(
            second,
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
                overdue_active_warnings: 0,
            }
        );
        assert!(runtime.get_task("expired-c").await?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn sustained_arrival_retries_failures_without_starving_fresh_work() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::with_store(Arc::new(
            FailOnceInvalidationStore::failing_invalidation_for("a-retry-artifact")?,
        )));
        let engine = Arc::new(MockEngine::with_failing_owners(HashSet::from([
            "retry-root".to_string(),
        ])));
        let factory = Arc::new(build_factory(engine));
        register_runtime_task(
            runtime.as_ref(),
            "retry-root",
            &encoded_proposal_task_id(49)?,
            RunnerStatus::Failed,
            1,
        )
        .await?;
        register_unowned_runtime_artifact(runtime.as_ref(), "a-retry-artifact").await?;
        let mut cleanup_state = RuntimeCleanupLoopState::default();

        for pass in 0..3 {
            let task_id = format!("fresh-root-{pass}");
            let proof_ref = format!("z-fresh-artifact-{pass}");
            register_runtime_task(
                runtime.as_ref(),
                &task_id,
                &encoded_proposal_task_id(50 + pass)?,
                RunnerStatus::Completed,
                10 + i64::try_from(pass).expect("small pass index"),
            )
            .await?;
            register_unowned_runtime_artifact(runtime.as_ref(), &proof_ref).await?;

            let stats = run_runtime_cleanup_pass(
                runtime.clone(),
                factory.clone(),
                7_200,
                7_200,
                2,
                &mut cleanup_state,
            )
            .await?;

            assert_eq!(stats.removed_roots, 1);
            assert_eq!(stats.retained_failures, 1);
            assert_eq!(stats.removed_artifacts, 1);
            assert_eq!(stats.retained_artifact_failures, 1);
            assert!(runtime.get_task(&task_id).await?.is_none());
            assert!(
                runtime
                    .get_proof_artifact_including_invalidated(
                        "taiko_dev/ethereum",
                        PipelineKey::ShastaRisc0,
                        PipelineKey::ShastaRisc0.route(),
                        &proof_ref,
                    )
                    .await?
                    .is_none()
            );
            assert_eq!(cleanup_state.root_retry_len(), 1);
            assert_eq!(cleanup_state.artifact_retry_len(), 1);
            assert!(cleanup_state.retry_queues_are_deduplicated());
        }

        assert!(runtime.get_task("retry-root").await?.is_some());
        assert!(
            runtime
                .get_proof_artifact_including_invalidated(
                    "taiko_dev/ethereum",
                    PipelineKey::ShastaRisc0,
                    PipelineKey::ShastaRisc0.route(),
                    "a-retry-artifact",
                )
                .await?
                .is_some()
        );
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

    async fn register_unowned_runtime_artifact(
        runtime: &RuntimeManager,
        proof_ref: &str,
    ) -> Result<ProofArtifactRegistration> {
        let pipeline_key = PipelineKey::ShastaRisc0;
        let route = pipeline_key.route();
        let object = runtime
            .publish_proof_artifact_bytes(
                "taiko_dev/ethereum",
                pipeline_key,
                route,
                proof_ref,
                proof_ref.as_bytes(),
            )
            .await?
            .try_object()
            .context("unowned runtime artifact")?
            .clone();
        let registration = ProofArtifactRegistration {
            network_pair: "taiko_dev/ethereum".into(),
            proof_ref: proof_ref.into(),
            pipeline_key,
            route,
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
