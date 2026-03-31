use anyhow::{Context, Result};
use async_trait::async_trait;
use raiko2_engine::{
    EngineObserver, EngineTaskId, EngineTaskKey, EngineTaskSuccess, ProposalStage,
    tasks::EngineTask,
};
use raiko2_prover::ProverProgress;
use raiko2_queue::encode_task_id;
use raiko2_runtime::{RunnerStatus, RuntimeManager, RuntimeTaskRecord};
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::server::task_metadata::HoodiTaskMetadata;

#[derive(Clone)]
pub(crate) struct RuntimeObserver {
    runtime: Arc<RuntimeManager>,
}

impl RuntimeObserver {
    pub(crate) const fn new(runtime: Arc<RuntimeManager>) -> Self {
        Self { runtime }
    }

    fn root_task_id(id: &EngineTaskId) -> EngineTaskId {
        match &id.0 {
            EngineTaskKey::Proposal {
                pipeline, request, ..
            } => EngineTaskId::new(EngineTaskKey::Proposal {
                pipeline: *pipeline,
                request: request.clone(),
                stage: ProposalStage::Prove,
            }),
            EngineTaskKey::Aggregate { .. } => id.clone(),
        }
    }

    const fn stage_name(task: &EngineTask) -> &'static str {
        match task {
            EngineTask::Preflight { .. } => "preflight",
            EngineTask::Validate { .. } => "validation",
            EngineTask::Encode { .. } => "encode",
            EngineTask::ProveProposal { .. } => "prove",
            EngineTask::Aggregate { .. } => "aggregate",
        }
    }

    async fn update_root_record<F>(&self, id: &EngineTaskId, mutator: F) -> Result<()>
    where
        F: FnOnce(&mut RuntimeTaskRecord, i64) -> Result<()>,
    {
        let root_id = Self::root_task_id(id);
        let root_id = encode_task_id(&root_id).context("failed to encode root task id")?;
        let Some(mut record) = self.runtime.find_task_by_engine_task_id(&root_id).await? else {
            return Ok(());
        };
        let updated_at = now_ts();
        mutator(&mut record, updated_at)?;
        record.updated_at = updated_at;
        self.runtime.upsert_task(&record).await
    }
}

#[async_trait]
impl EngineObserver for RuntimeObserver {
    async fn on_task_started(&self, id: &EngineTaskId, task: &EngineTask, worker: &str) {
        let stage = Self::stage_name(task);
        if let Err(err) = self
            .update_root_record(id, |record, updated_at| {
                record.runner_status = RunnerStatus::Running;
                record.error = None;
                update_task_metadata(record, |metadata| {
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
        let result = self
            .update_root_record(id, |record, updated_at| {
                record.runner_status = RunnerStatus::Running;
                record.error = None;
                let task_id = encode_task_id(id).context("failed to encode progress task id")?;
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
                            | EngineTask::Encode { .. } => {}
                        },
                        ProverProgress::Sp1NetworkSubmission(submission) => match task {
                            EngineTask::ProveProposal { .. } => {
                                metadata.upsert_proposal_sp1_network_runtime(
                                    &task_id, submission, updated_at,
                                );
                            }
                            EngineTask::Aggregate { .. } => {
                                metadata
                                    .upsert_aggregate_sp1_network_runtime(submission, updated_at);
                            }
                            EngineTask::Preflight { .. }
                            | EngineTask::Validate { .. }
                            | EngineTask::Encode { .. } => {}
                        },
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

    async fn on_task_succeeded(
        &self,
        id: &EngineTaskId,
        task: &EngineTask,
        success: &EngineTaskSuccess,
    ) {
        let stage = Self::stage_name(task);
        let result = match success {
            EngineTaskSuccess::Proof { proof, .. } => {
                self.update_root_record(id, |record, updated_at| {
                    record.runner_status = RunnerStatus::Completed;
                    record.error = None;
                    record.proof_path = Some(write_proof_file(record, proof)?);
                    update_task_metadata(record, |metadata| {
                        metadata.runtime.active_stage = Some(stage.to_string());
                        metadata.runtime.last_event = Some("completed".to_string());
                    })?;
                    record.updated_at = updated_at;
                    Ok(())
                })
                .await
            }
            EngineTaskSuccess::GuestInput { stage } | EngineTaskSuccess::EncodedInput { stage } => {
                self.update_root_record(id, |record, updated_at| {
                    record.runner_status = RunnerStatus::Running;
                    record.error = None;
                    update_task_metadata(record, |metadata| {
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
        if let Err(err) = self
            .update_root_record(id, |record, updated_at| {
                record.runner_status = RunnerStatus::Failed;
                record.error = Some(error.to_string());
                update_task_metadata(record, |metadata| {
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
        if let Err(err) = self
            .update_root_record(id, |record, updated_at| {
                record.runner_status = RunnerStatus::Cancelled;
                record.error = None;
                update_task_metadata(record, |metadata| {
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
}

fn update_task_metadata<F>(record: &mut RuntimeTaskRecord, mutator: F) -> Result<()>
where
    F: FnOnce(&mut HoodiTaskMetadata),
{
    let mut metadata: HoodiTaskMetadata = serde_json::from_value(record.metadata.clone())
        .context("failed to parse hoodi task metadata")?;
    mutator(&mut metadata);
    record.metadata =
        serde_json::to_value(metadata).context("failed to serialize task metadata")?;
    Ok(())
}

fn write_proof_file(
    record: &RuntimeTaskRecord,
    proof: &raiko2_primitives::Proof,
) -> Result<String> {
    let proof_path = Path::new(&record.task_dir).join("proof.json");
    std::fs::write(
        &proof_path,
        serde_json::to_vec_pretty(proof).context("failed to serialize proof output")?,
    )
    .with_context(|| format!("failed to write proof file {}", proof_path.display()))?;
    Ok(proof_path.display().to_string())
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
mod tests {
    use super::*;
    use crate::server::task_metadata::{HoodiProposalTask, HoodiRuntimeMetadata};
    use raiko2_engine::ProposalTaskRequest;
    use raiko2_pipeline::PipelineKey;
    use raiko2_primitives::ProofType;
    use raiko2_prover::{
        BoundlessSubmissionProgress, Sp1FulfillmentStrategy, Sp1NetworkMode,
        Sp1NetworkSubmissionProgress, sp1::ExecutionMode,
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
            blob_proof_type: None,
            prover: None,
            graffiti: None,
            prover_args_json: None,
        }
    }

    #[tokio::test]
    async fn runtime_observer_records_boundless_submission_metadata_immediately() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer",
        ))?);
        let proposal_task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaRisc0Boundless,
            request: proposal_request(),
            stage: ProposalStage::Prove,
        });
        let encoded_task_id = encode_task_id(&proposal_task_id).expect("encode proposal task");
        runtime
            .register_task(TaskRegistration {
                task_id: "task_public".to_string(),
                pipeline_key: "shasta-risc0-boundless".to_string(),
                route: "risc0/boundless".to_string(),
                guest_system: "risc0".to_string(),
                runner: "boundless".to_string(),
                task_kind: "hoodi_batch".to_string(),
                proposal_id: Some(42),
                proof_ids: vec![encoded_task_id.clone()],
                metadata: serde_json::to_value(HoodiTaskMetadata {
                    network_pair: "taiko_dev/ethereum".to_string(),
                    network: "taiko_dev".to_string(),
                    l1_network: "ethereum".to_string(),
                    proof_type: ProofType::Risc0,
                    api_proof_type: None,
                    execution_mode: None,
                    aggregate_requested: false,
                    proposals: vec![HoodiProposalTask {
                        proposal_id: 42,
                        l1_inclusion_block_number: 1,
                        l2_block_numbers: vec![42],
                        last_anchor_block_number: 0,
                        task_id: encoded_task_id.clone(),
                    }],
                    aggregate_task_id: None,
                    runtime: HoodiRuntimeMetadata::default(),
                })?,
            })
            .await?;

        let observer = RuntimeObserver::new(Arc::clone(&runtime));
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
                    image_ref: "0ximage".to_string(),
                    deployment: "base".to_string(),
                    offchain: false,
                    quoted_mcycles_count: Some(6_000),
                    evaluated_mcycles_count: Some(12_345),
                }),
            )
            .await;

        let record = runtime
            .get_task("task_public")
            .await?
            .expect("runtime task exists");
        let metadata: HoodiTaskMetadata = serde_json::from_value(record.metadata)?;
        let runtime_entry = metadata
            .proposal_runtime(&encoded_task_id)
            .expect("proposal runtime exists");
        assert_eq!(
            metadata.runtime.last_event.as_deref(),
            Some("submission_registered")
        );
        assert_eq!(runtime_entry.provider_request_id.as_deref(), Some("0x1234"));
        assert_eq!(runtime_entry.remote_tx_hash.as_deref(), Some("0xabcd"));
        assert_eq!(runtime_entry.image_ref.as_deref(), Some("0ximage"));
        assert_eq!(runtime_entry.quoted_mcycles_count, Some(6_000));
        assert_eq!(runtime_entry.evaluated_mcycles_count, Some(12_345));
        Ok(())
    }

    #[tokio::test]
    async fn runtime_observer_records_sp1_network_submission_metadata() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-sp1",
        ))?);
        let proposal_task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline: PipelineKey::ShastaSp1,
            request: proposal_request(),
            stage: ProposalStage::Prove,
        });
        let encoded_task_id = encode_task_id(&proposal_task_id).expect("encode proposal task");
        runtime
            .register_task(TaskRegistration {
                task_id: "task_public_sp1".to_string(),
                pipeline_key: "shasta-sp1-local".to_string(),
                route: "sp1/local".to_string(),
                guest_system: "sp1".to_string(),
                runner: "local".to_string(),
                task_kind: "hoodi_batch".to_string(),
                proposal_id: Some(42),
                proof_ids: vec![encoded_task_id.clone()],
                metadata: serde_json::to_value(HoodiTaskMetadata {
                    network_pair: "taiko_dev/ethereum".to_string(),
                    network: "taiko_dev".to_string(),
                    l1_network: "ethereum".to_string(),
                    proof_type: ProofType::Sp1,
                    api_proof_type: None,
                    execution_mode: Some(ExecutionMode::Prove),
                    aggregate_requested: false,
                    proposals: vec![HoodiProposalTask {
                        proposal_id: 42,
                        l1_inclusion_block_number: 1,
                        l2_block_numbers: vec![42],
                        last_anchor_block_number: 0,
                        task_id: encoded_task_id.clone(),
                    }],
                    aggregate_task_id: None,
                    runtime: HoodiRuntimeMetadata::default(),
                })?,
            })
            .await?;

        let observer = RuntimeObserver::new(Arc::clone(&runtime));
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
                }),
            )
            .await;

        let record = runtime
            .get_task("task_public_sp1")
            .await?
            .expect("runtime task exists");
        let metadata: HoodiTaskMetadata = serde_json::from_value(record.metadata)?;
        let runtime_entry = metadata
            .proposal_runtime(&encoded_task_id)
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
        Ok(())
    }
}
