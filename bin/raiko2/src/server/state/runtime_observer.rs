use anyhow::{Context, Result};
use async_trait::async_trait;
use raiko2_engine::{
    EngineObserver, EngineTaskId, EngineTaskKey, EngineTaskSuccess, ProposalStage,
    tasks::EngineTask,
};
use raiko2_primitives::{RaikoError, RaikoResult};
use raiko2_prover::{BoundlessSubmissionSnapshot, ProverProgress};
use raiko2_runtime::{ProofArtifactRegistration, RunnerStatus, RuntimeManager, RuntimeTaskRecord};
use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::server::task_metadata::{
    TaskMetadata, TaskRuntimeMetadata, proposal_task_ref, stage_task_ref_for_stage,
};
use crate::server::telemetry::{self, MetricContext};
#[cfg(test)]
use raiko2_engine::ProverTaskConfig;
#[cfg(test)]
use raiko2_pipeline::PipelineRoute;

#[derive(Clone)]
pub(crate) struct RuntimeObserver {
    runtime: Arc<RuntimeManager>,
    network_pair: String,
    started_stage_tasks: Arc<Mutex<HashSet<String>>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FailedRootPolicy {
    Exclude,
    Include,
}

impl RuntimeObserver {
    pub(crate) fn new(runtime: Arc<RuntimeManager>, network_pair: String) -> Self {
        Self {
            runtime,
            network_pair,
            started_stage_tasks: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    fn root_task_ref(id: &EngineTaskId) -> String {
        match &id.0 {
            EngineTaskKey::Proposal {
                pipeline, request, ..
            } => proposal_task_ref(*pipeline, request),
            EngineTaskKey::Aggregate { pipeline, request } => {
                crate::server::task_metadata::aggregate_task_ref(*pipeline, request)
            }
        }
    }

    const fn stage_name(task: &EngineTask) -> &'static str {
        match task {
            EngineTask::Proposal { .. } => "proposal",
            EngineTask::Preflight { .. } => "preflight",
            EngineTask::Validate { .. } => "validation",
            EngineTask::Encode { .. } => "encode",
            EngineTask::ProveProposal { .. } => "prove",
            EngineTask::Aggregate { .. } => "aggregate",
        }
    }

    async fn update_root_records<F>(&self, id: &EngineTaskId, mutator: F) -> Result<()>
    where
        F: Fn(&mut RuntimeTaskRecord, i64, i64) -> Result<()>,
    {
        self.update_root_records_with_policy(id, FailedRootPolicy::Exclude, mutator)
            .await
    }

    async fn update_retry_root_records<F>(&self, id: &EngineTaskId, mutator: F) -> Result<()>
    where
        F: Fn(&mut RuntimeTaskRecord, i64, i64) -> Result<()>,
    {
        self.update_root_records_with_policy(id, FailedRootPolicy::Include, mutator)
            .await
    }

    async fn update_root_records_with_policy<F>(
        &self,
        id: &EngineTaskId,
        failed_policy: FailedRootPolicy,
        mutator: F,
    ) -> Result<()>
    where
        F: Fn(&mut RuntimeTaskRecord, i64, i64) -> Result<()>,
    {
        let root_ref = Self::root_task_ref(id);
        let records = self.runtime.find_tasks_by_task_ref(&root_ref).await?;
        if records.is_empty() {
            anyhow::bail!("runtime task not registered for task ref {root_ref}");
        }
        let mut records = self.matching_root_records(id, records, failed_policy)?;
        let updated_at = now_ts();
        let observed_at_ms = now_ms();
        for record in &mut records {
            mutator(record, updated_at, observed_at_ms)?;
            record.updated_at = updated_at;
            self.runtime.upsert_task(record).await?;
        }
        Ok(())
    }

    async fn load_root_record(&self, id: &EngineTaskId) -> Result<Option<RuntimeTaskRecord>> {
        let root_ref = Self::root_task_ref(id);
        let records = self.runtime.find_tasks_by_task_ref(&root_ref).await?;
        Ok(self
            .matching_active_root_records(id, records)?
            .into_iter()
            .next())
    }

    fn matching_active_root_records(
        &self,
        id: &EngineTaskId,
        records: Vec<RuntimeTaskRecord>,
    ) -> Result<Vec<RuntimeTaskRecord>> {
        self.matching_root_records(id, records, FailedRootPolicy::Exclude)
    }

    fn matching_root_records(
        &self,
        id: &EngineTaskId,
        records: Vec<RuntimeTaskRecord>,
        failed_policy: FailedRootPolicy,
    ) -> Result<Vec<RuntimeTaskRecord>> {
        records
            .into_iter()
            .filter_map(|record| match self.record_matches_observer(id, &record) {
                Ok(true) if !is_terminal_status(record.runner_status) => Some(Ok(record)),
                Ok(true)
                    if failed_policy == FailedRootPolicy::Include
                        && record.runner_status == RunnerStatus::Failed =>
                {
                    Some(Ok(record))
                }
                Ok(_) => None,
                Err(err) => Some(Err(err)),
            })
            .collect()
    }

    async fn load_root_record_for_resume(
        &self,
        id: &EngineTaskId,
        task: &EngineTask,
    ) -> Result<Option<RuntimeTaskRecord>> {
        let root_ref = Self::root_task_ref(id);
        let records = self.runtime.find_tasks_by_task_ref(&root_ref).await?;
        let mut resumable_failed = None;

        for record in records {
            if !self.record_matches_observer(id, &record)? {
                continue;
            }
            if !is_terminal_status(record.runner_status) {
                return Ok(Some(record));
            }
            if record.runner_status == RunnerStatus::Failed
                && Self::record_has_resumable_remote_submission_for_task(&record, id, task)?
            {
                resumable_failed.get_or_insert(record);
            }
        }

        Ok(resumable_failed)
    }

    fn record_has_resumable_remote_submission_for_task(
        record: &RuntimeTaskRecord,
        id: &EngineTaskId,
        task: &EngineTask,
    ) -> Result<bool> {
        let metadata: TaskMetadata = serde_json::from_value(record.metadata.clone())
            .context("failed to parse runtime task metadata")?;
        let task_ref = Self::root_task_ref(id);
        Ok(match task {
            EngineTask::ProveProposal { .. } => metadata
                .proposal_runtime(&task_ref)
                .is_some_and(TaskRuntimeMetadata::has_resumable_remote_submission),
            EngineTask::Aggregate { .. } => metadata
                .aggregate_runtime()
                .is_some_and(TaskRuntimeMetadata::has_resumable_remote_submission),
            EngineTask::Preflight { .. }
            | EngineTask::Validate { .. }
            | EngineTask::Encode { .. }
            | EngineTask::Proposal { .. } => false,
        })
    }

    fn record_matches_observer(
        &self,
        id: &EngineTaskId,
        record: &RuntimeTaskRecord,
    ) -> Result<bool> {
        if record.pipeline_key != id.0.pipeline_key() {
            return Ok(false);
        }
        let metadata: TaskMetadata = serde_json::from_value(record.metadata.clone())
            .context("failed to parse runtime task metadata")?;
        Ok(metadata.network_pair == self.network_pair)
    }

    fn metric_context(record: &RuntimeTaskRecord) -> Result<MetricContext> {
        let metadata: TaskMetadata = serde_json::from_value(record.metadata.clone())
            .context("failed to parse runtime task metadata for telemetry")?;
        Ok(MetricContext::new(
            record.route.to_string(),
            metadata.proof_type,
            metadata.network_pair,
            metadata.aggregate_requested,
        ))
    }

    const fn submission_provider(progress: &ProverProgress) -> &'static str {
        match progress {
            ProverProgress::BoundlessSubmission(_) => "boundless",
            ProverProgress::Sp1NetworkSubmission(_) => "sp1_network",
        }
    }

    const fn stage_name_from_task_id(id: &EngineTaskId) -> &'static str {
        match &id.0 {
            EngineTaskKey::Proposal { .. } => "proposal",
            EngineTaskKey::Aggregate { .. } => "aggregate",
        }
    }

    const fn proposal_stage_from_task(task: &EngineTask) -> Option<ProposalStage> {
        match task {
            EngineTask::Preflight { .. } => Some(ProposalStage::Preflight),
            EngineTask::Validate { .. } => Some(ProposalStage::Validation),
            EngineTask::Encode { .. } => Some(ProposalStage::Encode),
            EngineTask::ProveProposal { .. } => Some(ProposalStage::Prove),
            EngineTask::Proposal { .. } | EngineTask::Aggregate { .. } => None,
        }
    }

    fn timing_key_for_stage(id: &EngineTaskId, stage: ProposalStage) -> String {
        stage_task_ref_for_stage(id, stage)
    }

    fn timing_key_for_stage_name(id: &EngineTaskId, stage: &str) -> String {
        match stage {
            "preflight" => Self::timing_key_for_stage(id, ProposalStage::Preflight),
            "validation" => Self::timing_key_for_stage(id, ProposalStage::Validation),
            "encode" => Self::timing_key_for_stage(id, ProposalStage::Encode),
            "prove" => Self::timing_key_for_stage(id, ProposalStage::Prove),
            _ => match &id.0 {
                EngineTaskKey::Proposal { .. } => {
                    Self::timing_key_for_stage(id, ProposalStage::Prove)
                }
                EngineTaskKey::Aggregate { pipeline, request } => {
                    crate::server::task_metadata::aggregate_task_ref(*pipeline, request)
                }
            },
        }
    }

    fn timing_key_for_task(id: &EngineTaskId, task: &EngineTask) -> String {
        Self::proposal_stage_from_task(task).map_or_else(
            || match &id.0 {
                EngineTaskKey::Proposal { .. } => {
                    Self::timing_key_for_stage(id, ProposalStage::Prove)
                }
                EngineTaskKey::Aggregate { pipeline, request } => {
                    crate::server::task_metadata::aggregate_task_ref(*pipeline, request)
                }
            },
            |stage| Self::timing_key_for_stage(id, stage),
        )
    }

    fn mark_stage_started_for_metrics(&self, id: &EngineTaskId, task: &EngineTask) -> bool {
        let task_id = Self::timing_key_for_task(id, task);
        let mut started = self
            .started_stage_tasks
            .lock()
            .expect("stage task telemetry mutex poisoned");
        started.insert(task_id)
    }

    fn stage_duration_secs(
        record: &RuntimeTaskRecord,
        task_id: &str,
        finished_at_ms: i64,
    ) -> Result<Option<f64>> {
        let metadata: TaskMetadata = serde_json::from_value(record.metadata.clone())
            .context("failed to parse runtime task metadata for stage duration")?;
        Ok(metadata.observe_stage_terminal_duration_secs(task_id, finished_at_ms))
    }

    async fn observe_stage_terminal_metrics(
        &self,
        id: &EngineTaskId,
        task_id: &str,
        stage: &str,
        status: &str,
        finished_at_ms: i64,
        failure_error: Option<&str>,
    ) {
        let should_decrement = {
            let mut started = self
                .started_stage_tasks
                .lock()
                .expect("stage task telemetry mutex poisoned");
            started.remove(task_id)
        };
        match self.load_root_record(id).await {
            Ok(Some(record)) => match Self::metric_context(&record) {
                Ok(context) => {
                    telemetry::record_stage_task_terminal(
                        &context,
                        stage,
                        status,
                        should_decrement,
                    );
                    if let Some(error) = failure_error {
                        telemetry::record_stage_task_failure(&context, stage, error);
                    }
                    match Self::stage_duration_secs(&record, task_id, finished_at_ms) {
                        Ok(Some(duration_seconds)) => {
                            telemetry::record_stage_task_duration(
                                &context,
                                stage,
                                status,
                                duration_seconds,
                            );
                        }
                        Ok(None) => {}
                        Err(err) => tracing::warn!(
                            task = ?id,
                            error = %err,
                            "failed to derive stage duration"
                        ),
                    }
                }
                Err(err) => {
                    tracing::warn!(task = ?id, error = %err, "failed to build telemetry context");
                }
            },
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(
                    task = ?id,
                    error = %err,
                    "failed to load runtime task for telemetry"
                );
            }
        }
    }

    async fn write_final_proof_files(
        &self,
        id: &EngineTaskId,
        stage: &str,
        proof: &raiko2_primitives::Proof,
    ) -> Result<HashMap<String, String>> {
        let root_ref = Self::root_task_ref(id);
        let records = self.runtime.find_tasks_by_task_ref(&root_ref).await?;
        if records.is_empty() {
            anyhow::bail!("runtime task not registered for task ref {root_ref}");
        }
        let records = self.matching_active_root_records(id, records)?;
        let Some(first_record) = records.first() else {
            return Ok(HashMap::new());
        };

        let proof_bytes =
            serde_json::to_vec_pretty(proof).context("failed to serialize proof output")?;
        let proof_path = self
            .runtime
            .write_proof_artifact_bytes(&self.network_pair, &root_ref, &proof_bytes)
            .await
            .context("failed to write proof artifact")?;
        let proof_path = proof_path.display().to_string();
        self.runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: self.network_pair.clone(),
                proof_ref: root_ref,
                pipeline_key: first_record.pipeline_key,
                route: first_record.route,
                proof_path: proof_path.clone(),
            })
            .await
            .context("failed to register proof artifact")?;

        let mut proof_paths = HashMap::with_capacity(records.len());
        for record in records {
            let mut metadata: TaskMetadata = serde_json::from_value(record.metadata.clone())
                .context("failed to parse task metadata")?;
            let task_id = Self::timing_key_for_stage_name(id, stage);
            metadata.mark_stage_terminal(&task_id, stage, 0, "completed");
            if !Self::root_completed_by_proof_success(id, &metadata, record.pipeline_key) {
                continue;
            }

            proof_paths.insert(record.task_id, proof_path.clone());
        }
        Ok(proof_paths)
    }

    async fn mark_proof_persistence_failed(
        &self,
        id: &EngineTaskId,
        stage: &'static str,
        message: &str,
    ) {
        let message = message.to_string();
        if let Err(sync_err) = self
            .update_root_records(id, |record, updated_at, observed_at_ms| {
                record.runner_status = RunnerStatus::Failed;
                record.error = Some(message.clone());
                let task_id = Self::timing_key_for_stage_name(id, stage);
                update_task_metadata(record, |metadata| {
                    metadata.mark_stage_terminal(&task_id, stage, observed_at_ms, "failed");
                    metadata.runtime.active_stage = Some(stage.to_string());
                    metadata.runtime.last_event = Some("failed".to_string());
                })?;
                record.updated_at = updated_at;
                Ok(())
            })
            .await
        {
            tracing::warn!(
                task = ?id,
                stage,
                error = %sync_err,
                "failed to sync proof persistence failure"
            );
        }
    }

    async fn handle_proof_success(
        &self,
        id: &EngineTaskId,
        task: &EngineTask,
        stage: &'static str,
        finished_at_ms: i64,
        proof: &raiko2_primitives::Proof,
    ) {
        let task_id = Self::timing_key_for_task(id, task);
        let proof_paths = match self.write_final_proof_files(id, stage, proof).await {
            Ok(paths) => {
                self.observe_stage_terminal_metrics(
                    id,
                    &task_id,
                    stage,
                    "completed",
                    finished_at_ms,
                    None,
                )
                .await;
                paths
            }
            Err(err) => {
                let message = format!("failed to persist proof output: {err}");
                tracing::warn!(
                    task = ?id,
                    stage,
                    error = %err,
                    "failed to persist proof output"
                );
                self.observe_stage_terminal_metrics(
                    id,
                    &task_id,
                    stage,
                    "failed",
                    finished_at_ms,
                    Some(message.as_str()),
                )
                .await;
                self.mark_proof_persistence_failed(id, stage, message.as_str())
                    .await;
                return;
            }
        };

        if let Err(err) = self
            .update_root_records(id, move |record, updated_at, observed_at_ms| {
                record.error = None;
                let task_id = Self::timing_key_for_task(id, task);
                let mut metadata: TaskMetadata = serde_json::from_value(record.metadata.clone())
                    .context("failed to parse task metadata")?;
                metadata.mark_stage_terminal(&task_id, stage, observed_at_ms, "completed");
                let root_completed =
                    Self::root_completed_by_proof_success(id, &metadata, record.pipeline_key);
                metadata.runtime.active_stage = Some(stage.to_string());
                metadata.runtime.last_event = Some(
                    if root_completed {
                        "completed"
                    } else {
                        "stage_completed"
                    }
                    .to_string(),
                );
                record.metadata =
                    serde_json::to_value(metadata).context("failed to serialize task metadata")?;
                if root_completed {
                    record.runner_status = RunnerStatus::Completed;
                    record.proof_path = proof_paths.get(&record.task_id).cloned();
                } else {
                    record.runner_status = RunnerStatus::Allocated;
                    record.proof_path = None;
                }
                record.updated_at = updated_at;
                Ok(())
            })
            .await
        {
            tracing::warn!(
                task = ?id,
                stage,
                error = %err,
                "failed to sync runtime task success"
            );
        }
    }

    fn root_completed_by_proof_success(
        id: &EngineTaskId,
        metadata: &TaskMetadata,
        pipeline_key: raiko2_pipeline::PipelineKey,
    ) -> bool {
        match &id.0 {
            EngineTaskKey::Aggregate { .. } => true,
            EngineTaskKey::Proposal { .. }
                if !metadata.aggregate_requested && metadata.aggregate_task_id.is_none() =>
            {
                !metadata.proposals.is_empty()
                    && metadata.proposals.iter().all(|proposal| {
                        let Some(task_id) = proposal.engine_task_id(pipeline_key) else {
                            return false;
                        };
                        let task_ref = Self::timing_key_for_stage(&task_id, ProposalStage::Prove);
                        metadata
                            .runtime
                            .stage_timings
                            .get(&task_ref)
                            .and_then(|timing| timing.terminal_status.as_deref())
                            == Some("completed")
                    })
            }
            EngineTaskKey::Proposal { .. } => false,
        }
    }
}

#[async_trait]
impl EngineObserver for RuntimeObserver {
    async fn on_task_started(&self, id: &EngineTaskId, task: &EngineTask, worker: &str) {
        let stage = Self::stage_name(task);
        let should_increment = self.mark_stage_started_for_metrics(id, task);
        match self.load_root_record(id).await {
            Ok(Some(record)) => match Self::metric_context(&record) {
                Ok(context) => {
                    if should_increment {
                        telemetry::record_stage_task_started(&context, stage);
                    }
                }
                Err(err) => {
                    tracing::warn!(task = ?id, error = %err, "failed to build telemetry context");
                }
            },
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(
                    task = ?id,
                    error = %err,
                    "failed to load runtime task for telemetry"
                );
            }
        }
        if let Err(err) = self
            .update_retry_root_records(id, |record, updated_at, observed_at_ms| {
                record.runner_status = RunnerStatus::Allocated;
                record.error = None;
                let task_id = Self::timing_key_for_task(id, task);
                update_task_metadata(record, |metadata| {
                    metadata.mark_stage_started(&task_id, stage, observed_at_ms);
                    metadata.runtime.active_stage = Some(stage.to_string());
                    metadata.runtime.last_event = Some(format!("started:{worker}"));
                })?;
                record.updated_at = updated_at;
                Ok(())
            })
            .await
        {
            tracing::warn!(task = ?id, error = %err, "failed to sync runtime task start");
        }
    }

    async fn on_task_progress(
        &self,
        id: &EngineTaskId,
        task: &EngineTask,
        progress: &ProverProgress,
    ) -> RaikoResult<()> {
        let stage = Self::stage_name(task);
        match self.load_root_record(id).await {
            Ok(Some(record)) => match Self::metric_context(&record) {
                Ok(context) => telemetry::record_external_submission(
                    &context,
                    stage,
                    Self::submission_provider(progress),
                ),
                Err(err) => {
                    tracing::warn!(task = ?id, error = %err, "failed to build telemetry context");
                }
            },
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(
                    task = ?id,
                    error = %err,
                    "failed to load runtime task for telemetry"
                );
            }
        }
        self.update_retry_root_records(id, |record, updated_at, _observed_at_ms| {
            record.runner_status = RunnerStatus::Allocated;
            record.error = None;
            record.provider_request_id = match progress {
                ProverProgress::BoundlessSubmission(submission) => {
                    Some(submission.provider_request_id.clone())
                }
                ProverProgress::Sp1NetworkSubmission(submission) => {
                    Some(submission.provider_request_id.clone())
                }
            };
            let task_id = Self::root_task_ref(id);
            update_task_metadata(record, |metadata| {
                metadata.runtime.active_stage = Some(stage.to_string());
                metadata.runtime.last_event = Some("submission_registered".to_string());
                match progress {
                    ProverProgress::BoundlessSubmission(submission) => match task {
                        EngineTask::ProveProposal { .. } => {
                            metadata.upsert_proposal_runtime(&task_id, submission, updated_at);
                        }
                        EngineTask::Aggregate { .. } => {
                            metadata.upsert_aggregate_runtime(submission, updated_at);
                        }
                        EngineTask::Preflight { .. }
                        | EngineTask::Validate { .. }
                        | EngineTask::Encode { .. }
                        | EngineTask::Proposal { .. } => {}
                    },
                    ProverProgress::Sp1NetworkSubmission(submission) => match task {
                        EngineTask::ProveProposal { .. } => {
                            metadata.upsert_proposal_sp1_network_runtime(
                                &task_id, submission, updated_at,
                            );
                        }
                        EngineTask::Aggregate { .. } => {
                            metadata.upsert_aggregate_sp1_network_runtime(submission, updated_at);
                        }
                        EngineTask::Preflight { .. }
                        | EngineTask::Validate { .. }
                        | EngineTask::Encode { .. }
                        | EngineTask::Proposal { .. } => {}
                    },
                }
            })?;
            record.updated_at = updated_at;
            Ok(())
        })
        .await
        .map_err(RaikoError::from)
    }

    async fn on_task_succeeded(
        &self,
        id: &EngineTaskId,
        task: &EngineTask,
        success: &EngineTaskSuccess,
    ) {
        let stage = Self::stage_name(task);
        let finished_at_ms = now_ms();
        let task_id = Self::timing_key_for_task(id, task);
        let result = match success {
            EngineTaskSuccess::Proof { proof, .. } => {
                self.handle_proof_success(id, task, stage, finished_at_ms, proof)
                    .await;
                return;
            }
            EngineTaskSuccess::GuestInput { stage } | EngineTaskSuccess::EncodedInput { stage } => {
                self.observe_stage_terminal_metrics(
                    id,
                    &task_id,
                    Self::stage_name(task),
                    "completed",
                    finished_at_ms,
                    None,
                )
                .await;
                self.update_root_records(id, |record, updated_at, observed_at_ms| {
                    record.runner_status = RunnerStatus::Allocated;
                    record.error = None;
                    let task_id = Self::timing_key_for_task(id, task);
                    update_task_metadata(record, |metadata| {
                        metadata.mark_stage_terminal(
                            &task_id,
                            Self::stage_name(task),
                            observed_at_ms,
                            "completed",
                        );
                        metadata.runtime.active_stage =
                            Some(stage_name_from_pipeline_stage(*stage).to_string());
                        metadata.runtime.last_event = Some("stage_completed".to_string());
                    })?;
                    record.updated_at = updated_at;
                    Ok(())
                })
                .await
            }
        };

        if let Err(err) = result {
            tracing::warn!(
                task = ?id,
                stage,
                error = %err,
                "failed to sync runtime task success"
            );
        }
    }

    async fn on_task_failed(&self, id: &EngineTaskId, task: &EngineTask, error: &str) {
        let stage = Self::stage_name(task);
        let finished_at_ms = now_ms();
        let task_id = Self::timing_key_for_task(id, task);
        self.observe_stage_terminal_metrics(
            id,
            &task_id,
            stage,
            "failed",
            finished_at_ms,
            Some(error),
        )
        .await;
        if let Err(err) = self
            .update_root_records(id, |record, updated_at, observed_at_ms| {
                record.runner_status = RunnerStatus::Failed;
                record.error = Some(error.to_string());
                update_task_metadata(record, |metadata| {
                    metadata.mark_stage_terminal(&task_id, stage, observed_at_ms, "failed");
                    metadata.runtime.active_stage = Some(stage.to_string());
                    metadata.runtime.last_event = Some("failed".to_string());
                })?;
                record.updated_at = updated_at;
                Ok(())
            })
            .await
        {
            tracing::warn!(task = ?id, error = %err, "failed to sync runtime task failure");
        }
    }

    async fn on_task_cancelled(&self, id: &EngineTaskId) {
        let stage = Self::stage_name_from_task_id(id);
        let finished_at_ms = now_ms();
        let task_id = Self::timing_key_for_stage_name(id, stage);
        self.observe_stage_terminal_metrics(id, &task_id, stage, "cancelled", finished_at_ms, None)
            .await;
        if let Err(err) = self
            .update_root_records(id, |record, updated_at, observed_at_ms| {
                record.runner_status = RunnerStatus::Cancelled;
                record.error = None;
                update_task_metadata(record, |metadata| {
                    metadata.mark_stage_terminal(&task_id, stage, observed_at_ms, "cancelled");
                    metadata.runtime.active_stage = Some(stage.to_string());
                    metadata.runtime.last_event = Some("cancelled".to_string());
                })?;
                record.updated_at = updated_at;
                Ok(())
            })
            .await
        {
            tracing::warn!(
                task = ?id,
                error = %err,
                "failed to sync runtime task cancellation"
            );
        }
    }

    async fn load_sp1_network_request_id(
        &self,
        id: &EngineTaskId,
        task: &EngineTask,
    ) -> RaikoResult<Option<String>> {
        // Like the Boundless twin below, read and parse failures propagate instead of degrading
        // to `Ok(None)`: `None` makes the SP1 prover mint a fresh network request, so a transient
        // storage error must not discard a live accepted id.
        let record = match task {
            EngineTask::ProveProposal { .. } | EngineTask::Aggregate { .. } => {
                self.load_root_record_for_resume(id, task).await?
            }
            EngineTask::Preflight { .. }
            | EngineTask::Validate { .. }
            | EngineTask::Encode { .. }
            | EngineTask::Proposal { .. } => return Ok(None),
        };
        let Some(record) = record else {
            return Ok(None);
        };

        let metadata: TaskMetadata =
            serde_json::from_value(record.metadata).context("failed to parse task metadata")?;
        let task_id = Self::root_task_ref(id);
        Ok(match task {
            EngineTask::ProveProposal { .. } => metadata
                .proposal_runtime(&task_id)
                .and_then(TaskRuntimeMetadata::sp1_network_request_id),
            // Proposal and aggregation must keep distinct SP1 network requests. Reusing the
            // root-level request id causes aggregate=true flows to resume the proposal request
            // instead of creating a new aggregation request.
            EngineTask::Aggregate { .. } => metadata
                .aggregate_runtime()
                .and_then(TaskRuntimeMetadata::sp1_network_request_id),
            EngineTask::Preflight { .. }
            | EngineTask::Validate { .. }
            | EngineTask::Encode { .. }
            | EngineTask::Proposal { .. } => None,
        })
    }

    async fn load_boundless_submission(
        &self,
        id: &EngineTaskId,
        task: &EngineTask,
    ) -> RaikoResult<Option<BoundlessSubmissionSnapshot>> {
        let record = match task {
            EngineTask::ProveProposal { .. } | EngineTask::Aggregate { .. } => {
                self.load_root_record_for_resume(id, task).await?
            }
            EngineTask::Preflight { .. }
            | EngineTask::Validate { .. }
            | EngineTask::Encode { .. }
            | EngineTask::Proposal { .. } => return Ok(None),
        };
        let Some(record) = record else {
            return Ok(None);
        };

        let metadata: TaskMetadata =
            serde_json::from_value(record.metadata).context("failed to parse task metadata")?;
        let task_id = Self::root_task_ref(id);
        let runtime = match task {
            EngineTask::ProveProposal { .. } => metadata.proposal_runtime(&task_id),
            EngineTask::Aggregate { .. } => metadata.aggregate_runtime(),
            EngineTask::Preflight { .. }
            | EngineTask::Validate { .. }
            | EngineTask::Encode { .. }
            | EngineTask::Proposal { .. } => None,
        };
        let Some(runtime) = runtime else {
            return Ok(None);
        };

        // Expired records must still reach the prover: it gives them one final market status
        // read (an expired-but-fulfilled request still reports Fulfilled, recovering a proof
        // that is already paid for) and otherwise counts the stored attempt against the rebid
        // budget. Dropping them here would reset the budget and the price-escalation ladder on
        // every restart after expiry.
        Ok(runtime.boundless_submission_for_resume(now_secs()))
    }
}

fn update_task_metadata<F>(record: &mut RuntimeTaskRecord, mutator: F) -> Result<()>
where
    F: FnOnce(&mut TaskMetadata),
{
    let mut metadata: TaskMetadata =
        serde_json::from_value(record.metadata.clone()).context("failed to parse task metadata")?;
    mutator(&mut metadata);
    record.metadata =
        serde_json::to_value(metadata).context("failed to serialize task metadata")?;
    Ok(())
}

const fn is_terminal_status(status: RunnerStatus) -> bool {
    matches!(
        status,
        RunnerStatus::Completed | RunnerStatus::Failed | RunnerStatus::Cancelled
    )
}

const fn stage_name_from_pipeline_stage(stage: raiko2_pipeline::PipelineStage) -> &'static str {
    match stage {
        raiko2_pipeline::PipelineStage::Preflight => "preflight",
        raiko2_pipeline::PipelineStage::Validation => "validation",
        raiko2_pipeline::PipelineStage::Encode => "encode",
        raiko2_pipeline::PipelineStage::Prove => "prove",
        raiko2_pipeline::PipelineStage::Aggregate => "aggregate",
    }
}

fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .cast_signed()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn now_ms() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::task_metadata::{
        ProposalTask, RuntimeMetadata, aggregate_task_ref, proposal_task_ref, stage_task_ref,
    };
    use crate::server::telemetry;
    use raiko2_engine::{AggregationTaskRequest, ProposalTaskRequest};
    use raiko2_pipeline::PipelineKey;
    use raiko2_primitives::ProofType;
    use raiko2_prover::{
        BoundlessSubmissionProgress, BoundlessSubmissionSnapshot, Sp1FulfillmentStrategy,
        Sp1NetworkMode, Sp1NetworkSubmissionProgress, sp1::ExecutionMode,
    };
    use raiko2_runtime::TaskRegistration;

    fn unique_runtime_root(prefix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "{prefix}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ))
    }

    fn proposal_request() -> ProposalTaskRequest {
        ProposalTaskRequest {
            proposal_id: 42,
            l2_block_range: None,
            l1_inclusion_block_number: 1,
            last_anchor_block_number: 0,
            checkpoint: None,
            blob_proof_type: None,
            prover: None,
            graffiti: None,
            prover_config: ProverTaskConfig::default(),
        }
    }

    fn proposal_request_with_id(proposal_id: u64) -> ProposalTaskRequest {
        ProposalTaskRequest {
            proposal_id,
            ..proposal_request()
        }
    }

    fn proposal_metadata_task(
        pipeline: PipelineKey,
        request: &ProposalTaskRequest,
    ) -> ProposalTask {
        let task_ref = proposal_task_ref(pipeline, request);
        ProposalTask {
            proposal_id: request.proposal_id,
            checkpoint: None,
            l1_inclusion_block_number: request.l1_inclusion_block_number,
            l2_block_numbers: vec![request.proposal_id],
            last_anchor_block_number: request.last_anchor_block_number,
            task_id: task_ref,
            request: Some(request.clone()),
        }
    }

    fn proof_fixture() -> raiko2_primitives::Proof {
        raiko2_primitives::Proof {
            proof: Some("0xproof".to_string()),
            input: None,
            quote: None,
            uuid: None,
            kzg_proof: None,
            extra_data: None,
        }
    }

    async fn register_observer_task(
        runtime: &RuntimeManager,
        task_id: &str,
        network_pair: &str,
        pipeline: PipelineKey,
        request: &ProposalTaskRequest,
        runner_status: RunnerStatus,
    ) -> Result<()> {
        let task_ref = proposal_task_ref(pipeline, request);
        let (network, l1_network) = network_pair
            .split_once('/')
            .unwrap_or((network_pair, "ethereum"));
        runtime
            .register_task(TaskRegistration {
                task_id: task_id.to_string(),
                pipeline_key: Some(pipeline),
                route: "native/local"
                    .parse::<PipelineRoute>()
                    .expect("parse route"),
                task_kind: "hoodi_batch".to_string(),
                proposal_id: Some(request.proposal_id),
                proof_ids: vec![task_ref.clone()],
                metadata: serde_json::to_value(TaskMetadata {
                    network_pair: network_pair.to_string(),
                    network: network.to_string(),
                    l1_network: l1_network.to_string(),
                    proof_type: ProofType::Native,
                    requested_proof_type: None,
                    prover_type: None,
                    execution_mode: None,
                    aggregate_requested: false,
                    proposals: vec![proposal_metadata_task(pipeline, request)],
                    aggregate_task_id: None,
                    aggregate_request: None,
                    aggregate_input_artifacts: Vec::new(),
                    runtime: RuntimeMetadata::default(),
                })?,
                request_fingerprint: None,
            })
            .await?;
        let mut record = runtime.get_task(task_id).await?.expect("runtime task");
        record.runner_status = runner_status;
        runtime.upsert_task(&record).await?;
        Ok(())
    }

    fn proposal_prove_task(id: &EngineTaskId) -> EngineTask {
        EngineTask::ProveProposal {
            request: proposal_request(),
            input_task: id.clone(),
        }
    }

    fn boundless_progress() -> ProverProgress {
        let expires_at = now_secs().saturating_add(3_600);
        ProverProgress::BoundlessSubmission(BoundlessSubmissionProgress {
            snapshot: BoundlessSubmissionSnapshot::new(
                "0x1234".to_string(),
                None,
                expires_at,
                expires_at.saturating_sub(600),
                expires_at.saturating_sub(300),
                2,
                "777".to_string(),
                3,
            ),
            image_ref: "0ximage".to_string(),
            deployment: "base".to_string(),
            offchain: true,
            quoted_mcycles_count: Some(6_000),
            evaluated_mcycles_count: Some(12_345),
        })
    }

    #[tokio::test]
    async fn runtime_observer_progress_write_errors_propagate() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-progress-error",
        ))?);
        let observer = RuntimeObserver::new(runtime, "taiko_dev/ethereum".to_string());
        let id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaRisc0Network,
            request: proposal_request(),
        });

        let err = observer
            .on_task_progress(&id, &proposal_prove_task(&id), &boundless_progress())
            .await
            .expect_err("missing runtime record must be observable to the prover");

        assert!(
            err.to_string().contains("runtime task not registered"),
            "{err}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn boundless_resume_runtime_read_errors_propagate() -> Result<()> {
        let root = unique_runtime_root("runtime-observer-read-error");
        let runtime = Arc::new(RuntimeManager::new(root.clone())?);
        std::fs::create_dir(root.join("state/runtime.sqlite"))?;
        let observer = RuntimeObserver::new(runtime, "taiko_dev/ethereum".to_string());
        let id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaRisc0Network,
            request: proposal_request(),
        });

        let err = observer
            .load_boundless_submission(&id, &proposal_prove_task(&id))
            .await
            .expect_err("runtime database read failure must not become no resume state");

        assert!(err.to_string().contains("runtime sqlite database"), "{err}");
        Ok(())
    }

    #[tokio::test]
    async fn boundless_resume_metadata_decode_errors_propagate() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-metadata-error",
        ))?);
        let request = proposal_request();
        register_observer_task(
            &runtime,
            "task_metadata_error",
            "taiko_dev/ethereum",
            PipelineKey::ShastaNative,
            &request,
            RunnerStatus::Allocated,
        )
        .await?;
        let mut record = runtime
            .get_task("task_metadata_error")
            .await?
            .expect("runtime task");
        record.metadata = serde_json::json!({"network_pair": "taiko_dev/ethereum"});
        runtime.upsert_task(&record).await?;
        let observer = RuntimeObserver::new(runtime, "taiko_dev/ethereum".to_string());
        let id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaNative,
            request,
        });

        let err = observer
            .load_boundless_submission(&id, &proposal_prove_task(&id))
            .await
            .expect_err("metadata decode failure must not become no resume state");

        assert!(
            err.to_string()
                .contains("failed to parse runtime task metadata"),
            "{err}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn boundless_resume_genuine_absence_is_allowed() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-no-resume-record",
        ))?);
        let observer = RuntimeObserver::new(runtime, "taiko_dev/ethereum".to_string());
        let id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaRisc0Network,
            request: proposal_request(),
        });

        let submission = observer
            .load_boundless_submission(&id, &proposal_prove_task(&id))
            .await?;

        assert!(submission.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn sp1_resume_runtime_read_errors_propagate() -> Result<()> {
        let root = unique_runtime_root("runtime-observer-sp1-read-error");
        let runtime = Arc::new(RuntimeManager::new(root.clone())?);
        std::fs::create_dir(root.join("state/runtime.sqlite"))?;
        let observer = RuntimeObserver::new(runtime, "taiko_dev/ethereum".to_string());
        let id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaSp1,
            request: proposal_request(),
        });

        let err = observer
            .load_sp1_network_request_id(&id, &proposal_prove_task(&id))
            .await
            .expect_err("runtime database read failure must not become a fresh SP1 request");

        assert!(err.to_string().contains("runtime sqlite database"), "{err}");
        Ok(())
    }

    #[tokio::test]
    async fn sp1_resume_metadata_decode_errors_propagate() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-sp1-metadata-error",
        ))?);
        let request = proposal_request();
        register_observer_task(
            &runtime,
            "task_sp1_metadata_error",
            "taiko_dev/ethereum",
            PipelineKey::ShastaNative,
            &request,
            RunnerStatus::Allocated,
        )
        .await?;
        let mut record = runtime
            .get_task("task_sp1_metadata_error")
            .await?
            .expect("runtime task");
        record.metadata = serde_json::json!({"network_pair": "taiko_dev/ethereum"});
        runtime.upsert_task(&record).await?;
        let observer = RuntimeObserver::new(runtime, "taiko_dev/ethereum".to_string());
        let id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaNative,
            request,
        });

        let err = observer
            .load_sp1_network_request_id(&id, &proposal_prove_task(&id))
            .await
            .expect_err("metadata decode failure must not become a fresh SP1 request");

        assert!(
            err.to_string()
                .contains("failed to parse runtime task metadata"),
            "{err}"
        );
        Ok(())
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn runtime_observer_records_boundless_submission_metadata_immediately() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer",
        ))?);
        let proposal_task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaRisc0Network,
            request: proposal_request(),
        });
        let task_ref = proposal_task_ref(PipelineKey::ShastaRisc0Network, &proposal_request());
        runtime
            .register_task(TaskRegistration {
                task_id: "task_public".to_string(),
                pipeline_key: None,
                route: "risc0/network"
                    .parse::<PipelineRoute>()
                    .expect("parse route"),
                task_kind: "hoodi_batch".to_string(),
                proposal_id: Some(42),
                proof_ids: vec![task_ref.clone()],
                metadata: serde_json::to_value(TaskMetadata {
                    network_pair: "taiko_dev/ethereum".to_string(),
                    network: "taiko_dev".to_string(),
                    l1_network: "ethereum".to_string(),
                    proof_type: ProofType::Risc0,
                    requested_proof_type: None,
                    prover_type: None,
                    execution_mode: None,
                    aggregate_requested: false,
                    proposals: vec![ProposalTask {
                        proposal_id: 42,
                        checkpoint: None,
                        l1_inclusion_block_number: 1,
                        l2_block_numbers: vec![42],
                        last_anchor_block_number: 0,
                        task_id: task_ref.clone(),
                        request: Some(proposal_request()),
                    }],
                    aggregate_task_id: None,
                    aggregate_request: None,
                    aggregate_input_artifacts: Vec::new(),
                    runtime: RuntimeMetadata::default(),
                })?,
                request_fingerprint: None,
            })
            .await?;

        let observer = RuntimeObserver::new(Arc::clone(&runtime), "taiko_dev/ethereum".to_string());
        let future_expires_at = now_secs().saturating_add(3_600);
        observer
            .on_task_progress(
                &proposal_task_id,
                &EngineTask::ProveProposal {
                    request: proposal_request(),
                    input_task: proposal_task_id.clone(),
                },
                &ProverProgress::BoundlessSubmission(BoundlessSubmissionProgress {
                    snapshot: BoundlessSubmissionSnapshot::new(
                        "0x1234".to_string(),
                        Some("0xabcd".to_string()),
                        future_expires_at,
                        future_expires_at - 600,
                        future_expires_at - 300,
                        4,
                        "9000000000000".to_string(),
                        3,
                    ),
                    image_ref: "0ximage".to_string(),
                    deployment: "base".to_string(),
                    offchain: false,
                    quoted_mcycles_count: Some(6_000),
                    evaluated_mcycles_count: Some(12_345),
                }),
            )
            .await?;

        let record = runtime
            .get_task("task_public")
            .await?
            .expect("runtime task exists");
        let metadata: TaskMetadata = serde_json::from_value(record.metadata)?;
        let runtime_entry = metadata
            .proposal_runtime(&task_ref)
            .expect("proposal runtime exists");
        assert_eq!(
            metadata.runtime.last_event.as_deref(),
            Some("submission_registered")
        );
        let snapshot = runtime_entry
            .boundless_submission()
            .expect("Boundless snapshot");
        let public = runtime_entry.public_remote_submission();
        assert_eq!(snapshot.provider_request_id, "0x1234");
        assert_eq!(snapshot.remote_tx_hash.as_deref(), Some("0xabcd"));
        assert_eq!(snapshot.expires_at, future_expires_at);
        assert_eq!(snapshot.submitted_at, future_expires_at - 300);
        assert_eq!(public.image_ref.as_deref(), Some("0ximage"));
        assert_eq!(public.quoted_mcycles_count, Some(6_000));
        assert_eq!(public.evaluated_mcycles_count, Some(12_345));
        assert_eq!(snapshot.max_price_multiplier, Some(4));
        assert_eq!(snapshot.max_price_wei.as_deref(), Some("9000000000000"));
        assert_eq!(snapshot.rebid_attempt, 3);
        assert_eq!(snapshot.lock_expires_at, future_expires_at - 600);
        let mut record = runtime
            .get_task("task_public")
            .await?
            .expect("runtime task");
        record.runner_status = RunnerStatus::Failed;
        runtime.upsert_task(&record).await?;
        let resumed = observer
            .load_boundless_submission(
                &proposal_task_id,
                &EngineTask::ProveProposal {
                    request: proposal_request(),
                    input_task: proposal_task_id.clone(),
                },
            )
            .await
            .expect("boundless resume load succeeds")
            .expect("boundless submission can resume");
        assert_eq!(resumed.provider_request_id, "0x1234");
        assert_eq!(resumed.remote_tx_hash.as_deref(), Some("0xabcd"));
        assert_eq!(resumed.expires_at, future_expires_at);
        assert_eq!(resumed.lock_expires_at, future_expires_at - 600);
        assert_eq!(resumed.submitted_at, future_expires_at - 300);
        assert_eq!(resumed.max_price_multiplier, Some(4));
        assert_eq!(resumed.max_price_wei.as_deref(), Some("9000000000000"));
        assert_eq!(resumed.rebid_attempt, 3);

        let mut record = runtime
            .get_task("task_public")
            .await?
            .expect("runtime task");
        let runtime_entry = record
            .metadata
            .get_mut("runtime")
            .and_then(|runtime| runtime.get_mut("proposals"))
            .and_then(serde_json::Value::as_object_mut)
            .and_then(|proposals| proposals.get_mut(&task_ref))
            .expect("proposal runtime exists");
        *runtime_entry = serde_json::json!({
            "updated_at": 123,
            "provider_request_id": "0x1234",
            "remote_tx_hash": "0xabcd",
            "expires_at": future_expires_at
        });
        runtime.upsert_task(&record).await?;
        let before_legacy_resume = now_secs();
        let resumed = observer
            .load_boundless_submission(
                &proposal_task_id,
                &EngineTask::ProveProposal {
                    request: proposal_request(),
                    input_task: proposal_task_id.clone(),
                },
            )
            .await
            .expect("legacy boundless resume load succeeds")
            .expect("legacy boundless submission can resume");
        let after_legacy_resume = now_secs();
        assert_eq!(resumed.provider_request_id, "0x1234");
        assert_eq!(resumed.expires_at, future_expires_at);
        assert_eq!(resumed.lock_expires_at, 0);
        assert!((before_legacy_resume..=after_legacy_resume).contains(&resumed.submitted_at));
        assert_eq!(resumed.max_price_multiplier, None);
        assert_eq!(resumed.rebid_attempt, 0);

        let mut record = runtime
            .get_task("task_public")
            .await?
            .expect("runtime task");
        let expired_at = now_secs().saturating_sub(1);
        let runtime_entry = record
            .metadata
            .get_mut("runtime")
            .and_then(|runtime| runtime.get_mut("proposals"))
            .and_then(serde_json::Value::as_object_mut)
            .and_then(|proposals| proposals.get_mut(&task_ref))
            .and_then(serde_json::Value::as_object_mut)
            .expect("proposal runtime exists");
        runtime_entry.insert("expires_at".to_string(), expired_at.into());
        runtime_entry.insert("rebid_attempt".to_string(), 5.into());
        runtime.upsert_task(&record).await?;
        // Expired records still resume: the prover gives them one final status read (recovering
        // an already-paid fulfillment) and counts the stored attempt against the rebid budget.
        let resumed = observer
            .load_boundless_submission(
                &proposal_task_id,
                &EngineTask::ProveProposal {
                    request: proposal_request(),
                    input_task: proposal_task_id.clone(),
                },
            )
            .await
            .expect("expired boundless resume load succeeds")
            .expect("expired boundless submission still resumes");
        assert_eq!(resumed.provider_request_id, "0x1234");
        assert_eq!(resumed.expires_at, expired_at);
        assert_eq!(resumed.rebid_attempt, 5);
        Ok(())
    }

    #[tokio::test]
    async fn runtime_observer_keeps_aggregate_root_allocated_after_proposal_proof() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-aggregate-pending",
        ))?);
        let pipeline = PipelineKey::ShastaSp1;
        let request = proposal_request();
        let proposal_task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        let proposal_ref = proposal_task_ref(pipeline, &request);
        let aggregate_request = AggregationTaskRequest {
            request_id: "agg-42".to_string(),
            proposal_ids: vec![42],
            prover_config: ProverTaskConfig::default(),
        };
        let aggregate_ref = aggregate_task_ref(pipeline, &aggregate_request);
        runtime
            .register_task(TaskRegistration {
                task_id: "task_public_aggregate_pending".to_string(),
                pipeline_key: None,
                route: "sp1/local".parse::<PipelineRoute>().expect("parse route"),
                task_kind: "hoodi_batch".to_string(),
                proposal_id: Some(42),
                proof_ids: vec![proposal_ref.clone(), aggregate_ref.clone()],
                metadata: serde_json::to_value(TaskMetadata {
                    network_pair: "taiko_dev/ethereum".to_string(),
                    network: "taiko_dev".to_string(),
                    l1_network: "ethereum".to_string(),
                    proof_type: ProofType::Sp1,
                    requested_proof_type: None,
                    prover_type: None,
                    execution_mode: Some(ExecutionMode::Prove),
                    aggregate_requested: true,
                    proposals: vec![proposal_metadata_task(pipeline, &request)],
                    aggregate_task_id: Some(aggregate_ref),
                    aggregate_request: Some(aggregate_request),
                    aggregate_input_artifacts: Vec::new(),
                    runtime: RuntimeMetadata::default(),
                })?,
                request_fingerprint: None,
            })
            .await?;

        let observer = RuntimeObserver::new(Arc::clone(&runtime), "taiko_dev/ethereum".to_string());
        observer
            .on_task_succeeded(
                &proposal_task_id,
                &EngineTask::ProveProposal {
                    request,
                    input_task: proposal_task_id.clone(),
                },
                &EngineTaskSuccess::Proof {
                    stage: raiko2_pipeline::PipelineStage::Prove,
                    proof: proof_fixture(),
                },
            )
            .await;

        let record = runtime
            .get_task("task_public_aggregate_pending")
            .await?
            .expect("runtime task exists");
        let metadata: TaskMetadata = serde_json::from_value(record.metadata)?;
        assert_eq!(record.runner_status, RunnerStatus::Allocated);
        assert_eq!(record.proof_path, None);
        assert_eq!(
            metadata.runtime.last_event.as_deref(),
            Some("stage_completed")
        );
        assert!(
            !tokio::fs::try_exists(Path::new(&record.task_dir).join("proof.json")).await?,
            "proposal proof must not become the root proof for aggregate requests"
        );
        let artifact = runtime
            .get_proof_artifact("taiko_dev/ethereum", &proposal_ref)
            .await?
            .expect("proposal proof artifact");
        assert!(tokio::fs::try_exists(Path::new(&artifact.proof_path)).await?);
        Ok(())
    }

    #[tokio::test]
    async fn runtime_observer_does_not_overwrite_terminal_shared_roots() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-terminal-root",
        ))?);
        let pipeline = PipelineKey::ShastaNative;
        let request = proposal_request();
        let proposal_task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });

        register_observer_task(
            runtime.as_ref(),
            "task_cancelled",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Cancelled,
        )
        .await?;
        register_observer_task(
            runtime.as_ref(),
            "task_active",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Allocated,
        )
        .await?;

        let observer = RuntimeObserver::new(Arc::clone(&runtime), "taiko_dev/ethereum".to_string());
        observer
            .on_task_succeeded(
                &proposal_task_id,
                &EngineTask::ProveProposal {
                    request,
                    input_task: proposal_task_id.clone(),
                },
                &EngineTaskSuccess::Proof {
                    stage: raiko2_pipeline::PipelineStage::Prove,
                    proof: proof_fixture(),
                },
            )
            .await;

        let cancelled = runtime
            .get_task("task_cancelled")
            .await?
            .expect("cancelled task");
        let active = runtime.get_task("task_active").await?.expect("active task");
        assert_eq!(cancelled.runner_status, RunnerStatus::Cancelled);
        assert_eq!(cancelled.proof_path, None);
        assert_eq!(active.runner_status, RunnerStatus::Completed);
        assert!(active.proof_path.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn runtime_observer_filters_matching_task_refs_by_network_pair() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-network-pair",
        ))?);
        let pipeline = PipelineKey::ShastaNative;
        let request = proposal_request();
        let proposal_task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        let task = EngineTask::Preflight {
            request: request.clone(),
        };

        register_observer_task(
            runtime.as_ref(),
            "task_current_pair",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Allocated,
        )
        .await?;
        register_observer_task(
            runtime.as_ref(),
            "task_other_pair",
            "taiko_alt/ethereum",
            pipeline,
            &request,
            RunnerStatus::Allocated,
        )
        .await?;

        let observer = RuntimeObserver::new(Arc::clone(&runtime), "taiko_dev/ethereum".to_string());
        observer
            .on_task_started(&proposal_task_id, &task, "worker-a")
            .await;

        let current = runtime
            .get_task("task_current_pair")
            .await?
            .expect("current pair task");
        let current_metadata: TaskMetadata = serde_json::from_value(current.metadata)?;
        let other = runtime
            .get_task("task_other_pair")
            .await?
            .expect("other pair task");
        let other_metadata: TaskMetadata = serde_json::from_value(other.metadata)?;

        assert_eq!(
            current_metadata.runtime.last_event.as_deref(),
            Some("started:worker-a")
        );
        assert_eq!(other_metadata.runtime.last_event, None);
        Ok(())
    }

    #[tokio::test]
    async fn runtime_observer_retry_start_recovers_failed_root() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-retry-start",
        ))?);
        let pipeline = PipelineKey::ShastaNative;
        let request = proposal_request();
        let proposal_task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        let task = EngineTask::Preflight {
            request: request.clone(),
        };

        register_observer_task(
            runtime.as_ref(),
            "task_failed_retry",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Failed,
        )
        .await?;
        let mut record = runtime
            .get_task("task_failed_retry")
            .await?
            .expect("failed task");
        record.error = Some("transient preflight failure".to_string());
        runtime.upsert_task(&record).await?;

        let observer = RuntimeObserver::new(Arc::clone(&runtime), "taiko_dev/ethereum".to_string());
        observer
            .on_task_started(&proposal_task_id, &task, "worker-a")
            .await;

        let record = runtime
            .get_task("task_failed_retry")
            .await?
            .expect("recovered task");
        let metadata: TaskMetadata = serde_json::from_value(record.metadata)?;
        assert_eq!(record.runner_status, RunnerStatus::Allocated);
        assert_eq!(record.error, None);
        assert_eq!(metadata.runtime.active_stage.as_deref(), Some("preflight"));
        assert_eq!(
            metadata.runtime.last_event.as_deref(),
            Some("started:worker-a")
        );
        Ok(())
    }

    #[tokio::test]
    async fn runtime_observer_completes_non_aggregate_root_after_all_proofs() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-multi-proposal",
        ))?);
        let pipeline = PipelineKey::ShastaSp1;
        let first_request = proposal_request_with_id(42);
        let second_request = proposal_request_with_id(43);
        let first_task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: first_request.clone(),
        });
        let second_task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: second_request.clone(),
        });
        let first_ref = proposal_task_ref(pipeline, &first_request);
        let second_ref = proposal_task_ref(pipeline, &second_request);
        runtime
            .register_task(TaskRegistration {
                task_id: "task_public_multi_proposal".to_string(),
                pipeline_key: None,
                route: "sp1/local".parse::<PipelineRoute>().expect("parse route"),
                task_kind: "hoodi_batch".to_string(),
                proposal_id: None,
                proof_ids: vec![first_ref, second_ref],
                metadata: serde_json::to_value(TaskMetadata {
                    network_pair: "taiko_dev/ethereum".to_string(),
                    network: "taiko_dev".to_string(),
                    l1_network: "ethereum".to_string(),
                    proof_type: ProofType::Sp1,
                    requested_proof_type: None,
                    prover_type: None,
                    execution_mode: Some(ExecutionMode::Prove),
                    aggregate_requested: false,
                    proposals: vec![
                        proposal_metadata_task(pipeline, &first_request),
                        proposal_metadata_task(pipeline, &second_request),
                    ],
                    aggregate_task_id: None,
                    aggregate_request: None,
                    aggregate_input_artifacts: Vec::new(),
                    runtime: RuntimeMetadata::default(),
                })?,
                request_fingerprint: None,
            })
            .await?;

        let observer = RuntimeObserver::new(Arc::clone(&runtime), "taiko_dev/ethereum".to_string());
        observer
            .on_task_succeeded(
                &first_task_id,
                &EngineTask::ProveProposal {
                    request: first_request.clone(),
                    input_task: first_task_id.clone(),
                },
                &EngineTaskSuccess::Proof {
                    stage: raiko2_pipeline::PipelineStage::Prove,
                    proof: proof_fixture(),
                },
            )
            .await;

        let record = runtime
            .get_task("task_public_multi_proposal")
            .await?
            .expect("runtime task exists");
        assert_eq!(record.runner_status, RunnerStatus::Allocated);
        assert_eq!(record.proof_path, None);
        assert!(
            !tokio::fs::try_exists(Path::new(&record.task_dir).join("proof.json")).await?,
            "partial proposal proof must not be persisted as final root proof"
        );
        let first_ref = proposal_task_ref(pipeline, &first_request);
        let first_artifact = runtime
            .get_proof_artifact("taiko_dev/ethereum", &first_ref)
            .await?
            .expect("first proposal proof artifact");
        assert!(tokio::fs::try_exists(Path::new(&first_artifact.proof_path)).await?);

        observer
            .on_task_succeeded(
                &second_task_id,
                &EngineTask::ProveProposal {
                    request: second_request,
                    input_task: second_task_id.clone(),
                },
                &EngineTaskSuccess::Proof {
                    stage: raiko2_pipeline::PipelineStage::Prove,
                    proof: proof_fixture(),
                },
            )
            .await;

        let record = runtime
            .get_task("task_public_multi_proposal")
            .await?
            .expect("runtime task exists");
        let metadata: TaskMetadata = serde_json::from_value(record.metadata)?;
        assert_eq!(record.runner_status, RunnerStatus::Completed);
        assert!(record.proof_path.is_some());
        assert_eq!(metadata.runtime.last_event.as_deref(), Some("completed"));
        let proof_path = record.proof_path.expect("root proof path");
        assert!(tokio::fs::try_exists(Path::new(&proof_path)).await?);
        assert!(!tokio::fs::try_exists(Path::new(&record.task_dir).join("proof.json")).await?);
        Ok(())
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn runtime_observer_records_sp1_network_submission_metadata() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-sp1",
        ))?);
        let proposal_task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaSp1,
            request: proposal_request(),
        });
        let task_ref = proposal_task_ref(PipelineKey::ShastaSp1, &proposal_request());
        runtime
            .register_task(TaskRegistration {
                task_id: "task_public_sp1".to_string(),
                pipeline_key: None,
                route: "sp1/local".parse::<PipelineRoute>().expect("parse route"),
                task_kind: "hoodi_batch".to_string(),
                proposal_id: Some(42),
                proof_ids: vec![task_ref.clone()],
                metadata: serde_json::to_value(TaskMetadata {
                    network_pair: "taiko_dev/ethereum".to_string(),
                    network: "taiko_dev".to_string(),
                    l1_network: "ethereum".to_string(),
                    proof_type: ProofType::Sp1,
                    requested_proof_type: None,
                    prover_type: None,
                    execution_mode: Some(ExecutionMode::Prove),
                    aggregate_requested: false,
                    proposals: vec![ProposalTask {
                        proposal_id: 42,
                        checkpoint: None,
                        l1_inclusion_block_number: 1,
                        l2_block_numbers: vec![42],
                        last_anchor_block_number: 0,
                        task_id: task_ref.clone(),
                        request: Some(proposal_request()),
                    }],
                    aggregate_task_id: None,
                    aggregate_request: None,
                    aggregate_input_artifacts: Vec::new(),
                    runtime: RuntimeMetadata::default(),
                })?,
                request_fingerprint: None,
            })
            .await?;

        let observer = RuntimeObserver::new(Arc::clone(&runtime), "taiko_dev/ethereum".to_string());
        observer
            .on_task_progress(
                &proposal_task_id,
                &EngineTask::ProveProposal {
                    request: proposal_request(),
                    input_task: proposal_task_id.clone(),
                },
                &ProverProgress::Sp1NetworkSubmission(Sp1NetworkSubmissionProgress {
                    provider_request_id: "0xsp1".to_string(),
                    network_mode: Sp1NetworkMode::Reserved,
                    fulfillment_strategy: Sp1FulfillmentStrategy::Reserved,
                    skip_simulation: true,
                    cycle_limit: 1_000_000_000_000,
                    timeout_secs: 3_600,
                    max_price_per_pgu: Some(42),
                    auction_timeout_secs: Some(120),
                }),
            )
            .await?;

        let record = runtime
            .get_task("task_public_sp1")
            .await?
            .expect("runtime task exists");
        let metadata: TaskMetadata = serde_json::from_value(record.metadata)?;
        let runtime_entry = metadata
            .proposal_runtime(&task_ref)
            .expect("proposal runtime exists");
        let public = runtime_entry.public_remote_submission();
        assert_eq!(public.provider_request_id.as_deref(), Some("0xsp1"));
        assert_eq!(public.sp1_network_mode.as_deref(), Some("reserved"));
        assert_eq!(public.sp1_fulfillment_strategy.as_deref(), Some("reserved"));
        assert_eq!(public.sp1_skip_simulation, Some(true));
        assert_eq!(public.sp1_cycle_limit, Some(1_000_000_000_000));
        assert_eq!(public.sp1_timeout_secs, Some(3_600));
        let persisted = serde_json::to_value(runtime_entry)?;
        assert_eq!(
            persisted["remote_submission"]["submission"]["max_price_per_pgu"],
            42
        );
        assert_eq!(
            persisted["remote_submission"]["submission"]["auction_timeout_secs"],
            120
        );
        let mut record = runtime
            .get_task("task_public_sp1")
            .await?
            .expect("runtime task exists");
        record.runner_status = RunnerStatus::Failed;
        runtime.upsert_task(&record).await?;
        assert_eq!(
            observer
                .load_sp1_network_request_id(
                    &proposal_task_id,
                    &EngineTask::ProveProposal {
                        request: proposal_request(),
                        input_task: proposal_task_id.clone(),
                    },
                )
                .await?
                .as_deref(),
            Some("0xsp1")
        );
        Ok(())
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn runtime_observer_loads_proposal_request_id_from_task_metadata_only() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-sp1-load-proposal",
        ))?);
        let proposal_task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaSp1,
            request: proposal_request(),
        });
        let other_request = ProposalTaskRequest {
            proposal_id: 43,
            ..proposal_request()
        };
        let other_task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaSp1,
            request: other_request.clone(),
        });
        let task_ref = proposal_task_ref(PipelineKey::ShastaSp1, &proposal_request());
        let other_task_ref = proposal_task_ref(PipelineKey::ShastaSp1, &other_request);
        runtime
            .register_task(TaskRegistration {
                task_id: "task_public_sp1_load".to_string(),
                pipeline_key: None,
                route: "sp1/local".parse::<PipelineRoute>().expect("parse route"),
                task_kind: "hoodi_batch".to_string(),
                proposal_id: Some(42),
                proof_ids: vec![task_ref.clone(), other_task_ref.clone()],
                metadata: serde_json::to_value(TaskMetadata {
                    network_pair: "taiko_dev/ethereum".to_string(),
                    network: "taiko_dev".to_string(),
                    l1_network: "ethereum".to_string(),
                    proof_type: ProofType::Sp1,
                    requested_proof_type: None,
                    prover_type: None,
                    execution_mode: Some(ExecutionMode::Prove),
                    aggregate_requested: false,
                    proposals: vec![
                        ProposalTask {
                            proposal_id: 42,
                            checkpoint: None,
                            l1_inclusion_block_number: 1,
                            l2_block_numbers: vec![42],
                            last_anchor_block_number: 0,
                            task_id: task_ref.clone(),
                            request: Some(proposal_request()),
                        },
                        ProposalTask {
                            proposal_id: 43,
                            checkpoint: None,
                            l1_inclusion_block_number: 1,
                            l2_block_numbers: vec![43],
                            last_anchor_block_number: 0,
                            task_id: other_task_ref.clone(),
                            request: Some(other_request.clone()),
                        },
                    ],
                    aggregate_task_id: None,
                    aggregate_request: None,
                    aggregate_input_artifacts: Vec::new(),
                    runtime: RuntimeMetadata::default(),
                })?,
                request_fingerprint: None,
            })
            .await?;

        let observer = RuntimeObserver::new(Arc::clone(&runtime), "taiko_dev/ethereum".to_string());
        observer
            .on_task_progress(
                &proposal_task_id,
                &EngineTask::ProveProposal {
                    request: proposal_request(),
                    input_task: proposal_task_id.clone(),
                },
                &ProverProgress::Sp1NetworkSubmission(Sp1NetworkSubmissionProgress {
                    provider_request_id: "0xsp1-proposal".to_string(),
                    network_mode: Sp1NetworkMode::Reserved,
                    fulfillment_strategy: Sp1FulfillmentStrategy::Reserved,
                    skip_simulation: true,
                    cycle_limit: 1_000_000_000_000,
                    timeout_secs: 3_600,
                    max_price_per_pgu: Some(42),
                    auction_timeout_secs: Some(120),
                }),
            )
            .await?;

        assert_eq!(
            observer
                .load_sp1_network_request_id(
                    &proposal_task_id,
                    &EngineTask::ProveProposal {
                        request: proposal_request(),
                        input_task: proposal_task_id.clone(),
                    },
                )
                .await?
                .as_deref(),
            Some("0xsp1-proposal")
        );
        assert_eq!(
            observer
                .load_sp1_network_request_id(
                    &other_task_id,
                    &EngineTask::ProveProposal {
                        request: other_request,
                        input_task: other_task_id.clone(),
                    },
                )
                .await?,
            None
        );
        Ok(())
    }

    #[tokio::test]
    async fn runtime_observer_loads_aggregate_request_id_from_aggregate_metadata_only() -> Result<()>
    {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-sp1-load-aggregate",
        ))?);
        let aggregate_request = AggregationTaskRequest {
            request_id: "agg-42".to_string(),
            proposal_ids: vec![42, 43],
            prover_config: ProverTaskConfig::default(),
        };
        let aggregate_task_id = EngineTaskId::new(EngineTaskKey::Aggregate {
            pipeline: PipelineKey::ShastaSp1,
            request: aggregate_request.clone(),
        });
        let task_ref = aggregate_task_ref(PipelineKey::ShastaSp1, &aggregate_request);
        runtime
            .register_task(TaskRegistration {
                task_id: "task_public_sp1_aggregate".to_string(),
                pipeline_key: None,
                route: "sp1/local".parse::<PipelineRoute>().expect("parse route"),
                task_kind: "hoodi_batch".to_string(),
                proposal_id: None,
                proof_ids: vec![task_ref.clone()],
                metadata: serde_json::to_value(TaskMetadata {
                    network_pair: "taiko_dev/ethereum".to_string(),
                    network: "taiko_dev".to_string(),
                    l1_network: "ethereum".to_string(),
                    proof_type: ProofType::Sp1,
                    requested_proof_type: None,
                    prover_type: None,
                    execution_mode: Some(ExecutionMode::Prove),
                    aggregate_requested: true,
                    proposals: vec![],
                    aggregate_task_id: Some(task_ref),
                    aggregate_request: Some(aggregate_request.clone()),
                    aggregate_input_artifacts: Vec::new(),
                    runtime: RuntimeMetadata::default(),
                })?,
                request_fingerprint: None,
            })
            .await?;

        let mut record = runtime
            .get_task("task_public_sp1_aggregate")
            .await?
            .expect("runtime task exists");
        record.provider_request_id = Some("0xsp1-proposal".to_string());
        runtime.upsert_task(&record).await?;

        let observer = RuntimeObserver::new(Arc::clone(&runtime), "taiko_dev/ethereum".to_string());
        assert_eq!(
            observer
                .load_sp1_network_request_id(
                    &aggregate_task_id,
                    &EngineTask::Aggregate {
                        request: aggregate_request.clone(),
                        source: raiko2_engine::AggregationSource::Inputs(vec![]),
                    },
                )
                .await?,
            None
        );

        observer
            .on_task_progress(
                &aggregate_task_id,
                &EngineTask::Aggregate {
                    request: aggregate_request.clone(),
                    source: raiko2_engine::AggregationSource::Inputs(vec![]),
                },
                &ProverProgress::Sp1NetworkSubmission(Sp1NetworkSubmissionProgress {
                    provider_request_id: "0xsp1-aggregate".to_string(),
                    network_mode: Sp1NetworkMode::Reserved,
                    fulfillment_strategy: Sp1FulfillmentStrategy::Reserved,
                    skip_simulation: true,
                    cycle_limit: 1_000_000_000_000,
                    timeout_secs: 3_600,
                    max_price_per_pgu: Some(42),
                    auction_timeout_secs: Some(120),
                }),
            )
            .await?;

        assert_eq!(
            observer
                .load_sp1_network_request_id(
                    &aggregate_task_id,
                    &EngineTask::Aggregate {
                        request: aggregate_request,
                        source: raiko2_engine::AggregationSource::Inputs(vec![]),
                    },
                )
                .await?
                .as_deref(),
            Some("0xsp1-aggregate")
        );
        Ok(())
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn runtime_observer_does_not_decrement_inflight_after_process_restart() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-no-negative-gauge",
        ))?);
        let proposal_task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaNative,
            request: proposal_request(),
        });
        let proposal_ref = proposal_task_ref(PipelineKey::ShastaNative, &proposal_request());
        let preflight_ref = stage_task_ref(&proposal_task_id);
        let mut metadata = TaskMetadata {
            network_pair: "telemetry_restart/ethereum".to_string(),
            network: "telemetry_restart".to_string(),
            l1_network: "ethereum".to_string(),
            proof_type: ProofType::Native,
            requested_proof_type: None,
            prover_type: None,
            execution_mode: None,
            aggregate_requested: false,
            proposals: vec![ProposalTask {
                proposal_id: 42,
                checkpoint: None,
                l1_inclusion_block_number: 1,
                l2_block_numbers: vec![42],
                last_anchor_block_number: 0,
                task_id: proposal_ref.clone(),
                request: Some(proposal_request()),
            }],
            aggregate_task_id: None,
            aggregate_request: None,
            aggregate_input_artifacts: Vec::new(),
            runtime: RuntimeMetadata::default(),
        };
        metadata.mark_stage_started(&preflight_ref, "preflight", now_ms() - 1_000);
        runtime
            .register_task(TaskRegistration {
                task_id: "task_public_restart".to_string(),
                pipeline_key: None,
                route: "native/local"
                    .parse::<PipelineRoute>()
                    .expect("parse route"),
                task_kind: "hoodi_batch".to_string(),
                proposal_id: Some(42),
                proof_ids: vec![proposal_ref],
                metadata: serde_json::to_value(metadata)?,
                request_fingerprint: None,
            })
            .await?;

        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "telemetry_restart/ethereum".to_string(),
        );
        observer
            .on_task_failed(
                &proposal_task_id,
                &EngineTask::Preflight {
                    request: proposal_request(),
                },
                "fixture failed",
            )
            .await;

        let (_, body) = telemetry::render().expect("render telemetry");
        let body = String::from_utf8(body).expect("utf8 telemetry");
        assert!(
            !body.contains(
                "raiko2_stage_tasks_inflight{aggregate=\"false\",pair=\"telemetry_restart/ethereum\",proof_type=\"native\",route=\"native/local\",stage=\"preflight\"} -1"
            ),
            "{body}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn runtime_observer_records_failure_kind_metrics() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-failure-kind",
        ))?);
        let pipeline = PipelineKey::ShastaNative;
        let request = proposal_request();
        let proposal_task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });

        register_observer_task(
            runtime.as_ref(),
            "task_failure_kind",
            "metrics_failure_kind/ethereum",
            pipeline,
            &request,
            RunnerStatus::Allocated,
        )
        .await?;

        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "metrics_failure_kind/ethereum".to_string(),
        );
        observer
            .on_task_failed(
                &proposal_task_id,
                &EngineTask::ProveProposal {
                    request,
                    input_task: proposal_task_id.clone(),
                },
                "sgx INVALID_REQUEST: aggregate proof 0 SGX instance id mismatch: got 4 expected 5",
            )
            .await;

        let (_, body) = telemetry::render().expect("render telemetry");
        let body = String::from_utf8(body).expect("utf8 telemetry");
        assert!(
            body.contains(
                "raiko2_stage_task_failures_total{aggregate=\"false\",error_kind=\"instance_id_mismatch\",pair=\"metrics_failure_kind/ethereum\",proof_type=\"native\",route=\"native/local\",stage=\"prove\"}"
            ),
            "{body}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn runtime_observer_records_proof_persistence_failure_metrics() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-proof-persistence",
        ))?);
        let pipeline = PipelineKey::ShastaNative;
        let request = proposal_request();
        let proposal_task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        let network_pair = "metrics_proof_persistence/ethereum";

        register_observer_task(
            runtime.as_ref(),
            "task_proof_persistence_failure",
            network_pair,
            pipeline,
            &request,
            RunnerStatus::Allocated,
        )
        .await?;

        let proof_path = runtime.proof_artifact_path(
            network_pair,
            &RuntimeObserver::root_task_ref(&proposal_task_id),
        );
        let artifact_dir = proof_path.parent().expect("proof artifact parent");
        tokio::fs::create_dir_all(artifact_dir.parent().expect("proof artifact root")).await?;
        tokio::fs::write(artifact_dir, b"not a directory").await?;

        let observer = RuntimeObserver::new(Arc::clone(&runtime), network_pair.to_string());
        observer
            .on_task_succeeded(
                &proposal_task_id,
                &EngineTask::ProveProposal {
                    request,
                    input_task: proposal_task_id.clone(),
                },
                &EngineTaskSuccess::Proof {
                    stage: raiko2_pipeline::PipelineStage::Prove,
                    proof: proof_fixture(),
                },
            )
            .await;

        let record = runtime
            .get_task("task_proof_persistence_failure")
            .await?
            .expect("runtime task exists");
        assert_eq!(record.runner_status, RunnerStatus::Failed);
        assert!(
            record
                .error
                .as_deref()
                .is_some_and(|error| error.contains("failed to persist proof output")),
            "{record:?}"
        );

        let (_, body) = telemetry::render().expect("render telemetry");
        let body = String::from_utf8(body).expect("utf8 telemetry");
        assert!(
            body.contains(
                "raiko2_stage_task_terminal_total{aggregate=\"false\",pair=\"metrics_proof_persistence/ethereum\",proof_type=\"native\",route=\"native/local\",stage=\"prove\",status=\"failed\"}"
            ),
            "{body}"
        );
        assert!(
            body.contains(
                "raiko2_stage_task_failures_total{aggregate=\"false\",error_kind=\"proof_persistence\",pair=\"metrics_proof_persistence/ethereum\",proof_type=\"native\",route=\"native/local\",stage=\"prove\"}"
            ),
            "{body}"
        );
        assert!(
            !body.contains(
                "raiko2_stage_task_terminal_total{aggregate=\"false\",pair=\"metrics_proof_persistence/ethereum\",proof_type=\"native\",route=\"native/local\",stage=\"prove\",status=\"completed\"}"
            ),
            "{body}"
        );
        Ok(())
    }
}
