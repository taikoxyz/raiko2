use alloy_primitives::{hex, keccak256};
use raiko2_engine::{
    AggregationTaskRequest, EngineTaskId, EngineTaskKey, ProposalStage, ProposalTaskRequest,
};
use raiko2_pipeline::PipelineKey;
use raiko2_primitives::{ProofType, ShastaCheckpoint, proof_type::lowercase};
use raiko2_prover::{
    BOUNDLESS_SUBMISSION_SNAPSHOT_VERSION, BoundlessSubmissionProgress,
    BoundlessSubmissionSnapshot, Sp1FulfillmentStrategy, Sp1NetworkMode,
    Sp1NetworkSubmissionProgress, sp1_config::ExecutionMode,
};
use raiko2_queue::decode_task_id;
use serde::{Deserialize, Deserializer, Serialize};
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
pub(crate) struct AggregateInputProofArtifact {
    pub(crate) proof_ref: String,
    pub(crate) proof_path: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "provider", content = "submission", rename_all = "snake_case")]
pub(crate) enum RemoteSubmissionMetadata {
    Boundless(BoundlessRemoteSubmissionMetadata),
    Sp1(Sp1RemoteSubmissionMetadata),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BoundlessRemoteSubmissionMetadata {
    #[serde(flatten)]
    snapshot: BoundlessSubmissionSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    image_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    deployment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    offchain: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    quoted_mcycles_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    evaluated_mcycles_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Sp1RemoteSubmissionMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider_request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    network_mode: Option<Sp1NetworkMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fulfillment_strategy: Option<Sp1FulfillmentStrategy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    skip_simulation: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cycle_limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_price_per_pgu: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auction_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RemoteSubmissionView {
    pub(crate) provider_request_id: Option<String>,
    pub(crate) remote_tx_hash: Option<String>,
    pub(crate) image_ref: Option<String>,
    pub(crate) deployment: Option<String>,
    pub(crate) offchain: Option<bool>,
    pub(crate) expires_at: Option<u64>,
    pub(crate) submitted_at: Option<u64>,
    pub(crate) quoted_mcycles_count: Option<u32>,
    pub(crate) evaluated_mcycles_count: Option<u32>,
    pub(crate) max_price_multiplier: Option<u32>,
    pub(crate) max_price_wei: Option<String>,
    pub(crate) sp1_network_mode: Option<String>,
    pub(crate) sp1_fulfillment_strategy: Option<String>,
    pub(crate) sp1_skip_simulation: Option<bool>,
    pub(crate) sp1_cycle_limit: Option<u64>,
    pub(crate) sp1_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct TaskRuntimeMetadata {
    pub(crate) updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) remote_submission: Option<RemoteSubmissionMetadata>,
}

#[derive(Debug, Default, Deserialize)]
struct TaskRuntimeMetadataWire {
    #[serde(default)]
    updated_at: i64,
    #[serde(default)]
    remote_submission: Option<RemoteSubmissionMetadata>,
    #[serde(flatten)]
    legacy: LegacyRemoteSubmissionMetadata,
}

#[derive(Debug, Default, Deserialize)]
struct LegacyRemoteSubmissionMetadata {
    #[serde(default)]
    provider_request_id: Option<String>,
    #[serde(default)]
    remote_tx_hash: Option<String>,
    #[serde(default)]
    image_ref: Option<String>,
    #[serde(default)]
    deployment: Option<String>,
    #[serde(default)]
    offchain: Option<bool>,
    #[serde(default)]
    expires_at: Option<u64>,
    #[serde(default)]
    lock_expires_at: Option<u64>,
    #[serde(default)]
    submitted_at: Option<u64>,
    #[serde(default)]
    quoted_mcycles_count: Option<u32>,
    #[serde(default)]
    evaluated_mcycles_count: Option<u32>,
    #[serde(default)]
    max_price_multiplier: Option<u32>,
    #[serde(default)]
    max_price_wei: Option<String>,
    #[serde(default)]
    rebid_attempt: Option<u32>,
    #[serde(default)]
    sp1_network_mode: Option<Sp1NetworkMode>,
    #[serde(default)]
    sp1_fulfillment_strategy: Option<Sp1FulfillmentStrategy>,
    #[serde(default)]
    sp1_skip_simulation: Option<bool>,
    #[serde(default)]
    sp1_cycle_limit: Option<u64>,
    #[serde(default)]
    sp1_timeout_secs: Option<u64>,
    #[serde(default)]
    sp1_max_price_per_pgu: Option<u64>,
    #[serde(default)]
    sp1_auction_timeout_secs: Option<u64>,
}

impl<'de> Deserialize<'de> for TaskRuntimeMetadata {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = TaskRuntimeMetadataWire::deserialize(deserializer)?;
        Ok(Self {
            updated_at: wire.updated_at,
            remote_submission: wire
                .remote_submission
                .or_else(|| wire.legacy.into_remote_submission()),
        })
    }
}

impl LegacyRemoteSubmissionMetadata {
    fn into_remote_submission(self) -> Option<RemoteSubmissionMetadata> {
        let has_sp1_fields = self.sp1_network_mode.is_some()
            || self.sp1_fulfillment_strategy.is_some()
            || self.sp1_skip_simulation.is_some()
            || self.sp1_cycle_limit.is_some()
            || self.sp1_timeout_secs.is_some()
            || self.sp1_max_price_per_pgu.is_some()
            || self.sp1_auction_timeout_secs.is_some();
        let has_boundless_specific_fields = self.remote_tx_hash.is_some()
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
            || self.rebid_attempt.is_some();
        // A provider-id-only legacy record is ambiguous. Preserve the old SP1 loader behavior:
        // Boundless could not resume without `expires_at`, while SP1 previously reused the id
        // directly. Any provider-specific field still takes precedence over this fallback.
        if has_sp1_fields || (self.provider_request_id.is_some() && !has_boundless_specific_fields)
        {
            return Some(RemoteSubmissionMetadata::Sp1(Sp1RemoteSubmissionMetadata {
                provider_request_id: self.provider_request_id,
                network_mode: self.sp1_network_mode,
                fulfillment_strategy: self.sp1_fulfillment_strategy,
                skip_simulation: self.sp1_skip_simulation,
                cycle_limit: self.sp1_cycle_limit,
                timeout_secs: self.sp1_timeout_secs,
                max_price_per_pgu: self.sp1_max_price_per_pgu,
                auction_timeout_secs: self.sp1_auction_timeout_secs,
            }));
        }

        let has_boundless_fields =
            self.provider_request_id.is_some() || has_boundless_specific_fields;
        has_boundless_fields.then(|| {
            RemoteSubmissionMetadata::Boundless(BoundlessRemoteSubmissionMetadata {
                snapshot: BoundlessSubmissionSnapshot {
                    version: BOUNDLESS_SUBMISSION_SNAPSHOT_VERSION,
                    provider_request_id: self.provider_request_id.unwrap_or_default(),
                    remote_tx_hash: self.remote_tx_hash,
                    expires_at: self.expires_at.unwrap_or_default(),
                    lock_expires_at: self.lock_expires_at.unwrap_or_default(),
                    submitted_at: self.submitted_at.unwrap_or_default(),
                    max_price_multiplier: self.max_price_multiplier,
                    max_price_wei: self.max_price_wei,
                    rebid_attempt: self.rebid_attempt.unwrap_or_default(),
                },
                image_ref: self.image_ref,
                deployment: self.deployment,
                offchain: self.offchain,
                quoted_mcycles_count: self.quoted_mcycles_count,
                evaluated_mcycles_count: self.evaluated_mcycles_count,
            })
        })
    }
}

impl TaskMetadata {
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
        self.remote_submission.is_some()
    }

    pub(crate) fn has_boundless_submission_resume(&self) -> bool {
        self.boundless_submission().is_some_and(|snapshot| {
            !snapshot.provider_request_id.is_empty() && snapshot.expires_at != 0
        })
    }

    pub(crate) fn has_sp1_network_submission_progress(&self) -> bool {
        self.sp1_network_submission().is_some_and(|submission| {
            submission
                .provider_request_id
                .as_deref()
                .is_some_and(|request_id| !request_id.is_empty())
                && (submission.network_mode.is_some()
                    || submission.fulfillment_strategy.is_some()
                    || submission.timeout_secs.is_some())
        })
    }

    pub(crate) fn has_resumable_remote_submission(&self) -> bool {
        self.has_boundless_submission_resume() || self.has_sp1_network_submission_progress()
    }

    pub(crate) const fn boundless_submission(&self) -> Option<&BoundlessSubmissionSnapshot> {
        match self.remote_submission.as_ref() {
            Some(RemoteSubmissionMetadata::Boundless(submission)) => Some(&submission.snapshot),
            Some(RemoteSubmissionMetadata::Sp1(_)) | None => None,
        }
    }

    pub(crate) fn boundless_submission_for_resume(
        &self,
        now_secs: u64,
    ) -> Option<BoundlessSubmissionSnapshot> {
        let mut snapshot = self.boundless_submission()?.clone();
        if snapshot.provider_request_id.is_empty() || snapshot.expires_at == 0 {
            return None;
        }
        if snapshot.submitted_at == 0 {
            snapshot.submitted_at = now_secs;
        }
        Some(snapshot)
    }

    const fn sp1_network_submission(&self) -> Option<&Sp1RemoteSubmissionMetadata> {
        match self.remote_submission.as_ref() {
            Some(RemoteSubmissionMetadata::Sp1(submission)) => Some(submission),
            Some(RemoteSubmissionMetadata::Boundless(_)) | None => None,
        }
    }

    pub(crate) fn sp1_network_request_id(&self) -> Option<String> {
        self.sp1_network_submission()
            .and_then(|submission| submission.provider_request_id.clone())
            .filter(|request_id| !request_id.is_empty())
    }

    pub(crate) fn public_remote_submission(&self) -> RemoteSubmissionView {
        match self.remote_submission.as_ref() {
            Some(RemoteSubmissionMetadata::Boundless(submission)) => RemoteSubmissionView {
                provider_request_id: (!submission.snapshot.provider_request_id.is_empty())
                    .then(|| submission.snapshot.provider_request_id.clone()),
                remote_tx_hash: submission.snapshot.remote_tx_hash.clone(),
                image_ref: submission.image_ref.clone(),
                deployment: submission.deployment.clone(),
                offchain: submission.offchain,
                expires_at: (submission.snapshot.expires_at != 0)
                    .then_some(submission.snapshot.expires_at),
                submitted_at: (submission.snapshot.submitted_at != 0)
                    .then_some(submission.snapshot.submitted_at),
                quoted_mcycles_count: submission.quoted_mcycles_count,
                evaluated_mcycles_count: submission.evaluated_mcycles_count,
                max_price_multiplier: submission.snapshot.max_price_multiplier,
                max_price_wei: submission.snapshot.max_price_wei.clone(),
                ..RemoteSubmissionView::default()
            },
            Some(RemoteSubmissionMetadata::Sp1(submission)) => RemoteSubmissionView {
                provider_request_id: submission.provider_request_id.clone(),
                sp1_network_mode: submission
                    .network_mode
                    .map(|mode| mode.as_str().to_string()),
                sp1_fulfillment_strategy: submission
                    .fulfillment_strategy
                    .map(|strategy| strategy.as_str().to_string()),
                sp1_skip_simulation: submission.skip_simulation,
                sp1_cycle_limit: submission.cycle_limit,
                sp1_timeout_secs: submission.timeout_secs,
                ..RemoteSubmissionView::default()
            },
            None => RemoteSubmissionView::default(),
        }
    }

    fn apply_boundless_submission(
        &mut self,
        progress: &BoundlessSubmissionProgress,
        updated_at: i64,
    ) {
        self.updated_at = updated_at;
        self.remote_submission = Some(RemoteSubmissionMetadata::Boundless(
            BoundlessRemoteSubmissionMetadata {
                snapshot: progress.snapshot.clone(),
                image_ref: Some(progress.image_ref.clone()),
                deployment: Some(progress.deployment.clone()),
                offchain: Some(progress.offchain),
                quoted_mcycles_count: progress.quoted_mcycles_count,
                evaluated_mcycles_count: progress.evaluated_mcycles_count,
            },
        ));
    }

    fn apply_sp1_network_submission(
        &mut self,
        progress: &Sp1NetworkSubmissionProgress,
        updated_at: i64,
    ) {
        self.updated_at = updated_at;
        self.remote_submission = Some(RemoteSubmissionMetadata::Sp1(Sp1RemoteSubmissionMetadata {
            provider_request_id: Some(progress.provider_request_id.clone()),
            network_mode: Some(progress.network_mode),
            fulfillment_strategy: Some(progress.fulfillment_strategy),
            skip_simulation: Some(progress.skip_simulation),
            cycle_limit: Some(progress.cycle_limit),
            timeout_secs: Some(progress.timeout_secs),
            max_price_per_pgu: progress.max_price_per_pgu,
            auction_timeout_secs: progress.auction_timeout_secs,
        }));
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

    fn tagged_boundless_runtime_metadata(version: Option<u32>) -> TaskRuntimeMetadata {
        let mut submission = serde_json::json!({
            "provider_request_id": "0x1234",
            "remote_tx_hash": null,
            "expires_at": 123_456,
            "lock_expires_at": 123_300,
            "submitted_at": 123_000,
            "max_price_multiplier": 1,
            "max_price_wei": "9000000000000",
            "rebid_attempt": 1
        });
        if let Some(version) = version {
            submission
                .as_object_mut()
                .expect("submission object")
                .insert("version".to_string(), version.into());
        }
        serde_json::from_value(serde_json::json!({
            "updated_at": 123,
            "remote_submission": {
                "provider": "boundless",
                "submission": submission
            }
        }))
        .expect("deserialize tagged Boundless runtime metadata")
    }

    #[test]
    fn legacy_flat_runtime_metadata_loads_as_boundless_submission() {
        let metadata: TaskRuntimeMetadata = serde_json::from_value(serde_json::json!({
            "updated_at": 123,
            "provider_request_id": "0x1234",
            "remote_tx_hash": "0xabcd",
            "image_ref": "0ximage",
            "deployment": "base",
            "offchain": false,
            "expires_at": 123_456,
            "lock_expires_at": 123_300,
            "submitted_at": 123_000,
            "quoted_mcycles_count": 6_000,
            "evaluated_mcycles_count": 12_345,
            "max_price_multiplier": 4,
            "max_price_wei": "9000000000000",
            "rebid_attempt": 3
        }))
        .expect("deserialize legacy runtime metadata");

        assert!(matches!(
            metadata.remote_submission.as_ref(),
            Some(RemoteSubmissionMetadata::Boundless(_))
        ));
        let snapshot = metadata
            .boundless_submission()
            .expect("legacy Boundless snapshot");
        assert_eq!(snapshot.version, 1);
        assert_eq!(snapshot.provider_request_id, "0x1234");
        assert_eq!(snapshot.remote_tx_hash.as_deref(), Some("0xabcd"));
        assert_eq!(snapshot.expires_at, 123_456);
        assert_eq!(snapshot.lock_expires_at, 123_300);
        assert_eq!(snapshot.submitted_at, 123_000);
        assert_eq!(snapshot.max_price_multiplier, Some(4));
        assert_eq!(snapshot.max_price_wei.as_deref(), Some("9000000000000"));
        assert_eq!(snapshot.rebid_attempt, 3);
    }

    #[test]
    fn legacy_provider_id_only_runtime_metadata_remains_sp1_resumable() {
        let metadata: TaskRuntimeMetadata = serde_json::from_value(serde_json::json!({
            "updated_at": 123,
            "provider_request_id": "0xsp1"
        }))
        .expect("deserialize provider-only legacy runtime metadata");

        assert_eq!(metadata.sp1_network_request_id().as_deref(), Some("0xsp1"));
        assert_eq!(
            metadata
                .public_remote_submission()
                .provider_request_id
                .as_deref(),
            Some("0xsp1")
        );
        assert!(metadata.boundless_submission().is_none());
    }

    #[test]
    fn unknown_tagged_boundless_version_stays_on_fail_closed_resume_path() {
        let metadata = tagged_boundless_runtime_metadata(Some(2));

        assert!(metadata.has_boundless_submission_resume());
        assert!(metadata.has_resumable_remote_submission());
        let snapshot = metadata
            .boundless_submission_for_resume(999)
            .expect("unsupported version must reach prover conversion");
        assert_eq!(snapshot.version, 2);
    }

    #[test]
    fn missing_tagged_boundless_version_is_not_promoted_to_current_schema() {
        let metadata = tagged_boundless_runtime_metadata(None);

        let snapshot = metadata
            .boundless_submission()
            .expect("tagged Boundless submission remains present");
        assert_eq!(snapshot.version, 0);
        assert!(metadata.has_boundless_submission_resume());
        assert_eq!(
            metadata
                .boundless_submission_for_resume(999)
                .expect("missing version must reach prover conversion")
                .version,
            0
        );
    }

    #[test]
    fn sparse_legacy_boundless_public_view_preserves_missing_multiplier() {
        let metadata: TaskRuntimeMetadata = serde_json::from_value(serde_json::json!({
            "provider_request_id": "0x1234",
            "expires_at": 123_456
        }))
        .expect("deserialize sparse legacy Boundless runtime metadata");

        assert!(metadata.has_boundless_submission_resume());
        assert_eq!(
            metadata.public_remote_submission().max_price_multiplier,
            None
        );
    }

    #[test]
    fn new_runtime_metadata_emits_only_tagged_boundless_submission() {
        let snapshot = raiko2_prover::BoundlessSubmissionSnapshot::new(
            "0x1234".to_string(),
            Some("0xabcd".to_string()),
            123_456,
            123_300,
            123_000,
            4,
            "9000000000000".to_string(),
            3,
        );
        let progress = BoundlessSubmissionProgress {
            snapshot: snapshot.clone(),
            image_ref: "0ximage".to_string(),
            deployment: "base".to_string(),
            offchain: false,
            quoted_mcycles_count: Some(6_000),
            evaluated_mcycles_count: Some(12_345),
        };
        let mut metadata = TaskRuntimeMetadata::default();
        metadata.apply_boundless_submission(&progress, 123);

        let encoded = serde_json::to_value(&metadata).expect("serialize runtime metadata");
        assert_eq!(encoded["remote_submission"]["provider"], "boundless");
        assert_eq!(
            encoded["remote_submission"]["submission"]["provider_request_id"],
            "0x1234"
        );
        for legacy_field in [
            "provider_request_id",
            "remote_tx_hash",
            "image_ref",
            "deployment",
            "offchain",
            "expires_at",
            "lock_expires_at",
            "submitted_at",
            "quoted_mcycles_count",
            "evaluated_mcycles_count",
            "max_price_multiplier",
            "max_price_wei",
            "rebid_attempt",
        ] {
            assert!(
                encoded.get(legacy_field).is_none(),
                "legacy field {legacy_field} must not be emitted"
            );
        }

        let decoded: TaskRuntimeMetadata =
            serde_json::from_value(encoded).expect("deserialize tagged runtime metadata");
        assert_eq!(decoded.boundless_submission(), Some(&snapshot));
    }

    #[test]
    fn new_runtime_metadata_keeps_sp1_submission_provider_scoped() {
        let progress = Sp1NetworkSubmissionProgress {
            provider_request_id: "0xsp1".to_string(),
            network_mode: Sp1NetworkMode::Reserved,
            fulfillment_strategy: Sp1FulfillmentStrategy::Hosted,
            skip_simulation: true,
            cycle_limit: 9_000_000,
            timeout_secs: 7_200,
            max_price_per_pgu: Some(42),
            auction_timeout_secs: Some(60),
        };
        let mut metadata = TaskRuntimeMetadata::default();
        metadata.apply_sp1_network_submission(&progress, 123);

        let encoded = serde_json::to_value(&metadata).expect("serialize runtime metadata");
        assert_eq!(encoded["remote_submission"]["provider"], "sp1");
        let submission = &encoded["remote_submission"]["submission"];
        assert_eq!(submission["provider_request_id"], "0xsp1");
        assert!(submission.get("expires_at").is_none());
        assert!(submission.get("max_price_wei").is_none());
        assert!(encoded.get("sp1_network_mode").is_none());
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
