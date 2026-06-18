use crate::config::Config;
use crate::server::state::{EngineHandle, PipelineFactory};
use crate::server::task_metadata::TaskMetadata;
use anyhow::{Context, Result, anyhow};
use raiko2_engine::{EngineTaskId, EngineTaskKey, ProposalTaskRequest};
use raiko2_pipeline::PipelineKey;
use raiko2_queue::{TaskStoreError, encode_task_id};
use raiko2_runtime::{ExpiredTaskCursor, RunnerStatus, RuntimeManager, RuntimeTaskRecord};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::MissedTickBehavior;
use tracing::{info, warn};

const RUNTIME_CLEANUP_BATCH_SIZE: usize = 64;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RuntimeCleanupStats {
    pub scanned: usize,
    pub expired: usize,
    pub removed_roots: usize,
    pub skipped_shared_children: usize,
    pub retained_failures: usize,
}

impl RuntimeCleanupStats {
    const fn is_idle(self) -> bool {
        self.scanned == 0
            && self.expired == 0
            && self.removed_roots == 0
            && self.skipped_shared_children == 0
            && self.retained_failures == 0
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
) {
    if config.runtime.inactive_ttl_secs == 0 {
        return;
    }

    tokio::spawn(async move {
        let mut cursor = None;
        let interval_duration = Duration::from_millis(config.queue.maintenance_interval_ms);
        log_runtime_cleanup_stats(
            run_runtime_cleanup_pass(
                Arc::clone(&runtime),
                Arc::clone(&pipelines),
                config.runtime.inactive_ttl_secs,
                &mut cursor,
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
                    config.runtime.inactive_ttl_secs,
                    &mut cursor,
                )
                .await,
            );
        }
    });
}

pub(crate) async fn run_runtime_cleanup_pass(
    runtime: Arc<RuntimeManager>,
    pipelines: Arc<dyn PipelineFactory>,
    ttl_secs: u64,
    cursor: &mut Option<ExpiredTaskCursor>,
) -> Result<RuntimeCleanupStats> {
    if ttl_secs == 0 {
        return Ok(RuntimeCleanupStats::default());
    }

    let records = runtime
        .list_expired_terminal_tasks(
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
    let mut stats = RuntimeCleanupStats {
        scanned: records.len(),
        expired: records.len(),
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
        ExpiredTaskCursor, RuntimeCleanupStats, cancel_registered_tasks, proposal_task_chain_ids,
        proposal_task_id, run_runtime_cleanup_pass,
    };
    use crate::server::state::{EngineHandle, StaticPipelineFactory};
    use crate::server::task_metadata::{ProposalTask, RuntimeMetadata, TaskMetadata};
    use anyhow::{Context, Result};
    use raiko2_engine::{
        AggregateProofInput, AggregationTaskRequest, EngineTaskId, EngineTaskKey,
        ProposalTaskRequest, ProverTaskConfig,
    };
    use raiko2_pipeline::PipelineKey;
    use raiko2_primitives::ProofType;
    use raiko2_queue::{TaskStoreError, decode_task_id, encode_task_id};
    use raiko2_runtime::{RunnerStatus, RuntimeManager, TaskRegistration};
    use std::collections::HashSet;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
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

    #[derive(Default)]
    struct MockEngine {
        removed: Mutex<Vec<String>>,
        cancelled: Mutex<Vec<String>>,
        fail_on: HashSet<String>,
    }

    impl MockEngine {
        fn new(fail_on: HashSet<String>) -> Self {
            Self {
                removed: Mutex::new(Vec::new()),
                cancelled: Mutex::new(Vec::new()),
                fail_on,
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

        fn list_tasks(
            &self,
        ) -> BoxFuture<'_, Result<Vec<crate::server::state::EngineQueueTaskView>, TaskStoreError>>
        {
            Box::pin(async { Ok(Vec::new()) })
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

        let mut cursor = None;
        let stats = run_runtime_cleanup_pass(runtime.clone(), factory, 7_200, &mut cursor).await?;

        assert_eq!(
            stats,
            RuntimeCleanupStats {
                scanned: 1,
                expired: 1,
                removed_roots: 1,
                skipped_shared_children: 1,
                retained_failures: 0,
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

        let mut cursor = None;
        let stats = run_runtime_cleanup_pass(runtime.clone(), factory, 7_200, &mut cursor).await?;

        assert_eq!(
            stats,
            RuntimeCleanupStats {
                scanned: 1,
                expired: 1,
                removed_roots: 0,
                skipped_shared_children: 0,
                retained_failures: 1,
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
        let mut cursor = None;

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

        let first =
            run_runtime_cleanup_pass(runtime.clone(), factory.clone(), 7_200, &mut cursor).await?;
        assert_eq!(
            first,
            RuntimeCleanupStats {
                scanned: 2,
                expired: 2,
                removed_roots: 1,
                skipped_shared_children: 0,
                retained_failures: 1,
            }
        );
        assert_eq!(
            cursor,
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

        let second = run_runtime_cleanup_pass(runtime.clone(), factory, 7_200, &mut cursor).await?;
        assert_eq!(
            second,
            RuntimeCleanupStats {
                scanned: 1,
                expired: 1,
                removed_roots: 1,
                skipped_shared_children: 0,
                retained_failures: 0,
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
