use anyhow::{Context, Result};
use async_trait::async_trait;
use raiko2_engine::{
    EngineObserver, EngineObserverError, EngineTaskId, EngineTaskKey, EngineTaskSuccess,
    ProposalStage,
    tasks::{EngineTask, ProofArtifactRef},
};
use raiko2_pipeline::PipelineRoute;
use raiko2_prover::{BoundlessSubmissionResume, ProverProgress};
use raiko2_queue::encode_task_id;
use raiko2_runtime::{
    ProofArtifactPublicationInvalidated, ProofArtifactPutResult, RunnerStatus, RuntimeManager,
    RuntimeTaskRecord,
};
use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::server::task_metadata::{
    TaskMetadata, TaskRuntimeMetadata, legacy_proposal_task_refs, proposal_task_ref,
    stage_task_ref_for_stage,
};
use crate::server::telemetry::{self, MetricContext};
#[cfg(test)]
use raiko2_engine::ProverTaskConfig;

#[derive(Clone)]
pub(crate) struct RuntimeObserver {
    runtime: Arc<RuntimeManager>,
    network_pair: String,
    route: PipelineRoute,
    root_updates: Arc<tokio::sync::Mutex<()>>,
}

static STARTED_STAGE_TASKS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

#[derive(Clone, Copy, PartialEq, Eq)]
enum TerminalRootPolicy {
    Exclude,
    IncludeFailed,
    IncludeCompleted,
}

struct PublishedProofCommit {
    proof_uris: HashMap<String, String>,
    root_ref: String,
    content_hash: String,
}

enum ProofCommitAttempt {
    Committed,
    Retryable(anyhow::Error),
    Invalidated(anyhow::Error),
}

#[derive(Clone, Copy)]
enum PublicationFailureDisposition {
    Retryable,
    Invalidated,
}

#[derive(Debug)]
struct ProofInvalidatedError(String);

impl std::fmt::Display for ProofInvalidatedError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ProofInvalidatedError {}

fn publication_observer_error(error: &anyhow::Error) -> EngineObserverError {
    let message = format!("{error:#}");
    if error.downcast_ref::<ProofInvalidatedError>().is_some()
        || error
            .downcast_ref::<ProofArtifactPublicationInvalidated>()
            .is_some()
        || error
            .downcast_ref::<raiko2_runtime::PendingProofPublicationRemoved>()
            .is_some()
    {
        EngineObserverError::ProofInvalidated(message)
    } else {
        EngineObserverError::ProofPublication(message)
    }
}

impl RuntimeObserver {
    pub(crate) fn new(
        runtime: Arc<RuntimeManager>,
        network_pair: String,
        route: PipelineRoute,
    ) -> Self {
        Self {
            runtime,
            network_pair,
            route,
            root_updates: Arc::new(tokio::sync::Mutex::new(())),
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

    async fn find_root_records(&self, id: &EngineTaskId) -> Result<Vec<RuntimeTaskRecord>> {
        let canonical_ref = Self::root_task_ref(id);
        let legacy_ref = encode_task_id(id).context("failed to encode legacy task ref")?;
        let mut records = self.runtime.find_tasks_by_task_ref(&canonical_ref).await?;
        let mut seen = records
            .iter()
            .map(|record| record.task_id.clone())
            .collect::<HashSet<_>>();
        let mut legacy_refs = vec![legacy_ref];
        if let EngineTaskKey::Proposal { pipeline, request } = &id.0 {
            legacy_refs.extend(legacy_proposal_task_refs(*pipeline, request));
        }
        legacy_refs.retain(|proof_ref| proof_ref != &canonical_ref);
        legacy_refs.sort();
        legacy_refs.dedup();
        for legacy_ref in legacy_refs {
            let legacy_records = self.runtime.find_tasks_by_task_ref(&legacy_ref).await?;
            records.extend(
                legacy_records
                    .into_iter()
                    .filter(|record| seen.insert(record.task_id.clone())),
            );
        }
        Ok(records)
    }

    fn stage_name(task: &EngineTask) -> &'static str {
        match task.publication_source() {
            EngineTask::Proposal { .. } => "proposal",
            EngineTask::Preflight { .. } => "preflight",
            EngineTask::Validate { .. } => "validation",
            EngineTask::Encode { .. } => "encode",
            EngineTask::ProveProposal { .. } => "prove",
            EngineTask::Aggregate { .. } => "aggregate",
            EngineTask::PublicationGeneration { .. } | EngineTask::PublishProof { .. } => {
                unreachable!()
            }
        }
    }

    async fn update_root_records<F>(&self, id: &EngineTaskId, mutator: F) -> Result<()>
    where
        F: Fn(&mut RuntimeTaskRecord, i64, i64) -> Result<()>,
    {
        self.update_root_records_with_policy(id, TerminalRootPolicy::Exclude, mutator)
            .await
    }

    async fn update_retry_root_records<F>(&self, id: &EngineTaskId, mutator: F) -> Result<()>
    where
        F: Fn(&mut RuntimeTaskRecord, i64, i64) -> Result<()>,
    {
        self.update_root_records_with_policy(id, TerminalRootPolicy::IncludeFailed, mutator)
            .await
    }

    async fn update_root_records_with_policy<F>(
        &self,
        id: &EngineTaskId,
        terminal_policy: TerminalRootPolicy,
        mutator: F,
    ) -> Result<()>
    where
        F: Fn(&mut RuntimeTaskRecord, i64, i64) -> Result<()>,
    {
        let _guard = self.root_updates.lock().await;
        let root_ref = Self::root_task_ref(id);
        let records = self.find_root_records(id).await?;
        if records.is_empty() {
            anyhow::bail!("runtime task not registered for task ref {root_ref}");
        }
        let mut records = self.matching_root_records(id, records, terminal_policy)?;
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
        let records = self.find_root_records(id).await?;
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
        self.matching_root_records(id, records, TerminalRootPolicy::Exclude)
    }

    fn matching_root_records(
        &self,
        id: &EngineTaskId,
        records: Vec<RuntimeTaskRecord>,
        terminal_policy: TerminalRootPolicy,
    ) -> Result<Vec<RuntimeTaskRecord>> {
        records
            .into_iter()
            .filter_map(|record| match self.record_matches_observer(id, &record) {
                Ok(true) if !is_terminal_status(record.runner_status) => Some(Ok(record)),
                Ok(true)
                    if terminal_policy == TerminalRootPolicy::IncludeFailed
                        && record.runner_status == RunnerStatus::Failed =>
                {
                    Some(Ok(record))
                }
                Ok(true)
                    if terminal_policy == TerminalRootPolicy::IncludeCompleted
                        && record.runner_status == RunnerStatus::Completed =>
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
        let records = self.find_root_records(id).await?;
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
        Ok(match task.publication_source() {
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
            EngineTask::PublicationGeneration { .. } | EngineTask::PublishProof { .. } => {
                unreachable!()
            }
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
        if record.route != self.route {
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

    fn proposal_stage_from_task(task: &EngineTask) -> Option<ProposalStage> {
        match task.publication_source() {
            EngineTask::Preflight { .. } => Some(ProposalStage::Preflight),
            EngineTask::Validate { .. } => Some(ProposalStage::Validation),
            EngineTask::Encode { .. } => Some(ProposalStage::Encode),
            EngineTask::ProveProposal { .. } => Some(ProposalStage::Prove),
            EngineTask::Proposal { .. } | EngineTask::Aggregate { .. } => None,
            EngineTask::PublicationGeneration { .. } | EngineTask::PublishProof { .. } => {
                unreachable!()
            }
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
        let task_id = self.metric_tracking_key(&Self::timing_key_for_task(id, task));
        let mut started = STARTED_STAGE_TASKS
            .lock()
            .expect("stage task telemetry mutex poisoned");
        started.insert(task_id)
    }

    fn metric_tracking_key(&self, task_id: &str) -> String {
        format!("{}|{}|{task_id}", self.network_pair, self.route)
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
            let mut started = STARTED_STAGE_TASKS
                .lock()
                .expect("stage task telemetry mutex poisoned");
            started.remove(&self.metric_tracking_key(task_id))
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

    async fn publish_final_proof_artifact(
        &self,
        id: &EngineTaskId,
        task: &EngineTask,
        stage: &str,
        proof: &raiko2_primitives::Proof,
    ) -> Result<Option<PublishedProofCommit>> {
        let root_ref = Self::root_task_ref(id);
        let records = self.find_root_records(id).await?;
        let records = self.matching_active_root_records(id, records)?;
        anyhow::ensure!(
            proof.proof.is_some(),
            "refusing to publish proof artifact without a proof payload"
        );

        let proof_bytes = serde_json::to_vec(proof).context("failed to serialize proof output")?;
        let publication = self
            .runtime
            .commit_proof_artifact_publication(
                &self.network_pair,
                id.0.pipeline_key(),
                self.route,
                &root_ref,
                task.publication_generation(),
                &proof_bytes,
            )
            .await
            .context("failed to commit proof artifact publication")?;
        let artifact = publication.object();
        if matches!(publication, ProofArtifactPutResult::Conflict(_)) {
            let canonical = serde_json::from_slice::<raiko2_primitives::Proof>(&artifact.bytes)
                .context("conflicting canonical proof artifact is invalid")?;
            anyhow::ensure!(
                canonical.proof.is_some(),
                "conflicting canonical proof artifact has no proof payload"
            );
            tracing::warn!(
                task = ?id,
                stage,
                proof_uri = %artifact.proof_uri,
                content_hash = %artifact.content_hash,
                "discarding late proof because a different canonical artifact already exists"
            );
        }
        let proof_uri = artifact.proof_uri.clone();

        let mut proof_uris = HashMap::with_capacity(records.len());
        // A shared queue worker may run on a replica that has no local runtime row. It must still
        // publish the canonical artifact; the replica that owns the row reconciles from the store.
        let has_local_records = !records.is_empty();
        for record in records {
            let mut metadata: TaskMetadata = serde_json::from_value(record.metadata.clone())
                .context("failed to parse task metadata")?;
            let task_id = Self::timing_key_for_stage_name(id, stage);
            metadata.mark_stage_terminal(&task_id, stage, 0, "completed");
            if !Self::root_completed_by_proof_success(id, &metadata, record.pipeline_key) {
                continue;
            }

            proof_uris.insert(record.task_id, proof_uri.clone());
        }
        Ok(has_local_records.then_some(PublishedProofCommit {
            proof_uris,
            root_ref,
            content_hash: artifact.content_hash.clone(),
        }))
    }

    async fn mark_proof_publication_failed(
        &self,
        id: &EngineTaskId,
        stage: &'static str,
        message: &str,
        disposition: PublicationFailureDisposition,
    ) {
        let message = message.to_string();
        let terminal_policy = match disposition {
            PublicationFailureDisposition::Retryable => TerminalRootPolicy::Exclude,
            PublicationFailureDisposition::Invalidated => TerminalRootPolicy::IncludeCompleted,
        };
        if let Err(sync_err) = self
            .update_root_records_with_policy(
                id,
                terminal_policy,
                |record, updated_at, observed_at_ms| {
                    record.runner_status = match disposition {
                        PublicationFailureDisposition::Retryable => RunnerStatus::Allocated,
                        PublicationFailureDisposition::Invalidated => RunnerStatus::Cancelled,
                    };
                    record.proof_uri = None;
                    record.error = Some(message.clone());
                    let task_id = Self::timing_key_for_stage_name(id, stage);
                    update_task_metadata(record, |metadata| {
                        let (terminal, active_stage) = match disposition {
                            PublicationFailureDisposition::Retryable => {
                                ("failed", Some(stage.to_string()))
                            }
                            PublicationFailureDisposition::Invalidated => ("cancelled", None),
                        };
                        metadata.mark_stage_terminal(&task_id, stage, observed_at_ms, terminal);
                        metadata.runtime.active_stage = active_stage;
                        metadata.runtime.last_event = Some(terminal.to_string());
                    })?;
                    record.updated_at = updated_at;
                    Ok(())
                },
            )
            .await
        {
            tracing::warn!(
                task = ?id,
                stage,
                error = %sync_err,
                "failed to sync proof publication failure"
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
    ) -> Result<()> {
        let task_id = Self::timing_key_for_task(id, task);
        let publication_delays = [Duration::from_millis(100), Duration::from_millis(500)];
        for attempt in 0..=publication_delays.len() {
            match self.commit_proof_attempt(id, task, stage, proof).await {
                ProofCommitAttempt::Committed => {
                    self.observe_stage_terminal_metrics(
                        id,
                        &task_id,
                        stage,
                        "completed",
                        finished_at_ms,
                        None,
                    )
                    .await;
                    return Ok(());
                }
                ProofCommitAttempt::Invalidated(error) => {
                    let message = format!("failed to publish proof artifact: {error}");
                    tracing::warn!(task = ?id, stage, error = %error, "proof invalidated during completion");
                    self.observe_stage_terminal_metrics(
                        id,
                        &task_id,
                        stage,
                        "failed",
                        finished_at_ms,
                        Some(message.as_str()),
                    )
                    .await;
                    self.mark_proof_publication_failed(
                        id,
                        stage,
                        message.as_str(),
                        PublicationFailureDisposition::Invalidated,
                    )
                    .await;
                    return Err(ProofInvalidatedError(message).into());
                }
                ProofCommitAttempt::Retryable(error) if attempt < publication_delays.len() => {
                    let delay = publication_delays[attempt];
                    tracing::warn!(
                        task = ?id,
                        stage,
                        publication_attempt = attempt + 1,
                        delay_ms = delay.as_millis(),
                        error = %error,
                        "proof publication commit failed; retrying"
                    );
                    tokio::time::sleep(delay).await;
                }
                ProofCommitAttempt::Retryable(error) => {
                    let message = format!("failed to publish proof artifact: {error}");
                    tracing::warn!(task = ?id, stage, error = %error, "failed to publish proof artifact");
                    self.observe_stage_terminal_metrics(
                        id,
                        &task_id,
                        stage,
                        "failed",
                        finished_at_ms,
                        Some(message.as_str()),
                    )
                    .await;
                    self.mark_proof_publication_failed(
                        id,
                        stage,
                        message.as_str(),
                        PublicationFailureDisposition::Retryable,
                    )
                    .await;
                    anyhow::bail!(message);
                }
            }
        }
        unreachable!("publication retry loop always returns")
    }

    async fn commit_proof_attempt(
        &self,
        id: &EngineTaskId,
        task: &EngineTask,
        stage: &'static str,
        proof: &raiko2_primitives::Proof,
    ) -> ProofCommitAttempt {
        let publication = match self
            .publish_final_proof_artifact(id, task, stage, proof)
            .await
        {
            Ok(Some(publication)) => publication,
            Ok(None) => return ProofCommitAttempt::Committed,
            Err(error) if error.downcast_ref::<ProofInvalidatedError>().is_some() => {
                return ProofCommitAttempt::Invalidated(error);
            }
            Err(error)
                if error
                    .downcast_ref::<ProofArtifactPublicationInvalidated>()
                    .is_some() =>
            {
                return ProofCommitAttempt::Invalidated(error);
            }
            Err(error) => return ProofCommitAttempt::Retryable(error),
        };
        if let Err(error) = self
            .sync_proof_success(id, task, stage, publication.proof_uris)
            .await
        {
            return ProofCommitAttempt::Retryable(error);
        }
        match self
            .runtime
            .proof_artifact_is_invalidated(
                &self.network_pair,
                id.0.pipeline_key(),
                self.route,
                &publication.root_ref,
                &publication.content_hash,
            )
            .await
            .context("failed to fence completed proof against invalidation")
        {
            Ok(false) => ProofCommitAttempt::Committed,
            Ok(true) => ProofCommitAttempt::Invalidated(anyhow::anyhow!(
                "canonical proof artifact {} was invalidated during completion",
                publication.root_ref
            )),
            Err(error) => ProofCommitAttempt::Retryable(error),
        }
    }

    async fn sync_proof_success(
        &self,
        id: &EngineTaskId,
        task: &EngineTask,
        stage: &'static str,
        proof_uris: HashMap<String, String>,
    ) -> Result<()> {
        self.update_root_records(id, move |record, updated_at, observed_at_ms| {
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
                record.proof_uri = proof_uris.get(&record.task_id).cloned();
            } else {
                record.runner_status = RunnerStatus::Allocated;
                record.proof_uri = None;
            }
            record.updated_at = updated_at;
            Ok(())
        })
        .await
        .context("failed to sync runtime task success")?;
        Ok(())
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
    async fn checkpoint_completed_proof(
        &self,
        id: &EngineTaskId,
        task: &EngineTask,
        proof: &raiko2_primitives::Proof,
    ) -> std::result::Result<(), EngineObserverError> {
        let proof_ref = Self::root_task_ref(id);
        let bytes = serde_json::to_vec(proof).map_err(|error| {
            EngineObserverError::ProofPublication(format!(
                "failed to serialize pending proof publication: {error}"
            ))
        })?;
        self.runtime
            .upsert_pending_proof_publication(
                &self.network_pair,
                id.0.pipeline_key(),
                self.route,
                &proof_ref,
                task.publication_generation(),
                &bytes,
            )
            .await
            .map_err(|error| publication_observer_error(&error))
    }

    async fn load_completed_proof(
        &self,
        id: &EngineTaskId,
        task: &EngineTask,
    ) -> std::result::Result<Option<raiko2_primitives::Proof>, String> {
        let pipeline_key = match id.0 {
            EngineTaskKey::Proposal { pipeline, .. }
            | EngineTaskKey::Aggregate { pipeline, .. } => pipeline,
        };
        let proof_ref = Self::root_task_ref(id);
        let material = crate::server::proof_artifact::load_proof_artifact_material(
            &self.runtime,
            &self.network_pair,
            pipeline_key,
            self.route,
            &proof_ref,
        )
        .await
        .map_err(|error| error.to_string())?;
        let proof = if let Some(material) = material {
            material.proof
        } else {
            let Some(bytes) = self
                .runtime
                .get_pending_proof_publication(
                    &self.network_pair,
                    pipeline_key,
                    self.route,
                    &proof_ref,
                    task.publication_generation(),
                )
                .await
                .map_err(|error| error.to_string())?
            else {
                return Ok(None);
            };
            serde_json::from_slice(&bytes).map_err(|error| {
                format!("invalid pending proof publication {proof_ref}: {error}")
            })?
        };
        if proof.proof.is_none() {
            return Err(format!(
                "completed proof artifact {proof_ref} has no proof payload"
            ));
        }
        Ok(Some(proof))
    }

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
    ) {
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
        let result = self
            .update_retry_root_records(id, |record, updated_at, _observed_at_ms| {
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
                        ProverProgress::BoundlessSubmission(submission) => {
                            match task.publication_source() {
                                EngineTask::ProveProposal { .. } => {
                                    metadata
                                        .upsert_proposal_runtime(&task_id, submission, updated_at);
                                }
                                EngineTask::Aggregate { .. } => {
                                    metadata.upsert_aggregate_runtime(submission, updated_at);
                                }
                                EngineTask::Preflight { .. }
                                | EngineTask::Validate { .. }
                                | EngineTask::Encode { .. }
                                | EngineTask::Proposal { .. } => {}
                                EngineTask::PublicationGeneration { .. }
                                | EngineTask::PublishProof { .. } => unreachable!(),
                            }
                        }
                        ProverProgress::Sp1NetworkSubmission(submission) => {
                            match task.publication_source() {
                                EngineTask::ProveProposal { .. } => {
                                    metadata.upsert_proposal_sp1_network_runtime(
                                        &task_id, submission, updated_at,
                                    );
                                }
                                EngineTask::Aggregate { .. } => {
                                    metadata.upsert_aggregate_sp1_network_runtime(
                                        submission, updated_at,
                                    );
                                }
                                EngineTask::Preflight { .. }
                                | EngineTask::Validate { .. }
                                | EngineTask::Encode { .. }
                                | EngineTask::Proposal { .. } => {}
                                EngineTask::PublicationGeneration { .. }
                                | EngineTask::PublishProof { .. } => unreachable!(),
                            }
                        }
                    }
                })?;
                record.updated_at = updated_at;
                Ok(())
            })
            .await;
        if let Err(err) = result {
            tracing::warn!(
                task = ?id,
                error = %err,
                "failed to sync runtime task progress"
            );
        }
    }

    async fn load_proof_artifact(
        &self,
        artifact: &ProofArtifactRef,
    ) -> std::result::Result<Option<raiko2_primitives::Proof>, String> {
        let material = crate::server::proof_artifact::load_aggregate_input_artifact_material(
            &self.runtime,
            &artifact.network_pair,
            artifact.pipeline_key,
            artifact.route,
            &artifact.proof_ref,
        )
        .await
        .map_err(|error| error.to_string())?;
        Ok(material.map(|material| material.proof))
    }

    async fn on_task_succeeded(
        &self,
        id: &EngineTaskId,
        task: &EngineTask,
        success: &EngineTaskSuccess,
    ) -> std::result::Result<(), EngineObserverError> {
        let stage = Self::stage_name(task);
        let finished_at_ms = now_ms();
        let task_id = Self::timing_key_for_task(id, task);
        let result = match success {
            EngineTaskSuccess::Proof { proof, .. } => {
                return self
                    .handle_proof_success(id, task, stage, finished_at_ms, proof)
                    .await
                    .map_err(|error| publication_observer_error(&error));
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
        Ok(())
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
    ) -> Option<String> {
        let source = task.publication_source();
        let record = match source {
            EngineTask::ProveProposal { .. } | EngineTask::Aggregate { .. } => self
                .load_root_record_for_resume(id, source)
                .await
                .ok()
                .flatten(),
            EngineTask::Preflight { .. }
            | EngineTask::Validate { .. }
            | EngineTask::Encode { .. }
            | EngineTask::Proposal { .. } => None,
            EngineTask::PublicationGeneration { .. } | EngineTask::PublishProof { .. } => {
                unreachable!()
            }
        }?;

        let metadata: TaskMetadata = serde_json::from_value(record.metadata).ok()?;
        let task_id = Self::root_task_ref(id);
        match source {
            EngineTask::ProveProposal { .. } => metadata
                .proposal_runtime(&task_id)
                .and_then(|runtime| runtime.provider_request_id.clone()),
            // Proposal and aggregation must keep distinct SP1 network requests. Reusing the
            // root-level request id causes aggregate=true flows to resume the proposal request
            // instead of creating a new aggregation request.
            EngineTask::Aggregate { .. } => metadata
                .aggregate_runtime()
                .and_then(|runtime| runtime.provider_request_id.clone()),
            EngineTask::Preflight { .. }
            | EngineTask::Validate { .. }
            | EngineTask::Encode { .. }
            | EngineTask::Proposal { .. } => None,
            EngineTask::PublicationGeneration { .. } | EngineTask::PublishProof { .. } => {
                unreachable!()
            }
        }
    }

    async fn load_boundless_submission(
        &self,
        id: &EngineTaskId,
        task: &EngineTask,
    ) -> Option<BoundlessSubmissionResume> {
        let source = task.publication_source();
        let record = match source {
            EngineTask::ProveProposal { .. } | EngineTask::Aggregate { .. } => self
                .load_root_record_for_resume(id, source)
                .await
                .ok()
                .flatten(),
            EngineTask::Preflight { .. }
            | EngineTask::Validate { .. }
            | EngineTask::Encode { .. }
            | EngineTask::Proposal { .. } => None,
            EngineTask::PublicationGeneration { .. } | EngineTask::PublishProof { .. } => {
                unreachable!()
            }
        }?;

        let metadata: TaskMetadata = serde_json::from_value(record.metadata).ok()?;
        let task_id = Self::root_task_ref(id);
        let runtime = match source {
            EngineTask::ProveProposal { .. } => metadata.proposal_runtime(&task_id),
            EngineTask::Aggregate { .. } => metadata.aggregate_runtime(),
            EngineTask::Preflight { .. }
            | EngineTask::Validate { .. }
            | EngineTask::Encode { .. }
            | EngineTask::Proposal { .. } => None,
            EngineTask::PublicationGeneration { .. } | EngineTask::PublishProof { .. } => {
                unreachable!()
            }
        }?;

        let now = now_secs();
        // Expired records must still reach the prover: it gives them one final market status
        // read (an expired-but-fulfilled request still reports Fulfilled, recovering a proof
        // that is already paid for) and otherwise counts the stored attempt against the rebid
        // budget. Dropping them here would reset the budget and the price-escalation ladder on
        // every restart after expiry.
        let expires_at = runtime.expires_at?;

        Some(BoundlessSubmissionResume {
            provider_request_id: runtime.provider_request_id.clone()?,
            remote_tx_hash: runtime.remote_tx_hash.clone(),
            expires_at,
            lock_expires_at: runtime.lock_expires_at.unwrap_or(0),
            submitted_at: runtime.submitted_at.unwrap_or(now),
            max_price_multiplier: runtime.max_price_multiplier.unwrap_or(1),
            max_price_wei: runtime.max_price_wei.clone(),
            rebid_attempt: runtime.rebid_attempt.unwrap_or(0),
        })
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
    use async_trait::async_trait;
    use raiko2_engine::{AggregationTaskRequest, ProposalTaskRequest};
    use raiko2_pipeline::PipelineKey;
    use raiko2_primitives::ProofType;
    use raiko2_prover::{
        BoundlessSubmissionProgress, Sp1FulfillmentStrategy, Sp1NetworkMode,
        Sp1NetworkSubmissionProgress, sp1::ExecutionMode,
    };
    use raiko2_runtime::{
        FilesystemProofArtifactStore, ProofArtifactKey, ProofArtifactObject, ProofArtifactPrefix,
        ProofArtifactPutResult, ProofArtifactRegistration, ProofArtifactStore, TaskRegistration,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug, Default)]
    struct FailingArtifactStore {
        attempts: AtomicUsize,
    }

    #[derive(Debug)]
    struct InvalidatesDuringPublicationStore {
        inner: FilesystemProofArtifactStore,
        checks: AtomicUsize,
        block_on_check: usize,
        recheck_entered: tokio::sync::Notify,
        allow_recheck: tokio::sync::Notify,
    }

    #[async_trait]
    impl ProofArtifactStore for InvalidatesDuringPublicationStore {
        fn environment_id(&self) -> &str {
            self.inner.environment_id()
        }

        fn proof_uri(&self, key: &ProofArtifactKey) -> String {
            self.inner.proof_uri(key)
        }

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

        async fn get_prefix(
            &self,
            key: &ProofArtifactKey,
            max_bytes: usize,
        ) -> Result<Option<ProofArtifactPrefix>> {
            self.inner.get_prefix(key, max_bytes).await
        }

        async fn mark_invalidated(&self, key: &ProofArtifactKey, content_hash: &str) -> Result<()> {
            self.inner.mark_invalidated(key, content_hash).await
        }

        async fn is_invalidated(&self, key: &ProofArtifactKey, content_hash: &str) -> Result<bool> {
            if self.checks.fetch_add(1, Ordering::SeqCst) == self.block_on_check {
                self.recheck_entered.notify_one();
                self.allow_recheck.notified().await;
            }
            self.inner.is_invalidated(key, content_hash).await
        }

        async fn delete(
            &self,
            key: &ProofArtifactKey,
            generation: Option<i64>,
            expected_content_hash: &str,
        ) -> Result<()> {
            self.inner
                .delete(key, generation, expected_content_hash)
                .await
        }
    }

    #[async_trait]
    impl ProofArtifactStore for FailingArtifactStore {
        fn environment_id(&self) -> &str {
            "test"
        }

        fn proof_uri(&self, key: &ProofArtifactKey) -> String {
            format!("failing://{}", key.proof_ref)
        }

        async fn put_if_absent(
            &self,
            _key: &ProofArtifactKey,
            _bytes: &[u8],
        ) -> Result<ProofArtifactPutResult> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            anyhow::bail!("injected publication failure")
        }

        async fn get(&self, _key: &ProofArtifactKey) -> Result<Option<ProofArtifactObject>> {
            Ok(None)
        }

        async fn get_prefix(
            &self,
            _key: &ProofArtifactKey,
            _max_bytes: usize,
        ) -> Result<Option<ProofArtifactPrefix>> {
            Ok(None)
        }

        async fn mark_invalidated(
            &self,
            _key: &ProofArtifactKey,
            _content_hash: &str,
        ) -> Result<()> {
            Ok(())
        }

        async fn is_invalidated(
            &self,
            _key: &ProofArtifactKey,
            _content_hash: &str,
        ) -> Result<bool> {
            Ok(false)
        }

        async fn delete(
            &self,
            _key: &ProofArtifactKey,
            _generation: Option<i64>,
            _expected_content_hash: &str,
        ) -> Result<()> {
            Ok(())
        }
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

        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            PipelineKey::ShastaRisc0Network.route(),
        );
        let future_expires_at = now_secs().saturating_add(3_600);
        observer
            .on_task_progress(
                &proposal_task_id,
                &EngineTask::ProveProposal {
                    request: proposal_request(),
                    input_task: proposal_task_id.clone(),
                },
                &ProverProgress::BoundlessSubmission(BoundlessSubmissionProgress {
                    provider_request_id: "0x1234".to_string(),
                    remote_tx_hash: Some("0xabcd".to_string()),
                    expires_at: future_expires_at,
                    lock_expires_at: future_expires_at - 600,
                    submitted_at: future_expires_at - 300,
                    image_ref: "0ximage".to_string(),
                    deployment: "base".to_string(),
                    offchain: false,
                    quoted_mcycles_count: Some(6_000),
                    evaluated_mcycles_count: Some(12_345),
                    max_price_multiplier: 4,
                    max_price_wei: Some("9000000000000".to_string()),
                    rebid_attempt: 3,
                }),
            )
            .await;

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
        assert_eq!(runtime_entry.provider_request_id.as_deref(), Some("0x1234"));
        assert_eq!(runtime_entry.remote_tx_hash.as_deref(), Some("0xabcd"));
        assert_eq!(runtime_entry.expires_at, Some(future_expires_at));
        assert_eq!(runtime_entry.submitted_at, Some(future_expires_at - 300));
        assert_eq!(runtime_entry.image_ref.as_deref(), Some("0ximage"));
        assert_eq!(runtime_entry.quoted_mcycles_count, Some(6_000));
        assert_eq!(runtime_entry.evaluated_mcycles_count, Some(12_345));
        assert_eq!(runtime_entry.max_price_multiplier, Some(4));
        assert_eq!(
            runtime_entry.max_price_wei.as_deref(),
            Some("9000000000000")
        );
        assert_eq!(runtime_entry.rebid_attempt, Some(3));
        assert_eq!(runtime_entry.lock_expires_at, Some(future_expires_at - 600));
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
            .expect("boundless submission can resume");
        assert_eq!(resumed.provider_request_id, "0x1234");
        assert_eq!(resumed.remote_tx_hash.as_deref(), Some("0xabcd"));
        assert_eq!(resumed.expires_at, future_expires_at);
        assert_eq!(resumed.lock_expires_at, future_expires_at - 600);
        assert_eq!(resumed.submitted_at, future_expires_at - 300);
        assert_eq!(resumed.max_price_multiplier, 4);
        assert_eq!(resumed.max_price_wei.as_deref(), Some("9000000000000"));
        assert_eq!(resumed.rebid_attempt, 3);

        let mut record = runtime
            .get_task("task_public")
            .await?
            .expect("runtime task");
        let mut metadata: TaskMetadata = serde_json::from_value(record.metadata.clone())?;
        let runtime_entry = metadata
            .runtime
            .proposals
            .get_mut(&task_ref)
            .expect("proposal runtime exists");
        runtime_entry.submitted_at = None;
        runtime_entry.max_price_multiplier = None;
        runtime_entry.rebid_attempt = None;
        runtime_entry.lock_expires_at = None;
        record.metadata = serde_json::to_value(metadata)?;
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
            .expect("legacy boundless submission can resume");
        let after_legacy_resume = now_secs();
        assert_eq!(resumed.provider_request_id, "0x1234");
        assert_eq!(resumed.expires_at, future_expires_at);
        assert_eq!(resumed.lock_expires_at, 0);
        assert!((before_legacy_resume..=after_legacy_resume).contains(&resumed.submitted_at));
        assert_eq!(resumed.max_price_multiplier, 1);
        assert_eq!(resumed.rebid_attempt, 0);

        let mut record = runtime
            .get_task("task_public")
            .await?
            .expect("runtime task");
        let mut metadata: TaskMetadata = serde_json::from_value(record.metadata.clone())?;
        let expired_at = now_secs().saturating_sub(1);
        {
            let runtime_entry = metadata
                .runtime
                .proposals
                .get_mut(&task_ref)
                .expect("proposal runtime exists");
            runtime_entry.expires_at = Some(expired_at);
            runtime_entry.rebid_attempt = Some(5);
        }
        record.metadata = serde_json::to_value(metadata)?;
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

        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            PipelineKey::ShastaSp1.route(),
        );
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
            .await
            .map_err(anyhow::Error::msg)?;

        let record = runtime
            .get_task("task_public_aggregate_pending")
            .await?
            .expect("runtime task exists");
        let metadata: TaskMetadata = serde_json::from_value(record.metadata)?;
        assert_eq!(record.runner_status, RunnerStatus::Allocated);
        assert_eq!(record.proof_uri, None);
        assert_eq!(
            metadata.runtime.last_event.as_deref(),
            Some("stage_completed")
        );
        assert!(
            !tokio::fs::try_exists(Path::new(&record.task_dir).join("proof.json")).await?,
            "proposal proof must not become the root proof for aggregate requests"
        );
        runtime
            .get_proof_artifact(
                "taiko_dev/ethereum",
                pipeline,
                pipeline.route(),
                &proposal_ref,
            )
            .await?
            .expect("proposal proof artifact");
        assert!(
            runtime
                .read_proof_artifact_bytes(
                    "taiko_dev/ethereum",
                    pipeline,
                    pipeline.route(),
                    &proposal_ref,
                )
                .await?
                .is_some()
        );
        Ok(())
    }

    #[tokio::test]
    async fn runtime_observer_publishes_without_replica_local_runtime_row() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-cross-replica-publication",
        ))?);
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let request = proposal_request();
        let task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        let proof_ref = proposal_task_ref(pipeline, &request);
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            route,
        );

        observer
            .on_task_succeeded(
                &task_id,
                &EngineTask::ProveProposal {
                    request,
                    input_task: task_id.clone(),
                },
                &EngineTaskSuccess::Proof {
                    stage: raiko2_pipeline::PipelineStage::Prove,
                    proof: proof_fixture(),
                },
            )
            .await
            .map_err(anyhow::Error::msg)?;

        assert!(runtime.list_tasks().await?.is_empty());
        assert!(
            runtime
                .read_proof_artifact_bytes("taiko_dev/ethereum", pipeline, route, &proof_ref,)
                .await?
                .is_some()
        );
        Ok(())
    }

    #[tokio::test]
    async fn runtime_observer_recovers_pending_publication_outbox() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-publication-outbox",
        ))?);
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let request = proposal_request();
        let task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        let proof_ref = proposal_task_ref(pipeline, &request);
        let proof = proof_fixture();
        runtime
            .upsert_pending_proof_publication(
                "taiko_dev/ethereum",
                pipeline,
                route,
                &proof_ref,
                "legacy",
                &serde_json::to_vec(&proof)?,
            )
            .await?;
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            route,
        );

        assert_eq!(
            observer
                .load_completed_proof(&task_id, &EngineTask::Proposal { request })
                .await
                .map_err(anyhow::Error::msg)?,
            Some(proof)
        );
        Ok(())
    }

    #[tokio::test]
    async fn invalidated_completion_cancels_the_publication_generation() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-invalidated-completion-rollback",
        ))?);
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let request = proposal_request();
        let task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        register_observer_task(
            runtime.as_ref(),
            "task_invalidated_completion",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Completed,
        )
        .await?;
        let mut completed = runtime
            .get_task("task_invalidated_completion")
            .await?
            .expect("completed runtime task");
        completed.proof_uri = Some("file:///deleted-proof.json".to_string());
        runtime.upsert_task(&completed).await?;
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            route,
        );

        observer
            .mark_proof_publication_failed(
                &task_id,
                "prove",
                "proof invalidated during completion",
                PublicationFailureDisposition::Invalidated,
            )
            .await;

        let rolled_back = runtime
            .get_task("task_invalidated_completion")
            .await?
            .expect("rolled back runtime task");
        assert_eq!(rolled_back.runner_status, RunnerStatus::Cancelled);
        assert_eq!(rolled_back.proof_uri, None);
        assert_eq!(
            rolled_back.error.as_deref(),
            Some("proof invalidated during completion")
        );
        Ok(())
    }

    #[tokio::test]
    async fn transient_success_sync_failure_stays_in_publication_retry_loop() -> Result<()> {
        let artifact_root = unique_runtime_root("runtime-observer-sync-retry-artifacts");
        let store = Arc::new(InvalidatesDuringPublicationStore {
            inner: FilesystemProofArtifactStore::new(
                "shared-environment".to_string(),
                artifact_root,
            )?,
            checks: AtomicUsize::new(0),
            block_on_check: 2,
            recheck_entered: tokio::sync::Notify::new(),
            allow_recheck: tokio::sync::Notify::new(),
        });
        let runtime = Arc::new(RuntimeManager::new_with_artifact_store(
            unique_runtime_root("runtime-observer-sync-retry"),
            store.clone(),
        )?);
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let request = proposal_request();
        let task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        register_observer_task(
            runtime.as_ref(),
            "task_sync_retry",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Running,
        )
        .await?;
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            route,
        );
        let publication = tokio::spawn(async move {
            observer
                .on_task_succeeded(
                    &task_id,
                    &EngineTask::ProveProposal {
                        request,
                        input_task: task_id.clone(),
                    },
                    &EngineTaskSuccess::Proof {
                        stage: raiko2_pipeline::PipelineStage::Prove,
                        proof: proof_fixture(),
                    },
                )
                .await
        });

        store.recheck_entered.notified().await;
        let mut record = runtime
            .get_task("task_sync_retry")
            .await?
            .expect("runtime task");
        let valid_metadata = record.metadata.clone();
        record.metadata = serde_json::json!({"invalid": true});
        runtime.upsert_task(&record).await?;
        store.allow_recheck.notify_one();
        tokio::time::sleep(Duration::from_millis(25)).await;
        record.metadata = valid_metadata;
        runtime.upsert_task(&record).await?;

        publication
            .await?
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let completed = runtime
            .get_task("task_sync_retry")
            .await?
            .expect("completed runtime task");
        assert_eq!(completed.runner_status, RunnerStatus::Completed);
        assert!(completed.proof_uri.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn publication_rechecks_shared_tombstone_before_clearing_outbox() -> Result<()> {
        let artifact_root = unique_runtime_root("runtime-observer-publication-race-artifacts");
        let store = Arc::new(InvalidatesDuringPublicationStore {
            inner: FilesystemProofArtifactStore::new(
                "shared-environment".to_string(),
                artifact_root,
            )?,
            checks: AtomicUsize::new(0),
            block_on_check: 2,
            recheck_entered: tokio::sync::Notify::new(),
            allow_recheck: tokio::sync::Notify::new(),
        });
        let first = Arc::new(RuntimeManager::new_with_artifact_store(
            unique_runtime_root("runtime-observer-publication-race-a"),
            store.clone(),
        )?);
        let second = Arc::new(RuntimeManager::new_with_artifact_store(
            unique_runtime_root("runtime-observer-publication-race-b"),
            store.clone(),
        )?);
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let request = proposal_request();
        let task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        let proof_ref = proposal_task_ref(pipeline, &request);
        let proof = proof_fixture();
        first
            .upsert_pending_proof_publication(
                "taiko_dev/ethereum",
                pipeline,
                route,
                &proof_ref,
                "legacy",
                &serde_json::to_vec(&proof)?,
            )
            .await?;
        let observer =
            RuntimeObserver::new(Arc::clone(&first), "taiko_dev/ethereum".to_string(), route);

        let publication = tokio::spawn(async move {
            observer
                .on_task_succeeded(
                    &task_id,
                    &EngineTask::ProveProposal {
                        request,
                        input_task: task_id.clone(),
                    },
                    &EngineTaskSuccess::Proof {
                        stage: raiko2_pipeline::PipelineStage::Prove,
                        proof,
                    },
                )
                .await
        });
        store.recheck_entered.notified().await;
        let canonical = first
            .read_proof_artifact_bytes("taiko_dev/ethereum", pipeline, route, &proof_ref)
            .await?
            .expect("published canonical artifact");
        second
            .mark_proof_artifact_invalidated(
                "taiko_dev/ethereum",
                pipeline,
                route,
                &proof_ref,
                &canonical.content_hash,
            )
            .await?;
        store.allow_recheck.notify_one();
        let error = publication
            .await?
            .expect_err("concurrent invalidation must reject publication");

        assert!(matches!(error, EngineObserverError::ProofInvalidated(_)));
        let pending_key = ProofArtifactKey {
            network_pair: "taiko_dev/ethereum".to_string(),
            pipeline_key: pipeline,
            route,
            proof_ref: format!("pending:legacy:{proof_ref}"),
        };
        assert!(store.get(&pending_key).await?.is_some());
        assert!(
            first
                .get_proof_artifact("taiko_dev/ethereum", pipeline, route, &proof_ref)
                .await?
                .is_none()
        );
        Ok(())
    }

    #[tokio::test]
    async fn artifact_reconciliation_treats_concurrent_local_invalidation_as_cache_miss()
    -> Result<()> {
        let artifact_root = unique_runtime_root("runtime-observer-reconcile-race-artifacts");
        let store = Arc::new(InvalidatesDuringPublicationStore {
            inner: FilesystemProofArtifactStore::new(
                "shared-environment".to_string(),
                artifact_root,
            )?,
            checks: AtomicUsize::new(0),
            block_on_check: 0,
            recheck_entered: tokio::sync::Notify::new(),
            allow_recheck: tokio::sync::Notify::new(),
        });
        let runtime = Arc::new(RuntimeManager::new_with_artifact_store(
            unique_runtime_root("runtime-observer-reconcile-race"),
            store.clone(),
        )?);
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let request = proposal_request();
        let proof_ref = proposal_task_ref(pipeline, &request);
        let bytes = serde_json::to_vec(&proof_fixture())?;
        let publication = runtime
            .publish_proof_artifact_bytes("taiko_dev/ethereum", pipeline, route, &proof_ref, &bytes)
            .await?;
        let object = publication.object().clone();
        let loading_runtime = Arc::clone(&runtime);
        let loading_ref = proof_ref.clone();
        let loading = tokio::spawn(async move {
            crate::server::proof_artifact::load_proof_artifact_material(
                &loading_runtime,
                "taiko_dev/ethereum",
                pipeline,
                route,
                &loading_ref,
            )
            .await
        });
        store.recheck_entered.notified().await;
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: "taiko_dev/ethereum".to_string(),
                proof_ref: proof_ref.clone(),
                pipeline_key: pipeline,
                route,
                proof_uri: object.proof_uri.clone(),
                content_hash: object.content_hash.clone(),
                generation: object.generation,
            })
            .await?;
        runtime
            .mark_proof_artifact_invalidated(
                "taiko_dev/ethereum",
                pipeline,
                route,
                &proof_ref,
                &object.content_hash,
            )
            .await?;
        store.allow_recheck.notify_one();

        assert!(loading.await??.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn artifact_reconciliation_tolerates_concurrent_full_invalidation() -> Result<()> {
        let artifact_root = unique_runtime_root("runtime-observer-full-invalidation-artifacts");
        let store = Arc::new(InvalidatesDuringPublicationStore {
            inner: FilesystemProofArtifactStore::new(
                "shared-environment".to_string(),
                artifact_root,
            )?,
            checks: AtomicUsize::new(0),
            block_on_check: 1,
            recheck_entered: tokio::sync::Notify::new(),
            allow_recheck: tokio::sync::Notify::new(),
        });
        let runtime = Arc::new(RuntimeManager::new_with_artifact_store(
            unique_runtime_root("runtime-observer-full-invalidation"),
            store.clone(),
        )?);
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let proof_ref = proposal_task_ref(pipeline, &proposal_request());
        let bytes = serde_json::to_vec(&proof_fixture())?;
        let publication = runtime
            .publish_proof_artifact_bytes("taiko_dev/ethereum", pipeline, route, &proof_ref, &bytes)
            .await?;
        let object = publication.object().clone();
        let loading_runtime = Arc::clone(&runtime);
        let loading_ref = proof_ref.clone();
        let loading = tokio::spawn(async move {
            crate::server::proof_artifact::load_proof_artifact_material(
                &loading_runtime,
                "taiko_dev/ethereum",
                pipeline,
                route,
                &loading_ref,
            )
            .await
        });

        store.recheck_entered.notified().await;
        runtime
            .mark_proof_artifact_invalidated(
                "taiko_dev/ethereum",
                pipeline,
                route,
                &proof_ref,
                &object.content_hash,
            )
            .await?;
        runtime
            .delete_proof_artifact(
                "taiko_dev/ethereum",
                pipeline,
                route,
                &proof_ref,
                object.generation,
                &object.content_hash,
            )
            .await?;
        runtime
            .remove_proof_artifact("taiko_dev/ethereum", pipeline, route, &proof_ref)
            .await?;
        store.allow_recheck.notify_one();

        assert!(loading.await??.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn engine_artifact_loaders_honor_tombstones() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-tombstone-recovery",
        ))?);
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let request = proposal_request();
        let task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        let proof_ref = proposal_task_ref(pipeline, &request);
        let bytes = serde_json::to_vec(&proof_fixture())?;
        let publication = runtime
            .publish_proof_artifact_bytes("taiko_dev/ethereum", pipeline, route, &proof_ref, &bytes)
            .await?;
        let object = publication.object();
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: "taiko_dev/ethereum".to_string(),
                proof_ref: proof_ref.clone(),
                pipeline_key: pipeline,
                route,
                proof_uri: object.proof_uri.clone(),
                content_hash: object.content_hash.clone(),
                generation: object.generation,
            })
            .await?;
        runtime
            .mark_proof_artifact_invalidated(
                "taiko_dev/ethereum",
                pipeline,
                route,
                &proof_ref,
                &object.content_hash,
            )
            .await?;
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            route,
        );

        assert!(
            observer
                .load_completed_proof(
                    &task_id,
                    &EngineTask::Proposal {
                        request: request.clone(),
                    },
                )
                .await
                .map_err(anyhow::Error::msg)?
                .is_none()
        );
        assert!(
            observer
                .load_proof_artifact(&ProofArtifactRef {
                    network_pair: "taiko_dev/ethereum".to_string(),
                    pipeline_key: pipeline,
                    route,
                    proof_ref,
                })
                .await
                .map_err(anyhow::Error::msg)?
                .is_none()
        );
        Ok(())
    }

    #[tokio::test]
    async fn engine_recovery_honors_tombstone_from_another_replica() -> Result<()> {
        let artifact_root = unique_runtime_root("runtime-observer-shared-artifacts");
        let store: Arc<dyn ProofArtifactStore> = Arc::new(FilesystemProofArtifactStore::new(
            "shared-environment".to_string(),
            artifact_root,
        )?);
        let first = Arc::new(RuntimeManager::new_with_artifact_store(
            unique_runtime_root("runtime-observer-replica-a"),
            Arc::clone(&store),
        )?);
        let second = Arc::new(RuntimeManager::new_with_artifact_store(
            unique_runtime_root("runtime-observer-replica-b"),
            store,
        )?);
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let request = proposal_request();
        let task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        let proof_ref = proposal_task_ref(pipeline, &request);
        let bytes = serde_json::to_vec(&proof_fixture())?;
        let publication = first
            .publish_proof_artifact_bytes("taiko_dev/ethereum", pipeline, route, &proof_ref, &bytes)
            .await?;
        let object = publication.object();
        first
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: "taiko_dev/ethereum".to_string(),
                proof_ref: proof_ref.clone(),
                pipeline_key: pipeline,
                route,
                proof_uri: object.proof_uri.clone(),
                content_hash: object.content_hash.clone(),
                generation: object.generation,
            })
            .await?;
        first
            .mark_proof_artifact_invalidated(
                "taiko_dev/ethereum",
                pipeline,
                route,
                &proof_ref,
                &object.content_hash,
            )
            .await?;
        let observer =
            RuntimeObserver::new(Arc::clone(&second), "taiko_dev/ethereum".to_string(), route);

        assert!(
            observer
                .load_completed_proof(&task_id, &EngineTask::Proposal { request })
                .await
                .map_err(anyhow::Error::msg)?
                .is_none()
        );
        assert!(second.list_proof_artifacts().await?.is_empty());
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

        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            PipelineKey::ShastaNative.route(),
        );
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
            .await
            .map_err(anyhow::Error::msg)?;

        let cancelled = runtime
            .get_task("task_cancelled")
            .await?
            .expect("cancelled task");
        let active = runtime.get_task("task_active").await?.expect("active task");
        assert_eq!(cancelled.runner_status, RunnerStatus::Cancelled);
        assert_eq!(cancelled.proof_uri, None);
        assert_eq!(active.runner_status, RunnerStatus::Completed);
        assert!(active.proof_uri.is_some());
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

        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            PipelineKey::ShastaNative.route(),
        );
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
    async fn runtime_observer_finds_stage_bearing_legacy_proposal_record() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-legacy-stage-ref",
        ))?);
        let pipeline = PipelineKey::ShastaNative;
        let request = proposal_request();
        let task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        let legacy_ref = legacy_proposal_task_refs(pipeline, &request)
            .pop()
            .expect("prove stage ref");
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
                proposal_id: request.proposal_id,
                checkpoint: None,
                l1_inclusion_block_number: request.l1_inclusion_block_number,
                l2_block_numbers: vec![request.proposal_id],
                last_anchor_block_number: request.last_anchor_block_number,
                task_id: legacy_ref.clone(),
                request: None,
            }],
            aggregate_task_id: None,
            aggregate_request: None,
            aggregate_input_artifacts: Vec::new(),
            runtime: RuntimeMetadata::default(),
        };
        runtime
            .register_task(TaskRegistration {
                task_id: "legacy-stage-root".to_string(),
                pipeline_key: Some(pipeline),
                route: pipeline.route(),
                task_kind: "hoodi_batch".to_string(),
                proposal_id: Some(request.proposal_id),
                proof_ids: vec![legacy_ref],
                metadata: serde_json::to_value(&metadata)?,
                request_fingerprint: None,
            })
            .await?;

        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            pipeline.route(),
        );
        let records = observer.find_root_records(&task_id).await?;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].task_id, "legacy-stage-root");
        assert_eq!(
            crate::server::task_metadata::proposal_proof_artifact_refs(
                pipeline,
                &metadata.proposals[0],
            )[0],
            proposal_task_ref(pipeline, &request)
        );
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

        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            PipelineKey::ShastaNative.route(),
        );
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

        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            PipelineKey::ShastaSp1.route(),
        );
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
            .await
            .map_err(anyhow::Error::msg)?;

        let record = runtime
            .get_task("task_public_multi_proposal")
            .await?
            .expect("runtime task exists");
        assert_eq!(record.runner_status, RunnerStatus::Allocated);
        assert_eq!(record.proof_uri, None);
        assert!(
            !tokio::fs::try_exists(Path::new(&record.task_dir).join("proof.json")).await?,
            "partial proposal proof must not be persisted as final root proof"
        );
        let first_ref = proposal_task_ref(pipeline, &first_request);
        runtime
            .get_proof_artifact("taiko_dev/ethereum", pipeline, pipeline.route(), &first_ref)
            .await?
            .expect("first proposal proof artifact");
        assert!(
            runtime
                .read_proof_artifact_bytes(
                    "taiko_dev/ethereum",
                    pipeline,
                    pipeline.route(),
                    &first_ref,
                )
                .await?
                .is_some()
        );

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
            .await
            .map_err(anyhow::Error::msg)?;

        let record = runtime
            .get_task("task_public_multi_proposal")
            .await?
            .expect("runtime task exists");
        let metadata: TaskMetadata = serde_json::from_value(record.metadata)?;
        assert_eq!(record.runner_status, RunnerStatus::Completed);
        assert!(record.proof_uri.is_some());
        assert_eq!(metadata.runtime.last_event.as_deref(), Some("completed"));
        assert!(record.proof_uri.is_some());
        assert!(
            runtime
                .read_proof_artifact_bytes(
                    "taiko_dev/ethereum",
                    pipeline,
                    pipeline.route(),
                    &RuntimeObserver::root_task_ref(&second_task_id),
                )
                .await?
                .is_some()
        );
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

        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            PipelineKey::ShastaSp1.route(),
        );
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
            .await;

        let record = runtime
            .get_task("task_public_sp1")
            .await?
            .expect("runtime task exists");
        let metadata: TaskMetadata = serde_json::from_value(record.metadata)?;
        let runtime_entry = metadata
            .proposal_runtime(&task_ref)
            .expect("proposal runtime exists");
        assert_eq!(runtime_entry.provider_request_id.as_deref(), Some("0xsp1"));
        assert_eq!(
            runtime_entry.sp1_network_mode,
            Some(Sp1NetworkMode::Reserved)
        );
        assert_eq!(
            runtime_entry.sp1_fulfillment_strategy,
            Some(Sp1FulfillmentStrategy::Reserved)
        );
        assert_eq!(runtime_entry.sp1_skip_simulation, Some(true));
        assert_eq!(runtime_entry.sp1_cycle_limit, Some(1_000_000_000_000));
        assert_eq!(runtime_entry.sp1_timeout_secs, Some(3_600));
        assert_eq!(runtime_entry.sp1_max_price_per_pgu, Some(42));
        assert_eq!(runtime_entry.sp1_auction_timeout_secs, Some(120));
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
                .await
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

        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            PipelineKey::ShastaSp1.route(),
        );
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
            .await;

        assert_eq!(
            observer
                .load_sp1_network_request_id(
                    &proposal_task_id,
                    &EngineTask::ProveProposal {
                        request: proposal_request(),
                        input_task: proposal_task_id.clone(),
                    },
                )
                .await
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
                .await,
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

        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            PipelineKey::ShastaSp1.route(),
        );
        assert_eq!(
            observer
                .load_sp1_network_request_id(
                    &aggregate_task_id,
                    &EngineTask::Aggregate {
                        request: aggregate_request.clone(),
                        source: raiko2_engine::AggregationSource::Inputs(vec![]),
                    },
                )
                .await,
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
            .await;

        assert_eq!(
            observer
                .load_sp1_network_request_id(
                    &aggregate_task_id,
                    &EngineTask::Aggregate {
                        request: aggregate_request,
                        source: raiko2_engine::AggregationSource::Inputs(vec![]),
                    },
                )
                .await
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
            PipelineKey::ShastaNative.route(),
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
            PipelineKey::ShastaNative.route(),
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
        let store = Arc::new(FailingArtifactStore::default());
        let runtime = Arc::new(RuntimeManager::new_with_artifact_store(
            unique_runtime_root("runtime-observer-proof-persistence"),
            store.clone(),
        )?);
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

        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            network_pair.to_string(),
            PipelineKey::ShastaNative.route(),
        );
        let error = observer
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
            .await
            .expect_err("publication must fail");
        assert!(
            matches!(error, EngineObserverError::ProofPublication(_)),
            "{error}"
        );
        assert_eq!(store.attempts.load(Ordering::SeqCst), 3);

        let record = runtime
            .get_task("task_proof_persistence_failure")
            .await?
            .expect("runtime task exists");
        assert_eq!(record.runner_status, RunnerStatus::Allocated);
        assert!(
            record
                .error
                .as_deref()
                .is_some_and(|error| error.contains("failed to publish proof artifact")),
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
