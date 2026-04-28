use alloy_primitives::{hex, keccak256};
use raiko2_engine::{
    AggregationTaskRequest, EngineTaskId, EngineTaskKey, ProposalStage, ProposalTaskRequest,
};
use raiko2_pipeline::PipelineKey;
use raiko2_primitives::{ProofType, ShastaCheckpoint, proof_type::lowercase};
use raiko2_prover::{
    BoundlessSubmissionProgress, Sp1FulfillmentStrategy, Sp1NetworkMode,
    Sp1NetworkSubmissionProgress, sp1::ExecutionMode,
};
use raiko2_queue::decode_task_id;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TaskMetadata {
    pub(crate) network_pair: String,
    pub(crate) network: String,
    pub(crate) l1_network: String,
    #[serde(with = "lowercase")]
    pub(crate) proof_type: ProofType,
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
pub(crate) struct AggregateInputProofArtifact {
    pub(crate) proof_ref: String,
    pub(crate) proof_path: String,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct BuildTaskMetadataParams<'a> {
    pub(crate) network: &'a str,
    pub(crate) l1_network: &'a str,
    pub(crate) proof_type: ProofType,
    pub(crate) execution_mode: Option<ExecutionMode>,
    pub(crate) aggregate_requested: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProposalTask {
    pub(crate) proposal_id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) checkpoint: Option<ShastaCheckpoint>,
    pub(crate) l1_inclusion_block_number: u64,
    pub(crate) l2_block_numbers: Vec<u64>,
    pub(crate) last_anchor_block_number: u64,
    pub(crate) task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) request: Option<ProposalTaskRequest>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StageTimingMetadata {
    pub(crate) stage: String,
    pub(crate) started_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) finished_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) terminal_status: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    pub(crate) quoted_mcycles_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) evaluated_mcycles_count: Option<u32>,
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
}

impl TaskMetadata {
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
        self.aggregate_request
            .clone()
            .map(|request| {
                EngineTaskId::new(EngineTaskKey::Aggregate {
                    pipeline: pipeline_key,
                    request,
                })
            })
            .or_else(|| {
                self.aggregate_task_id
                    .as_deref()
                    .and_then(decode_legacy_aggregate_task_id)
            })
    }

    pub(crate) fn upsert_proposal_runtime(
        &mut self,
        task_id: &str,
        progress: &BoundlessSubmissionProgress,
        updated_at: i64,
    ) {
        self.runtime
            .proposals
            .entry(task_id.to_string())
            .or_default()
            .apply_boundless_submission(progress, updated_at);
    }

    pub(crate) fn upsert_aggregate_runtime(
        &mut self,
        progress: &BoundlessSubmissionProgress,
        updated_at: i64,
    ) {
        self.runtime
            .aggregate
            .get_or_insert_with(TaskRuntimeMetadata::default)
            .apply_boundless_submission(progress, updated_at);
    }

    pub(crate) fn upsert_proposal_sp1_network_runtime(
        &mut self,
        task_id: &str,
        progress: &Sp1NetworkSubmissionProgress,
        updated_at: i64,
    ) {
        self.runtime
            .proposals
            .entry(task_id.to_string())
            .or_default()
            .apply_sp1_network_submission(progress, updated_at);
    }

    pub(crate) fn upsert_aggregate_sp1_network_runtime(
        &mut self,
        progress: &Sp1NetworkSubmissionProgress,
        updated_at: i64,
    ) {
        self.runtime
            .aggregate
            .get_or_insert_with(TaskRuntimeMetadata::default)
            .apply_sp1_network_submission(progress, updated_at);
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

impl ProposalTask {
    pub(crate) fn engine_task_id(&self, pipeline_key: PipelineKey) -> Option<EngineTaskId> {
        self.request
            .clone()
            .map(|request| {
                EngineTaskId::new(EngineTaskKey::Proposal {
                    pipeline: pipeline_key,
                    request,
                })
            })
            .or_else(|| decode_legacy_proposal_task_id(&self.task_id))
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
    let mut refs = proposal
        .request
        .as_ref()
        .map(|request| vec![proposal_task_ref(pipeline_key, request)])
        .unwrap_or_default();
    if !refs.contains(&proposal.task_id) {
        refs.push(proposal.task_id.clone());
    }
    refs
}

pub(crate) fn root_proof_artifact_refs(
    metadata: &TaskMetadata,
    pipeline_key: PipelineKey,
) -> Option<ProofArtifactRefs> {
    if let Some(request) = metadata.aggregate_request.as_ref() {
        let mut refs = vec![aggregate_task_ref(pipeline_key, request)];
        if let Some(legacy_ref) = metadata.aggregate_task_id.as_ref()
            && !refs.contains(legacy_ref)
        {
            refs.push(legacy_ref.clone());
        }
        return Some(ProofArtifactRefs {
            refs,
            kind: ProofArtifactKind::Aggregate,
        });
    }
    if let Some(legacy_ref) = metadata.aggregate_task_id.as_ref() {
        return Some(ProofArtifactRefs {
            refs: vec![legacy_ref.clone()],
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

fn decode_legacy_proposal_task_id(raw: &str) -> Option<EngineTaskId> {
    if let Ok(task_id) = decode_task_id::<EngineTaskKey>(raw) {
        return match task_id.0 {
            EngineTaskKey::Proposal { pipeline, request } => {
                Some(EngineTaskId::new(EngineTaskKey::Proposal {
                    pipeline,
                    request,
                }))
            }
            EngineTaskKey::Aggregate { .. } => None,
        };
    }

    match decode_task_id::<LegacyEngineTaskKey>(raw).ok()?.0 {
        LegacyEngineTaskKey::Proposal {
            pipeline, request, ..
        } => Some(EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request,
        })),
        LegacyEngineTaskKey::Aggregate { .. } => None,
    }
}

#[derive(Deserialize)]
#[allow(dead_code)]
enum LegacyEngineTaskKey {
    Proposal {
        pipeline: PipelineKey,
        request: ProposalTaskRequest,
        stage: ProposalStage,
    },
    Aggregate {
        pipeline: PipelineKey,
        request: AggregationTaskRequest,
    },
}

fn decode_legacy_aggregate_task_id(raw: &str) -> Option<EngineTaskId> {
    let task_id = decode_task_id::<EngineTaskKey>(raw).ok()?;
    matches!(task_id.0, EngineTaskKey::Aggregate { .. }).then_some(task_id)
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
            || self.expires_at.is_some()
            || self.sp1_network_mode.is_some()
            || self.sp1_fulfillment_strategy.is_some()
    }

    const fn has_boundless_submission_resume(&self) -> bool {
        self.provider_request_id.is_some() && self.expires_at.is_some()
    }

    const fn has_sp1_network_submission_progress(&self) -> bool {
        self.provider_request_id.is_some()
            && (self.sp1_network_mode.is_some()
                || self.sp1_fulfillment_strategy.is_some()
                || self.sp1_timeout_secs.is_some())
    }

    pub(crate) const fn has_resumable_remote_submission(&self) -> bool {
        self.has_boundless_submission_resume() || self.has_sp1_network_submission_progress()
    }

    fn apply_boundless_submission(
        &mut self,
        progress: &BoundlessSubmissionProgress,
        updated_at: i64,
    ) {
        self.updated_at = updated_at;
        self.provider_request_id = Some(progress.provider_request_id.clone());
        self.remote_tx_hash.clone_from(&progress.remote_tx_hash);
        self.image_ref = Some(progress.image_ref.clone());
        self.deployment = Some(progress.deployment.clone());
        self.offchain = Some(progress.offchain);
        self.expires_at = Some(progress.expires_at);
        self.quoted_mcycles_count = progress.quoted_mcycles_count;
        self.evaluated_mcycles_count = progress.evaluated_mcycles_count;
    }

    fn apply_sp1_network_submission(
        &mut self,
        progress: &Sp1NetworkSubmissionProgress,
        updated_at: i64,
    ) {
        self.updated_at = updated_at;
        self.provider_request_id = Some(progress.provider_request_id.clone());
        self.sp1_network_mode = Some(progress.network_mode);
        self.sp1_fulfillment_strategy = Some(progress.fulfillment_strategy);
        self.sp1_skip_simulation = Some(progress.skip_simulation);
        self.sp1_cycle_limit = Some(progress.cycle_limit);
        self.sp1_timeout_secs = Some(progress.timeout_secs);
    }
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

    #[test]
    fn task_metadata_roundtrips_canonical_proof_type() {
        let metadata = TaskMetadata {
            network_pair: "taiko_hoodi/hoodi".to_string(),
            network: "taiko_hoodi".to_string(),
            l1_network: "hoodi".to_string(),
            proof_type: ProofType::Risc0,
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
    fn task_metadata_tracks_stage_timing_durations() {
        let mut metadata = TaskMetadata {
            network_pair: "taiko_dev/ethereum".to_string(),
            network: "taiko_dev".to_string(),
            l1_network: "ethereum".to_string(),
            proof_type: ProofType::Native,
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
