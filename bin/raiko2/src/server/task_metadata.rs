use alloy_primitives::{U256, hex, keccak256};
use anyhow::{Context, Result};
use raiko2_engine::{
    AggregationTaskRequest, EngineTaskId, EngineTaskKey, ProposalStage, ProposalTaskRequest,
};
use raiko2_pipeline::{GuestSystem, PipelineKey, PipelineRoute, RunnerKind};
use raiko2_primitives::{L2BlockRange, ProofType, ShastaCheckpoint, proof_type::lowercase};
use raiko2_prover::{
    BoundlessSubmissionProgress, Sp1FulfillmentStrategy, Sp1NetworkMode,
    Sp1NetworkSubmissionProgress, sp1_config::ExecutionMode,
};
use raiko2_runtime::{ProofArtifactDescriptor, RuntimeTaskRecord};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ProverType {
    Mock,
    Local,
    Network,
}

impl ProverType {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Mock => "mock",
            Self::Local => "local",
            Self::Network => "network",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskMetadata {
    pub(crate) network_pair: String,
    pub(crate) network: String,
    pub(crate) l1_network: String,
    #[serde(with = "lowercase")]
    pub(crate) proof_type: ProofType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) requested_proof_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) prover_type: Option<ProverType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) execution_mode: Option<ExecutionMode>,
    pub(crate) aggregate_requested: bool,
    pub(crate) proposals: Vec<ProposalTask>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) aggregate_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) aggregate_request: Option<AggregationTaskRequest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) aggregate_input_artifacts: Vec<AggregateInputProofArtifact>,
    #[serde(default)]
    pub(crate) runtime: RuntimeMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AggregateInputProofArtifact {
    pub(crate) proof_ref: String,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct BuildTaskMetadataParams<'a> {
    pub(crate) network: &'a str,
    pub(crate) l1_network: &'a str,
    pub(crate) proof_type: ProofType,
    pub(crate) requested_proof_type: Option<&'a str>,
    pub(crate) prover_type: Option<ProverType>,
    pub(crate) execution_mode: Option<ExecutionMode>,
    pub(crate) aggregate_requested: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProposalTask {
    pub(crate) proposal_id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) checkpoint: Option<ShastaCheckpoint>,
    pub(crate) l1_inclusion_block_number: u64,
    pub(crate) l2_block_numbers: Vec<u64>,
    pub(crate) last_anchor_block_number: u64,
    pub(crate) task_id: String,
    pub(crate) request: ProposalTaskRequest,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuntimeMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) active_stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_event: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) stage_timings: BTreeMap<String, StageTimingMetadata>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) proposals: BTreeMap<String, TaskRuntimeMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) aggregate: Option<TaskRuntimeMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) proof_artifact: Option<ProofArtifactDescriptor>,
}

impl RuntimeMetadata {
    pub(crate) fn current() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StageTimingMetadata {
    pub(crate) stage: String,
    pub(crate) started_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) finished_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) terminal_status: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskRuntimeMetadata {
    pub(crate) updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) provider_request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) remote_tx_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) image_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) deployment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) offchain: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) lock_expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) submitted_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) quoted_mcycles_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) evaluated_mcycles_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) max_price_multiplier: Option<u32>,
    /// Exact escalated max price bid, in wei, as a decimal string. The floored
    /// `max_price_multiplier` renders the common ×1.5 rung as `1`, so this carries the precise bid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) max_price_wei: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) rebid_attempt: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) sp1_network_mode: Option<Sp1NetworkMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) sp1_fulfillment_strategy: Option<Sp1FulfillmentStrategy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) sp1_skip_simulation: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) sp1_cycle_limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) sp1_timeout_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) sp1_max_price_per_pgu: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) sp1_auction_timeout_secs: Option<u64>,
}

#[derive(Debug)]
pub(crate) struct RemoteSubmissionConflict(String);

impl RemoteSubmissionConflict {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for RemoteSubmissionConflict {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "remote submission checkpoint conflict: {}",
            self.0
        )
    }
}

impl std::error::Error for RemoteSubmissionConflict {}

impl TaskMetadata {
    pub(crate) fn decode_for_record(record: &RuntimeTaskRecord) -> Result<Self> {
        let metadata: Self = serde_json::from_value(record.metadata.clone())
            .context("failed to parse runtime task metadata")?;
        anyhow::ensure!(
            record.pipeline_key.supports_route(record.route),
            "runtime task route does not match the canonical pipeline"
        );
        anyhow::ensure!(
            metadata.network_pair == format!("{}/{}", metadata.network, metadata.l1_network),
            "runtime task metadata network fields are inconsistent"
        );
        anyhow::ensure!(
            metadata.network_pair == record.network_pair,
            "runtime task metadata network_pair does not match the canonical record"
        );
        anyhow::ensure!(
            metadata.proof_type == record.pipeline_key.proof_type(),
            "runtime task metadata proof_type does not match the canonical pipeline"
        );
        metadata.validate_execution_identity(record)?;
        metadata.validate_runtime_progress(record.route)?;
        anyhow::ensure!(
            publication_proof_artifact_refs(&metadata, record.pipeline_key) == record.artifact_refs,
            "runtime task metadata artifact references do not match the canonical record"
        );
        Ok(metadata)
    }

    fn validate_execution_identity(&self, record: &RuntimeTaskRecord) -> Result<()> {
        anyhow::ensure!(
            !self.proposals.is_empty() || self.aggregate_request.is_some(),
            "runtime task metadata has no execution request"
        );
        anyhow::ensure!(
            self.aggregate_requested == self.aggregate_request.is_some(),
            "runtime task aggregate flag does not match its request"
        );

        for proposal in &self.proposals {
            let request = &proposal.request;
            anyhow::ensure!(
                proposal.proposal_id == request.proposal_id
                    && proposal.l1_inclusion_block_number == request.l1_inclusion_block_number
                    && proposal.last_anchor_block_number == request.last_anchor_block_number
                    && proposal.checkpoint == request.checkpoint,
                "runtime proposal projection does not match its canonical request"
            );
            anyhow::ensure!(
                request.l2_block_range
                    == Some(canonical_l2_block_range(&proposal.l2_block_numbers)?),
                "runtime proposal block projection does not match its canonical request"
            );
            anyhow::ensure!(
                proposal.task_id == proposal_task_ref(record.pipeline_key, request),
                "runtime proposal task_id does not match its canonical request"
            );
        }

        match (&self.aggregate_request, &self.aggregate_task_id) {
            (Some(request), Some(task_id)) => {
                anyhow::ensure!(
                    *task_id == aggregate_task_ref(record.pipeline_key, request),
                    "runtime aggregate task_id does not match its canonical request"
                );
                if self.proposals.is_empty() {
                    anyhow::ensure!(
                        !self.aggregate_input_artifacts.is_empty(),
                        "external aggregate metadata contains no input artifacts"
                    );
                    anyhow::ensure!(
                        request.proposal_ids.is_empty()
                            || request.proposal_ids.len() == self.aggregate_input_artifacts.len(),
                        "external aggregate proposal ids do not match input artifacts"
                    );
                } else {
                    let persisted_ids = self
                        .proposals
                        .iter()
                        .map(|proposal| proposal.proposal_id)
                        .collect::<Vec<_>>();
                    anyhow::ensure!(
                        request.proposal_ids == persisted_ids,
                        "runtime aggregate proposal ids do not match its proposal requests"
                    );
                    anyhow::ensure!(
                        self.aggregate_input_artifacts.is_empty(),
                        "batch aggregate metadata contains external input artifacts"
                    );
                }
            }
            (None, None) => {
                anyhow::ensure!(
                    self.aggregate_input_artifacts.is_empty(),
                    "proposal metadata contains external aggregate input artifacts"
                );
            }
            _ => anyhow::bail!("runtime aggregate task identity is incomplete"),
        }

        Ok(())
    }

    pub(crate) fn validate_runtime_progress(&self, route: PipelineRoute) -> Result<()> {
        for (task_ref, runtime) in &self.runtime.proposals {
            anyhow::ensure!(
                self.proposals
                    .iter()
                    .any(|proposal| proposal.task_id == task_ref.as_str()),
                "runtime proposal progress does not belong to a canonical proposal"
            );
            validate_remote_submission_route(runtime.validate_remote_submission()?, route)?;
        }
        if let Some(runtime) = &self.runtime.aggregate {
            anyhow::ensure!(
                self.aggregate_request.is_some(),
                "runtime aggregate progress has no canonical aggregate request"
            );
            validate_remote_submission_route(runtime.validate_remote_submission()?, route)?;
        }
        Ok(())
    }

    pub(crate) fn prover_type_str(&self) -> Option<String> {
        self.prover_type.map(|kind| kind.as_str().to_string())
    }

    pub(crate) fn execution_mode_str(&self) -> Option<String> {
        self.execution_mode.map(|mode| mode.as_str().to_string())
    }

    pub(crate) fn has_runtime_progress(&self) -> bool {
        self.runtime.active_stage.is_some()
            || self.runtime.last_event.is_some()
            || !self.runtime.stage_timings.is_empty()
            || !self.runtime.proposals.is_empty()
            || self.runtime.aggregate.is_some()
    }

    pub(crate) fn has_remote_submission_progress(&self) -> bool {
        self.runtime
            .proposals
            .values()
            .any(TaskRuntimeMetadata::has_remote_submission_progress)
            || self
                .runtime
                .aggregate
                .as_ref()
                .is_some_and(TaskRuntimeMetadata::has_remote_submission_progress)
    }

    pub(crate) fn proposal_runtime(&self, task_id: &str) -> Option<&TaskRuntimeMetadata> {
        self.runtime.proposals.get(task_id)
    }

    pub(crate) const fn aggregate_runtime(&self) -> Option<&TaskRuntimeMetadata> {
        self.runtime.aggregate.as_ref()
    }

    pub(crate) fn aggregate_engine_task_id(
        &self,
        pipeline_key: PipelineKey,
    ) -> Option<EngineTaskId> {
        self.aggregate_request.clone().map(|request| {
            EngineTaskId::new(EngineTaskKey::Aggregate {
                pipeline: pipeline_key,
                request,
            })
        })
    }

    pub(crate) fn owns_engine_task(&self, task_id: &EngineTaskId) -> bool {
        match &task_id.0 {
            EngineTaskKey::Proposal { pipeline, .. } => self
                .proposals
                .iter()
                .any(|proposal| proposal.engine_task_id(*pipeline) == *task_id),
            EngineTaskKey::Aggregate { pipeline, .. } => self
                .aggregate_engine_task_id(*pipeline)
                .as_ref()
                .is_some_and(|aggregate| aggregate == task_id),
        }
    }

    pub(crate) fn upsert_proposal_runtime(
        &mut self,
        task_id: &str,
        progress: &BoundlessSubmissionProgress,
        updated_at: i64,
    ) -> Result<bool> {
        self.runtime
            .proposals
            .entry(task_id.to_string())
            .or_default()
            .merge_boundless_submission(progress, updated_at)
    }

    pub(crate) fn upsert_aggregate_runtime(
        &mut self,
        progress: &BoundlessSubmissionProgress,
        updated_at: i64,
    ) -> Result<bool> {
        self.runtime
            .aggregate
            .get_or_insert_with(TaskRuntimeMetadata::default)
            .merge_boundless_submission(progress, updated_at)
    }

    pub(crate) fn upsert_proposal_sp1_network_runtime(
        &mut self,
        task_id: &str,
        progress: &Sp1NetworkSubmissionProgress,
        updated_at: i64,
    ) -> Result<bool> {
        self.runtime
            .proposals
            .entry(task_id.to_string())
            .or_default()
            .merge_sp1_network_submission(progress, updated_at)
    }

    pub(crate) fn upsert_aggregate_sp1_network_runtime(
        &mut self,
        progress: &Sp1NetworkSubmissionProgress,
        updated_at: i64,
    ) -> Result<bool> {
        self.runtime
            .aggregate
            .get_or_insert_with(TaskRuntimeMetadata::default)
            .merge_sp1_network_submission(progress, updated_at)
    }

    pub(crate) fn mark_stage_started(&mut self, task_id: &str, stage: &str, started_at_ms: i64) {
        self.runtime.stage_timings.insert(
            task_id.to_string(),
            StageTimingMetadata {
                stage: stage.to_string(),
                started_at_ms,
                finished_at_ms: None,
                terminal_status: None,
            },
        );
    }

    pub(crate) fn observe_stage_terminal_duration_secs(
        &self,
        task_id: &str,
        finished_at_ms: i64,
    ) -> Option<f64> {
        let timing = self.runtime.stage_timings.get(task_id)?;
        Some(timing.duration_secs(finished_at_ms))
    }

    pub(crate) fn mark_stage_terminal(
        &mut self,
        task_id: &str,
        stage: &str,
        finished_at_ms: i64,
        status: &str,
    ) {
        let timing = self
            .runtime
            .stage_timings
            .entry(task_id.to_string())
            .or_insert_with(|| StageTimingMetadata {
                stage: stage.to_string(),
                started_at_ms: finished_at_ms,
                finished_at_ms: None,
                terminal_status: None,
            });
        timing.stage = stage.to_string();
        timing.finished_at_ms = Some(finished_at_ms);
        timing.terminal_status = Some(status.to_string());
    }
}

fn canonical_l2_block_range(block_numbers: &[u64]) -> Result<L2BlockRange> {
    let start = *block_numbers
        .first()
        .context("runtime proposal block projection is empty")?;
    let end = *block_numbers
        .last()
        .expect("checked non-empty block projection");
    anyhow::ensure!(
        block_numbers
            .windows(2)
            .all(|window| window[0].checked_add(1) == Some(window[1])),
        "runtime proposal block projection is not contiguous"
    );
    Ok(L2BlockRange { start, end })
}

impl ProposalTask {
    pub(crate) fn engine_task_id(&self, pipeline_key: PipelineKey) -> EngineTaskId {
        EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: pipeline_key,
            request: self.request.clone(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProofArtifactKind {
    Proposal,
    Aggregate,
}

pub(crate) struct ProofArtifactRefs {
    pub(crate) refs: Vec<String>,
    pub(crate) kind: ProofArtifactKind,
}

pub(crate) fn proposal_task_ref(
    pipeline_key: PipelineKey,
    request: &ProposalTaskRequest,
) -> String {
    stable_task_ref("proposal", pipeline_key, request)
}

pub(crate) fn aggregate_task_ref(
    pipeline_key: PipelineKey,
    request: &AggregationTaskRequest,
) -> String {
    stable_task_ref("aggregate", pipeline_key, request)
}

pub(crate) fn aggregate_input_proof_ref(request_fingerprint: &str, index: usize) -> String {
    let payload = serde_json::json!({
        "kind": "aggregate_input",
        "request_fingerprint": request_fingerprint,
        "index": index,
    });
    stable_ref("proof", &payload)
}

pub(crate) fn proposal_proof_artifact_refs(
    pipeline_key: PipelineKey,
    proposal: &ProposalTask,
) -> Vec<String> {
    vec![proposal_task_ref(pipeline_key, &proposal.request)]
}

pub(crate) fn root_proof_artifact_refs(
    metadata: &TaskMetadata,
    pipeline_key: PipelineKey,
) -> Option<ProofArtifactRefs> {
    if let Some(request) = metadata.aggregate_request.as_ref() {
        return Some(ProofArtifactRefs {
            refs: vec![aggregate_task_ref(pipeline_key, request)],
            kind: ProofArtifactKind::Aggregate,
        });
    }
    match metadata.proposals.as_slice() {
        [proposal] => Some(ProofArtifactRefs {
            refs: proposal_proof_artifact_refs(pipeline_key, proposal),
            kind: ProofArtifactKind::Proposal,
        }),
        _ => None,
    }
}

pub(crate) fn publication_proof_artifact_refs(
    metadata: &TaskMetadata,
    pipeline_key: PipelineKey,
) -> Vec<String> {
    let mut refs = root_proof_artifact_refs(metadata, pipeline_key)
        .map(|root| root.refs)
        .unwrap_or_default();
    for proposal in &metadata.proposals {
        for proof_ref in proposal_proof_artifact_refs(pipeline_key, proposal) {
            if !refs.contains(&proof_ref) {
                refs.push(proof_ref);
            }
        }
    }
    for artifact in &metadata.aggregate_input_artifacts {
        if !refs.contains(&artifact.proof_ref) {
            refs.push(artifact.proof_ref.clone());
        }
    }
    refs
}

pub(crate) fn stage_task_ref(task_id: &EngineTaskId) -> String {
    match &task_id.0 {
        EngineTaskKey::Proposal { pipeline, request } => {
            stable_stage_ref(*pipeline, request, ProposalStage::Prove)
        }
        EngineTaskKey::Aggregate { pipeline, request } => aggregate_task_ref(*pipeline, request),
    }
}

pub(crate) fn stage_task_ref_for_stage(task_id: &EngineTaskId, stage: ProposalStage) -> String {
    match &task_id.0 {
        EngineTaskKey::Proposal { pipeline, request } => {
            stable_stage_ref(*pipeline, request, stage)
        }
        EngineTaskKey::Aggregate { pipeline, request } => aggregate_task_ref(*pipeline, request),
    }
}

fn stable_task_ref<T>(kind: &str, pipeline_key: PipelineKey, request: &T) -> String
where
    T: Serialize,
{
    let payload = serde_json::json!({
        "kind": kind,
        "pipeline_key": pipeline_key.as_str(),
        "request": request,
    });
    stable_ref("task", &payload)
}

fn stable_stage_ref(
    pipeline_key: PipelineKey,
    request: &ProposalTaskRequest,
    stage: ProposalStage,
) -> String {
    let payload = serde_json::json!({
        "kind": "proposal_stage",
        "pipeline_key": pipeline_key.as_str(),
        "request": request,
        "stage": stage_name(stage),
    });
    stable_ref("stage", &payload)
}

fn stable_ref(prefix: &str, payload: &serde_json::Value) -> String {
    let encoded =
        serde_json::to_vec(payload).expect("internal task reference serialization should not fail");
    format!(
        "{prefix}_{}",
        hex::encode_prefixed(keccak256(encoded).as_slice())
    )
}

const fn stage_name(stage: ProposalStage) -> &'static str {
    match stage {
        ProposalStage::Preflight => "preflight",
        ProposalStage::Validation => "validation",
        ProposalStage::Encode => "encode",
        ProposalStage::Prove => "prove",
    }
}

impl TaskRuntimeMetadata {
    pub(crate) const fn has_remote_submission_progress(&self) -> bool {
        self.provider_request_id.is_some()
            || self.remote_tx_hash.is_some()
            || self.image_ref.is_some()
            || self.deployment.is_some()
            || self.offchain.is_some()
            || self.expires_at.is_some()
            || self.lock_expires_at.is_some()
            || self.submitted_at.is_some()
            || self.quoted_mcycles_count.is_some()
            || self.evaluated_mcycles_count.is_some()
            || self.max_price_multiplier.is_some()
            || self.max_price_wei.is_some()
            || self.rebid_attempt.is_some()
            || self.sp1_network_mode.is_some()
            || self.sp1_fulfillment_strategy.is_some()
            || self.sp1_skip_simulation.is_some()
            || self.sp1_cycle_limit.is_some()
            || self.sp1_timeout_secs.is_some()
            || self.sp1_max_price_per_pgu.is_some()
            || self.sp1_auction_timeout_secs.is_some()
    }

    pub(crate) const fn has_boundless_submission_resume(&self) -> bool {
        self.provider_request_id.is_some()
            && self.image_ref.is_some()
            && self.deployment.is_some()
            && self.offchain.is_some()
            && self.expires_at.is_some()
            && self.lock_expires_at.is_some()
            && self.submitted_at.is_some()
            && self.quoted_mcycles_count.is_some()
            && self.evaluated_mcycles_count.is_some()
            && self.max_price_multiplier.is_some()
            && self.max_price_wei.is_some()
            && matches!(self.rebid_attempt, Some(attempt) if attempt > 0)
    }

    pub(crate) const fn has_sp1_network_submission_progress(&self) -> bool {
        self.provider_request_id.is_some()
            && self.expires_at.is_some()
            && self.submitted_at.is_some()
            && matches!(self.rebid_attempt, Some(attempt) if attempt > 0)
            && self.sp1_network_mode.is_some()
            && self.sp1_fulfillment_strategy.is_some()
            && self.sp1_skip_simulation.is_some()
            && self.sp1_cycle_limit.is_some()
            && self.sp1_timeout_secs.is_some()
    }

    pub(crate) const fn has_resumable_remote_submission(&self) -> bool {
        self.has_boundless_submission_resume() || self.has_sp1_network_submission_progress()
    }

    fn validate_remote_submission(&self) -> Result<Option<RemoteSubmissionKind>> {
        if !self.has_remote_submission_progress() {
            return Ok(None);
        }

        let has_boundless_fields = self.remote_tx_hash.is_some()
            || self.image_ref.is_some()
            || self.deployment.is_some()
            || self.offchain.is_some()
            || self.lock_expires_at.is_some()
            || self.quoted_mcycles_count.is_some()
            || self.evaluated_mcycles_count.is_some()
            || self.max_price_multiplier.is_some()
            || self.max_price_wei.is_some();
        let has_sp1_fields = self.sp1_network_mode.is_some()
            || self.sp1_fulfillment_strategy.is_some()
            || self.sp1_skip_simulation.is_some()
            || self.sp1_cycle_limit.is_some()
            || self.sp1_timeout_secs.is_some()
            || self.sp1_max_price_per_pgu.is_some()
            || self.sp1_auction_timeout_secs.is_some();
        anyhow::ensure!(
            has_boundless_fields ^ has_sp1_fields,
            "runtime provider checkpoint has no unique backend or mixes provider fields"
        );

        let provider_request_id = self
            .provider_request_id
            .as_deref()
            .context("runtime provider checkpoint is missing provider_request_id")?;
        anyhow::ensure!(
            !provider_request_id.trim().is_empty(),
            "runtime provider checkpoint has an empty provider_request_id"
        );
        let submitted_at = self
            .submitted_at
            .context("runtime provider checkpoint is missing submitted_at")?;
        let expires_at = self
            .expires_at
            .context("runtime provider checkpoint is missing expires_at")?;
        anyhow::ensure!(
            submitted_at > 0 && expires_at > 0 && submitted_at < expires_at,
            "runtime provider checkpoint has invalid submission deadlines"
        );
        anyhow::ensure!(
            matches!(self.rebid_attempt, Some(attempt) if attempt > 0),
            "runtime provider checkpoint has an invalid attempt"
        );

        let kind = if has_boundless_fields {
            self.validate_boundless_submission(submitted_at, expires_at)?;
            RemoteSubmissionKind::Boundless
        } else {
            self.validate_sp1_submission(submitted_at, expires_at)?;
            RemoteSubmissionKind::Sp1
        };
        Ok(Some(kind))
    }

    fn validate_boundless_submission(&self, submitted_at: u64, expires_at: u64) -> Result<()> {
        anyhow::ensure!(
            self.has_boundless_submission_resume(),
            "runtime Boundless checkpoint is incomplete"
        );
        anyhow::ensure!(
            self.image_ref
                .as_deref()
                .is_some_and(|value| !value.is_empty())
                && self
                    .deployment
                    .as_deref()
                    .is_some_and(|value| !value.is_empty()),
            "runtime Boundless checkpoint has an empty image or deployment"
        );
        let lock_expires_at = self
            .lock_expires_at
            .context("runtime Boundless checkpoint is missing lock_expires_at")?;
        anyhow::ensure!(
            submitted_at < lock_expires_at && lock_expires_at <= expires_at,
            "runtime Boundless checkpoint has invalid lock deadlines"
        );
        anyhow::ensure!(
            matches!(self.max_price_multiplier, Some(multiplier) if multiplier > 0),
            "runtime Boundless checkpoint has an invalid max_price_multiplier"
        );
        let max_price_wei = self
            .max_price_wei
            .as_deref()
            .context("runtime Boundless checkpoint is missing max_price_wei")?
            .parse::<U256>()
            .context("runtime Boundless checkpoint has invalid max_price_wei")?;
        anyhow::ensure!(
            max_price_wei != U256::ZERO,
            "runtime Boundless checkpoint has zero max_price_wei"
        );
        Ok(())
    }

    fn validate_sp1_submission(&self, submitted_at: u64, expires_at: u64) -> Result<()> {
        anyhow::ensure!(
            self.has_sp1_network_submission_progress(),
            "runtime SP1 checkpoint is incomplete"
        );
        let timeout_secs = self
            .sp1_timeout_secs
            .context("runtime SP1 checkpoint is missing timeout_secs")?;
        anyhow::ensure!(
            matches!(self.sp1_cycle_limit, Some(limit) if limit > 0)
                && timeout_secs > 0
                && self.sp1_max_price_per_pgu != Some(0)
                && self.sp1_auction_timeout_secs != Some(0),
            "runtime SP1 checkpoint has invalid provider limits"
        );
        anyhow::ensure!(
            submitted_at.checked_add(timeout_secs) == Some(expires_at),
            "runtime SP1 checkpoint deadline does not match its timeout"
        );
        Ok(())
    }

    fn merge_boundless_submission(
        &mut self,
        progress: &BoundlessSubmissionProgress,
        updated_at: i64,
    ) -> Result<bool> {
        let candidate = Self {
            updated_at,
            provider_request_id: Some(progress.provider_request_id.clone()),
            remote_tx_hash: progress.remote_tx_hash.clone(),
            image_ref: Some(progress.image_ref.clone()),
            deployment: Some(progress.deployment.clone()),
            offchain: Some(progress.offchain),
            expires_at: Some(progress.expires_at),
            lock_expires_at: Some(progress.lock_expires_at),
            submitted_at: Some(progress.submitted_at),
            quoted_mcycles_count: progress.quoted_mcycles_count,
            evaluated_mcycles_count: progress.evaluated_mcycles_count,
            max_price_multiplier: Some(progress.max_price_multiplier),
            max_price_wei: progress.max_price_wei.clone(),
            rebid_attempt: Some(progress.rebid_attempt),
            ..Self::default()
        };
        anyhow::ensure!(
            candidate.validate_remote_submission()?.is_some(),
            "Boundless progress did not produce a provider checkpoint"
        );

        let Some(current_kind) = self.validate_remote_submission()? else {
            *self = candidate;
            return Ok(true);
        };
        if !matches!(current_kind, RemoteSubmissionKind::Boundless) {
            return Err(RemoteSubmissionConflict::new(
                "Boundless progress cannot replace an SP1 checkpoint",
            )
            .into());
        }
        let current_attempt = self
            .rebid_attempt
            .context("runtime Boundless checkpoint is missing rebid_attempt")?;
        match progress.rebid_attempt.cmp(&current_attempt) {
            std::cmp::Ordering::Less => {
                return Err(RemoteSubmissionConflict::new(
                    "Boundless attempt regressed below the durable checkpoint",
                )
                .into());
            }
            std::cmp::Ordering::Greater => {
                *self = candidate;
                return Ok(true);
            }
            std::cmp::Ordering::Equal => {}
        }

        let mut current_identity = self.clone();
        let mut candidate_identity = candidate.clone();
        let current_tx_hash = current_identity.remote_tx_hash.take();
        let candidate_tx_hash = candidate_identity.remote_tx_hash.take();
        current_identity.updated_at = 0;
        candidate_identity.updated_at = 0;
        if current_identity != candidate_identity {
            return Err(RemoteSubmissionConflict::new(
                "Boundless identity changed within one attempt",
            )
            .into());
        }
        match (current_tx_hash, candidate_tx_hash) {
            (Some(current), Some(candidate)) if current != candidate => {
                Err(RemoteSubmissionConflict::new(
                    "Boundless transaction hash changed within one attempt",
                )
                .into())
            }
            (Some(_), None) => Err(RemoteSubmissionConflict::new(
                "Boundless transaction hash cannot be removed from one attempt",
            )
            .into()),
            _ => {
                *self = candidate;
                Ok(false)
            }
        }
    }

    fn merge_sp1_network_submission(
        &mut self,
        progress: &Sp1NetworkSubmissionProgress,
        updated_at: i64,
    ) -> Result<bool> {
        let expires_at = progress
            .submitted_at
            .checked_add(progress.timeout_secs)
            .context("SP1 checkpoint deadline overflow")?;
        let mut candidate = Self {
            updated_at,
            provider_request_id: Some(progress.provider_request_id.clone()),
            expires_at: Some(expires_at),
            submitted_at: Some(progress.submitted_at),
            rebid_attempt: Some(progress.attempt),
            sp1_network_mode: Some(progress.network_mode),
            sp1_fulfillment_strategy: Some(progress.fulfillment_strategy),
            sp1_skip_simulation: Some(progress.skip_simulation),
            sp1_cycle_limit: Some(progress.cycle_limit),
            sp1_timeout_secs: Some(progress.timeout_secs),
            sp1_max_price_per_pgu: progress.max_price_per_pgu,
            sp1_auction_timeout_secs: progress.auction_timeout_secs,
            ..Self::default()
        };
        anyhow::ensure!(
            candidate.validate_remote_submission()?.is_some(),
            "SP1 progress did not produce a provider checkpoint"
        );

        let Some(current_kind) = self.validate_remote_submission()? else {
            *self = candidate;
            return Ok(true);
        };
        if !matches!(current_kind, RemoteSubmissionKind::Sp1) {
            return Err(RemoteSubmissionConflict::new(
                "SP1 progress cannot replace a Boundless checkpoint",
            )
            .into());
        }
        let current_attempt = self
            .rebid_attempt
            .context("runtime SP1 checkpoint is missing rebid_attempt")?;
        match progress.attempt.cmp(&current_attempt) {
            std::cmp::Ordering::Less => {
                return Err(RemoteSubmissionConflict::new(
                    "SP1 attempt regressed below the durable checkpoint",
                )
                .into());
            }
            std::cmp::Ordering::Greater => {
                *self = candidate;
                return Ok(true);
            }
            std::cmp::Ordering::Equal => {}
        }

        let mut current_identity = self.clone();
        current_identity.updated_at = 0;
        candidate.updated_at = 0;
        if current_identity != candidate {
            return Err(
                RemoteSubmissionConflict::new("SP1 identity changed within one attempt").into(),
            );
        }
        candidate.updated_at = updated_at;
        *self = candidate;
        Ok(false)
    }
}

#[derive(Clone, Copy)]
enum RemoteSubmissionKind {
    Boundless,
    Sp1,
}

fn validate_remote_submission_route(
    kind: Option<RemoteSubmissionKind>,
    route: PipelineRoute,
) -> Result<()> {
    let matches_route = match kind {
        None => true,
        Some(RemoteSubmissionKind::Boundless) => matches!(
            route,
            PipelineRoute {
                guest_system: GuestSystem::Risc0,
                runner: RunnerKind::Network,
            }
        ),
        Some(RemoteSubmissionKind::Sp1) => matches!(
            route,
            PipelineRoute {
                guest_system: GuestSystem::Sp1,
                runner: RunnerKind::Network,
            }
        ),
    };
    anyhow::ensure!(
        matches_route,
        "runtime provider checkpoint does not match the canonical proving route"
    );
    Ok(())
}

impl StageTimingMetadata {
    fn duration_secs(&self, finished_at_ms: i64) -> f64 {
        let elapsed_ms =
            u64::try_from(finished_at_ms.saturating_sub(self.started_at_ms)).unwrap_or_default();
        std::time::Duration::from_millis(elapsed_ms).as_secs_f64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raiko2_runtime::RunnerStatus;

    fn runtime_record(metadata: &TaskMetadata, artifact_refs: Vec<String>) -> RuntimeTaskRecord {
        RuntimeTaskRecord {
            task_id: "root".to_string(),
            incarnation_id: uuid::Uuid::new_v4(),
            pipeline_key: PipelineKey::ShastaSp1,
            route: PipelineKey::ShastaSp1.route(),
            task_kind: "proposal".to_string(),
            network_pair: metadata.network_pair.clone(),
            artifact_refs,
            runner_status: RunnerStatus::Allocated,
            image_ref: None,
            proof_uri: None,
            error: None,
            metadata: serde_json::to_value(metadata).expect("serialize metadata"),
            request_fingerprint: "root-request".into(),
            updated_at: 0,
        }
    }

    fn external_aggregate_metadata() -> TaskMetadata {
        let request = AggregationTaskRequest {
            request_id: "aggregate-request".to_string(),
            proposal_ids: vec![1],
            prover_config: raiko2_engine::ProverTaskConfig::default(),
        };
        TaskMetadata {
            network_pair: "taiko_hoodi/hoodi".to_string(),
            network: "taiko_hoodi".to_string(),
            l1_network: "hoodi".to_string(),
            proof_type: ProofType::Sp1,
            requested_proof_type: None,
            prover_type: None,
            execution_mode: None,
            aggregate_requested: true,
            proposals: Vec::new(),
            aggregate_task_id: Some(aggregate_task_ref(PipelineKey::ShastaSp1, &request)),
            aggregate_request: Some(request),
            aggregate_input_artifacts: vec![AggregateInputProofArtifact {
                proof_ref: "external-input".to_string(),
            }],
            runtime: RuntimeMetadata::default(),
        }
    }

    fn complete_boundless_runtime() -> TaskRuntimeMetadata {
        TaskRuntimeMetadata {
            provider_request_id: Some("0x1".to_string()),
            image_ref: Some("0ximage".to_string()),
            deployment: Some("base".to_string()),
            offchain: Some(false),
            expires_at: Some(3),
            lock_expires_at: Some(2),
            submitted_at: Some(1),
            quoted_mcycles_count: Some(1),
            evaluated_mcycles_count: Some(1),
            max_price_multiplier: Some(1),
            max_price_wei: Some("1".to_string()),
            rebid_attempt: Some(1),
            ..TaskRuntimeMetadata::default()
        }
    }

    fn complete_sp1_runtime() -> TaskRuntimeMetadata {
        TaskRuntimeMetadata {
            provider_request_id: Some("sp1-request".to_string()),
            expires_at: Some(3_601),
            submitted_at: Some(1),
            rebid_attempt: Some(1),
            sp1_network_mode: Some(Sp1NetworkMode::Reserved),
            sp1_fulfillment_strategy: Some(Sp1FulfillmentStrategy::Reserved),
            sp1_skip_simulation: Some(false),
            sp1_cycle_limit: Some(1_000_000),
            sp1_timeout_secs: Some(3_600),
            ..TaskRuntimeMetadata::default()
        }
    }

    fn sp1_progress(provider_request_id: &str, attempt: u32) -> Sp1NetworkSubmissionProgress {
        Sp1NetworkSubmissionProgress {
            provider_request_id: provider_request_id.to_string(),
            submitted_at: 1,
            network_mode: Sp1NetworkMode::Reserved,
            fulfillment_strategy: Sp1FulfillmentStrategy::Reserved,
            skip_simulation: false,
            cycle_limit: 1_000_000,
            timeout_secs: 3_600,
            attempt,
            max_price_per_pgu: None,
            auction_timeout_secs: None,
        }
    }

    #[test]
    fn sp1_progress_merge_rejects_regression_and_same_attempt_identity_drift() {
        let mut runtime = TaskRuntimeMetadata::default();
        assert!(
            runtime
                .merge_sp1_network_submission(&sp1_progress("request-2", 2), 100)
                .expect("initial checkpoint")
        );
        assert!(
            !runtime
                .merge_sp1_network_submission(&sp1_progress("request-2", 2), 101)
                .expect("replayed checkpoint")
        );
        let checkpoint = runtime.clone();

        let stale = runtime
            .merge_sp1_network_submission(&sp1_progress("request-1", 1), 200)
            .expect_err("an older attempt is rejected");
        assert!(stale.downcast_ref::<RemoteSubmissionConflict>().is_some());
        assert_eq!(runtime, checkpoint);

        let error = runtime
            .merge_sp1_network_submission(&sp1_progress("other-request", 2), 200)
            .expect_err("one attempt cannot change provider identity");
        assert!(error.downcast_ref::<RemoteSubmissionConflict>().is_some());
        assert_eq!(runtime, checkpoint);
    }

    #[test]
    fn boundless_progress_merge_only_allows_transaction_hash_enrichment() {
        let mut runtime = TaskRuntimeMetadata::default();
        let mut progress = BoundlessSubmissionProgress {
            provider_request_id: "0x1".to_string(),
            remote_tx_hash: None,
            expires_at: 200,
            lock_expires_at: 180,
            submitted_at: 100,
            image_ref: "0ximage".to_string(),
            deployment: "base".to_string(),
            offchain: false,
            quoted_mcycles_count: Some(1),
            evaluated_mcycles_count: Some(1),
            max_price_multiplier: 1,
            max_price_wei: Some("1".to_string()),
            rebid_attempt: 1,
        };
        assert!(
            runtime
                .merge_boundless_submission(&progress, 100)
                .expect("initial checkpoint")
        );
        progress.remote_tx_hash = Some("0xtx".to_string());
        assert!(
            !runtime
                .merge_boundless_submission(&progress, 101)
                .expect("transaction hash enrichment")
        );

        progress.remote_tx_hash = None;
        let stale_hash = runtime
            .merge_boundless_submission(&progress, 102)
            .expect_err("a transaction hash cannot regress to missing");
        assert!(
            stale_hash
                .downcast_ref::<RemoteSubmissionConflict>()
                .is_some()
        );
        assert_eq!(runtime.remote_tx_hash.as_deref(), Some("0xtx"));

        progress.remote_tx_hash = Some("0xother".to_string());
        let error = runtime
            .merge_boundless_submission(&progress, 103)
            .expect_err("one attempt cannot change transaction hash");
        assert!(error.downcast_ref::<RemoteSubmissionConflict>().is_some());
        assert_eq!(runtime.remote_tx_hash.as_deref(), Some("0xtx"));
    }

    #[test]
    fn task_metadata_roundtrips_canonical_proof_type() {
        let metadata = TaskMetadata {
            network_pair: "taiko_hoodi/hoodi".to_string(),
            network: "taiko_hoodi".to_string(),
            l1_network: "hoodi".to_string(),
            proof_type: ProofType::Risc0,
            requested_proof_type: None,
            prover_type: None,
            execution_mode: None,
            aggregate_requested: false,
            proposals: Vec::new(),
            aggregate_task_id: None,
            aggregate_request: None,
            aggregate_input_artifacts: Vec::new(),
            runtime: RuntimeMetadata::default(),
        };
        let json = serde_json::to_value(&metadata).expect("serialize metadata");
        let roundtrip: TaskMetadata = serde_json::from_value(json).expect("deserialize metadata");

        assert_eq!(roundtrip.proof_type, ProofType::Risc0);
    }

    #[test]
    fn publication_refs_include_external_aggregate_inputs() {
        let metadata = external_aggregate_metadata();

        assert_eq!(
            publication_proof_artifact_refs(&metadata, PipelineKey::ShastaSp1),
            vec![
                metadata.aggregate_task_id.clone().expect("aggregate ref"),
                "external-input".to_string(),
            ]
        );
    }

    #[test]
    fn decode_for_record_rejects_identity_drift() {
        let metadata = external_aggregate_metadata();
        let artifact_refs = publication_proof_artifact_refs(&metadata, PipelineKey::ShastaSp1);
        let valid = runtime_record(&metadata, artifact_refs);
        assert!(TaskMetadata::decode_for_record(&valid).is_ok());

        let mut wrong_network = valid.clone();
        wrong_network.network_pair = "taiko_dev/ethereum".to_string();
        assert!(TaskMetadata::decode_for_record(&wrong_network).is_err());

        let mut wrong_pipeline = valid.clone();
        wrong_pipeline.pipeline_key = PipelineKey::ShastaRisc0;
        wrong_pipeline.route = wrong_pipeline.pipeline_key.route();
        assert!(TaskMetadata::decode_for_record(&wrong_pipeline).is_err());

        let mut wrong_artifacts = valid;
        wrong_artifacts.artifact_refs = vec!["different-input".to_string()];
        assert!(TaskMetadata::decode_for_record(&wrong_artifacts).is_err());
    }

    #[test]
    fn decode_for_record_rejects_noncanonical_metadata_shape() {
        let metadata = external_aggregate_metadata();
        let artifact_refs = publication_proof_artifact_refs(&metadata, PipelineKey::ShastaSp1);
        let valid = runtime_record(&metadata, artifact_refs);

        let mut missing_request = valid.clone();
        missing_request.metadata["aggregate_request"] = serde_json::Value::Null;
        assert!(TaskMetadata::decode_for_record(&missing_request).is_err());

        let mut wrong_task_id = valid.clone();
        wrong_task_id.metadata["aggregate_task_id"] = serde_json::json!("wrong");
        assert!(TaskMetadata::decode_for_record(&wrong_task_id).is_err());

        let mut mismatched_inputs = metadata;
        let request = mismatched_inputs
            .aggregate_request
            .as_mut()
            .expect("aggregate request");
        request.proposal_ids.push(2);
        mismatched_inputs.aggregate_task_id = Some(aggregate_task_ref(
            PipelineKey::ShastaSp1,
            mismatched_inputs
                .aggregate_request
                .as_ref()
                .expect("aggregate request"),
        ));
        let mismatch_refs =
            publication_proof_artifact_refs(&mismatched_inputs, PipelineKey::ShastaSp1);
        let mismatched_inputs = runtime_record(&mismatched_inputs, mismatch_refs);
        assert!(TaskMetadata::decode_for_record(&mismatched_inputs).is_err());

        let mut unknown_legacy_index = valid;
        unknown_legacy_index.metadata["proof_ids"] = serde_json::json!(["legacy"]);
        assert!(TaskMetadata::decode_for_record(&unknown_legacy_index).is_err());
    }

    #[test]
    fn decode_for_record_rejects_partial_provider_checkpoint() {
        let mut metadata = external_aggregate_metadata();
        metadata.runtime.aggregate = Some(TaskRuntimeMetadata {
            provider_request_id: Some("0xpartial".to_string()),
            submitted_at: Some(1),
            expires_at: Some(2),
            rebid_attempt: Some(1),
            ..TaskRuntimeMetadata::default()
        });
        let artifact_refs = publication_proof_artifact_refs(&metadata, PipelineKey::ShastaSp1);
        let record = runtime_record(&metadata, artifact_refs);

        let error = TaskMetadata::decode_for_record(&record)
            .expect_err("partial provider checkpoint must fail closed");
        assert!(error.to_string().contains("no unique backend"));
    }

    #[test]
    fn decode_for_record_accepts_sp1_checkpoint_only_on_the_network_route() {
        let mut metadata = external_aggregate_metadata();
        metadata.runtime.aggregate = Some(complete_sp1_runtime());
        let artifact_refs = publication_proof_artifact_refs(&metadata, PipelineKey::ShastaSp1);
        let mut network = runtime_record(&metadata, artifact_refs);
        network.route = "sp1/network".parse().expect("parse SP1 network route");
        assert!(TaskMetadata::decode_for_record(&network).is_ok());

        let mut local = network;
        local.route = PipelineKey::ShastaSp1.route();
        let error = TaskMetadata::decode_for_record(&local)
            .expect_err("SP1 provider checkpoint must not load on a local route");
        assert!(error.to_string().contains("canonical proving route"));
    }

    #[test]
    fn decode_for_record_accepts_boundless_checkpoint_only_on_the_network_pipeline() {
        let mut metadata = external_aggregate_metadata();
        metadata.proof_type = ProofType::Risc0;
        let request = metadata
            .aggregate_request
            .clone()
            .expect("aggregate request");
        metadata.aggregate_task_id = Some(aggregate_task_ref(
            PipelineKey::ShastaRisc0Network,
            &request,
        ));
        metadata.runtime.aggregate = Some(complete_boundless_runtime());
        let artifact_refs =
            publication_proof_artifact_refs(&metadata, PipelineKey::ShastaRisc0Network);
        let mut network = runtime_record(&metadata, artifact_refs);
        network.pipeline_key = PipelineKey::ShastaRisc0Network;
        network.route = PipelineKey::ShastaRisc0Network.route();
        assert!(TaskMetadata::decode_for_record(&network).is_ok());

        metadata.aggregate_task_id = Some(aggregate_task_ref(PipelineKey::ShastaRisc0, &request));
        let artifact_refs = publication_proof_artifact_refs(&metadata, PipelineKey::ShastaRisc0);
        let mut local = runtime_record(&metadata, artifact_refs);
        local.pipeline_key = PipelineKey::ShastaRisc0;
        local.route = PipelineKey::ShastaRisc0.route();
        let error = TaskMetadata::decode_for_record(&local)
            .expect_err("Boundless provider checkpoint must not load on a local pipeline");
        assert!(error.to_string().contains("canonical proving route"));
    }

    #[test]
    fn decode_for_record_rejects_mixed_provider_checkpoint_fields() {
        let mut metadata = external_aggregate_metadata();
        let mut runtime = complete_sp1_runtime();
        runtime.remote_tx_hash = Some("0xdeadbeef".to_string());
        metadata.runtime.aggregate = Some(runtime);
        let artifact_refs = publication_proof_artifact_refs(&metadata, PipelineKey::ShastaSp1);
        let mut record = runtime_record(&metadata, artifact_refs);
        record.route = "sp1/network".parse().expect("parse SP1 network route");

        let error = TaskMetadata::decode_for_record(&record)
            .expect_err("mixed provider checkpoint fields must fail closed");
        assert!(error.to_string().contains("mixes provider fields"));
    }

    #[test]
    fn task_metadata_tracks_stage_timing_durations() {
        let mut metadata = TaskMetadata {
            network_pair: "taiko_dev/ethereum".to_string(),
            network: "taiko_dev".to_string(),
            l1_network: "ethereum".to_string(),
            proof_type: ProofType::Native,
            requested_proof_type: None,
            prover_type: None,
            execution_mode: None,
            aggregate_requested: false,
            proposals: Vec::new(),
            aggregate_task_id: None,
            aggregate_request: None,
            aggregate_input_artifacts: Vec::new(),
            runtime: RuntimeMetadata::default(),
        };

        metadata.mark_stage_started("task-proof", "prove", 1_000);
        assert_eq!(
            metadata.observe_stage_terminal_duration_secs("task-proof", 4_250),
            Some(3.25)
        );

        metadata.mark_stage_terminal("task-proof", "prove", 4_250, "completed");
        let timing = metadata
            .runtime
            .stage_timings
            .get("task-proof")
            .expect("stage timing");
        assert_eq!(timing.stage, "prove");
        assert_eq!(timing.finished_at_ms, Some(4_250));
        assert_eq!(timing.terminal_status.as_deref(), Some("completed"));
    }
}
