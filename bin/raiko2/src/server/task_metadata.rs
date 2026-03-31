use raiko2_primitives::ProofType;
use raiko2_prover::{
    BoundlessSubmissionProgress, Sp1FulfillmentStrategy, Sp1NetworkMode,
    Sp1NetworkSubmissionProgress, sp1::ExecutionMode,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

mod hoodi_proof_type {
    use raiko2_primitives::{ProofType, proof_type::lowercase};
    use serde::Serializer;

    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub fn serialize<S>(proof_type: &ProofType, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        lowercase::serialize(proof_type, serializer)
    }

    pub fn parse(raw: &str) -> Result<ProofType, String> {
        if raw.trim().eq_ignore_ascii_case("zk_any") {
            return Ok(ProofType::Risc0);
        }
        raw.parse()
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct HoodiTaskMetadata {
    pub(crate) network_pair: String,
    pub(crate) network: String,
    pub(crate) l1_network: String,
    #[serde(serialize_with = "hoodi_proof_type::serialize")]
    pub(crate) proof_type: ProofType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) api_proof_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) execution_mode: Option<ExecutionMode>,
    pub(crate) aggregate_requested: bool,
    pub(crate) proposals: Vec<HoodiProposalTask>,
    pub(crate) aggregate_task_id: Option<String>,
    #[serde(default)]
    pub(crate) runtime: HoodiRuntimeMetadata,
}

#[derive(Debug, Clone, Deserialize)]
struct HoodiTaskMetadataSerde {
    network_pair: String,
    network: String,
    l1_network: String,
    proof_type: String,
    #[serde(default)]
    api_proof_type: Option<String>,
    #[serde(default)]
    execution_mode: Option<ExecutionMode>,
    aggregate_requested: bool,
    proposals: Vec<HoodiProposalTask>,
    aggregate_task_id: Option<String>,
    #[serde(default)]
    runtime: HoodiRuntimeMetadata,
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

impl HoodiTaskMetadata {
    pub(crate) fn response_proof_type(&self) -> String {
        self.api_proof_type
            .clone()
            .unwrap_or_else(|| self.proof_type.to_string())
    }

    pub(crate) fn execution_mode_str(&self) -> Option<String> {
        self.execution_mode.map(|mode| mode.as_str().to_string())
    }

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
            .get_or_insert_with(HoodiTaskRuntimeMetadata::default)
            .apply_sp1_network_submission(progress, updated_at);
    }
}

impl<'de> Deserialize<'de> for HoodiTaskMetadata {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let metadata = HoodiTaskMetadataSerde::deserialize(deserializer)?;
        let proof_type =
            hoodi_proof_type::parse(&metadata.proof_type).map_err(serde::de::Error::custom)?;
        let normalized_proof_type = metadata.proof_type.trim().to_lowercase();
        let api_proof_type = metadata.api_proof_type.or_else(|| {
            (normalized_proof_type != proof_type.to_string()).then_some(normalized_proof_type)
        });

        Ok(Self {
            network_pair: metadata.network_pair,
            network: metadata.network,
            l1_network: metadata.l1_network,
            proof_type,
            api_proof_type,
            execution_mode: metadata.execution_mode,
            aggregate_requested: metadata.aggregate_requested,
            proposals: metadata.proposals,
            aggregate_task_id: metadata.aggregate_task_id,
            runtime: metadata.runtime,
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn task_metadata_deserializes_legacy_zk_any_as_risc0() {
        let metadata: HoodiTaskMetadata = serde_json::from_value(json!({
            "network_pair": "taiko_hoodi/hoodi",
            "network": "taiko_hoodi",
            "l1_network": "hoodi",
            "proof_type": "zk_any",
            "aggregate_requested": false,
            "proposals": []
        }))
        .expect("deserialize legacy metadata");

        assert_eq!(metadata.proof_type, ProofType::Risc0);
        assert_eq!(metadata.response_proof_type(), "zk_any");
    }

    #[test]
    fn task_metadata_prefers_api_proof_type_for_response() {
        let metadata = HoodiTaskMetadata {
            network_pair: "taiko_hoodi/hoodi".to_string(),
            network: "taiko_hoodi".to_string(),
            l1_network: "hoodi".to_string(),
            proof_type: ProofType::Risc0,
            api_proof_type: Some("zk_any".to_string()),
            execution_mode: None,
            aggregate_requested: false,
            proposals: Vec::new(),
            aggregate_task_id: None,
            runtime: HoodiRuntimeMetadata::default(),
        };

        assert_eq!(metadata.response_proof_type(), "zk_any");
    }
}
