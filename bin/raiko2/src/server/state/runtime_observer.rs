use anyhow::{Context, Result};
use async_trait::async_trait;
use raiko2_engine::{
    EngineObserver, EngineObserverError, EngineTaskId, EngineTaskKey, EngineTaskSuccess,
    ProofCompletionPermit, ProposalStage, TaskExecutionPermit, TerminalFailurePermit,
    TerminalFailureProjection,
    tasks::{EngineTask, ProofArtifactRef},
};
use raiko2_pipeline::PipelineRoute;
use raiko2_prover::{
    BoundlessSubmissionResume, NetworkProverBackend, PendingProofCheckpoint,
    PendingProofCheckpointIdentity, ProgressPersistenceError, ProverProgress,
    Sp1NetworkSubmissionProgress, SubmissionCheckpointPermit,
};
use raiko2_queue::RootOwner;
use raiko2_runtime::{
    ProofArtifactDescriptor, ProofArtifactPutResult, ProofArtifactRegistration, RunnerStatus,
    RuntimeManager, RuntimeTaskRecord,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::server::proof_artifact::ProofArtifactPayload;
#[cfg(test)]
use crate::server::task_metadata::publication_proof_artifact_refs;
use crate::server::task_metadata::{
    RemoteSubmissionConflict, TaskMetadata, TaskRuntimeMetadata, format_proposal_ids,
    proposal_task_ref, stage_task_ref_for_stage,
};
use crate::server::telemetry::{self, MetricContext};
#[cfg(test)]
use raiko2_engine::ProverTaskConfig;

#[derive(Clone)]
pub(crate) struct RuntimeObserver {
    runtime: Arc<RuntimeManager>,
    network_pair: String,
    route: PipelineRoute,
}

static STARTED_STAGE_TASKS: LazyLock<Mutex<HashMap<String, MetricContext>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Clone, Copy, PartialEq, Eq)]
enum TerminalRootPolicy {
    Exclude,
    IncludeCompleted,
    IncludeAll,
}

#[derive(Clone)]
struct RuntimeExecutionOwners {
    by_task_id: HashMap<String, uuid::Uuid>,
    incarnations: Vec<uuid::Uuid>,
}

struct RuntimeProofCompletionPermit {
    owners: RuntimeExecutionOwners,
}

struct RuntimeTerminalFailurePermit {
    active_owner_incarnations: Vec<uuid::Uuid>,
    projected_owners: Vec<RootOwner>,
    _lifecycle_guard: tokio::sync::OwnedMutexGuard<()>,
}

#[derive(Clone)]
struct PublishedProofCommit {
    proof_uri: String,
    synchronized_roots: HashSet<String>,
    root_ref: String,
    descriptor: ProofArtifactDescriptor,
}

struct ProofTaskLogContext {
    aggregate: bool,
    proposal_ids: Vec<u64>,
}

fn proof_task_log_context(id: &EngineTaskId) -> ProofTaskLogContext {
    match &id.0 {
        EngineTaskKey::Proposal { request, .. } => ProofTaskLogContext {
            aggregate: false,
            proposal_ids: vec![request.proposal_id],
        },
        EngineTaskKey::Aggregate { request, .. } => ProofTaskLogContext {
            aggregate: true,
            proposal_ids: request.proposal_ids.clone(),
        },
    }
}

enum ProofCommitAttempt {
    Committed(PublishedProofCommit),
    Retryable(anyhow::Error),
    Invalidated {
        error: anyhow::Error,
        publication: Option<PublishedProofCommit>,
    },
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
            .downcast_ref::<raiko2_runtime::PendingProofPublicationRemoved>()
            .is_some()
    {
        EngineObserverError::ProofInvalidated(message)
    } else {
        EngineObserverError::ProofPublication(message)
    }
}

impl RuntimeObserver {
    fn progress_sync_error(
        &self,
        id: &EngineTaskId,
        permit: &raiko2_runtime::RuntimeSubmissionCheckpointPermit,
        error: &anyhow::Error,
    ) -> EngineObserverError {
        tracing::warn!(task = ?id, %error, "failed to sync runtime task progress");
        let message = format!("failed to sync runtime task progress: {error}");
        if error.downcast_ref::<RemoteSubmissionConflict>().is_some() {
            return EngineObserverError::ProgressRejected(message);
        }
        if !self.runtime.checkpoint_failure_is_retryable(permit) {
            return EngineObserverError::RuntimeInactive(message);
        }
        EngineObserverError::RuntimeSync(message)
    }

    pub(crate) const fn new(
        runtime: Arc<RuntimeManager>,
        network_pair: String,
        route: PipelineRoute,
    ) -> Self {
        Self {
            runtime,
            network_pair,
            route,
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

    fn log_completed_proof_tasks(
        &self,
        id: &EngineTaskId,
        stage: &'static str,
        publication: &PublishedProofCommit,
    ) {
        let context = proof_task_log_context(id);
        let proposal_count = context.proposal_ids.len();
        let proposal_ids = format_proposal_ids(&context.proposal_ids);
        let pipeline_key = id.0.pipeline_key();
        let mut root_task_ids = publication
            .synchronized_roots
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        root_task_ids.sort_unstable();
        for task_id in root_task_ids {
            tracing::info!(
                task_id,
                aggregate = context.aggregate,
                proposal_ids,
                proposal_count,
                stage,
                route = %self.route,
                proof_type = %pipeline_key.proof_type(),
                network_pair = %self.network_pair,
                proof_ref = %publication.root_ref,
                proof_uri = %publication.proof_uri,
                content_hash = %publication.descriptor.content_hash,
                "completed shasta proof task"
            );
        }
    }

    fn execution_owners(
        permit: &TaskExecutionPermit,
    ) -> std::result::Result<&RuntimeExecutionOwners, EngineObserverError> {
        permit.guard::<RuntimeExecutionOwners>().ok_or_else(|| {
            EngineObserverError::RuntimeInactive(
                "task execution permit does not belong to the runtime".to_string(),
            )
        })
    }

    async fn find_root_records(&self, id: &EngineTaskId) -> Vec<RuntimeTaskRecord> {
        let canonical_ref = Self::root_task_ref(id);
        self.runtime.tasks_referencing(&canonical_ref).await
    }

    async fn current_execution_owners(
        &self,
        id: &EngineTaskId,
        include_completed: bool,
        expected_owners: Option<&HashMap<String, uuid::Uuid>>,
    ) -> std::result::Result<RuntimeExecutionOwners, EngineObserverError> {
        let mut owner_records = Vec::new();
        for record in self
            .runtime
            .tasks_referencing(&Self::root_task_ref(id))
            .await
        {
            if is_terminal_status(record.runner_status)
                && !(include_completed && record.runner_status == RunnerStatus::Completed)
            {
                continue;
            }
            if expected_owners
                .and_then(|owners| owners.get(&record.task_id))
                .is_some_and(|expected| *expected != record.incarnation_id)
            {
                continue;
            }
            if !self.record_matches_observer(id, &record).map_err(|error| {
                EngineObserverError::ProgressRejected(format!(
                    "failed to validate runtime owner {}: {error}",
                    record.task_id
                ))
            })? {
                continue;
            }
            owner_records.push((record.task_id, record.incarnation_id));
        }
        let mut incarnations = owner_records
            .iter()
            .map(|(_, incarnation)| *incarnation)
            .collect::<Vec<_>>();
        incarnations.sort_unstable();
        incarnations.dedup();
        Ok(RuntimeExecutionOwners {
            by_task_id: owner_records.into_iter().collect(),
            incarnations,
        })
    }

    fn stage_name(task: &EngineTask) -> &'static str {
        match task.publication_source() {
            EngineTask::Proposal { .. } => "proposal",
            EngineTask::Preflight { .. } => "preflight",
            EngineTask::Validate { .. } => "validation",
            EngineTask::Encode { .. } => "encode",
            EngineTask::ProveProposal { .. } => "prove",
            EngineTask::Aggregate { .. } => "aggregate",
            EngineTask::PublishProof { .. } => {
                unreachable!()
            }
        }
    }

    async fn update_execution_root_records<F>(
        &self,
        id: &EngineTaskId,
        owner_incarnations: &[uuid::Uuid],
        mutator: F,
    ) -> Result<()>
    where
        F: Fn(&mut RuntimeTaskRecord, i64, i64) -> Result<()>,
    {
        self.update_root_records_with_policy_filter_count_mode(
            id,
            TerminalRootPolicy::Exclude,
            |record, updated_at, observed_at_ms| {
                mutator(record, updated_at, observed_at_ms)?;
                Ok(true)
            },
            Some(owner_incarnations),
            None,
        )
        .await
        .and_then(|updated| {
            anyhow::ensure!(updated != 0, "execution permit matched no runtime root");
            Ok(())
        })
    }

    async fn checkpoint_retry_root_records<F>(
        &self,
        id: &EngineTaskId,
        owner_incarnations: &[uuid::Uuid],
        permit: &raiko2_runtime::RuntimeSubmissionCheckpointPermit,
        mutator: F,
    ) -> Result<usize>
    where
        F: Fn(&mut RuntimeTaskRecord, i64, i64) -> Result<bool>,
    {
        self.update_root_records_with_policy_filter_count_mode(
            id,
            TerminalRootPolicy::Exclude,
            mutator,
            Some(owner_incarnations),
            Some(permit),
        )
        .await
    }

    async fn update_root_records_with_policy_filter_count<F>(
        &self,
        id: &EngineTaskId,
        terminal_policy: TerminalRootPolicy,
        mutator: F,
    ) -> Result<usize>
    where
        F: Fn(&mut RuntimeTaskRecord, i64, i64) -> Result<bool>,
    {
        self.update_root_records_with_policy_filter_count_mode(
            id,
            terminal_policy,
            mutator,
            None,
            None,
        )
        .await
    }

    async fn update_root_records_with_policy_filter_count_mode<F>(
        &self,
        id: &EngineTaskId,
        terminal_policy: TerminalRootPolicy,
        mutator: F,
        owner_incarnations: Option<&[uuid::Uuid]>,
        checkpoint_permit: Option<&raiko2_runtime::RuntimeSubmissionCheckpointPermit>,
    ) -> Result<usize>
    where
        F: Fn(&mut RuntimeTaskRecord, i64, i64) -> Result<bool>,
    {
        let root_ref = Self::root_task_ref(id);
        let update = |records: &mut Vec<RuntimeTaskRecord>| {
            if records.is_empty() {
                anyhow::bail!("runtime task not registered for task ref {root_ref}");
            }
            let updated_at = now_ts();
            let observed_at_ms = now_ms();
            let mut updated_records = 0;
            for record in records {
                if owner_incarnations.is_some_and(|owners| !owners.contains(&record.incarnation_id))
                    || (is_terminal_status(record.runner_status)
                        && !matches!(
                            (terminal_policy, record.runner_status),
                            (
                                TerminalRootPolicy::IncludeCompleted,
                                RunnerStatus::Completed
                            ) | (TerminalRootPolicy::IncludeAll, _)
                        ))
                {
                    continue;
                }
                let matches_observer = match self.record_matches_observer(id, record) {
                    Ok(matches_observer) => matches_observer,
                    Err(error) if checkpoint_permit.is_some() => {
                        return Err(RemoteSubmissionConflict::new(format!(
                            "provider checkpoint owner metadata is invalid: {error}"
                        ))
                        .into());
                    }
                    Err(error) => return Err(error),
                };
                if !matches_observer {
                    continue;
                }
                if !mutator(record, updated_at, observed_at_ms)? {
                    continue;
                }
                record.updated_at = updated_at;
                updated_records += 1;
            }
            Ok(updated_records)
        };
        if let Some(permit) = checkpoint_permit {
            self.runtime
                .checkpoint_tasks_by_ref(permit, &root_ref, update)
                .await
        } else {
            self.runtime.update_tasks_by_ref(&root_ref, update).await
        }
    }

    async fn load_root_record(&self, id: &EngineTaskId) -> Result<Option<RuntimeTaskRecord>> {
        let records = self.find_root_records(id).await;
        Ok(self
            .matching_active_root_records(id, records)?
            .into_iter()
            .next())
    }

    async fn load_root_record_including_terminal(
        &self,
        id: &EngineTaskId,
    ) -> Result<Option<RuntimeTaskRecord>> {
        let records = self.find_root_records(id).await;
        Ok(self
            .matching_root_records(id, records, TerminalRootPolicy::IncludeAll)?
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
                    if terminal_policy == TerminalRootPolicy::IncludeCompleted
                        && record.runner_status == RunnerStatus::Completed =>
                {
                    Some(Ok(record))
                }
                Ok(true) if terminal_policy == TerminalRootPolicy::IncludeAll => Some(Ok(record)),
                Ok(_) => None,
                Err(err) => Some(Err(err)),
            })
            .collect()
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
        if record.network_pair != self.network_pair {
            return Ok(false);
        }
        let metadata = TaskMetadata::decode_for_record(record)?;
        Ok(metadata.owns_engine_task(id))
    }

    const fn expected_proof_payload(id: &EngineTaskId) -> ProofArtifactPayload {
        match id.0 {
            EngineTaskKey::Proposal { .. } => ProofArtifactPayload::Proposal,
            EngineTaskKey::Aggregate { .. } => ProofArtifactPayload::Final,
        }
    }

    fn metric_context(record: &RuntimeTaskRecord) -> Result<MetricContext> {
        let metadata = TaskMetadata::decode_for_record(record)
            .context("failed to validate runtime task metadata for telemetry")?;
        Ok(MetricContext::new(
            record.route.to_string(),
            metadata.proof_type,
            record.network_pair.clone(),
            metadata.aggregate_requested,
        ))
    }

    const fn submission_provider(progress: &ProverProgress) -> &'static str {
        match progress {
            ProverProgress::BoundlessSubmission(_) => "boundless",
            ProverProgress::Sp1NetworkSubmission(_) => "sp1_network",
        }
    }

    async fn record_external_submission_metrics(
        &self,
        id: &EngineTaskId,
        stage: &'static str,
        progress: &ProverProgress,
    ) {
        match self.load_root_record(id).await {
            Ok(Some(record)) => match Self::metric_context(&record) {
                Ok(context) => telemetry::record_external_submission(
                    &context,
                    stage,
                    Self::submission_provider(progress),
                ),
                Err(error) => {
                    tracing::warn!(task = ?id, %error, "failed to build telemetry context");
                }
            },
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(task = ?id, %error, "failed to load runtime task for telemetry");
            }
        }
    }

    fn proposal_stage_from_task(task: &EngineTask) -> Option<ProposalStage> {
        match task.publication_source() {
            EngineTask::Preflight { .. } => Some(ProposalStage::Preflight),
            EngineTask::Validate { .. } => Some(ProposalStage::Validation),
            EngineTask::Encode { .. } => Some(ProposalStage::Encode),
            EngineTask::ProveProposal { .. } => Some(ProposalStage::Prove),
            EngineTask::Proposal { .. } | EngineTask::Aggregate { .. } => None,
            EngineTask::PublishProof { .. } => {
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

    fn mark_stage_started_for_metrics(
        &self,
        id: &EngineTaskId,
        task: &EngineTask,
        context: MetricContext,
    ) -> bool {
        let task_id = self.metric_tracking_key(&Self::timing_key_for_task(id, task));
        let mut started = STARTED_STAGE_TASKS
            .lock()
            .expect("stage task telemetry mutex poisoned");
        match started.entry(task_id) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(context);
                true
            }
            std::collections::hash_map::Entry::Occupied(_) => false,
        }
    }

    fn metric_tracking_key(&self, task_id: &str) -> String {
        format!("{}|{}|{task_id}", self.network_pair, self.route)
    }

    fn stage_duration_secs(
        record: &RuntimeTaskRecord,
        task_id: &str,
        finished_at_ms: i64,
    ) -> Result<Option<f64>> {
        let metadata = TaskMetadata::decode_for_record(record)
            .context("failed to validate runtime task metadata for stage duration")?;
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
        let started_context = {
            let mut started = STARTED_STAGE_TASKS
                .lock()
                .expect("stage task telemetry mutex poisoned");
            started.remove(&self.metric_tracking_key(task_id))
        };
        let record = match self.load_root_record_including_terminal(id).await {
            Ok(record) => record,
            Err(err) => {
                tracing::warn!(
                    task = ?id,
                    error = %err,
                    "failed to load runtime task for telemetry"
                );
                None
            }
        };
        let should_decrement = started_context.is_some();
        let context = started_context.or_else(|| {
            record
                .as_ref()
                .and_then(|record| match Self::metric_context(record) {
                    Ok(context) => Some(context),
                    Err(err) => {
                        tracing::warn!(
                            task = ?id,
                            error = %err,
                            "failed to build telemetry context"
                        );
                        None
                    }
                })
        });
        let Some(context) = context else {
            return;
        };
        telemetry::record_stage_task_terminal(&context, stage, status, should_decrement);
        if let Some(error) = failure_error {
            telemetry::record_stage_task_failure(&context, stage, error);
        }
        if let Some(record) = record {
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
    }

    async fn observe_cancelled_stage_metrics(&self, id: &EngineTaskId) {
        let stages = match &id.0 {
            EngineTaskKey::Proposal { .. } => [
                (ProposalStage::Preflight, "preflight"),
                (ProposalStage::Validation, "validation"),
                (ProposalStage::Encode, "encode"),
                (ProposalStage::Prove, "prove"),
            ]
            .into_iter()
            .map(|(stage, name)| (Self::timing_key_for_stage(id, stage), name))
            .collect::<Vec<_>>(),
            EngineTaskKey::Aggregate { pipeline, request } => vec![(
                crate::server::task_metadata::aggregate_task_ref(*pipeline, request),
                "aggregate",
            )],
        };
        let cancelled = {
            let mut started = STARTED_STAGE_TASKS
                .lock()
                .expect("stage task telemetry mutex poisoned");
            stages
                .into_iter()
                .filter_map(|(task_id, stage)| {
                    started
                        .remove(&self.metric_tracking_key(&task_id))
                        .map(|context| (task_id, stage, context))
                })
                .collect::<Vec<_>>()
        };
        if cancelled.is_empty() {
            return;
        }
        let record = match self.load_root_record_including_terminal(id).await {
            Ok(record) => record,
            Err(error) => {
                tracing::warn!(task = ?id, %error, "failed to load cancelled root telemetry");
                None
            }
        };
        for (task_id, stage, context) in cancelled {
            telemetry::record_stage_task_terminal(&context, stage, "cancelled", true);
            if let Some(record) = record.as_ref()
                && let Ok(Some(duration_seconds)) =
                    Self::stage_duration_secs(record, &task_id, now_ms())
            {
                telemetry::record_stage_task_duration(
                    &context,
                    stage,
                    "cancelled",
                    duration_seconds,
                );
            }
        }
    }

    async fn publish_final_proof_artifact(
        &self,
        id: &EngineTaskId,
        _task: &EngineTask,
        stage: &str,
        proof: &raiko2_primitives::Proof,
    ) -> Result<PublishedProofCommit> {
        let root_ref = Self::root_task_ref(id);
        let records = self.find_root_records(id).await;
        // Reconciliation may complete a root after the canonical artifact commit but before this
        // publication retry. Completed is therefore still an eligible owner; failed/cancelled
        // roots remain excluded so late worker results cannot reopen terminal requests.
        let records =
            self.matching_root_records(id, records, TerminalRootPolicy::IncludeCompleted)?;
        if records.is_empty() {
            let invalidated = self
                .runtime
                .invalidate_pending_proof_publication(
                    &self.network_pair,
                    id.0.pipeline_key(),
                    self.route,
                    &root_ref,
                )
                .await
                .context("failed to invalidate detached proof publication")?;
            anyhow::ensure!(
                invalidated,
                "runtime owner appeared while invalidating detached proof publication"
            );
            return Err(ProofInvalidatedError(format!(
                "proof task {root_ref} no longer has an active runtime owner"
            ))
            .into());
        }
        anyhow::ensure!(
            Self::expected_proof_payload(id).accepts(id.0.pipeline_key(), proof),
            "refusing to publish an invalid {:?} proof artifact payload",
            Self::expected_proof_payload(id)
        );

        let proof_bytes = serde_json::to_vec(proof).context("failed to serialize proof output")?;
        let publication = self
            .runtime
            .commit_proof_artifact_publication(
                &self.network_pair,
                id.0.pipeline_key(),
                self.route,
                &root_ref,
                &proof_bytes,
            )
            .await
            .context("failed to commit proof artifact publication")?;
        let artifact = publication
            .try_object()
            .context("committed proof conflict references missing content")?;
        if matches!(publication, ProofArtifactPutResult::Conflict(_)) {
            tracing::warn!(
                task = ?id,
                stage,
                proof_uri = %artifact.proof_uri,
                content_hash = %artifact.content_hash,
                "discarding late proof because a different canonical artifact already exists"
            );
        }
        Ok(PublishedProofCommit {
            proof_uri: artifact.proof_uri.clone(),
            synchronized_roots: HashSet::new(),
            root_ref,
            descriptor: artifact.descriptor(),
        })
    }

    async fn mark_proof_publication_failed(
        &self,
        id: &EngineTaskId,
        stage: &'static str,
        message: &str,
        disposition: PublicationFailureDisposition,
        publication: Option<&PublishedProofCommit>,
        owner_incarnations: &[uuid::Uuid],
    ) {
        let message = message.to_string();
        if matches!(disposition, PublicationFailureDisposition::Invalidated) {
            self.invalidate_failed_publication(id, stage, message, publication, owner_incarnations)
                .await;
            return;
        }
        if let Err(sync_err) = self
            .update_root_records_with_policy_filter_count(
                id,
                TerminalRootPolicy::Exclude,
                |record, updated_at, observed_at_ms| {
                    if !owner_incarnations.contains(&record.incarnation_id) {
                        return Ok(false);
                    }
                    record.runner_status = RunnerStatus::Allocated;
                    record.proof_uri = None;
                    record.error = Some(message.clone());
                    let task_id = Self::timing_key_for_stage_name(id, stage);
                    update_task_metadata(record, |metadata| {
                        metadata.mark_stage_terminal(&task_id, stage, observed_at_ms, "failed");
                        metadata.runtime.active_stage = Some(stage.to_string());
                        metadata.runtime.last_event = Some("failed".to_string());
                        metadata.runtime.proof_artifact = None;
                        Ok(())
                    })?;
                    record.updated_at = updated_at;
                    Ok(true)
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

    async fn invalidate_failed_publication(
        &self,
        id: &EngineTaskId,
        stage: &'static str,
        message: String,
        publication: Option<&PublishedProofCommit>,
        owner_incarnations: &[uuid::Uuid],
    ) {
        let root_ref = Self::root_task_ref(id);
        let transition = self
            .runtime
            .update_tasks_and_invalidate_artifact(
                &root_ref,
                &self.network_pair,
                id.0.pipeline_key(),
                self.route,
                |records| {
                    let updated_at = now_ts();
                    let observed_at_ms = now_ms();
                    let mut completed_root_preserves_artifact = false;
                    let mut matched_owner = false;
                    for record in records {
                        if !owner_incarnations.contains(&record.incarnation_id)
                            || !self.record_matches_observer(id, record)?
                            || (is_terminal_status(record.runner_status)
                                && record.runner_status != RunnerStatus::Completed)
                        {
                            continue;
                        }
                        matched_owner = true;
                        if record.runner_status == RunnerStatus::Completed
                            && publication.is_none_or(|publication| {
                                !publication.synchronized_roots.contains(&record.task_id)
                            })
                        {
                            completed_root_preserves_artifact = true;
                            continue;
                        }
                        record.runner_status = RunnerStatus::Cancelled;
                        record.proof_uri = None;
                        record.error = Some(message.clone());
                        let task_id = Self::timing_key_for_stage_name(id, stage);
                        update_task_metadata(record, |metadata| {
                            metadata.mark_stage_terminal(
                                &task_id,
                                stage,
                                observed_at_ms,
                                "cancelled",
                            );
                            metadata.runtime.active_stage = None;
                            metadata.runtime.last_event = Some("cancelled".to_string());
                            metadata.runtime.proof_artifact = None;
                            Ok(())
                        })?;
                        record.updated_at = updated_at;
                    }
                    let invalidate = matched_owner && !completed_root_preserves_artifact;
                    Ok((invalidate, invalidate))
                },
            )
            .await;
        let should_invalidate = match transition {
            Ok((_, should_invalidate, _)) => should_invalidate,
            Err(error) => {
                tracing::warn!(task = ?id, stage, %error, "failed to atomically invalidate proof publication and runtime roots");
                return;
            }
        };
        let cleanup = if should_invalidate {
            self.runtime
                .invalidate_pending_proof_publication(
                    &self.network_pair,
                    id.0.pipeline_key(),
                    self.route,
                    &root_ref,
                )
                .await
        } else {
            self.runtime
                .remove_pending_proof_publication_if_unowned(
                    &self.network_pair,
                    id.0.pipeline_key(),
                    self.route,
                    &root_ref,
                )
                .await
        };
        if let Err(error) = cleanup {
            tracing::warn!(task = ?id, stage, %error, "failed to finalize proof invalidation");
        }
    }

    async fn handle_proof_success(
        &self,
        id: &EngineTaskId,
        task: &EngineTask,
        stage: &'static str,
        finished_at_ms: i64,
        proof: &raiko2_primitives::Proof,
        completion_owners: &RuntimeExecutionOwners,
    ) -> Result<()> {
        let task_id = Self::timing_key_for_task(id, task);
        let publication_delays = [Duration::from_millis(100), Duration::from_millis(500)];
        for attempt in 0..=publication_delays.len() {
            match self
                .commit_proof_attempt(id, task, stage, proof, completion_owners)
                .await
            {
                ProofCommitAttempt::Committed(publication) => {
                    self.observe_stage_terminal_metrics(
                        id,
                        &task_id,
                        stage,
                        "completed",
                        finished_at_ms,
                        None,
                    )
                    .await;
                    self.log_completed_proof_tasks(id, stage, &publication);
                    return Ok(());
                }
                ProofCommitAttempt::Invalidated { error, publication } => {
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
                        publication.as_ref(),
                        &completion_owners.incarnations,
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
                        None,
                        &completion_owners.incarnations,
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
        completion_owners: &RuntimeExecutionOwners,
    ) -> ProofCommitAttempt {
        let publication = match self
            .publish_final_proof_artifact(id, task, stage, proof)
            .await
        {
            Ok(publication) => publication,
            Err(error) if error.downcast_ref::<ProofInvalidatedError>().is_some() => {
                return ProofCommitAttempt::Invalidated {
                    error,
                    publication: None,
                };
            }
            Err(error) => return ProofCommitAttempt::Retryable(error),
        };
        match self
            .runtime
            .proof_artifact_descriptor_is_current(
                &self.network_pair,
                id.0.pipeline_key(),
                self.route,
                &publication.root_ref,
                &publication.descriptor,
            )
            .await
            .context("failed to verify canonical proof publication before root synchronization")
        {
            Ok(true) => {}
            Ok(false) => {
                return ProofCommitAttempt::Invalidated {
                    error: anyhow::anyhow!(
                        "canonical proof artifact {} changed before root synchronization",
                        publication.root_ref
                    ),
                    publication: Some(publication),
                };
            }
            Err(error) => return ProofCommitAttempt::Retryable(error),
        }
        let (updated_roots, synchronized_roots) = match self
            .sync_proof_success(id, task, stage, &publication, completion_owners)
            .await
        {
            Ok(updated_roots) => updated_roots,
            Err(error) => return ProofCommitAttempt::Retryable(error),
        };
        self.finalize_committed_publication(id, publication, updated_roots, synchronized_roots)
            .await
    }

    async fn finalize_committed_publication(
        &self,
        id: &EngineTaskId,
        mut publication: PublishedProofCommit,
        updated_roots: usize,
        synchronized_roots: HashSet<String>,
    ) -> ProofCommitAttempt {
        if updated_roots == 0 {
            let invalidation = self
                .runtime
                .invalidate_pending_proof_publication(
                    &self.network_pair,
                    id.0.pipeline_key(),
                    self.route,
                    &publication.root_ref,
                )
                .await
                .context("failed to invalidate proof publication without an active runtime owner");
            return match invalidation {
                Ok(true) => ProofCommitAttempt::Invalidated {
                    error: anyhow::anyhow!(
                        "proof task {} no longer has an active runtime owner",
                        publication.root_ref
                    ),
                    publication: Some(publication),
                },
                Ok(false) => ProofCommitAttempt::Retryable(anyhow::anyhow!(
                    "runtime owner appeared while invalidating proof publication {}",
                    publication.root_ref
                )),
                Err(error) => ProofCommitAttempt::Retryable(error),
            };
        }
        if let Err(error) = self
            .runtime
            .remove_pending_proof_publication_if_unowned(
                &self.network_pair,
                id.0.pipeline_key(),
                self.route,
                &publication.root_ref,
            )
            .await
            .context("failed to clear committed proof publication outbox")
        {
            return ProofCommitAttempt::Retryable(error);
        }
        publication.synchronized_roots = synchronized_roots;
        match self
            .runtime
            .get_proof_artifact(
                &self.network_pair,
                id.0.pipeline_key(),
                self.route,
                &publication.root_ref,
            )
            .await
            .context("failed to fence completed proof against invalidation")
        {
            Ok(Some(record)) if record.descriptor() == publication.descriptor => {
                ProofCommitAttempt::Committed(publication)
            }
            Ok(_) => ProofCommitAttempt::Invalidated {
                error: anyhow::anyhow!(
                    "canonical proof artifact {} was invalidated during completion",
                    publication.root_ref
                ),
                publication: Some(publication),
            },
            Err(error) => ProofCommitAttempt::Retryable(error),
        }
    }

    async fn current_owner_incarnations(
        &self,
        id: &EngineTaskId,
        execution_owners: &RuntimeExecutionOwners,
        include_completed: bool,
    ) -> std::result::Result<Vec<uuid::Uuid>, EngineObserverError> {
        Ok(self
            .current_execution_owners(id, include_completed, Some(&execution_owners.by_task_id))
            .await?
            .incarnations)
    }

    async fn sync_proof_success(
        &self,
        id: &EngineTaskId,
        task: &EngineTask,
        stage: &'static str,
        publication: &PublishedProofCommit,
        completion_owners: &RuntimeExecutionOwners,
    ) -> Result<(usize, HashSet<String>)> {
        let gate = self.runtime.execution_lifecycle_gate();
        let _lifecycle_guard = gate.lock().await;
        let owner_incarnations = self
            .current_owner_incarnations(id, completion_owners, true)
            .await?;
        if owner_incarnations.is_empty() {
            return Ok((0, HashSet::new()));
        }
        let registration = ProofArtifactRegistration {
            network_pair: self.network_pair.clone(),
            proof_ref: publication.root_ref.clone(),
            pipeline_key: id.0.pipeline_key(),
            route: self.route,
            proof_uri: publication.descriptor.proof_uri.clone(),
            content_hash: publication.descriptor.content_hash.clone(),
            generation: publication.descriptor.generation,
        };
        let committed = self
            .runtime
            .activate_proof_artifact_with_tasks(
                &publication.root_ref,
                registration,
                &owner_incarnations,
                |records| {
                    let updated_at = now_ts();
                    let observed_at_ms = now_ms();
                    let mut synchronized_roots = HashSet::new();
                    let mut updated_roots = 0usize;
                    for record in records {
                        if !owner_incarnations.contains(&record.incarnation_id)
                            || !self.record_matches_observer(id, record)?
                            || (is_terminal_status(record.runner_status)
                                && record.runner_status != RunnerStatus::Completed)
                        {
                            continue;
                        }
                        record.error = None;
                        let task_id = Self::timing_key_for_task(id, task);
                        let mut metadata = TaskMetadata::decode_for_record(record)?;
                        metadata.mark_stage_terminal(&task_id, stage, observed_at_ms, "completed");
                        let root_completed = Self::root_completed_by_proof_success(
                            id,
                            &metadata,
                            record.pipeline_key,
                        );
                        if record.runner_status == RunnerStatus::Completed {
                            if !root_completed {
                                continue;
                            }
                            if record.proof_uri.as_deref() == Some(publication.proof_uri.as_str())
                                && metadata.runtime.proof_artifact.as_ref()
                                    == Some(&publication.descriptor)
                            {
                                synchronized_roots.insert(record.task_id.clone());
                                updated_roots += 1;
                                continue;
                            }
                            if record.proof_uri.is_some() {
                                continue;
                            }
                        }
                        metadata.runtime.active_stage = Some(stage.to_string());
                        metadata.runtime.last_event = Some(
                            if root_completed {
                                "completed"
                            } else {
                                "stage_completed"
                            }
                            .to_string(),
                        );
                        metadata.runtime.proof_artifact =
                            root_completed.then(|| publication.descriptor.clone());
                        record.metadata = serde_json::to_value(metadata)
                            .context("failed to serialize task metadata")?;
                        if root_completed {
                            synchronized_roots.insert(record.task_id.clone());
                            record.runner_status = RunnerStatus::Completed;
                            record.proof_uri = Some(publication.proof_uri.clone());
                        } else {
                            record.runner_status = RunnerStatus::Allocated;
                            record.proof_uri = None;
                        }
                        record.updated_at = updated_at;
                        updated_roots += 1;
                    }
                    Ok((updated_roots > 0).then_some((updated_roots, synchronized_roots)))
                },
            )
            .await
            .context("failed to sync runtime task success")?;
        Ok(committed.unwrap_or_else(|| (0, HashSet::new())))
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
                        let task_id = proposal.engine_task_id(pipeline_key);
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
    async fn acquire_task_execution_permit(
        &self,
        id: &EngineTaskId,
        task: &EngineTask,
    ) -> std::result::Result<TaskExecutionPermit, EngineObserverError> {
        let proof_ref = Self::root_task_ref(id);
        // A publication-only retry may resume after root synchronization already committed
        // `Completed`. It owns no further proving work, but it must retain authority to finish the
        // pending artifact saga and make the queue result durable.
        let owners = self
            .current_execution_owners(id, matches!(task, EngineTask::PublishProof { .. }), None)
            .await?;
        if owners.incarnations.is_empty() {
            if matches!(
                task.publication_source(),
                EngineTask::ProveProposal { .. }
                    | EngineTask::Aggregate { .. }
                    | EngineTask::PublishProof { .. }
            ) {
                let invalidated = self
                    .runtime
                    .invalidate_pending_proof_publication(
                        &self.network_pair,
                        id.0.pipeline_key(),
                        self.route,
                        &proof_ref,
                    )
                    .await
                    .map_err(|error| publication_observer_error(&error))?;
                if !invalidated {
                    return Err(EngineObserverError::RuntimeSync(format!(
                        "runtime owner appeared while invalidating detached task {proof_ref}"
                    )));
                }
            }
            return Err(EngineObserverError::ProofInvalidated(format!(
                "task {proof_ref} has no active runtime owner"
            )));
        }
        Ok(TaskExecutionPermit::tracked(owners))
    }

    async fn acquire_submission_checkpoint_permit(
        &self,
    ) -> std::result::Result<SubmissionCheckpointPermit, ProgressPersistenceError> {
        self.runtime
            .acquire_submission_checkpoint_permit()
            .map(SubmissionCheckpointPermit::tracked)
            .map_err(|error| ProgressPersistenceError::Permanent(error.to_string()))
    }

    async fn acquire_terminal_failure_permit(
        &self,
        id: &EngineTaskId,
        _task: &EngineTask,
        execution_permit: &TaskExecutionPermit,
    ) -> std::result::Result<TerminalFailurePermit, EngineObserverError> {
        let execution_owners = Self::execution_owners(execution_permit)?.clone();
        let gate = self.runtime.execution_lifecycle_gate();
        let lifecycle_guard = gate.lock_owned().await;
        let active_owners = self
            .current_execution_owners(id, false, Some(&execution_owners.by_task_id))
            .await?
            .by_task_id;
        let mut active_owner_incarnations = active_owners.values().copied().collect::<Vec<_>>();
        active_owner_incarnations.sort_unstable();
        active_owner_incarnations.dedup();
        let mut projected_owners = execution_owners
            .by_task_id
            .into_iter()
            .chain(active_owners)
            .map(|(task_id, incarnation_id)| RootOwner::new(task_id, incarnation_id))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        projected_owners.sort_unstable_by(|left, right| {
            left.task_id
                .cmp(&right.task_id)
                .then(left.incarnation_id.cmp(&right.incarnation_id))
        });
        Ok(TerminalFailurePermit::tracked(
            RuntimeTerminalFailurePermit {
                active_owner_incarnations,
                projected_owners,
                _lifecycle_guard: lifecycle_guard,
            },
        ))
    }

    async fn checkpoint_completed_proof(
        &self,
        id: &EngineTaskId,
        _task: &EngineTask,
        proof: &raiko2_primitives::Proof,
        execution_permit: &TaskExecutionPermit,
    ) -> std::result::Result<ProofCompletionPermit, EngineObserverError> {
        let proof_ref = Self::root_task_ref(id);
        let execution_owners = Self::execution_owners(execution_permit)?;
        let active_owners = self
            .current_execution_owners(id, true, Some(&execution_owners.by_task_id))
            .await?
            .by_task_id;
        if active_owners.is_empty() {
            let invalidated = self
                .runtime
                .invalidate_pending_proof_publication(
                    &self.network_pair,
                    id.0.pipeline_key(),
                    self.route,
                    &proof_ref,
                )
                .await
                .map_err(|error| publication_observer_error(&error))?;
            if !invalidated {
                return Err(EngineObserverError::RuntimeSync(format!(
                    "runtime owner appeared while invalidating completed proof {proof_ref}"
                )));
            }
            return Err(EngineObserverError::ProofInvalidated(format!(
                "proof task {proof_ref} has no active runtime owner"
            )));
        }
        let bytes = serde_json::to_vec(proof).map_err(|error| {
            EngineObserverError::ProofPublication(format!(
                "failed to serialize pending proof publication: {error}"
            ))
        })?;
        let mut owner_incarnations = active_owners.values().copied().collect::<Vec<_>>();
        owner_incarnations.sort_unstable();
        owner_incarnations.dedup();
        let checkpointed = self
            .runtime
            .checkpoint_pending_proof_publication(
                &self.network_pair,
                id.0.pipeline_key(),
                self.route,
                &proof_ref,
                &owner_incarnations,
                &bytes,
            )
            .await
            .map_err(|error| publication_observer_error(&error))?;
        if checkpointed {
            return Ok(ProofCompletionPermit::tracked(
                RuntimeProofCompletionPermit {
                    owners: RuntimeExecutionOwners {
                        by_task_id: active_owners,
                        incarnations: owner_incarnations,
                    },
                },
            ));
        }
        Err(EngineObserverError::ProofInvalidated(format!(
            "proof task {proof_ref} lost its runtime owner during checkpoint"
        )))
    }

    async fn load_completed_proof(
        &self,
        id: &EngineTaskId,
        _task: &EngineTask,
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
            Self::expected_proof_payload(id),
        )
        .await
        .map_err(|error| error.to_string())?;
        let proof = if let Some(material) = material {
            material.proof
        } else {
            let Some(pending) = self
                .runtime
                .get_recoverable_pending_proof_publication(
                    &self.network_pair,
                    pipeline_key,
                    self.route,
                    &proof_ref,
                )
                .await
                .map_err(|error| error.to_string())?
            else {
                return Ok(None);
            };
            serde_json::from_slice(&pending.bytes).map_err(|error| {
                format!("invalid pending proof publication {proof_ref}: {error}")
            })?
        };
        if !Self::expected_proof_payload(id).accepts(pipeline_key, &proof) {
            return Err(format!(
                "completed proof artifact {proof_ref} has an invalid {:?} payload",
                Self::expected_proof_payload(id)
            ));
        }
        Ok(Some(proof))
    }

    async fn on_task_started(
        &self,
        id: &EngineTaskId,
        task: &EngineTask,
        worker: &str,
        execution_permit: &TaskExecutionPermit,
    ) {
        let owner_incarnations = match Self::execution_owners(execution_permit) {
            Ok(owners) => &owners.incarnations,
            Err(error) => {
                tracing::warn!(task = ?id, %error, "rejected foreign task execution permit");
                return;
            }
        };
        let stage = Self::stage_name(task);
        if let Err(err) = self
            .update_execution_root_records(
                id,
                owner_incarnations,
                |record, updated_at, observed_at_ms| {
                    record.runner_status = RunnerStatus::Allocated;
                    record.error = None;
                    let task_id = Self::timing_key_for_task(id, task);
                    update_task_metadata(record, |metadata| {
                        metadata.mark_stage_started(&task_id, stage, observed_at_ms);
                        metadata.runtime.active_stage = Some(stage.to_string());
                        metadata.runtime.last_event = Some(format!("started:{worker}"));
                        Ok(())
                    })?;
                    record.updated_at = updated_at;
                    Ok(())
                },
            )
            .await
        {
            tracing::warn!(task = ?id, error = %err, "failed to sync runtime task start");
            return;
        }
        match self.load_root_record(id).await {
            Ok(Some(record)) => match Self::metric_context(&record) {
                Ok(context) => {
                    if self.mark_stage_started_for_metrics(id, task, context.clone()) {
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
    }

    async fn on_task_cancelled(&self, id: &EngineTaskId) {
        self.observe_cancelled_stage_metrics(id).await;
    }

    async fn on_task_progress(
        &self,
        id: &EngineTaskId,
        task: &EngineTask,
        progress: &ProverProgress,
        permit: &SubmissionCheckpointPermit,
        execution_permit: &TaskExecutionPermit,
    ) -> std::result::Result<(), EngineObserverError> {
        let execution_owners = Self::execution_owners(execution_permit)?.clone();
        let stage = Self::stage_name(task);
        let runtime_permit = permit
            .guard::<raiko2_runtime::RuntimeSubmissionCheckpointPermit>()
            .ok_or_else(|| {
                EngineObserverError::RuntimeInactive(
                    "submission checkpoint permit does not belong to the runtime".to_string(),
                )
            })?;
        let gate = self.runtime.execution_lifecycle_gate();
        let _lifecycle_guard = gate.lock().await;
        let owner_incarnations = self
            .current_owner_incarnations(id, &execution_owners, false)
            .await?;
        if owner_incarnations.is_empty() {
            return Err(EngineObserverError::RuntimeInactive(format!(
                "provider checkpoint matched no active runtime root for {}",
                Self::root_task_ref(id)
            )));
        }
        let new_submission = AtomicBool::new(false);
        let result = self
            .checkpoint_retry_root_records(
                id,
                &owner_incarnations,
                runtime_permit,
                |record, updated_at, _observed_at_ms| {
                    let task_id = Self::root_task_ref(id);
                    update_task_metadata(record, |metadata| {
                        let applied_new_submission = match progress {
                            ProverProgress::BoundlessSubmission(submission) => {
                                match task.publication_source() {
                                    EngineTask::ProveProposal { .. } => metadata
                                        .upsert_proposal_runtime(&task_id, submission, updated_at),
                                    EngineTask::Aggregate { .. } => {
                                        metadata.upsert_aggregate_runtime(submission, updated_at)
                                    }
                                    EngineTask::Preflight { .. }
                                    | EngineTask::Validate { .. }
                                    | EngineTask::Encode { .. }
                                    | EngineTask::Proposal { .. }
                                    | EngineTask::PublishProof { .. } => unreachable!(),
                                }
                            }
                            ProverProgress::Sp1NetworkSubmission(submission) => {
                                match task.publication_source() {
                                    EngineTask::ProveProposal { .. } => metadata
                                        .upsert_proposal_sp1_network_runtime(
                                            &task_id, submission, updated_at,
                                        ),
                                    EngineTask::Aggregate { .. } => metadata
                                        .upsert_aggregate_sp1_network_runtime(
                                            submission, updated_at,
                                        ),
                                    EngineTask::Preflight { .. }
                                    | EngineTask::Validate { .. }
                                    | EngineTask::Encode { .. }
                                    | EngineTask::Proposal { .. }
                                    | EngineTask::PublishProof { .. } => unreachable!(),
                                }
                            }
                        }?;
                        new_submission.fetch_or(applied_new_submission, Ordering::Relaxed);
                        metadata.runtime.active_stage = Some(stage.to_string());
                        metadata.runtime.last_event = Some("submission_registered".to_string());
                        Ok(())
                    })
                    .map_err(|error| {
                        RemoteSubmissionConflict::new(format!(
                            "provider progress is not a canonical runtime checkpoint: {error}"
                        ))
                    })?;
                    record.runner_status = RunnerStatus::Allocated;
                    record.error = None;
                    Ok(true)
                },
            )
            .await;
        match result {
            Ok(updated) if updated > 0 => {
                if new_submission.load(Ordering::Relaxed) {
                    self.record_external_submission_metrics(id, stage, progress)
                        .await;
                }
                Ok(())
            }
            Ok(_) => Err(EngineObserverError::RuntimeInactive(format!(
                "provider checkpoint matched no active runtime root for {}",
                Self::root_task_ref(id)
            ))),
            Err(error) => Err(self.progress_sync_error(id, runtime_permit, &error)),
        }
    }

    async fn clear_pending_proof_checkpoint(
        &self,
        id: &EngineTaskId,
        task: &EngineTask,
        identity: &PendingProofCheckpointIdentity,
        permit: &SubmissionCheckpointPermit,
        execution_permit: &TaskExecutionPermit,
    ) -> std::result::Result<(), EngineObserverError> {
        let execution_owners = Self::execution_owners(execution_permit)?.clone();
        let runtime_permit = permit
            .guard::<raiko2_runtime::RuntimeSubmissionCheckpointPermit>()
            .ok_or_else(|| {
                EngineObserverError::RuntimeInactive(
                    "submission checkpoint permit does not belong to the runtime".to_string(),
                )
            })?;
        let gate = self.runtime.execution_lifecycle_gate();
        let _lifecycle_guard = gate.lock().await;
        let owner_incarnations = self
            .current_owner_incarnations(id, &execution_owners, false)
            .await?;
        if owner_incarnations.is_empty() {
            return Err(EngineObserverError::RuntimeInactive(format!(
                "provider checkpoint matched no active runtime root for {}",
                Self::root_task_ref(id)
            )));
        }

        let task_id = Self::root_task_ref(id);
        let result = self
            .checkpoint_retry_root_records(
                id,
                &owner_incarnations,
                runtime_permit,
                |record, updated_at, _observed_at_ms| {
                    update_task_metadata(record, |metadata| {
                        let runtime = match task.publication_source() {
                            EngineTask::ProveProposal { .. } => metadata
                                .runtime
                                .proposals
                                .get_mut(&task_id)
                                .context("proposal provider checkpoint is missing"),
                            EngineTask::Aggregate { .. } => metadata
                                .runtime
                                .aggregate
                                .as_mut()
                                .context("aggregate provider checkpoint is missing"),
                            EngineTask::Preflight { .. }
                            | EngineTask::Validate { .. }
                            | EngineTask::Encode { .. }
                            | EngineTask::Proposal { .. }
                            | EngineTask::PublishProof { .. } => unreachable!(),
                        }
                        .map_err(|error| {
                            RemoteSubmissionConflict::new(format!(
                                "terminal provider checkpoint is not present: {error}"
                            ))
                        })?;
                        runtime
                            .clear_remote_submission(identity, updated_at)
                            .map_err(|error| {
                                RemoteSubmissionConflict::new(format!(
                                    "terminal provider checkpoint identity mismatch: {error}"
                                ))
                            })?;
                        metadata.runtime.last_event = Some("submission_terminalized".to_string());
                        Ok(())
                    })
                    .map_err(|error| {
                        RemoteSubmissionConflict::new(format!(
                            "terminal provider checkpoint is not canonical runtime metadata: {error}"
                        ))
                    })?;
                    record.runner_status = RunnerStatus::Allocated;
                    record.error = None;
                    Ok(true)
                },
            )
            .await;
        match result {
            Ok(updated) if updated > 0 => Ok(()),
            Ok(_) => Err(EngineObserverError::RuntimeInactive(format!(
                "provider checkpoint matched no active runtime root for {task_id}"
            ))),
            Err(error) => Err(self.progress_sync_error(id, runtime_permit, &error)),
        }
    }

    async fn load_proof_artifact(
        &self,
        artifact: &ProofArtifactRef,
    ) -> std::result::Result<Option<raiko2_primitives::Proof>, String> {
        let material = crate::server::proof_artifact::load_proof_artifact_material(
            &self.runtime,
            &artifact.network_pair,
            artifact.pipeline_key,
            artifact.route,
            &artifact.proof_ref,
            ProofArtifactPayload::AggregateInput,
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
        permit: Option<&ProofCompletionPermit>,
        execution_permit: &TaskExecutionPermit,
    ) -> std::result::Result<(), EngineObserverError> {
        let execution_owners = &Self::execution_owners(execution_permit)?.incarnations;
        let stage = Self::stage_name(task);
        let finished_at_ms = now_ms();
        let task_id = Self::timing_key_for_task(id, task);
        let result = match success {
            EngineTaskSuccess::Proof { proof, .. } => {
                let permit = permit.ok_or_else(|| {
                    EngineObserverError::RuntimeInactive(
                        "proof completion requires its pre-issued permit".to_string(),
                    )
                })?;
                let completion_owners = permit
                    .guard::<RuntimeProofCompletionPermit>()
                    .map(|permit| permit.owners.clone())
                    .ok_or_else(|| {
                        EngineObserverError::RuntimeInactive(
                            "proof completion permit does not belong to the runtime".to_string(),
                        )
                    })?;
                return self
                    .handle_proof_success(
                        id,
                        task,
                        stage,
                        finished_at_ms,
                        proof,
                        &completion_owners,
                    )
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
                self.update_execution_root_records(
                    id,
                    execution_owners,
                    |record, updated_at, observed_at_ms| {
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
                            Ok(())
                        })?;
                        record.updated_at = updated_at;
                        Ok(())
                    },
                )
                .await
            }
        };

        result.map_err(|err| {
            tracing::warn!(
                task = ?id,
                stage,
                error = %err,
                "failed to sync runtime task success"
            );
            EngineObserverError::RuntimeSync(format!("failed to sync runtime task success: {err}"))
        })
    }

    async fn on_task_failed(
        &self,
        id: &EngineTaskId,
        task: &EngineTask,
        error: &str,
        permit: TerminalFailurePermit,
    ) -> std::result::Result<TerminalFailureProjection, EngineObserverError> {
        let runtime_permit = permit
            .guard::<RuntimeTerminalFailurePermit>()
            .ok_or_else(|| {
                EngineObserverError::RuntimeInactive(
                    "terminal failure permit does not belong to the runtime".to_string(),
                )
            })?;
        let owner_incarnations = runtime_permit.active_owner_incarnations.clone();
        let root_owners = runtime_permit.projected_owners.clone();
        let stage = Self::stage_name(task);
        let finished_at_ms = now_ms();
        let task_id = Self::timing_key_for_task(id, task);
        if !owner_incarnations.is_empty() {
            self.update_execution_root_records(
                id,
                &owner_incarnations,
                |record, updated_at, observed_at_ms| {
                    record.runner_status = RunnerStatus::Failed;
                    record.error = Some(error.to_string());
                    update_task_metadata(record, |metadata| {
                        metadata.mark_stage_terminal(&task_id, stage, observed_at_ms, "failed");
                        metadata.runtime.active_stage = Some(stage.to_string());
                        metadata.runtime.last_event = Some("failed".to_string());
                        Ok(())
                    })?;
                    record.updated_at = updated_at;
                    Ok(())
                },
            )
            .await
            .map_err(|err| {
                tracing::warn!(task = ?id, error = %err, "failed to sync runtime task failure");
                EngineObserverError::RuntimeSync(format!(
                    "failed to sync runtime task failure: {err}"
                ))
            })?;
            self.observe_stage_terminal_metrics(
                id,
                &task_id,
                stage,
                "failed",
                finished_at_ms,
                Some(error),
            )
            .await;
        }
        Ok(permit.into_projection(root_owners))
    }

    async fn load_pending_proof_checkpoint(
        &self,
        id: &EngineTaskId,
        task: &EngineTask,
        backend: NetworkProverBackend,
    ) -> std::result::Result<Option<PendingProofCheckpoint>, ProgressPersistenceError> {
        let permanent = |message: String| ProgressPersistenceError::Permanent(message);
        let source = task.publication_source();
        match source {
            EngineTask::ProveProposal { .. } | EngineTask::Aggregate { .. } => {}
            EngineTask::Preflight { .. }
            | EngineTask::Validate { .. }
            | EngineTask::Encode { .. }
            | EngineTask::Proposal { .. } => return Ok(None),
            EngineTask::PublishProof { .. } => {
                unreachable!()
            }
        }

        let task_id = Self::root_task_ref(id);
        let mut has_active_owner = false;
        let mut matched_owner = false;
        let mut checkpoints = BTreeMap::<u32, (i64, PendingProofCheckpoint)>::new();
        for record in self.find_root_records(id).await {
            if !self.record_matches_observer(id, &record).map_err(|error| {
                permanent(format!(
                    "failed to validate runtime checkpoint owner: {error}"
                ))
            })? {
                continue;
            }
            matched_owner = true;
            has_active_owner |= !is_terminal_status(record.runner_status);
            let metadata = TaskMetadata::decode_for_record(&record).map_err(|error| {
                permanent(format!("runtime checkpoint metadata is invalid: {error}"))
            })?;
            let runtime = match source {
                EngineTask::ProveProposal { .. } => metadata.proposal_runtime(&task_id),
                EngineTask::Aggregate { .. } => metadata.aggregate_runtime(),
                EngineTask::Preflight { .. }
                | EngineTask::Validate { .. }
                | EngineTask::Encode { .. }
                | EngineTask::Proposal { .. }
                | EngineTask::PublishProof { .. } => unreachable!(),
            };
            let Some(runtime) = runtime.filter(|runtime| runtime.has_remote_submission_progress())
            else {
                continue;
            };
            let checkpoint = build_pending_proof_checkpoint(runtime, backend)?;
            let attempt = checkpoint.attempt.get();
            match checkpoints.entry(attempt) {
                std::collections::btree_map::Entry::Occupied(mut selected) => {
                    let (selected_updated_at, current) = selected.get();
                    if prefer_checkpoint_candidate(
                        current,
                        *selected_updated_at,
                        &checkpoint,
                        runtime.updated_at,
                    )
                    .map_err(|reason| {
                        permanent(format!(
                            "conflicting provider checkpoints share attempt {} for task {task_id}: {reason}",
                            checkpoint.attempt
                        ))
                    })? {
                        selected.insert((runtime.updated_at, checkpoint));
                    }
                }
                std::collections::btree_map::Entry::Vacant(selected) => {
                    selected.insert((runtime.updated_at, checkpoint));
                }
            }
        }
        if !matched_owner {
            return Err(permanent(
                "runtime task record is missing while loading checkpoint".into(),
            ));
        }
        if !has_active_owner {
            return Err(permanent(format!(
                "runtime task {task_id} has no active owner while loading checkpoint"
            )));
        }
        Ok(checkpoints
            .into_iter()
            .next_back()
            .map(|(_, (_, checkpoint))| checkpoint))
    }
}

fn checkpoint_error(message: impl Into<String>) -> ProgressPersistenceError {
    ProgressPersistenceError::Permanent(message.into())
}

fn prefer_checkpoint_candidate(
    selected: &PendingProofCheckpoint,
    selected_updated_at: i64,
    candidate: &PendingProofCheckpoint,
    candidate_updated_at: i64,
) -> std::result::Result<bool, String> {
    if selected.backend != candidate.backend
        || selected.attempt != candidate.attempt
        || selected.submitted_at_secs != candidate.submitted_at_secs
        || selected.deadline_secs != candidate.deadline_secs
    {
        return Err("checkpoint identity differs".to_string());
    }

    match selected.backend {
        NetworkProverBackend::Sp1 => {
            let selected_resume = selected
                .decode_payload_for::<Sp1NetworkSubmissionProgress>(NetworkProverBackend::Sp1)
                .map_err(|error| error.to_string())?;
            let candidate_resume = candidate
                .decode_payload_for::<Sp1NetworkSubmissionProgress>(NetworkProverBackend::Sp1)
                .map_err(|error| error.to_string())?;
            if selected_resume != candidate_resume {
                return Err("SP1 submission identity differs".to_string());
            }
            Ok(candidate_updated_at > selected_updated_at)
        }
        NetworkProverBackend::Boundless => {
            let mut selected_resume = selected
                .decode_payload_for::<BoundlessSubmissionResume>(NetworkProverBackend::Boundless)
                .map_err(|error| error.to_string())?;
            let mut candidate_resume = candidate
                .decode_payload_for::<BoundlessSubmissionResume>(NetworkProverBackend::Boundless)
                .map_err(|error| error.to_string())?;
            let selected_tx_hash = selected_resume.remote_tx_hash.take();
            let candidate_tx_hash = candidate_resume.remote_tx_hash.take();
            if selected_resume != candidate_resume {
                return Err("Boundless submission identity differs".to_string());
            }
            match (selected_tx_hash, candidate_tx_hash) {
                (Some(selected), Some(candidate)) if selected != candidate => {
                    Err("Boundless transaction hash differs".to_string())
                }
                (None, Some(_)) => Ok(true),
                (Some(_), None) => Ok(false),
                _ => Ok(candidate_updated_at > selected_updated_at),
            }
        }
    }
}

fn build_pending_proof_checkpoint(
    runtime: &TaskRuntimeMetadata,
    backend: NetworkProverBackend,
) -> std::result::Result<PendingProofCheckpoint, ProgressPersistenceError> {
    let required = |value: Option<u64>, field: &str| {
        value.ok_or_else(|| checkpoint_error(format!("checkpoint is missing {field}")))
    };
    match backend {
        NetworkProverBackend::Sp1 => {
            let attempt = std::num::NonZeroU32::new(
                runtime
                    .rebid_attempt
                    .ok_or_else(|| checkpoint_error("SP1 checkpoint is missing rebid_attempt"))?,
            )
            .ok_or_else(|| checkpoint_error("SP1 checkpoint rebid_attempt must be non-zero"))?;
            let resume = Sp1NetworkSubmissionProgress {
                provider_request_id: runtime.provider_request_id.clone().ok_or_else(|| {
                    checkpoint_error("SP1 checkpoint is missing provider_request_id")
                })?,
                submitted_at: required(runtime.submitted_at, "submitted_at")?,
                network_mode: runtime
                    .sp1_network_mode
                    .ok_or_else(|| checkpoint_error("SP1 checkpoint is missing network_mode"))?,
                fulfillment_strategy: runtime.sp1_fulfillment_strategy.ok_or_else(|| {
                    checkpoint_error("SP1 checkpoint is missing fulfillment_strategy")
                })?,
                skip_simulation: runtime
                    .sp1_skip_simulation
                    .ok_or_else(|| checkpoint_error("SP1 checkpoint is missing skip_simulation"))?,
                cycle_limit: runtime
                    .sp1_cycle_limit
                    .ok_or_else(|| checkpoint_error("SP1 checkpoint is missing cycle_limit"))?,
                timeout_secs: runtime
                    .sp1_timeout_secs
                    .ok_or_else(|| checkpoint_error("SP1 checkpoint is missing timeout_secs"))?,
                attempt: attempt.get(),
                max_price_per_pgu: runtime.sp1_max_price_per_pgu,
                auction_timeout_secs: runtime.sp1_auction_timeout_secs,
            };
            PendingProofCheckpoint::from_payload(
                backend,
                attempt,
                resume.submitted_at,
                required(runtime.expires_at, "expires_at")?,
                &resume,
            )
            .map_err(|error| checkpoint_error(error.to_string()))
        }
        NetworkProverBackend::Boundless => {
            let resume = BoundlessSubmissionResume {
                provider_request_id: runtime.provider_request_id.clone().ok_or_else(|| {
                    checkpoint_error("Boundless checkpoint is missing provider_request_id")
                })?,
                remote_tx_hash: runtime.remote_tx_hash.clone(),
                request_id_has_confirmed_submission: runtime.request_id_has_confirmed_submission,
                request_digest: runtime.request_digest.clone(),
                broadcast_from_block: runtime.broadcast_from_block,
                image_ref: runtime
                    .image_ref
                    .clone()
                    .ok_or_else(|| checkpoint_error("Boundless checkpoint is missing image_ref"))?,
                deployment: runtime.deployment.clone().ok_or_else(|| {
                    checkpoint_error("Boundless checkpoint is missing deployment")
                })?,
                offchain: runtime
                    .offchain
                    .ok_or_else(|| checkpoint_error("Boundless checkpoint is missing offchain"))?,
                expires_at: required(runtime.expires_at, "expires_at")?,
                lock_expires_at: required(runtime.lock_expires_at, "lock_expires_at")?,
                submitted_at: required(runtime.submitted_at, "submitted_at")?,
                max_price_multiplier: runtime.max_price_multiplier.ok_or_else(|| {
                    checkpoint_error("Boundless checkpoint is missing max_price_multiplier")
                })?,
                max_price_wei: Some(runtime.max_price_wei.clone().ok_or_else(|| {
                    checkpoint_error("Boundless checkpoint is missing max_price_wei")
                })?),
                rebid_attempt: runtime.rebid_attempt.ok_or_else(|| {
                    checkpoint_error("Boundless checkpoint is missing rebid_attempt")
                })?,
            };
            let attempt = std::num::NonZeroU32::new(resume.rebid_attempt).ok_or_else(|| {
                checkpoint_error("Boundless checkpoint rebid_attempt must be non-zero")
            })?;
            PendingProofCheckpoint::from_payload(
                backend,
                attempt,
                resume.submitted_at,
                resume.expires_at,
                &resume,
            )
            .map_err(|error| checkpoint_error(error.to_string()))
        }
    }
}

fn update_task_metadata<T, F>(record: &mut RuntimeTaskRecord, mutator: F) -> Result<T>
where
    F: FnOnce(&mut TaskMetadata) -> Result<T>,
{
    let mut metadata = TaskMetadata::decode_for_record(record)?;
    let output = mutator(&mut metadata)?;
    metadata.validate_runtime_progress(record.route)?;
    record.metadata =
        serde_json::to_value(metadata).context("failed to serialize task metadata")?;
    Ok(output)
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

#[cfg(test)]
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
        AggregateInputProofArtifact, ProposalTask, RuntimeMetadata, aggregate_task_ref,
        proposal_task_ref, stage_task_ref,
    };
    use crate::server::telemetry;
    use async_trait::async_trait;
    use raiko2_engine::{AggregationTaskRequest, ProposalTaskRequest};
    use raiko2_pipeline::PipelineKey;
    use raiko2_primitives::ProofType;
    use raiko2_prover::{
        BoundlessSubmissionProgress, Sp1FulfillmentStrategy, Sp1NetworkMode,
        Sp1NetworkSubmissionProgress, sp1_config::ExecutionMode,
    };
    use raiko2_runtime::test_support::{
        MemoryProofArtifactStore, ProofObjectStore, RuntimeStateObject, RuntimeStateStore,
        RuntimeStateWriteResult, RuntimeStoreScope,
    };
    use raiko2_runtime::{
        ExactDeleteResult, ProofArtifactDescriptor, ProofArtifactKey, ProofArtifactObject,
        ProofArtifactPutResult, ProofArtifactRegistration, TaskRegistration,
    };
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[test]
    fn proof_task_log_context_uses_public_proposal_shape() {
        let proposal = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaSgx,
            request: proposal_request_with_id(42),
        });
        let proposal_context = proof_task_log_context(&proposal);
        assert!(!proposal_context.aggregate);
        assert_eq!(proposal_context.proposal_ids, vec![42]);

        let aggregate = EngineTaskId::new(EngineTaskKey::Aggregate {
            pipeline: PipelineKey::ShastaSgx,
            request: AggregationTaskRequest {
                request_id: "sample-request".to_string(),
                proposal_ids: vec![42, 43, 44],
                prover_config: ProverTaskConfig::default(),
            },
        });
        let aggregate_context = proof_task_log_context(&aggregate);
        assert!(aggregate_context.aggregate);
        assert_eq!(aggregate_context.proposal_ids, vec![42, 43, 44]);
    }

    async fn engine_execution_permit(
        observer: &RuntimeObserver,
        id: &EngineTaskId,
        task: &EngineTask,
    ) -> std::result::Result<TaskExecutionPermit, EngineObserverError> {
        EngineObserver::acquire_task_execution_permit(observer, id, task).await
    }

    async fn engine_checkpoint_proof(
        observer: &RuntimeObserver,
        id: &EngineTaskId,
        task: &EngineTask,
        proof: &raiko2_primitives::Proof,
    ) -> std::result::Result<ProofCompletionPermit, EngineObserverError> {
        let execution_permit = engine_execution_permit(observer, id, task).await?;
        EngineObserver::checkpoint_completed_proof(observer, id, task, proof, &execution_permit)
            .await
    }

    async fn drive_engine_success(
        observer: &RuntimeObserver,
        id: &EngineTaskId,
        task: &EngineTask,
        success: &EngineTaskSuccess,
    ) -> std::result::Result<(), EngineObserverError> {
        let execution_permit = engine_execution_permit(observer, id, task).await?;
        let completion_permit = match success {
            EngineTaskSuccess::Proof { proof, .. } => Some(
                EngineObserver::checkpoint_completed_proof(
                    observer,
                    id,
                    task,
                    proof,
                    &execution_permit,
                )
                .await?,
            ),
            EngineTaskSuccess::GuestInput { .. } | EngineTaskSuccess::EncodedInput { .. } => None,
        };
        EngineObserver::on_task_succeeded(
            observer,
            id,
            task,
            success,
            completion_permit.as_ref(),
            &execution_permit,
        )
        .await
    }

    async fn drive_engine_start(
        observer: &RuntimeObserver,
        id: &EngineTaskId,
        task: &EngineTask,
        worker: &str,
    ) -> std::result::Result<(), EngineObserverError> {
        let execution_permit = engine_execution_permit(observer, id, task).await?;
        EngineObserver::on_task_started(observer, id, task, worker, &execution_permit).await;
        Ok(())
    }

    async fn drive_engine_progress(
        observer: &RuntimeObserver,
        id: &EngineTaskId,
        task: &EngineTask,
        progress: &ProverProgress,
        checkpoint_permit: &SubmissionCheckpointPermit,
    ) -> std::result::Result<(), EngineObserverError> {
        let execution_permit = engine_execution_permit(observer, id, task).await?;
        EngineObserver::on_task_progress(
            observer,
            id,
            task,
            progress,
            checkpoint_permit,
            &execution_permit,
        )
        .await
    }

    async fn drive_engine_checkpoint_clear(
        observer: &RuntimeObserver,
        id: &EngineTaskId,
        task: &EngineTask,
        identity: &PendingProofCheckpointIdentity,
        checkpoint_permit: &SubmissionCheckpointPermit,
    ) -> std::result::Result<(), EngineObserverError> {
        let execution_permit = engine_execution_permit(observer, id, task).await?;
        EngineObserver::clear_pending_proof_checkpoint(
            observer,
            id,
            task,
            identity,
            checkpoint_permit,
            &execution_permit,
        )
        .await
    }

    async fn drive_engine_failure(
        observer: &RuntimeObserver,
        id: &EngineTaskId,
        task: &EngineTask,
        error: &str,
    ) -> std::result::Result<TerminalFailureProjection, EngineObserverError> {
        let execution_permit = engine_execution_permit(observer, id, task).await?;
        let terminal_permit =
            EngineObserver::acquire_terminal_failure_permit(observer, id, task, &execution_permit)
                .await?;
        EngineObserver::on_task_failed(observer, id, task, error, terminal_permit).await
    }

    async fn load_checkpoint_payload<T: serde::de::DeserializeOwned>(
        observer: &RuntimeObserver,
        id: &EngineTaskId,
        task: &EngineTask,
        backend: NetworkProverBackend,
    ) -> Result<Option<T>> {
        EngineObserver::load_pending_proof_checkpoint(observer, id, task, backend)
            .await?
            .map(|checkpoint| checkpoint.decode_payload_for(backend).map_err(Into::into))
            .transpose()
    }

    #[test]
    fn newer_sp1_checkpoint_cannot_replace_a_different_same_attempt_submission() {
        let mut progress = Sp1NetworkSubmissionProgress {
            provider_request_id: "request-a".to_string(),
            submitted_at: 100,
            network_mode: Sp1NetworkMode::Reserved,
            fulfillment_strategy: Sp1FulfillmentStrategy::Reserved,
            skip_simulation: true,
            cycle_limit: 1_000_000,
            timeout_secs: 3_600,
            attempt: 1,
            max_price_per_pgu: None,
            auction_timeout_secs: None,
        };
        let selected = PendingProofCheckpoint::from_payload(
            NetworkProverBackend::Sp1,
            std::num::NonZeroU32::MIN,
            100,
            3_700,
            &progress,
        )
        .expect("valid selected checkpoint");
        progress.provider_request_id = "request-b".to_string();
        let candidate = PendingProofCheckpoint::from_payload(
            NetworkProverBackend::Sp1,
            std::num::NonZeroU32::MIN,
            100,
            3_700,
            &progress,
        )
        .expect("valid candidate checkpoint");

        let error = prefer_checkpoint_candidate(&selected, 10, &candidate, 20)
            .expect_err("a newer timestamp must not hide a conflicting provider request");

        assert_eq!(error, "SP1 submission identity differs");
    }

    #[test]
    fn boundless_transaction_hash_is_a_monotonic_same_attempt_refinement() {
        let mut resume = BoundlessSubmissionResume {
            provider_request_id: "request-a".to_string(),
            remote_tx_hash: None,
            request_id_has_confirmed_submission: false,
            request_digest: Some(format!("{}", alloy_primitives::B256::repeat_byte(0x11))),
            broadcast_from_block: Some(90),
            image_ref: "image-a".to_string(),
            deployment: "base".to_string(),
            offchain: false,
            expires_at: 200,
            lock_expires_at: 180,
            submitted_at: 100,
            max_price_multiplier: 1,
            max_price_wei: Some("1000".to_string()),
            rebid_attempt: 1,
        };
        let pending = PendingProofCheckpoint::from_payload(
            NetworkProverBackend::Boundless,
            std::num::NonZeroU32::MIN,
            100,
            200,
            &resume,
        )
        .expect("valid pending checkpoint");
        resume.remote_tx_hash = Some("0xtx".to_string());
        let dispatched = PendingProofCheckpoint::from_payload(
            NetworkProverBackend::Boundless,
            std::num::NonZeroU32::MIN,
            100,
            200,
            &resume,
        )
        .expect("valid dispatched checkpoint");

        assert!(
            prefer_checkpoint_candidate(&pending, 20, &dispatched, 10)
                .expect("adding the transaction hash is compatible")
        );
        assert!(
            !prefer_checkpoint_candidate(&dispatched, 10, &pending, 20)
                .expect("removing the transaction hash is compatible but stale")
        );

        resume.remote_tx_hash = Some("0xother".to_string());
        let conflicting = PendingProofCheckpoint::from_payload(
            NetworkProverBackend::Boundless,
            std::num::NonZeroU32::MIN,
            100,
            200,
            &resume,
        )
        .expect("valid conflicting checkpoint");
        let error = prefer_checkpoint_candidate(&dispatched, 10, &conflicting, 20)
            .expect_err("two transaction hashes cannot identify the same submission attempt");
        assert_eq!(error, "Boundless transaction hash differs");
    }

    include!("runtime_observer_state_tests.rs");
    include!("runtime_observer_race_tests.rs");

    #[derive(Debug, Default)]
    struct FailingArtifactStore {
        attempts: AtomicUsize,
    }

    #[derive(Debug)]
    struct BlocksCanonicalPutStore {
        inner: MemoryProofArtifactStore,
        block_next_put: AtomicBool,
        put_entered: tokio::sync::Notify,
        allow_put: tokio::sync::Notify,
    }

    impl RuntimeStoreScope for BlocksCanonicalPutStore {
        fn environment(&self) -> &str {
            self.inner.environment()
        }

        fn namespace(&self) -> &str {
            self.inner.namespace()
        }

        fn backend_name(&self) -> &'static str {
            "test"
        }
    }

    #[async_trait]
    impl ProofObjectStore for BlocksCanonicalPutStore {
        async fn put_if_absent(
            &self,
            key: &ProofArtifactKey,
            bytes: &[u8],
        ) -> Result<ProofArtifactPutResult> {
            if !key.proof_ref.starts_with("__pending__:")
                && self.block_next_put.swap(false, Ordering::SeqCst)
            {
                self.put_entered.notify_one();
                self.allow_put.notified().await;
            }
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

        async fn delete_exact(
            &self,
            key: &ProofArtifactKey,
            descriptor: &ProofArtifactDescriptor,
        ) -> Result<ExactDeleteResult> {
            self.inner.delete_exact(key, descriptor).await
        }
    }

    #[async_trait]
    impl RuntimeStateStore for BlocksCanonicalPutStore {
        async fn load_runtime_state(&self) -> Result<Option<RuntimeStateObject>> {
            self.inner.load_runtime_state().await
        }

        async fn store_runtime_state(
            &self,
            bytes: &[u8],
            expected_generation: Option<i64>,
        ) -> Result<RuntimeStateWriteResult> {
            self.inner
                .store_runtime_state(bytes, expected_generation)
                .await
        }
    }

    impl RuntimeStoreScope for FailingArtifactStore {
        fn environment(&self) -> &'static str {
            "test"
        }

        fn namespace(&self) -> &'static str {
            "failure"
        }

        fn backend_name(&self) -> &'static str {
            "test"
        }
    }

    #[async_trait]
    impl ProofObjectStore for FailingArtifactStore {
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

        async fn get_descriptor(
            &self,
            _key: &ProofArtifactKey,
        ) -> Result<Option<ProofArtifactDescriptor>> {
            Ok(None)
        }

        async fn delete_exact(
            &self,
            _key: &ProofArtifactKey,
            _descriptor: &ProofArtifactDescriptor,
        ) -> Result<ExactDeleteResult> {
            Ok(ExactDeleteResult::Missing)
        }
    }

    #[async_trait]
    impl RuntimeStateStore for FailingArtifactStore {
        async fn load_runtime_state(&self) -> Result<Option<RuntimeStateObject>> {
            Ok(None)
        }

        async fn store_runtime_state(
            &self,
            _bytes: &[u8],
            expected_generation: Option<i64>,
        ) -> Result<RuntimeStateWriteResult> {
            Ok(RuntimeStateWriteResult::Stored {
                generation: Some(expected_generation.unwrap_or(0).saturating_add(1)),
            })
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

    fn checkpoint_permit(runtime: &RuntimeManager) -> SubmissionCheckpointPermit {
        SubmissionCheckpointPermit::tracked(
            runtime
                .acquire_submission_checkpoint_permit()
                .expect("checkpoint permit"),
        )
    }

    fn proposal_request() -> ProposalTaskRequest {
        proposal_request_with_id(42)
    }

    fn proposal_request_with_id(proposal_id: u64) -> ProposalTaskRequest {
        ProposalTaskRequest {
            proposal_id,
            l2_block_range: Some(raiko2_primitives::L2BlockRange {
                start: proposal_id,
                end: proposal_id,
            }),
            l1_inclusion_block_number: 1,
            last_anchor_block_number: 0,
            checkpoint: None,
            blob_proof_type: None,
            prover: None,
            graffiti: None,
            prover_config: ProverTaskConfig::default(),
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
            request: request.clone(),
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

    async fn install_cleanup_pending_artifact(
        runtime: &RuntimeManager,
        network_pair: &str,
        pipeline: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<()> {
        let publication = runtime
            .publish_proof_artifact_bytes(
                network_pair,
                pipeline,
                route,
                proof_ref,
                br#"{"proof":"0xstale"}"#,
            )
            .await?;
        let object = publication
            .try_object()
            .context("published cleanup-pending proof")?
            .clone();
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: network_pair.to_string(),
                proof_ref: proof_ref.to_string(),
                pipeline_key: pipeline,
                route,
                proof_uri: object.proof_uri,
                content_hash: object.content_hash,
                generation: object.generation,
            })
            .await?;
        let record = runtime
            .get_proof_artifact(network_pair, pipeline, route, proof_ref)
            .await?
            .context("active cleanup-pending proof")?;
        let prepared = runtime.prepare_artifact_retention_batch(&[record]).await?;
        assert_eq!(prepared.newly_invalidated_artifacts, 1);
        Ok(())
    }

    fn compressed_sp1_proof_fixture() -> raiko2_primitives::Proof {
        raiko2_primitives::Proof {
            proof: None,
            input: Some(alloy_primitives::B256::ZERO),
            quote: Some(r#"{"Compressed":{}}"#.to_string()),
            uuid: Some("sp1-verifying-key".to_string()),
            kzg_proof: None,
            extra_data: Some(serde_json::json!({ "shasta": {} })),
        }
    }

    fn boundless_progress_fixture(provider_request_id: &str, rebid_attempt: u32) -> ProverProgress {
        let expires_at = now_secs().saturating_add(3_600);
        ProverProgress::BoundlessSubmission(BoundlessSubmissionProgress {
            provider_request_id: provider_request_id.to_string(),
            remote_tx_hash: Some("0xabcd".to_string()),
            request_id_has_confirmed_submission: false,
            request_digest: None,
            broadcast_from_block: None,
            expires_at,
            lock_expires_at: expires_at - 600,
            submitted_at: expires_at - 900,
            image_ref: "0ximage".to_string(),
            deployment: "base".to_string(),
            offchain: false,
            quoted_mcycles_count: Some(6_000),
            evaluated_mcycles_count: Some(12_345),
            max_price_multiplier: 4,
            max_price_wei: Some("9000000000000".to_string()),
            rebid_attempt,
        })
    }

    async fn checkpoint_proof_fixture(
        observer: &RuntimeObserver,
        id: &EngineTaskId,
        task: &EngineTask,
    ) -> Result<()> {
        engine_checkpoint_proof(observer, id, task, &proof_fixture())
            .await
            .map(|_| ())
            .map_err(|error| anyhow::anyhow!(error.to_string()))
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
        let route = if pipeline == PipelineKey::ShastaSp1 {
            "sp1/network"
                .parse::<PipelineRoute>()
                .expect("parse SP1 network route")
        } else {
            pipeline.route()
        };
        let (network, l1_network) = network_pair
            .split_once('/')
            .unwrap_or((network_pair, "ethereum"));
        runtime
            .register_task(TaskRegistration {
                task_id: task_id.to_string(),
                pipeline_key: pipeline,
                route,
                task_kind: "hoodi_batch".to_string(),
                network_pair: network_pair.to_string(),
                artifact_refs: vec![task_ref.clone()],
                metadata: serde_json::to_value(TaskMetadata {
                    network_pair: network_pair.to_string(),
                    network: network.to_string(),
                    l1_network: l1_network.to_string(),
                    proof_type: pipeline.proof_type(),
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
                request_fingerprint: format!("request-{task_id}"),
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
                pipeline_key: PipelineKey::ShastaRisc0Network,
                route: "risc0/network"
                    .parse::<PipelineRoute>()
                    .expect("parse route"),
                task_kind: "hoodi_batch".to_string(),
                network_pair: "taiko_dev/ethereum".into(),
                artifact_refs: vec![task_ref.clone()],
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
                        request: proposal_request(),
                    }],
                    aggregate_task_id: None,
                    aggregate_request: None,
                    aggregate_input_artifacts: Vec::new(),
                    runtime: RuntimeMetadata::default(),
                })?,
                request_fingerprint: "task-public".into(),
            })
            .await?;

        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            PipelineKey::ShastaRisc0Network.route(),
        );
        let future_expires_at = now_secs().saturating_add(3_600);
        drive_engine_progress(
            &observer,
            &proposal_task_id,
            &EngineTask::ProveProposal {
                request: proposal_request(),
                input_task: proposal_task_id.clone(),
            },
            &ProverProgress::BoundlessSubmission(BoundlessSubmissionProgress {
                provider_request_id: "0x1234".to_string(),
                remote_tx_hash: Some("0xabcd".to_string()),
                request_id_has_confirmed_submission: false,
                request_digest: Some(format!("{}", alloy_primitives::B256::repeat_byte(0x11))),
                broadcast_from_block: Some(90),
                expires_at: future_expires_at,
                lock_expires_at: future_expires_at - 600,
                submitted_at: future_expires_at - 900,
                image_ref: "0ximage".to_string(),
                deployment: "base".to_string(),
                offchain: false,
                quoted_mcycles_count: Some(6_000),
                evaluated_mcycles_count: Some(12_345),
                max_price_multiplier: 4,
                max_price_wei: Some("9000000000000".to_string()),
                rebid_attempt: 3,
            }),
            &checkpoint_permit(&runtime),
        )
        .await
        .expect("persist Boundless submission progress");

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
        assert_eq!(
            runtime_entry.request_digest.as_deref(),
            Some(format!("{}", alloy_primitives::B256::repeat_byte(0x11)).as_str())
        );
        assert_eq!(runtime_entry.broadcast_from_block, Some(90));
        assert_eq!(runtime_entry.expires_at, Some(future_expires_at));
        assert_eq!(runtime_entry.submitted_at, Some(future_expires_at - 900));
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
        record.runner_status = RunnerStatus::Allocated;
        runtime.upsert_task(&record).await?;
        let resumed = load_checkpoint_payload::<BoundlessSubmissionResume>(
            &observer,
            &proposal_task_id,
            &EngineTask::ProveProposal {
                request: proposal_request(),
                input_task: proposal_task_id.clone(),
            },
            NetworkProverBackend::Boundless,
        )
        .await
        .expect("load Boundless checkpoint")
        .expect("boundless submission can resume");
        assert_eq!(resumed.provider_request_id, "0x1234");
        assert_eq!(resumed.remote_tx_hash.as_deref(), Some("0xabcd"));
        assert_eq!(
            resumed.request_digest.as_deref(),
            Some(format!("{}", alloy_primitives::B256::repeat_byte(0x11)).as_str())
        );
        assert_eq!(resumed.broadcast_from_block, Some(90));
        assert_eq!(resumed.image_ref, "0ximage");
        assert_eq!(resumed.deployment, "base");
        assert!(!resumed.offchain);
        assert_eq!(resumed.expires_at, future_expires_at);
        assert_eq!(resumed.lock_expires_at, future_expires_at - 600);
        assert_eq!(resumed.submitted_at, future_expires_at - 900);
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
        runtime_entry.image_ref = None;
        runtime_entry.deployment = None;
        runtime_entry.max_price_multiplier = None;
        runtime_entry.rebid_attempt = None;
        runtime_entry.lock_expires_at = None;
        record.metadata = serde_json::to_value(metadata)?;
        runtime.upsert_task(&record).await?;
        let checkpoint_error = observer
            .load_pending_proof_checkpoint(
                &proposal_task_id,
                &EngineTask::ProveProposal {
                    request: proposal_request(),
                    input_task: proposal_task_id.clone(),
                },
                NetworkProverBackend::Boundless,
            )
            .await
            .expect_err("incomplete checkpoint must fail closed");
        assert!(matches!(
            checkpoint_error,
            ProgressPersistenceError::Permanent(_)
        ));

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
            runtime_entry.image_ref = Some("0ximage".to_string());
            runtime_entry.deployment = Some("base".to_string());
            runtime_entry.expires_at = Some(expired_at);
            runtime_entry.lock_expires_at = Some(expired_at.saturating_sub(300));
            runtime_entry.submitted_at = Some(expired_at.saturating_sub(600));
            runtime_entry.max_price_multiplier = Some(4);
            runtime_entry.rebid_attempt = Some(5);
        }
        record.metadata = serde_json::to_value(metadata)?;
        runtime.upsert_task(&record).await?;
        // Expired records still resume: the prover gives them one final status read (recovering
        // an already-paid fulfillment) and counts the stored attempt against the rebid budget.
        let resumed = load_checkpoint_payload::<BoundlessSubmissionResume>(
            &observer,
            &proposal_task_id,
            &EngineTask::ProveProposal {
                request: proposal_request(),
                input_task: proposal_task_id.clone(),
            },
            NetworkProverBackend::Boundless,
        )
        .await
        .expect("load Boundless checkpoint")
        .expect("expired boundless submission still resumes");
        assert_eq!(resumed.provider_request_id, "0x1234");
        assert_eq!(resumed.expires_at, expired_at);
        assert_eq!(resumed.rebid_attempt, 5);
        Ok(())
    }

    #[tokio::test]
    async fn runtime_observer_clears_only_matching_boundless_proposal_checkpoint() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-clear-boundless",
        ))?);
        let request = proposal_request();
        register_observer_task(
            &runtime,
            "task_public",
            "taiko_dev/ethereum",
            PipelineKey::ShastaRisc0Network,
            &request,
            RunnerStatus::Allocated,
        )
        .await?;
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            PipelineKey::ShastaRisc0Network.route(),
        );
        let id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaRisc0Network,
            request: request.clone(),
        });
        let task = EngineTask::ProveProposal {
            request,
            input_task: id.clone(),
        };
        let expires_at = now_secs().saturating_add(3_600);
        let checkpoint_permit = checkpoint_permit(&runtime);
        drive_engine_progress(
            &observer,
            &id,
            &task,
            &ProverProgress::BoundlessSubmission(BoundlessSubmissionProgress {
                provider_request_id: "0x1234".to_string(),
                remote_tx_hash: Some("0xabcd".to_string()),
                request_id_has_confirmed_submission: false,
                request_digest: None,
                broadcast_from_block: None,
                expires_at,
                lock_expires_at: expires_at - 600,
                submitted_at: expires_at - 900,
                image_ref: "0ximage".to_string(),
                deployment: "base".to_string(),
                offchain: false,
                quoted_mcycles_count: Some(6_000),
                evaluated_mcycles_count: Some(12_345),
                max_price_multiplier: 4,
                max_price_wei: Some("9000000000000".to_string()),
                rebid_attempt: 3,
            }),
            &checkpoint_permit,
        )
        .await?;

        let wrong_identity = PendingProofCheckpointIdentity {
            backend: NetworkProverBackend::Boundless,
            provider_request_id: "0xother".to_string(),
            attempt: std::num::NonZeroU32::new(3).expect("non-zero attempt"),
        };
        drive_engine_checkpoint_clear(&observer, &id, &task, &wrong_identity, &checkpoint_permit)
            .await
            .expect_err("mismatched checkpoint must not clear");
        assert!(
            load_checkpoint_payload::<BoundlessSubmissionResume>(
                &observer,
                &id,
                &task,
                NetworkProverBackend::Boundless,
            )
            .await?
            .is_some()
        );

        let identity = PendingProofCheckpointIdentity {
            backend: NetworkProverBackend::Boundless,
            provider_request_id: "0x1234".to_string(),
            attempt: std::num::NonZeroU32::new(3).expect("non-zero attempt"),
        };
        drive_engine_checkpoint_clear(&observer, &id, &task, &identity, &checkpoint_permit).await?;
        assert!(
            load_checkpoint_payload::<BoundlessSubmissionResume>(
                &observer,
                &id,
                &task,
                NetworkProverBackend::Boundless,
            )
            .await?
            .is_none()
        );

        drive_engine_failure(&observer, &id, &task, "no-lock rebids exhausted").await?;
        let mut record = runtime
            .get_task("task_public")
            .await?
            .expect("runtime task after terminal failure");
        assert_eq!(record.runner_status, RunnerStatus::Failed);
        let metadata = TaskMetadata::decode_for_record(&record)?;
        assert!(
            metadata
                .proposal_runtime(&RuntimeObserver::root_task_ref(&id))
                .is_some_and(|runtime| !runtime.has_remote_submission_progress())
        );

        // The HTTP retry path reactivates the same deterministic task. Once active again, the
        // prover must see no resumable provider checkpoint and therefore submit attempt one.
        record.runner_status = RunnerStatus::Allocated;
        record.error = None;
        runtime.upsert_task(&record).await?;
        assert!(
            load_checkpoint_payload::<BoundlessSubmissionResume>(
                &observer,
                &id,
                &task,
                NetworkProverBackend::Boundless,
            )
            .await?
            .is_none()
        );
        Ok(())
    }

    #[tokio::test]
    async fn runtime_observer_rejects_invalid_metadata_during_checkpoint_clear() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-reject-invalid-clear-metadata",
        ))?);
        let request = proposal_request();
        register_observer_task(
            &runtime,
            "task_active",
            "taiko_dev/ethereum",
            PipelineKey::ShastaRisc0Network,
            &request,
            RunnerStatus::Allocated,
        )
        .await?;
        register_observer_task(
            &runtime,
            "task_invalid",
            "taiko_dev/ethereum",
            PipelineKey::ShastaRisc0Network,
            &request,
            RunnerStatus::Allocated,
        )
        .await?;
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            PipelineKey::ShastaRisc0Network.route(),
        );
        let id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaRisc0Network,
            request: request.clone(),
        });
        let task = EngineTask::ProveProposal {
            request,
            input_task: id.clone(),
        };
        let checkpoint_permit = checkpoint_permit(&runtime);
        drive_engine_progress(
            &observer,
            &id,
            &task,
            &boundless_progress_fixture("0x1234", 1),
            &checkpoint_permit,
        )
        .await?;
        let execution_permit = engine_execution_permit(&observer, &id, &task).await?;

        let mut record = runtime
            .get_task("task_invalid")
            .await?
            .expect("runtime task");
        record.metadata = serde_json::json!({"invalid": "metadata"});
        runtime.upsert_task(&record).await?;

        let error = EngineObserver::clear_pending_proof_checkpoint(
            &observer,
            &id,
            &task,
            &PendingProofCheckpointIdentity {
                backend: NetworkProverBackend::Boundless,
                provider_request_id: "0x1234".to_string(),
                attempt: std::num::NonZeroU32::MIN,
            },
            &checkpoint_permit,
            &execution_permit,
        )
        .await
        .expect_err("invalid checkpoint metadata must fail permanently");

        assert!(matches!(error, EngineObserverError::ProgressRejected(_)));
        let active = runtime
            .get_task("task_active")
            .await?
            .expect("active runtime task");
        let active_metadata = TaskMetadata::decode_for_record(&active)?;
        assert!(
            active_metadata
                .proposal_runtime(&RuntimeObserver::root_task_ref(&id))
                .is_some_and(TaskRuntimeMetadata::has_remote_submission_progress)
        );
        assert_eq!(
            runtime
                .get_task("task_invalid")
                .await?
                .expect("invalid runtime task")
                .metadata,
            serde_json::json!({"invalid": "metadata"})
        );
        Ok(())
    }

    #[tokio::test]
    async fn runtime_observer_ignores_malformed_terminal_sibling_during_checkpoint_clear()
    -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-ignore-terminal-clear-metadata",
        ))?);
        let request = proposal_request();
        register_observer_task(
            &runtime,
            "task_active",
            "taiko_dev/ethereum",
            PipelineKey::ShastaRisc0Network,
            &request,
            RunnerStatus::Allocated,
        )
        .await?;
        register_observer_task(
            &runtime,
            "task_terminal",
            "taiko_dev/ethereum",
            PipelineKey::ShastaRisc0Network,
            &request,
            RunnerStatus::Failed,
        )
        .await?;
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            PipelineKey::ShastaRisc0Network.route(),
        );
        let id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaRisc0Network,
            request: request.clone(),
        });
        let task = EngineTask::ProveProposal {
            request,
            input_task: id.clone(),
        };
        let checkpoint_permit = checkpoint_permit(&runtime);
        drive_engine_progress(
            &observer,
            &id,
            &task,
            &boundless_progress_fixture("0x1234", 1),
            &checkpoint_permit,
        )
        .await?;
        let execution_permit = engine_execution_permit(&observer, &id, &task).await?;

        let mut terminal = runtime
            .get_task("task_terminal")
            .await?
            .expect("terminal runtime task");
        terminal.metadata = serde_json::json!({"invalid": "metadata"});
        runtime.upsert_task(&terminal).await?;

        EngineObserver::clear_pending_proof_checkpoint(
            &observer,
            &id,
            &task,
            &PendingProofCheckpointIdentity {
                backend: NetworkProverBackend::Boundless,
                provider_request_id: "0x1234".to_string(),
                attempt: std::num::NonZeroU32::MIN,
            },
            &checkpoint_permit,
            &execution_permit,
        )
        .await
        .expect("terminal sibling must not block active checkpoint clear");

        let active = runtime
            .get_task("task_active")
            .await?
            .expect("active runtime task");
        let active_metadata = TaskMetadata::decode_for_record(&active)?;
        assert!(
            active_metadata
                .proposal_runtime(&RuntimeObserver::root_task_ref(&id))
                .is_some_and(|runtime| !runtime.has_remote_submission_progress())
        );
        assert_eq!(
            runtime
                .get_task("task_terminal")
                .await?
                .expect("terminal runtime task")
                .metadata,
            serde_json::json!({"invalid": "metadata"})
        );
        Ok(())
    }

    #[tokio::test]
    async fn runtime_observer_ignores_malformed_reincarnated_sibling_during_checkpoint_clear()
    -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-ignore-reincarnated-clear-metadata",
        ))?);
        let request = proposal_request();
        for task_id in ["task_active", "task_reincarnated"] {
            register_observer_task(
                &runtime,
                task_id,
                "taiko_dev/ethereum",
                PipelineKey::ShastaRisc0Network,
                &request,
                RunnerStatus::Allocated,
            )
            .await?;
        }
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            PipelineKey::ShastaRisc0Network.route(),
        );
        let id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaRisc0Network,
            request: request.clone(),
        });
        let task = EngineTask::ProveProposal {
            request: request.clone(),
            input_task: id.clone(),
        };
        let checkpoint_permit = checkpoint_permit(&runtime);
        drive_engine_progress(
            &observer,
            &id,
            &task,
            &boundless_progress_fixture("0x1234", 1),
            &checkpoint_permit,
        )
        .await?;
        let execution_permit = engine_execution_permit(&observer, &id, &task).await?;

        let stale = runtime
            .get_task("task_reincarnated")
            .await?
            .expect("original runtime task");
        runtime
            .cancel_task_if_current(&stale.lifetime(), None)
            .await?;
        runtime.remove_task_if_current(&stale.lifetime()).await?;
        register_observer_task(
            &runtime,
            "task_reincarnated",
            "taiko_dev/ethereum",
            PipelineKey::ShastaRisc0Network,
            &request,
            RunnerStatus::Allocated,
        )
        .await?;
        let mut replacement = runtime
            .get_task("task_reincarnated")
            .await?
            .expect("replacement runtime task");
        assert_ne!(replacement.incarnation_id, stale.incarnation_id);
        replacement.metadata = serde_json::json!({"invalid": "metadata"});
        runtime.upsert_task(&replacement).await?;

        EngineObserver::clear_pending_proof_checkpoint(
            &observer,
            &id,
            &task,
            &PendingProofCheckpointIdentity {
                backend: NetworkProverBackend::Boundless,
                provider_request_id: "0x1234".to_string(),
                attempt: std::num::NonZeroU32::MIN,
            },
            &checkpoint_permit,
            &execution_permit,
        )
        .await
        .expect("replacement sibling must not block the valid owner checkpoint clear");

        let active = runtime
            .get_task("task_active")
            .await?
            .expect("active runtime task");
        let active_metadata = TaskMetadata::decode_for_record(&active)?;
        assert!(
            active_metadata
                .proposal_runtime(&RuntimeObserver::root_task_ref(&id))
                .is_some_and(|runtime| !runtime.has_remote_submission_progress())
        );
        assert_eq!(
            runtime
                .get_task("task_reincarnated")
                .await?
                .expect("replacement runtime task")
                .metadata,
            serde_json::json!({"invalid": "metadata"})
        );
        Ok(())
    }

    #[tokio::test]
    async fn runtime_observer_clears_matching_boundless_aggregate_checkpoint() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-clear-boundless-aggregate",
        ))?);
        let pipeline = PipelineKey::ShastaRisc0Network;
        let aggregate_request = AggregationTaskRequest {
            request_id: "agg-42".to_string(),
            proposal_ids: vec![42],
            prover_config: ProverTaskConfig::default(),
        };
        let task_ref = aggregate_task_ref(pipeline, &aggregate_request);
        let metadata = TaskMetadata {
            network_pair: "taiko_dev/ethereum".to_string(),
            network: "taiko_dev".to_string(),
            l1_network: "ethereum".to_string(),
            proof_type: ProofType::Risc0,
            requested_proof_type: None,
            prover_type: None,
            execution_mode: None,
            aggregate_requested: true,
            proposals: Vec::new(),
            aggregate_task_id: Some(task_ref),
            aggregate_request: Some(aggregate_request.clone()),
            aggregate_input_artifacts: vec![AggregateInputProofArtifact {
                proof_ref: "aggregate-input-42".to_string(),
            }],
            runtime: RuntimeMetadata::default(),
        };
        let artifact_refs = publication_proof_artifact_refs(&metadata, pipeline);
        runtime
            .register_task(TaskRegistration {
                task_id: "task_public_aggregate".to_string(),
                pipeline_key: pipeline,
                route: pipeline.route(),
                task_kind: "hoodi_batch".to_string(),
                network_pair: "taiko_dev/ethereum".to_string(),
                artifact_refs,
                metadata: serde_json::to_value(metadata)?,
                request_fingerprint: "task-public-aggregate".to_string(),
            })
            .await?;
        let mut record = runtime
            .get_task("task_public_aggregate")
            .await?
            .expect("runtime task");
        record.runner_status = RunnerStatus::Allocated;
        runtime.upsert_task(&record).await?;

        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            pipeline.route(),
        );
        let id = EngineTaskId::new(EngineTaskKey::Aggregate {
            pipeline,
            request: aggregate_request.clone(),
        });
        let task = EngineTask::Aggregate {
            request: aggregate_request,
            source: raiko2_engine::AggregationSource::Inputs(Vec::new()),
        };
        let expires_at = now_secs().saturating_add(3_600);
        let checkpoint_permit = checkpoint_permit(&runtime);
        drive_engine_progress(
            &observer,
            &id,
            &task,
            &ProverProgress::BoundlessSubmission(BoundlessSubmissionProgress {
                provider_request_id: "0xaggregate".to_string(),
                remote_tx_hash: Some("0xaggregate-tx".to_string()),
                request_id_has_confirmed_submission: false,
                request_digest: None,
                broadcast_from_block: None,
                expires_at,
                lock_expires_at: expires_at - 600,
                submitted_at: expires_at - 900,
                image_ref: "0xaggregate-image".to_string(),
                deployment: "base".to_string(),
                offchain: false,
                quoted_mcycles_count: Some(164),
                evaluated_mcycles_count: Some(164),
                max_price_multiplier: 5,
                max_price_wei: Some("162360000000000".to_string()),
                rebid_attempt: 5,
            }),
            &checkpoint_permit,
        )
        .await?;

        drive_engine_checkpoint_clear(
            &observer,
            &id,
            &task,
            &PendingProofCheckpointIdentity {
                backend: NetworkProverBackend::Boundless,
                provider_request_id: "0xaggregate".to_string(),
                attempt: std::num::NonZeroU32::new(5).expect("non-zero attempt"),
            },
            &checkpoint_permit,
        )
        .await?;

        assert!(
            load_checkpoint_payload::<BoundlessSubmissionResume>(
                &observer,
                &id,
                &task,
                NetworkProverBackend::Boundless,
            )
            .await?
            .is_none()
        );
        Ok(())
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn runtime_observer_keeps_aggregate_root_allocated_after_proposal_proof() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-aggregate-pending",
        ))?);
        let pipeline = PipelineKey::ShastaSp1;
        let route = "sp1/network"
            .parse::<PipelineRoute>()
            .expect("parse SP1 network route");
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
        let metadata = TaskMetadata {
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
        };
        let artifact_refs = publication_proof_artifact_refs(&metadata, pipeline);
        runtime
            .register_task(TaskRegistration {
                task_id: "task_public_aggregate_pending".to_string(),
                pipeline_key: PipelineKey::ShastaSp1,
                route,
                task_kind: "hoodi_batch".to_string(),
                network_pair: "taiko_dev/ethereum".into(),
                artifact_refs,
                metadata: serde_json::to_value(metadata)?,
                request_fingerprint: "task-public-aggregate-pending".into(),
            })
            .await?;

        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            route,
        );
        let proof = compressed_sp1_proof_fixture();
        engine_checkpoint_proof(
            &observer,
            &proposal_task_id,
            &EngineTask::ProveProposal {
                request: request.clone(),
                input_task: proposal_task_id.clone(),
            },
            &proof,
        )
        .await?;
        assert_eq!(
            observer
                .load_completed_proof(
                    &proposal_task_id,
                    &EngineTask::Proposal {
                        request: request.clone(),
                    },
                )
                .await
                .map_err(anyhow::Error::msg)?,
            Some(proof.clone())
        );
        drive_engine_success(
            &observer,
            &proposal_task_id,
            &EngineTask::ProveProposal {
                request: request.clone(),
                input_task: proposal_task_id.clone(),
            },
            &EngineTaskSuccess::Proof {
                stage: raiko2_pipeline::PipelineStage::Prove,
                proof: proof.clone(),
            },
        )
        .await
        .map_err(anyhow::Error::msg)?;
        assert_eq!(
            observer
                .load_completed_proof(&proposal_task_id, &EngineTask::Proposal { request })
                .await
                .map_err(anyhow::Error::msg)?,
            Some(proof)
        );

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
        runtime
            .get_proof_artifact("taiko_dev/ethereum", pipeline, route, &proposal_ref)
            .await?
            .expect("proposal proof artifact");
        assert!(
            runtime
                .read_proof_artifact_bytes("taiko_dev/ethereum", pipeline, route, &proposal_ref,)
                .await?
                .is_some()
        );
        Ok(())
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn compressed_sp1_proposal_remains_valid_when_standalone_owner_joins_late() -> Result<()>
    {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-late-standalone-owner",
        ))?);
        let pipeline = PipelineKey::ShastaSp1;
        let route = "sp1/network"
            .parse::<PipelineRoute>()
            .expect("parse SP1 network route");
        let request = proposal_request();
        let proposal_task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        let task = EngineTask::ProveProposal {
            request: request.clone(),
            input_task: proposal_task_id.clone(),
        };
        let aggregate_request = AggregationTaskRequest {
            request_id: "agg-late-owner".to_string(),
            proposal_ids: vec![42],
            prover_config: ProverTaskConfig::default(),
        };
        let aggregate_metadata = TaskMetadata {
            network_pair: "taiko_dev/ethereum".to_string(),
            network: "taiko_dev".to_string(),
            l1_network: "ethereum".to_string(),
            proof_type: ProofType::Sp1,
            requested_proof_type: None,
            prover_type: None,
            execution_mode: Some(ExecutionMode::Prove),
            aggregate_requested: true,
            proposals: vec![proposal_metadata_task(pipeline, &request)],
            aggregate_task_id: Some(aggregate_task_ref(pipeline, &aggregate_request)),
            aggregate_request: Some(aggregate_request),
            aggregate_input_artifacts: Vec::new(),
            runtime: RuntimeMetadata::default(),
        };
        runtime
            .register_task(TaskRegistration {
                task_id: "aggregate-root".to_string(),
                pipeline_key: pipeline,
                route,
                task_kind: "hoodi_batch".to_string(),
                network_pair: "taiko_dev/ethereum".to_string(),
                artifact_refs: publication_proof_artifact_refs(&aggregate_metadata, pipeline),
                metadata: serde_json::to_value(aggregate_metadata)?,
                request_fingerprint: "aggregate-root".to_string(),
            })
            .await?;
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            route,
        );
        let execution_permit = engine_execution_permit(&observer, &proposal_task_id, &task).await?;
        let proof = compressed_sp1_proof_fixture();
        let completion_permit = EngineObserver::checkpoint_completed_proof(
            &observer,
            &proposal_task_id,
            &task,
            &proof,
            &execution_permit,
        )
        .await?;

        register_observer_task(
            runtime.as_ref(),
            "standalone-root",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Allocated,
        )
        .await?;
        EngineObserver::on_task_succeeded(
            &observer,
            &proposal_task_id,
            &task,
            &EngineTaskSuccess::Proof {
                stage: raiko2_pipeline::PipelineStage::Prove,
                proof: proof.clone(),
            },
            Some(&completion_permit),
            &execution_permit,
        )
        .await?;

        let aggregate = runtime
            .get_task("aggregate-root")
            .await?
            .expect("aggregate root");
        assert_eq!(aggregate.runner_status, RunnerStatus::Allocated);
        let mut standalone = runtime
            .get_task("standalone-root")
            .await?
            .expect("standalone root");
        if standalone.runner_status != RunnerStatus::Completed {
            let metadata = TaskMetadata::decode_for_record(&standalone)?;
            crate::server::task_cleanup::reconcile_runtime_task_from_artifacts(
                runtime.as_ref(),
                &standalone,
                &metadata,
            )
            .await?;
            standalone = runtime
                .get_task("standalone-root")
                .await?
                .expect("reconciled standalone root");
        }
        assert_eq!(standalone.runner_status, RunnerStatus::Completed);

        let proof_ref = proposal_task_ref(pipeline, &request);
        let proposal_artifact = crate::server::proof_artifact::load_proof_artifact_material(
            runtime.as_ref(),
            "taiko_dev/ethereum",
            pipeline,
            route,
            &proof_ref,
            ProofArtifactPayload::Proposal,
        )
        .await?
        .expect("proposal artifact");
        assert_eq!(proposal_artifact.proof, proof);
        assert!(
            crate::server::proof_artifact::load_proof_artifact_material(
                runtime.as_ref(),
                "taiko_dev/ethereum",
                pipeline,
                route,
                &proof_ref,
                ProofArtifactPayload::Final,
            )
            .await
            .is_err(),
            "a compressed proposal must never become a final aggregate proof"
        );
        Ok(())
    }

    #[tokio::test]
    async fn runtime_observer_publishes_compressed_sp1_root_artifact() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-compressed-sp1-root",
        ))?);
        let pipeline = PipelineKey::ShastaSp1;
        let request = proposal_request();
        let task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        register_observer_task(
            runtime.as_ref(),
            "task_compressed_sp1_root",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Running,
        )
        .await?;
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            "sp1/network"
                .parse::<PipelineRoute>()
                .expect("parse SP1 network route"),
        );
        let proof = compressed_sp1_proof_fixture();

        drive_engine_success(
            &observer,
            &task_id,
            &EngineTask::ProveProposal {
                request: request.clone(),
                input_task: task_id.clone(),
            },
            &EngineTaskSuccess::Proof {
                stage: raiko2_pipeline::PipelineStage::Prove,
                proof: proof.clone(),
            },
        )
        .await
        .map_err(anyhow::Error::msg)?;

        let record = runtime
            .get_task("task_compressed_sp1_root")
            .await?
            .expect("runtime task exists");
        assert_eq!(record.runner_status, RunnerStatus::Completed);
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
    async fn runtime_observer_loads_do_not_reuse_an_invalidated_artifact() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-invalidated-read-fence",
        ))?);
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let request = proposal_request();
        let task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        let proof_ref = RuntimeObserver::root_task_ref(&task_id);
        register_observer_task(
            runtime.as_ref(),
            "task_invalidated_read_fence",
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
        drive_engine_success(
            &observer,
            &task_id,
            &EngineTask::ProveProposal {
                request: request.clone(),
                input_task: task_id.clone(),
            },
            &EngineTaskSuccess::Proof {
                stage: raiko2_pipeline::PipelineStage::Prove,
                proof: proof_fixture(),
            },
        )
        .await
        .map_err(anyhow::Error::msg)?;

        let (_, invalidated, _) = runtime
            .update_tasks_and_invalidate_artifact(
                &proof_ref,
                "taiko_dev/ethereum",
                pipeline,
                route,
                |records| {
                    for record in records {
                        record.runner_status = RunnerStatus::Cancelled;
                    }
                    Ok(((), true))
                },
            )
            .await?;
        assert!(invalidated);

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
    async fn runtime_observer_rejects_incomplete_sp1_root_artifact() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-incomplete-sp1-root",
        ))?);
        let pipeline = PipelineKey::ShastaSp1;
        let request = proposal_request();
        let task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        register_observer_task(
            runtime.as_ref(),
            "task_incomplete_sp1_root",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Running,
        )
        .await?;
        let observer = RuntimeObserver::new(
            runtime,
            "taiko_dev/ethereum".to_string(),
            "sp1/network"
                .parse::<PipelineRoute>()
                .expect("parse SP1 network route"),
        );
        let mut proof = compressed_sp1_proof_fixture();
        proof.input = None;

        let error = drive_engine_success(
            &observer,
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
        .expect_err("incomplete SP1 root proof must be rejected");

        assert!(
            error
                .to_string()
                .contains("invalid Proposal proof artifact payload")
        );
        Ok(())
    }

    #[tokio::test]
    async fn runtime_observer_rejects_publication_without_active_runtime_row() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-detached-publication",
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

        let error = drive_engine_success(
            &observer,
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
        .expect_err("detached publication must be rejected");

        assert!(matches!(error, EngineObserverError::ProofInvalidated(_)));
        assert!(runtime.list_tasks().await?.is_empty());
        assert!(
            runtime
                .read_proof_artifact_bytes("taiko_dev/ethereum", pipeline, route, &proof_ref,)
                .await?
                .is_none()
        );
        Ok(())
    }

    #[tokio::test]
    async fn proposal_cleanup_pending_keeps_root_nonterminal() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-proposal-cleanup-pending",
        ))?);
        let network_pair = "taiko_dev/ethereum";
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let request = proposal_request();
        let task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        let proof_ref = proposal_task_ref(pipeline, &request);
        install_cleanup_pending_artifact(
            runtime.as_ref(),
            network_pair,
            pipeline,
            route,
            &proof_ref,
        )
        .await?;
        register_observer_task(
            runtime.as_ref(),
            "task_proposal_cleanup_pending",
            network_pair,
            pipeline,
            &request,
            RunnerStatus::Running,
        )
        .await?;
        let observer = RuntimeObserver::new(Arc::clone(&runtime), network_pair.to_string(), route);

        let error = drive_engine_success(
            &observer,
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
        .expect_err("cleanup-pending proposal publication must retry");

        assert!(matches!(error, EngineObserverError::ProofPublication(_)));
        let record = runtime
            .get_task("task_proposal_cleanup_pending")
            .await?
            .context("proposal cleanup-pending root")?;
        assert_eq!(record.runner_status, RunnerStatus::Allocated);
        Ok(())
    }

    #[tokio::test]
    async fn aggregate_cleanup_pending_keeps_root_nonterminal() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-aggregate-cleanup-pending",
        ))?);
        let network_pair = "taiko_dev/ethereum";
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let request = AggregationTaskRequest {
            request_id: "cleanup-pending-aggregate".to_string(),
            proposal_ids: vec![42, 43],
            prover_config: ProverTaskConfig::default(),
        };
        let proof_ref = aggregate_task_ref(pipeline, &request);
        install_cleanup_pending_artifact(
            runtime.as_ref(),
            network_pair,
            pipeline,
            route,
            &proof_ref,
        )
        .await?;
        let metadata = TaskMetadata {
            network_pair: network_pair.to_string(),
            network: "taiko_dev".to_string(),
            l1_network: "ethereum".to_string(),
            proof_type: ProofType::Native,
            requested_proof_type: None,
            prover_type: None,
            execution_mode: None,
            aggregate_requested: true,
            proposals: Vec::new(),
            aggregate_task_id: Some(proof_ref.clone()),
            aggregate_request: Some(request.clone()),
            aggregate_input_artifacts: vec![
                AggregateInputProofArtifact {
                    proof_ref: "aggregate-cleanup-pending-input-42".to_string(),
                },
                AggregateInputProofArtifact {
                    proof_ref: "aggregate-cleanup-pending-input-43".to_string(),
                },
            ],
            runtime: RuntimeMetadata::default(),
        };
        runtime
            .register_task(TaskRegistration {
                task_id: "task_aggregate_cleanup_pending".to_string(),
                pipeline_key: pipeline,
                route,
                task_kind: "hoodi_batch".to_string(),
                network_pair: network_pair.to_string(),
                artifact_refs: publication_proof_artifact_refs(&metadata, pipeline),
                metadata: serde_json::to_value(metadata)?,
                request_fingerprint: "aggregate-cleanup-pending".to_string(),
            })
            .await?;
        let observer = RuntimeObserver::new(Arc::clone(&runtime), network_pair.to_string(), route);
        let task_id = EngineTaskId::new(EngineTaskKey::Aggregate {
            pipeline,
            request: request.clone(),
        });

        let error = drive_engine_success(
            &observer,
            &task_id,
            &EngineTask::Aggregate {
                request,
                source: raiko2_engine::AggregationSource::Inputs(Vec::new()),
            },
            &EngineTaskSuccess::Proof {
                stage: raiko2_pipeline::PipelineStage::Aggregate,
                proof: proof_fixture(),
            },
        )
        .await
        .expect_err("cleanup-pending aggregate publication must retry");

        assert!(
            matches!(error, EngineObserverError::ProofPublication(_)),
            "{error:?}"
        );
        let record = runtime
            .get_task("task_aggregate_cleanup_pending")
            .await?
            .context("aggregate cleanup-pending root")?;
        assert_eq!(record.runner_status, RunnerStatus::Allocated);
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
        let proof = proof_fixture();
        register_observer_task(
            runtime.as_ref(),
            "task_publication_outbox",
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
        checkpoint_proof_fixture(
            &observer,
            &task_id,
            &EngineTask::ProveProposal {
                request: request.clone(),
                input_task: task_id.clone(),
            },
        )
        .await?;

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
    async fn publication_retry_can_finish_after_root_sync_completed() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-completed-publication-retry",
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
            "task_completed_publication_retry",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Completed,
        )
        .await?;
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            route,
        );
        let source = EngineTask::ProveProposal {
            request,
            input_task: task_id.clone(),
        };
        let proof = proof_fixture();
        let publication_task = source.clone().with_pending_publication(proof.clone());

        let execution_permit =
            EngineObserver::acquire_task_execution_permit(&observer, &task_id, &publication_task)
                .await?;
        let completion_permit = EngineObserver::checkpoint_completed_proof(
            &observer,
            &task_id,
            &source,
            &proof,
            &execution_permit,
        )
        .await?;
        EngineObserver::on_task_succeeded(
            &observer,
            &task_id,
            &source,
            &EngineTaskSuccess::Proof {
                stage: raiko2_pipeline::PipelineStage::Prove,
                proof,
            },
            Some(&completion_permit),
            &execution_permit,
        )
        .await?;

        let completed = runtime
            .get_task("task_completed_publication_retry")
            .await?
            .expect("completed runtime task");
        assert_eq!(completed.runner_status, RunnerStatus::Completed);
        assert!(completed.proof_uri.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn transient_success_sync_failure_stays_in_publication_retry_loop() -> Result<()> {
        let artifact_root = unique_runtime_root("runtime-observer-sync-retry-artifacts");
        let store = Arc::new(BlocksCanonicalPutStore {
            inner: MemoryProofArtifactStore::new(
                "shared-environment".to_string(),
                artifact_root
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            )?,
            block_next_put: AtomicBool::new(true),
            put_entered: tokio::sync::Notify::new(),
            allow_put: tokio::sync::Notify::new(),
        });
        let runtime = Arc::new(RuntimeManager::with_store(store.clone()));
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
        checkpoint_proof_fixture(
            &observer,
            &task_id,
            &EngineTask::ProveProposal {
                request: request.clone(),
                input_task: task_id.clone(),
            },
        )
        .await?;
        let publication = tokio::spawn(async move {
            drive_engine_success(
                &observer,
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

        store.put_entered.notified().await;
        let mut record = runtime
            .get_task("task_sync_retry")
            .await?
            .expect("runtime task");
        let valid_metadata = record.metadata.clone();
        record.metadata = serde_json::json!({"invalid": true});
        runtime.upsert_task(&record).await?;
        store.allow_put.notify_one();
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
    async fn cancellation_before_canonical_put_invalidates_late_publication() -> Result<()> {
        let artifact_root = unique_runtime_root("runtime-observer-cancel-before-put-artifacts");
        let store = Arc::new(BlocksCanonicalPutStore {
            inner: MemoryProofArtifactStore::new(
                "shared-environment".to_string(),
                artifact_root
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            )?,
            block_next_put: AtomicBool::new(true),
            put_entered: tokio::sync::Notify::new(),
            allow_put: tokio::sync::Notify::new(),
        });
        let runtime = Arc::new(RuntimeManager::with_store(store.clone()));
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let request = proposal_request();
        let task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        let proof_ref = proposal_task_ref(pipeline, &request);
        register_observer_task(
            runtime.as_ref(),
            "task_cancel_before_put",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Running,
        )
        .await?;
        let observer = Arc::new(RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            route,
        ));
        let publishing_observer = Arc::clone(&observer);
        let publishing_id = task_id.clone();
        let publication = tokio::spawn(async move {
            drive_engine_success(
                publishing_observer.as_ref(),
                &publishing_id,
                &EngineTask::ProveProposal {
                    request,
                    input_task: publishing_id.clone(),
                },
                &EngineTaskSuccess::Proof {
                    stage: raiko2_pipeline::PipelineStage::Prove,
                    proof: proof_fixture(),
                },
            )
            .await
        });

        store.put_entered.notified().await;
        let root = runtime
            .get_task("task_cancel_before_put")
            .await?
            .expect("running runtime task");
        runtime
            .cancel_task_if_current(&root.lifetime(), None)
            .await?;
        store.allow_put.notify_one();

        let error = publication
            .await?
            .expect_err("late publication must lose to cancellation");
        assert!(matches!(error, EngineObserverError::ProofInvalidated(_)));
        assert!(
            runtime
                .get_proof_artifact("taiko_dev/ethereum", pipeline, route, &proof_ref)
                .await?
                .is_none()
        );
        let cancelled = runtime
            .get_task("task_cancel_before_put")
            .await?
            .expect("cancelled runtime task");
        assert_eq!(cancelled.runner_status, RunnerStatus::Cancelled);
        assert_eq!(cancelled.proof_uri, None);
        Ok(())
    }

    #[tokio::test]
    async fn global_drain_waits_for_inflight_external_store_write() -> Result<()> {
        let store = Arc::new(BlocksCanonicalPutStore {
            inner: MemoryProofArtifactStore::new(
                "test".to_string(),
                "global-drain-store-write".to_string(),
            )?,
            block_next_put: AtomicBool::new(true),
            put_entered: tokio::sync::Notify::new(),
            allow_put: tokio::sync::Notify::new(),
        });
        let runtime = Arc::new(RuntimeManager::with_store(store.clone()));
        let publishing_runtime = Arc::clone(&runtime);
        let publication = tokio::spawn(async move {
            publishing_runtime
                .publish_proof_artifact_bytes(
                    "taiko_dev/ethereum",
                    PipelineKey::ShastaNative,
                    PipelineKey::ShastaNative.route(),
                    "proposal-1",
                    br#"{"proof":"0x01"}"#,
                )
                .await
        });

        store.put_entered.notified().await;
        let draining_runtime = Arc::clone(&runtime);
        let mut draining = tokio::spawn(async move { draining_runtime.begin_draining().await });
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut draining)
                .await
                .is_err(),
            "draining must wait until the external store write leaves the global fence"
        );

        store.allow_put.notify_one();
        draining.await?;
        let error = publication
            .await?
            .expect_err("publication must not commit after the global fence begins draining");
        assert!(format!("{error:#}").contains("runtime is draining"));
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
        drive_engine_start(&observer, &proposal_task_id, &task, "worker-a")
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;

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
    async fn runtime_observer_retry_start_does_not_reopen_failed_root() -> Result<()> {
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
        let error = drive_engine_start(&observer, &proposal_task_id, &task, "worker-a")
            .await
            .expect_err("a failed root cannot acquire an execution permit");
        assert!(matches!(error, EngineObserverError::ProofInvalidated(_)));

        let record = runtime
            .get_task("task_failed_retry")
            .await?
            .expect("failed task");
        let metadata: TaskMetadata = serde_json::from_value(record.metadata)?;
        assert_eq!(record.runner_status, RunnerStatus::Failed);
        assert_eq!(record.error.as_deref(), Some("transient preflight failure"));
        assert_eq!(metadata.runtime.active_stage, None);
        assert_eq!(metadata.runtime.last_event, None);
        Ok(())
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
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
                pipeline_key: PipelineKey::ShastaSp1,
                route: "sp1/local".parse::<PipelineRoute>().expect("parse route"),
                task_kind: "hoodi_batch".to_string(),
                network_pair: "taiko_dev/ethereum".into(),
                artifact_refs: vec![first_ref.clone(), second_ref.clone()],
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
                request_fingerprint: "task-public-multi-proposal".into(),
            })
            .await?;

        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            PipelineKey::ShastaSp1.route(),
        );
        checkpoint_proof_fixture(
            &observer,
            &first_task_id,
            &EngineTask::ProveProposal {
                request: first_request.clone(),
                input_task: first_task_id.clone(),
            },
        )
        .await?;
        drive_engine_success(
            &observer,
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

        checkpoint_proof_fixture(
            &observer,
            &second_task_id,
            &EngineTask::ProveProposal {
                request: second_request.clone(),
                input_task: second_task_id.clone(),
            },
        )
        .await?;
        drive_engine_success(
            &observer,
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
                pipeline_key: PipelineKey::ShastaSp1,
                route: "sp1/network".parse::<PipelineRoute>().expect("parse route"),
                task_kind: "hoodi_batch".to_string(),
                network_pair: "taiko_dev/ethereum".into(),
                artifact_refs: vec![task_ref.clone()],
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
                        request: proposal_request(),
                    }],
                    aggregate_task_id: None,
                    aggregate_request: None,
                    aggregate_input_artifacts: Vec::new(),
                    runtime: RuntimeMetadata::default(),
                })?,
                request_fingerprint: "task-public-sp1".into(),
            })
            .await?;

        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            "sp1/network".parse::<PipelineRoute>().expect("parse route"),
        );
        drive_engine_progress(
            &observer,
            &proposal_task_id,
            &EngineTask::ProveProposal {
                request: proposal_request(),
                input_task: proposal_task_id.clone(),
            },
            &ProverProgress::Sp1NetworkSubmission(Sp1NetworkSubmissionProgress {
                attempt: 1,
                provider_request_id: "0xsp1".to_string(),
                submitted_at: now_secs(),
                network_mode: Sp1NetworkMode::Reserved,
                fulfillment_strategy: Sp1FulfillmentStrategy::Reserved,
                skip_simulation: true,
                cycle_limit: 1_000_000_000_000,
                timeout_secs: 3_600,
                max_price_per_pgu: Some(42),
                auction_timeout_secs: Some(120),
            }),
            &checkpoint_permit(&runtime),
        )
        .await
        .expect("persist SP1 submission progress");

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
        record.runner_status = RunnerStatus::Allocated;
        runtime.upsert_task(&record).await?;
        let resumed = load_checkpoint_payload::<Sp1NetworkSubmissionProgress>(
            &observer,
            &proposal_task_id,
            &EngineTask::ProveProposal {
                request: proposal_request(),
                input_task: proposal_task_id.clone(),
            },
            NetworkProverBackend::Sp1,
        )
        .await?;
        assert_eq!(
            resumed
                .as_ref()
                .map(|resume| resume.provider_request_id.as_str()),
            Some("0xsp1")
        );
        Ok(())
    }

    #[tokio::test]
    async fn runtime_observer_counts_replayed_external_submission_once() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-submission-metrics",
        ))?);
        let pipeline = PipelineKey::ShastaSp1;
        let request = proposal_request_with_id(9_901);
        let id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        let network_pair = "submission_metrics/ethereum";
        register_observer_task(
            runtime.as_ref(),
            "task_submission_metrics",
            network_pair,
            pipeline,
            &request,
            RunnerStatus::Running,
        )
        .await?;
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            network_pair.to_string(),
            "sp1/network"
                .parse::<PipelineRoute>()
                .expect("parse SP1 network route"),
        );
        let task = EngineTask::ProveProposal {
            request,
            input_task: id.clone(),
        };
        let progress = ProverProgress::Sp1NetworkSubmission(Sp1NetworkSubmissionProgress {
            attempt: 1,
            provider_request_id: "0xmetric-submission".to_string(),
            submitted_at: now_secs(),
            network_mode: Sp1NetworkMode::Reserved,
            fulfillment_strategy: Sp1FulfillmentStrategy::Reserved,
            skip_simulation: true,
            cycle_limit: 1_000_000,
            timeout_secs: 3_600,
            max_price_per_pgu: None,
            auction_timeout_secs: None,
        });
        let permit = checkpoint_permit(&runtime);

        drive_engine_progress(&observer, &id, &task, &progress, &permit).await?;
        drive_engine_progress(&observer, &id, &task, &progress, &permit).await?;

        let (_, body) = telemetry::render()?;
        let body = String::from_utf8(body)?;
        let metric = body
            .lines()
            .find(|line| {
                line.starts_with("raiko2_external_submission_total{")
                    && line.contains("pair=\"submission_metrics/ethereum\"")
                    && line.contains("provider=\"sp1_network\"")
                    && line.contains("stage=\"prove\"")
            })
            .context("external submission metric")?;
        assert_eq!(metric.split_whitespace().last(), Some("1"));
        Ok(())
    }

    #[tokio::test]
    async fn runtime_generation_conflict_is_permanent() -> Result<()> {
        let store = Arc::new(MemoryProofArtifactStore::new(
            "test".into(),
            "progress-conflict".into(),
        )?);
        let runtime = Arc::new(RuntimeManager::with_store(store.clone()));
        let pipeline = PipelineKey::ShastaSp1;
        let request = proposal_request();
        let proposal_task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        register_observer_task(
            runtime.as_ref(),
            "task_progress_conflict",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Running,
        )
        .await?;
        let permit = checkpoint_permit(&runtime);

        let competing_runtime = RuntimeManager::with_store(store);
        competing_runtime.initialize().await?;
        competing_runtime
            .register_task(TaskRegistration {
                task_id: "other-task".into(),
                pipeline_key: PipelineKey::ShastaNative,
                route: PipelineKey::ShastaNative.route(),
                task_kind: "proposal".into(),
                network_pair: "taiko_dev/ethereum".into(),
                artifact_refs: Vec::new(),
                metadata: serde_json::json!({}),
                request_fingerprint: "other-task".into(),
            })
            .await?;

        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            "sp1/network".parse::<PipelineRoute>().expect("parse route"),
        );
        let error = drive_engine_progress(
            &observer,
            &proposal_task_id,
            &EngineTask::ProveProposal {
                request,
                input_task: proposal_task_id.clone(),
            },
            &ProverProgress::Sp1NetworkSubmission(Sp1NetworkSubmissionProgress {
                attempt: 1,
                provider_request_id: "0xaccepted".to_string(),
                submitted_at: now_secs(),
                network_mode: Sp1NetworkMode::Reserved,
                fulfillment_strategy: Sp1FulfillmentStrategy::Reserved,
                skip_simulation: true,
                cycle_limit: 1_000_000,
                timeout_secs: 3_600,
                max_price_per_pgu: None,
                auction_timeout_secs: None,
            }),
            &permit,
        )
        .await
        .expect_err("the stale runtime generation must reject progress");

        assert!(runtime.is_lifecycle_active());
        assert!(!runtime.accepts_mutations());
        assert!(matches!(error, EngineObserverError::RuntimeInactive(_)));
        assert!(runtime.check_readiness().await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn provider_checkpoint_includes_a_root_joined_after_execution_admission() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-late-provider-owner",
        ))?);
        let pipeline = PipelineKey::ShastaSp1;
        let request = proposal_request();
        let proposal_task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        let task = EngineTask::ProveProposal {
            request: request.clone(),
            input_task: proposal_task_id.clone(),
        };
        register_observer_task(
            runtime.as_ref(),
            "root-a",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Running,
        )
        .await?;
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            "sp1/network".parse::<PipelineRoute>().expect("parse route"),
        );
        let execution_permit = engine_execution_permit(&observer, &proposal_task_id, &task).await?;

        register_observer_task(
            runtime.as_ref(),
            "root-b",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Allocated,
        )
        .await?;
        let progress = Sp1NetworkSubmissionProgress {
            attempt: 1,
            provider_request_id: "0xshared".to_string(),
            submitted_at: now_secs(),
            network_mode: Sp1NetworkMode::Reserved,
            fulfillment_strategy: Sp1FulfillmentStrategy::Reserved,
            skip_simulation: true,
            cycle_limit: 1_000_000,
            timeout_secs: 3_600,
            max_price_per_pgu: None,
            auction_timeout_secs: None,
        };
        EngineObserver::on_task_progress(
            &observer,
            &proposal_task_id,
            &task,
            &ProverProgress::Sp1NetworkSubmission(progress),
            &checkpoint_permit(&runtime),
            &execution_permit,
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

        for task_id in ["root-a", "root-b"] {
            let record = runtime.get_task(task_id).await?.expect("shared root");
            let metadata = TaskMetadata::decode_for_record(&record)?;
            assert_eq!(
                metadata
                    .proposal_runtime(&RuntimeObserver::root_task_ref(&proposal_task_id))
                    .and_then(|runtime| runtime.provider_request_id.as_deref()),
                Some("0xshared"),
                "{task_id}"
            );
        }

        let root_a = runtime.get_task("root-a").await?.expect("root a");
        assert_eq!(
            runtime
                .cancel_task_if_current(&root_a.lifetime(), None)
                .await?,
            raiko2_runtime::RuntimeMutationOutcome::Applied
        );
        let resumed = load_checkpoint_payload::<Sp1NetworkSubmissionProgress>(
            &observer,
            &proposal_task_id,
            &task,
            NetworkProverBackend::Sp1,
        )
        .await?
        .expect("remaining owner must retain the shared provider checkpoint");
        assert_eq!(resumed.provider_request_id, "0xshared");
        Ok(())
    }

    #[tokio::test]
    async fn same_attempt_provider_identity_drift_is_permanently_rejected() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-provider-identity-drift",
        ))?);
        let pipeline = PipelineKey::ShastaSp1;
        let request = proposal_request();
        let proposal_task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        let task = EngineTask::ProveProposal {
            request: request.clone(),
            input_task: proposal_task_id.clone(),
        };
        register_observer_task(
            runtime.as_ref(),
            "root-a",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Running,
        )
        .await?;
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            "sp1/network".parse::<PipelineRoute>().expect("parse route"),
        );
        let execution_permit = engine_execution_permit(&observer, &proposal_task_id, &task).await?;
        let mut progress = Sp1NetworkSubmissionProgress {
            attempt: 1,
            provider_request_id: "0xoriginal".to_string(),
            submitted_at: now_secs(),
            network_mode: Sp1NetworkMode::Reserved,
            fulfillment_strategy: Sp1FulfillmentStrategy::Reserved,
            skip_simulation: true,
            cycle_limit: 1_000_000,
            timeout_secs: 3_600,
            max_price_per_pgu: None,
            auction_timeout_secs: None,
        };
        EngineObserver::on_task_progress(
            &observer,
            &proposal_task_id,
            &task,
            &ProverProgress::Sp1NetworkSubmission(progress.clone()),
            &checkpoint_permit(&runtime),
            &execution_permit,
        )
        .await
        .expect("persist original provider identity");

        progress.provider_request_id = "0xconflict".to_string();
        let error = EngineObserver::on_task_progress(
            &observer,
            &proposal_task_id,
            &task,
            &ProverProgress::Sp1NetworkSubmission(progress),
            &checkpoint_permit(&runtime),
            &execution_permit,
        )
        .await
        .expect_err("one attempt cannot replace its provider identity");
        assert!(matches!(error, EngineObserverError::ProgressRejected(_)));

        let resumed = load_checkpoint_payload::<Sp1NetworkSubmissionProgress>(
            &observer,
            &proposal_task_id,
            &task,
            NetworkProverBackend::Sp1,
        )
        .await?
        .expect("original checkpoint remains durable");
        assert_eq!(resumed.provider_request_id, "0xoriginal");
        Ok(())
    }

    #[tokio::test]
    async fn late_owner_recovers_checkpoint_from_a_cancelled_shared_owner() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-post-checkpoint-owner",
        ))?);
        let pipeline = PipelineKey::ShastaSp1;
        let request = proposal_request();
        let proposal_task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        let task = EngineTask::ProveProposal {
            request: request.clone(),
            input_task: proposal_task_id.clone(),
        };
        register_observer_task(
            runtime.as_ref(),
            "root-a",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Running,
        )
        .await?;
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            "sp1/network".parse::<PipelineRoute>().expect("parse route"),
        );
        let execution_permit = engine_execution_permit(&observer, &proposal_task_id, &task).await?;
        EngineObserver::on_task_progress(
            &observer,
            &proposal_task_id,
            &task,
            &ProverProgress::Sp1NetworkSubmission(Sp1NetworkSubmissionProgress {
                attempt: 1,
                provider_request_id: "0xoriginal".to_string(),
                submitted_at: now_secs(),
                network_mode: Sp1NetworkMode::Reserved,
                fulfillment_strategy: Sp1FulfillmentStrategy::Reserved,
                skip_simulation: true,
                cycle_limit: 1_000_000,
                timeout_secs: 3_600,
                max_price_per_pgu: None,
                auction_timeout_secs: None,
            }),
            &checkpoint_permit(&runtime),
            &execution_permit,
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

        register_observer_task(
            runtime.as_ref(),
            "root-b",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Allocated,
        )
        .await?;
        let root_a = runtime.get_task("root-a").await?.expect("root a");
        runtime
            .cancel_task_if_current(&root_a.lifetime(), None)
            .await?;

        let resumed = load_checkpoint_payload::<Sp1NetworkSubmissionProgress>(
            &observer,
            &proposal_task_id,
            &task,
            NetworkProverBackend::Sp1,
        )
        .await?
        .expect("late owner must reuse the shared request after restart");
        assert_eq!(resumed.provider_request_id, "0xoriginal");
        EngineObserver::on_task_progress(
            &observer,
            &proposal_task_id,
            &task,
            &ProverProgress::Sp1NetworkSubmission(resumed.clone()),
            &checkpoint_permit(&runtime),
            &execution_permit,
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let root_b = runtime.get_task("root-b").await?.expect("root b");
        let root_b_metadata = TaskMetadata::decode_for_record(&root_b)?;
        let root_b_checkpoint = root_b_metadata
            .proposal_runtime(&RuntimeObserver::root_task_ref(&proposal_task_id))
            .expect("resumed owner receives the durable checkpoint");
        assert_eq!(root_b_checkpoint.submitted_at, Some(resumed.submitted_at));
        let reloaded = load_checkpoint_payload::<Sp1NetworkSubmissionProgress>(
            &observer,
            &proposal_task_id,
            &task,
            NetworkProverBackend::Sp1,
        )
        .await?
        .expect("reprojected checkpoint remains canonical across owners");
        assert_eq!(reloaded, resumed);
        Ok(())
    }

    #[tokio::test]
    async fn cancelled_root_rejects_a_late_provider_checkpoint() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-cancelled-progress",
        ))?);
        let pipeline = PipelineKey::ShastaSp1;
        let request = proposal_request();
        let proposal_task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        register_observer_task(
            runtime.as_ref(),
            "task_progress_cancelled",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Running,
        )
        .await?;
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            "sp1/network".parse::<PipelineRoute>().expect("parse route"),
        );
        let task = EngineTask::ProveProposal {
            request,
            input_task: proposal_task_id.clone(),
        };
        let execution_permit = engine_execution_permit(&observer, &proposal_task_id, &task).await?;
        let checkpoint_permit = checkpoint_permit(&runtime);
        let record = runtime
            .get_task("task_progress_cancelled")
            .await?
            .expect("runtime task");
        assert_eq!(
            runtime
                .cancel_task_if_current(&record.lifetime(), None)
                .await?,
            raiko2_runtime::RuntimeMutationOutcome::Applied
        );

        let error = EngineObserver::on_task_progress(
            &observer,
            &proposal_task_id,
            &task,
            &ProverProgress::Sp1NetworkSubmission(Sp1NetworkSubmissionProgress {
                attempt: 1,
                provider_request_id: "0xlate".to_string(),
                submitted_at: now_secs(),
                network_mode: Sp1NetworkMode::Reserved,
                fulfillment_strategy: Sp1FulfillmentStrategy::Reserved,
                skip_simulation: true,
                cycle_limit: 1_000_000,
                timeout_secs: 3_600,
                max_price_per_pgu: None,
                auction_timeout_secs: None,
            }),
            &checkpoint_permit,
            &execution_permit,
        )
        .await
        .expect_err("cancelled root must close provider checkpoint admission");

        assert!(matches!(error, EngineObserverError::RuntimeInactive(_)));
        let cancelled = runtime
            .get_task("task_progress_cancelled")
            .await?
            .expect("runtime task");
        assert_eq!(cancelled.runner_status, RunnerStatus::Cancelled);
        let metadata = TaskMetadata::decode_for_record(&cancelled)?;
        assert!(
            metadata
                .proposal_runtime(&RuntimeObserver::root_task_ref(&proposal_task_id))
                .is_none_or(|runtime| runtime.provider_request_id.is_none())
        );
        Ok(())
    }

    #[tokio::test]
    async fn draining_lifecycle_allows_pre_admitted_checkpoint() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-progress-draining",
        ))?);
        let pipeline = PipelineKey::ShastaSp1;
        let request = proposal_request();
        let proposal_task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        register_observer_task(
            runtime.as_ref(),
            "task_progress_draining",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Running,
        )
        .await?;
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            "sp1/network".parse::<PipelineRoute>().expect("parse route"),
        );
        let task = EngineTask::ProveProposal {
            request,
            input_task: proposal_task_id.clone(),
        };
        let execution_permit = engine_execution_permit(&observer, &proposal_task_id, &task).await?;
        let permit = checkpoint_permit(&runtime);
        runtime.start_draining();

        EngineObserver::on_task_progress(
            &observer,
            &proposal_task_id,
            &task,
            &ProverProgress::Sp1NetworkSubmission(Sp1NetworkSubmissionProgress {
                attempt: 1,
                provider_request_id: "0xaccepted".to_string(),
                submitted_at: now_secs(),
                network_mode: Sp1NetworkMode::Reserved,
                fulfillment_strategy: Sp1FulfillmentStrategy::Reserved,
                skip_simulation: true,
                cycle_limit: 1_000_000,
                timeout_secs: 3_600,
                max_price_per_pgu: None,
                auction_timeout_secs: None,
            }),
            &permit,
            &execution_permit,
        )
        .await
        .expect("pre-admitted checkpoint must persist while draining");

        drop(permit);
        runtime.begin_draining().await;
        assert!(!runtime.is_lifecycle_active());
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
        let other_request = proposal_request_with_id(43);
        let other_task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaSp1,
            request: other_request.clone(),
        });
        let task_ref = proposal_task_ref(PipelineKey::ShastaSp1, &proposal_request());
        let other_task_ref = proposal_task_ref(PipelineKey::ShastaSp1, &other_request);
        runtime
            .register_task(TaskRegistration {
                task_id: "task_public_sp1_load".to_string(),
                pipeline_key: PipelineKey::ShastaSp1,
                route: "sp1/network".parse::<PipelineRoute>().expect("parse route"),
                task_kind: "hoodi_batch".to_string(),
                network_pair: "taiko_dev/ethereum".into(),
                artifact_refs: vec![task_ref.clone(), other_task_ref.clone()],
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
                            request: proposal_request(),
                        },
                        ProposalTask {
                            proposal_id: 43,
                            checkpoint: None,
                            l1_inclusion_block_number: 1,
                            l2_block_numbers: vec![43],
                            last_anchor_block_number: 0,
                            task_id: other_task_ref.clone(),
                            request: other_request.clone(),
                        },
                    ],
                    aggregate_task_id: None,
                    aggregate_request: None,
                    aggregate_input_artifacts: Vec::new(),
                    runtime: RuntimeMetadata::default(),
                })?,
                request_fingerprint: "task-public-sp1-load".into(),
            })
            .await?;

        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            "sp1/network".parse::<PipelineRoute>().expect("parse route"),
        );
        drive_engine_progress(
            &observer,
            &proposal_task_id,
            &EngineTask::ProveProposal {
                request: proposal_request(),
                input_task: proposal_task_id.clone(),
            },
            &ProverProgress::Sp1NetworkSubmission(Sp1NetworkSubmissionProgress {
                attempt: 1,
                provider_request_id: "0xsp1-proposal".to_string(),
                submitted_at: now_secs(),
                network_mode: Sp1NetworkMode::Reserved,
                fulfillment_strategy: Sp1FulfillmentStrategy::Reserved,
                skip_simulation: true,
                cycle_limit: 1_000_000_000_000,
                timeout_secs: 3_600,
                max_price_per_pgu: Some(42),
                auction_timeout_secs: Some(120),
            }),
            &checkpoint_permit(&runtime),
        )
        .await
        .expect("persist proposal SP1 submission progress");

        let resumed = load_checkpoint_payload::<Sp1NetworkSubmissionProgress>(
            &observer,
            &proposal_task_id,
            &EngineTask::ProveProposal {
                request: proposal_request(),
                input_task: proposal_task_id.clone(),
            },
            NetworkProverBackend::Sp1,
        )
        .await?;
        assert_eq!(
            resumed
                .as_ref()
                .map(|resume| resume.provider_request_id.as_str()),
            Some("0xsp1-proposal")
        );
        assert!(
            load_checkpoint_payload::<Sp1NetworkSubmissionProgress>(
                &observer,
                &other_task_id,
                &EngineTask::ProveProposal {
                    request: other_request,
                    input_task: other_task_id.clone(),
                },
                NetworkProverBackend::Sp1,
            )
            .await?
            .is_none()
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
                pipeline_key: PipelineKey::ShastaSp1,
                route: "sp1/network".parse::<PipelineRoute>().expect("parse route"),
                task_kind: "hoodi_batch".to_string(),
                network_pair: "taiko_dev/ethereum".into(),
                artifact_refs: vec![
                    task_ref.clone(),
                    "aggregate-input-42".to_string(),
                    "aggregate-input-43".to_string(),
                ],
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
                    aggregate_input_artifacts: vec![
                        AggregateInputProofArtifact {
                            proof_ref: "aggregate-input-42".to_string(),
                        },
                        AggregateInputProofArtifact {
                            proof_ref: "aggregate-input-43".to_string(),
                        },
                    ],
                    runtime: RuntimeMetadata::default(),
                })?,
                request_fingerprint: "task-public-sp1-aggregate".into(),
            })
            .await?;

        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            "sp1/network".parse::<PipelineRoute>().expect("parse route"),
        );
        assert!(
            load_checkpoint_payload::<Sp1NetworkSubmissionProgress>(
                &observer,
                &aggregate_task_id,
                &EngineTask::Aggregate {
                    request: aggregate_request.clone(),
                    source: raiko2_engine::AggregationSource::Inputs(vec![]),
                },
                NetworkProverBackend::Sp1,
            )
            .await?
            .is_none()
        );

        drive_engine_progress(
            &observer,
            &aggregate_task_id,
            &EngineTask::Aggregate {
                request: aggregate_request.clone(),
                source: raiko2_engine::AggregationSource::Inputs(vec![]),
            },
            &ProverProgress::Sp1NetworkSubmission(Sp1NetworkSubmissionProgress {
                attempt: 1,
                provider_request_id: "0xsp1-aggregate".to_string(),
                submitted_at: now_secs(),
                network_mode: Sp1NetworkMode::Reserved,
                fulfillment_strategy: Sp1FulfillmentStrategy::Reserved,
                skip_simulation: true,
                cycle_limit: 1_000_000_000_000,
                timeout_secs: 3_600,
                max_price_per_pgu: Some(42),
                auction_timeout_secs: Some(120),
            }),
            &checkpoint_permit(&runtime),
        )
        .await
        .expect("persist aggregate SP1 submission progress");

        let resumed = load_checkpoint_payload::<Sp1NetworkSubmissionProgress>(
            &observer,
            &aggregate_task_id,
            &EngineTask::Aggregate {
                request: aggregate_request,
                source: raiko2_engine::AggregationSource::Inputs(vec![]),
            },
            NetworkProverBackend::Sp1,
        )
        .await?;
        assert_eq!(
            resumed
                .as_ref()
                .map(|resume| resume.provider_request_id.as_str()),
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
                request: proposal_request(),
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
                pipeline_key: PipelineKey::ShastaNative,
                route: "native/local"
                    .parse::<PipelineRoute>()
                    .expect("parse route"),
                task_kind: "hoodi_batch".to_string(),
                network_pair: metadata.network_pair.clone(),
                artifact_refs: vec![proposal_ref.clone()],
                metadata: serde_json::to_value(metadata)?,
                request_fingerprint: "task-public-restart".into(),
            })
            .await?;

        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "telemetry_restart/ethereum".to_string(),
            PipelineKey::ShastaNative.route(),
        );
        let projection = drive_engine_failure(
            &observer,
            &proposal_task_id,
            &EngineTask::Preflight {
                request: proposal_request(),
            },
            "fixture failed",
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        drop(projection);

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
    async fn runtime_observer_decrements_inflight_for_completed_root() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-terminal-metrics",
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
        let network_pair = "terminal_metrics/ethereum";
        register_observer_task(
            runtime.as_ref(),
            "task_terminal_metrics",
            network_pair,
            pipeline,
            &request,
            RunnerStatus::Allocated,
        )
        .await?;

        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            network_pair.to_string(),
            pipeline.route(),
        );
        drive_engine_start(&observer, &proposal_task_id, &task, "worker-a")
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let mut root = runtime
            .get_task("task_terminal_metrics")
            .await?
            .context("runtime root should exist")?;
        root.runner_status = RunnerStatus::Completed;
        runtime.upsert_task(&root).await?;

        observer
            .observe_stage_terminal_metrics(
                &proposal_task_id,
                &RuntimeObserver::timing_key_for_task(&proposal_task_id, &task),
                "preflight",
                "completed",
                now_ms(),
                None,
            )
            .await;

        let (_, body) = telemetry::render().expect("render telemetry");
        let body = String::from_utf8(body).expect("utf8 telemetry");
        assert!(
            body.contains(
                "raiko2_stage_tasks_inflight{aggregate=\"false\",pair=\"terminal_metrics/ethereum\",proof_type=\"native\",route=\"native/local\",stage=\"preflight\"} 0"
            ),
            "{body}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn runtime_observer_decrements_inflight_when_last_owner_is_cancelled() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-cancelled-metrics",
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
        let network_pair = "cancelled_metrics/ethereum";
        register_observer_task(
            runtime.as_ref(),
            "task_cancelled_metrics",
            network_pair,
            pipeline,
            &request,
            RunnerStatus::Allocated,
        )
        .await?;
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            network_pair.to_string(),
            pipeline.route(),
        );
        drive_engine_start(&observer, &proposal_task_id, &task, "worker-a")
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let root = runtime
            .get_task("task_cancelled_metrics")
            .await?
            .context("runtime root should exist")?;
        runtime
            .cancel_task_if_current(&root.lifetime(), None)
            .await?;

        EngineObserver::on_task_cancelled(&observer, &proposal_task_id).await;

        let (_, body) = telemetry::render().expect("render telemetry");
        let body = String::from_utf8(body).expect("utf8 telemetry");
        assert!(
            body.contains(
                "raiko2_stage_tasks_inflight{aggregate=\"false\",pair=\"cancelled_metrics/ethereum\",proof_type=\"native\",route=\"native/local\",stage=\"preflight\"} 0"
            ),
            "{body}"
        );
        assert!(
            body.contains(
                "raiko2_stage_task_terminal_total{aggregate=\"false\",pair=\"cancelled_metrics/ethereum\",proof_type=\"native\",route=\"native/local\",stage=\"preflight\",status=\"cancelled\"} 1"
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
        let projection = drive_engine_failure(
            &observer,
            &proposal_task_id,
            &EngineTask::ProveProposal {
                request,
                input_task: proposal_task_id.clone(),
            },
            "sgx INVALID_REQUEST: aggregate proof 0 SGX instance id mismatch: got 4 expected 5",
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        drop(projection);

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
    async fn terminal_failure_persists_roots_and_returns_their_projection_owners() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-terminal-failure-projection",
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
            "task_terminal_failure_projection",
            "terminal_failure_projection/ethereum",
            pipeline,
            &request,
            RunnerStatus::Allocated,
        )
        .await?;
        let registered = runtime
            .get_task("task_terminal_failure_projection")
            .await?
            .expect("registered runtime root");
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "terminal_failure_projection/ethereum".to_string(),
            pipeline.route(),
        );
        let execution_permit = engine_execution_permit(&observer, &proposal_task_id, &task).await?;
        let terminal_permit = EngineObserver::acquire_terminal_failure_permit(
            &observer,
            &proposal_task_id,
            &task,
            &execution_permit,
        )
        .await?;

        let projection = EngineObserver::on_task_failed(
            &observer,
            &proposal_task_id,
            &task,
            "terminal failure",
            terminal_permit,
        )
        .await?;

        assert_eq!(
            projection.root_owners(),
            &[RootOwner::new(
                registered.task_id,
                registered.incarnation_id,
            )]
        );
        let failed = runtime
            .get_task("task_terminal_failure_projection")
            .await?
            .expect("failed runtime root");
        assert_eq!(failed.runner_status, RunnerStatus::Failed);
        assert_eq!(failed.error.as_deref(), Some("terminal failure"));
        drop(projection);
        Ok(())
    }

    #[tokio::test]
    async fn runtime_observer_fails_closed_when_outbox_checkpoint_cannot_persist() -> Result<()> {
        let store = Arc::new(FailingArtifactStore::default());
        let runtime = Arc::new(RuntimeManager::with_store(store.clone()));
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
        let error = drive_engine_success(
            &observer,
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
        assert_eq!(store.attempts.load(Ordering::SeqCst), 1);

        let record = runtime
            .get_task("task_proof_persistence_failure")
            .await?
            .expect("runtime task exists");
        assert_eq!(record.runner_status, RunnerStatus::Allocated);
        assert_eq!(record.error, None);

        let (_, body) = telemetry::render().expect("render telemetry");
        let body = String::from_utf8(body).expect("utf8 telemetry");
        assert!(
            !body.contains(
                "raiko2_stage_task_terminal_total{aggregate=\"false\",pair=\"metrics_proof_persistence/ethereum\",proof_type=\"native\",route=\"native/local\",stage=\"prove\",status=\"completed\"}"
            ),
            "{body}"
        );
        Ok(())
    }
}
