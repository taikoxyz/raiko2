use anyhow::{Context, Result};
use async_trait::async_trait;
use raiko2_engine::{
    EngineObserver, EngineTaskId, EngineTaskKey, EngineTaskSuccess, ProposalStage,
    tasks::EngineTask,
};
use raiko2_queue::encode_task_id;
use raiko2_runtime::{RunnerStatus, RuntimeManager, RuntimeTaskRecord};
use serde_json::{Map, Value, json};
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

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
        F: FnOnce(&mut RuntimeTaskRecord) -> Result<()>,
    {
        let root_id = Self::root_task_id(id);
        let root_id = encode_task_id(&root_id).context("failed to encode root task id")?;
        let Some(mut record) = self.runtime.get_task(&root_id).await? else {
            return Ok(());
        };
        mutator(&mut record)?;
        record.updated_at = now_ts();
        self.runtime.upsert_task(&record).await
    }
}

#[async_trait]
impl EngineObserver for RuntimeObserver {
    async fn on_task_started(&self, id: &EngineTaskId, task: &EngineTask, worker: &str) {
        let stage = Self::stage_name(task);
        if let Err(err) = self
            .update_root_record(id, |record| {
                record.runner_status = RunnerStatus::Running;
                record.error = None;
                merge_metadata(
                    record,
                    json!({
                        "active_stage": stage,
                        "last_event": "started",
                        "worker": worker,
                    }),
                );
                Ok(())
            })
            .await
        {
            tracing::warn!(task = ?id, error = %err, "failed to sync runtime task start");
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
                self.update_root_record(id, |record| {
                    record.runner_status = RunnerStatus::Completed;
                    record.error = None;
                    record.proof_path = Some(write_proof_file(record, proof)?);
                    if let Some(extra_data) = proof.extra_data.clone() {
                        merge_metadata(
                            record,
                            json!({
                                "active_stage": stage,
                                "last_event": "completed",
                                "proof_extra_data": extra_data,
                            }),
                        );
                        apply_boundless_metadata(record, proof.extra_data.as_ref());
                    } else {
                        merge_metadata(
                            record,
                            json!({
                                "active_stage": stage,
                                "last_event": "completed",
                            }),
                        );
                    }
                    Ok(())
                })
                .await
            }
            EngineTaskSuccess::GuestInput { stage } | EngineTaskSuccess::EncodedInput { stage } => {
                self.update_root_record(id, |record| {
                    record.runner_status = RunnerStatus::Running;
                    record.error = None;
                    merge_metadata(
                        record,
                        json!({
                            "active_stage": stage_name_from_pipeline_stage(*stage),
                            "last_event": "stage_completed",
                        }),
                    );
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
            .update_root_record(id, |record| {
                record.runner_status = RunnerStatus::Failed;
                record.error = Some(error.to_string());
                merge_metadata(
                    record,
                    json!({
                        "active_stage": stage,
                        "last_event": "failed",
                    }),
                );
                Ok(())
            })
            .await
        {
            tracing::warn!(task = ?id, error = %err, "failed to sync runtime task failure");
        }
    }

    async fn on_task_cancelled(&self, id: &EngineTaskId) {
        if let Err(err) = self
            .update_root_record(id, |record| {
                record.runner_status = RunnerStatus::Cancelled;
                record.error = None;
                merge_metadata(record, json!({"last_event": "cancelled"}));
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

fn merge_metadata(record: &mut RuntimeTaskRecord, patch: Value) {
    let Value::Object(patch) = patch else {
        return;
    };

    if !record.metadata.is_object() {
        record.metadata = Value::Object(Map::new());
    }

    let metadata = record
        .metadata
        .as_object_mut()
        .expect("metadata forced to object");
    for (key, value) in patch {
        metadata.insert(key, value);
    }
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

fn apply_boundless_metadata(record: &mut RuntimeTaskRecord, extra_data: Option<&Value>) {
    let Some(extra_data) = extra_data else {
        return;
    };

    let root = extra_data
        .get("boundless")
        .and_then(Value::as_object)
        .or_else(|| extra_data.as_object());

    let Some(root) = root else {
        return;
    };

    if let Some(image_id) = root
        .get("image_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
    {
        record.image_ref = Some(image_id);
    }

    if let Some(provider_request_id) = root
        .get("provider_request_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
    {
        record.provider_request_id = Some(provider_request_id);
    }

    let tx_hash = root
        .get("remote_tx_hash")
        .or_else(|| root.get("tx_hash"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    if tx_hash.is_some() {
        record.remote_tx_hash = tx_hash;
    }
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
