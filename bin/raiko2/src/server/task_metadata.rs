use raiko2_prover::BoundlessSubmissionProgress;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct HoodiTaskMetadata {
    pub(crate) network_pair: String,
    pub(crate) network: String,
    pub(crate) l1_network: String,
    pub(crate) proof_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) execution_mode: Option<String>,
    pub(crate) aggregate_requested: bool,
    pub(crate) proposals: Vec<HoodiProposalTask>,
    pub(crate) aggregate_task_id: Option<String>,
    #[serde(default)]
    pub(crate) runtime: HoodiRuntimeMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct HoodiProposalTask {
    pub(crate) proposal_id: u64,
    pub(crate) l1_inclusion_block_number: u64,
    pub(crate) l2_block_numbers: Vec<u64>,
    pub(crate) last_anchor_block_number: u64,
    pub(crate) task_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct HoodiRuntimeMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) active_stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_event: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) proposals: BTreeMap<String, HoodiTaskRuntimeMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) aggregate: Option<HoodiTaskRuntimeMetadata>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct HoodiTaskRuntimeMetadata {
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
    pub(crate) quoted_mcycles_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) evaluated_mcycles_count: Option<u32>,
}

impl HoodiTaskMetadata {
    pub(crate) fn has_runtime_progress(&self) -> bool {
        self.runtime.active_stage.is_some()
            || self.runtime.last_event.is_some()
            || !self.runtime.proposals.is_empty()
            || self.runtime.aggregate.is_some()
    }

    pub(crate) fn proposal_runtime(&self, task_id: &str) -> Option<&HoodiTaskRuntimeMetadata> {
        self.runtime.proposals.get(task_id)
    }

    pub(crate) const fn aggregate_runtime(&self) -> Option<&HoodiTaskRuntimeMetadata> {
        self.runtime.aggregate.as_ref()
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
            .get_or_insert_with(HoodiTaskRuntimeMetadata::default)
            .apply_boundless_submission(progress, updated_at);
    }
}

impl HoodiTaskRuntimeMetadata {
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
        self.quoted_mcycles_count = progress.quoted_mcycles_count;
        self.evaluated_mcycles_count = progress.evaluated_mcycles_count;
    }
}
