use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::StatusCode,
};
use raiko2_engine::{EngineTaskKey, ProposalStage, ProposalTaskRequest};
use raiko2_pipeline::PipelineKey;
use raiko2_primitives::L2BlockRange;
use raiko2_queue::{decode_task_id, encode_task_id};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tracing::info;

use super::super::state::{AppState, ProofStatus};
use super::errors::ApiError;
use crate::config::ProverType;

/// Proposal proof request.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalProofRequest {
    pub proposal_id: u64,
    #[serde(default)]
    pub prover_type: Option<ProverType>,
    #[serde(default)]
    pub l2_block_range: Option<L2BlockRangeRequest>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
pub struct L2BlockRangeRequest {
    pub start: u64,
    pub end: u64,
}

/// Aggregation proof request.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AggregationProofRequest {
    pub proof_ids: Vec<String>,
    #[serde(default)]
    pub prover_type: Option<ProverType>,
}

/// Proof response.
#[derive(Serialize)]
pub struct ProofResponse {
    pub id: String,
    pub status: ProofStatus,
}

fn pipeline_key_from_request(state: &AppState, req: &ProposalProofRequest) -> PipelineKey {
    let prover_type = req.prover_type.unwrap_or(state.config.prover.prover_type);
    pipeline_key_from_prover_type(prover_type)
}

fn proposal_task_request(req: &ProposalProofRequest) -> Result<ProposalTaskRequest, ApiError> {
    let l2_block_range = match req.l2_block_range {
        Some(range) if range.start > range.end => {
            return Err(ApiError {
                status: StatusCode::BAD_REQUEST,
                message: "l2_block_range.start must be <= l2_block_range.end".to_string(),
            });
        }
        Some(range) => Some(L2BlockRange {
            start: range.start,
            end: range.end,
        }),
        None => None,
    };

    Ok(ProposalTaskRequest {
        proposal_id: req.proposal_id,
        l2_block_range,
    })
}

fn pipeline_key_from_prover_type(prover_type: ProverType) -> PipelineKey {
    match prover_type {
        ProverType::Risc0 => PipelineKey::ShastaRisc0,
        ProverType::AgentRisc0 => PipelineKey::ShastaAgentRisc0,
        ProverType::Sp1 => PipelineKey::ShastaSp1,
        ProverType::Native => PipelineKey::ShastaNative,
    }
}

fn validate_aggregation_proof_ids(
    proof_ids: &[String],
) -> Result<(PipelineKey, Vec<raiko2_engine::EngineTaskId>), ApiError> {
    if proof_ids.len() < 2 {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "proof_ids must contain at least 2 entries".to_string(),
        });
    }

    let mut seen = HashSet::with_capacity(proof_ids.len());
    let mut decoded = Vec::with_capacity(proof_ids.len());
    let mut pipeline = None;

    for proof_id in proof_ids {
        if !seen.insert(proof_id.clone()) {
            return Err(ApiError {
                status: StatusCode::BAD_REQUEST,
                message: format!("duplicate proof id: {proof_id}"),
            });
        }

        let task_id = decode_task_id::<EngineTaskKey>(proof_id).map_err(|e| ApiError {
            status: StatusCode::BAD_REQUEST,
            message: format!("Invalid proof id '{proof_id}': {e}"),
        })?;

        match &task_id.0 {
            EngineTaskKey::Proposal {
                pipeline: task_pipeline,
                stage: ProposalStage::Prove,
                ..
            } => {
                if let Some(expected) = pipeline {
                    if expected != *task_pipeline {
                        return Err(ApiError {
                            status: StatusCode::BAD_REQUEST,
                            message: "proof_ids must belong to the same pipeline".to_string(),
                        });
                    }
                } else {
                    pipeline = Some(*task_pipeline);
                }
            }
            EngineTaskKey::Proposal { stage, .. } => {
                return Err(ApiError {
                    status: StatusCode::BAD_REQUEST,
                    message: format!(
                        "proof id '{proof_id}' must reference a proposal prove task, got {stage:?}"
                    ),
                });
            }
            EngineTaskKey::Aggregate { .. } => {
                return Err(ApiError {
                    status: StatusCode::BAD_REQUEST,
                    message: format!(
                        "proof id '{proof_id}' must reference a proposal prove task, not an aggregate task"
                    ),
                });
            }
        }

        decoded.push(task_id);
    }

    Ok((
        pipeline.expect("validated proof_ids must contain at least one task"),
        decoded,
    ))
}

/// Request a proposal proof.
pub async fn request_proposal_proof(
    State(state): State<AppState>,
    req: Result<Json<ProposalProofRequest>, JsonRejection>,
) -> Result<Json<ProofResponse>, ApiError> {
    let Json(req) = req.map_err(|err| ApiError {
        status: StatusCode::BAD_REQUEST,
        message: err.to_string(),
    })?;
    info!(
        "Received proposal proof request: proposal_id={}",
        req.proposal_id
    );

    let pipeline_key = pipeline_key_from_request(&state, &req);
    let engine = state.pipelines.get(pipeline_key).ok_or(ApiError {
        status: StatusCode::NOT_FOUND,
        message: format!("Pipeline not available: {}", pipeline_key.as_str()),
    })?;
    let request = proposal_task_request(&req)?;
    let id = engine
        .submit_proposal_proof(request)
        .await
        .map_err(|e| ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("Failed to enqueue proof job: {e}"),
        })?;
    let id = encode_task_id(&id).map_err(|e| ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: format!("Failed to encode task id: {e}"),
    })?;

    Ok(Json(ProofResponse {
        id,
        status: ProofStatus::Pending,
    }))
}

/// Request an aggregation proof.
pub async fn request_aggregation_proof(
    State(state): State<AppState>,
    req: Result<Json<AggregationProofRequest>, JsonRejection>,
) -> Result<Json<ProofResponse>, ApiError> {
    let Json(req) = req.map_err(|err| ApiError {
        status: StatusCode::BAD_REQUEST,
        message: err.to_string(),
    })?;
    info!(
        "Received aggregation proof request with {} proof ids",
        req.proof_ids.len()
    );

    let (pipeline_key, proof_tasks) = validate_aggregation_proof_ids(&req.proof_ids)?;
    if let Some(prover_type) = req.prover_type {
        let requested = pipeline_key_from_prover_type(prover_type);
        if requested != pipeline_key {
            return Err(ApiError {
                status: StatusCode::BAD_REQUEST,
                message: format!(
                    "prover_type {} does not match proof pipeline {}",
                    match prover_type {
                        ProverType::Risc0 => "risc0",
                        ProverType::AgentRisc0 => "agent-risc0",
                        ProverType::Sp1 => "sp1",
                        ProverType::Native => "native",
                    },
                    pipeline_key.as_str()
                ),
            });
        }
    }

    let engine = state.pipelines.get(pipeline_key).ok_or(ApiError {
        status: StatusCode::NOT_FOUND,
        message: format!("Pipeline not available: {}", pipeline_key.as_str()),
    })?;
    let id = engine
        .submit_aggregation_proof(proof_tasks)
        .await
        .map_err(|e| ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("Failed to enqueue aggregation job: {e}"),
        })?;
    let id = encode_task_id(&id).map_err(|e| ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: format!("Failed to encode task id: {e}"),
    })?;

    Ok(Json(ProofResponse {
        id,
        status: ProofStatus::Pending,
    }))
}

#[cfg(test)]
mod tests {
    use super::{
        AggregationProofRequest, L2BlockRangeRequest, ProposalProofRequest,
        pipeline_key_from_request, proposal_task_request, validate_aggregation_proof_ids,
    };
    use crate::config::{Config, ProverType};
    use crate::server::state::{AppState, StaticPipelineFactory};
    use axum::http::StatusCode;
    use raiko2_engine::{EngineTaskId, EngineTaskKey, ProposalStage, ProposalTaskRequest};
    use raiko2_pipeline::PipelineKey;
    use raiko2_queue::encode_task_id;
    use std::sync::Arc;

    #[test]
    fn proposal_proof_request_rejects_unknown_fields() {
        let raw = r#"{"proposal_id": 1, "l1_inclusion_block": 2}"#;
        let err = serde_json::from_str::<ProposalProofRequest>(raw).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn proposal_proof_request_accepts_optional_prover_type() -> Result<(), serde_json::Error> {
        let raw = r#"{"proposal_id": 1, "prover_type": "agent-risc0"}"#;
        let req = serde_json::from_str::<ProposalProofRequest>(raw)?;
        assert_eq!(req.proposal_id, 1);
        assert!(req.prover_type.is_some());
        assert_eq!(req.l2_block_range, None);
        Ok(())
    }

    #[test]
    fn proposal_proof_request_accepts_optional_l2_block_range() -> Result<(), serde_json::Error> {
        let raw = r#"{"proposal_id": 1, "l2_block_range": {"start": 10, "end": 12}}"#;
        let req = serde_json::from_str::<ProposalProofRequest>(raw)?;
        assert_eq!(
            req.l2_block_range,
            Some(L2BlockRangeRequest { start: 10, end: 12 })
        );
        Ok(())
    }

    #[test]
    fn proposal_proof_request_rejects_invalid_prover_type() {
        let raw = r#"{"proposal_id": 1, "prover_type": "bogus"}"#;
        let err = serde_json::from_str::<ProposalProofRequest>(raw).unwrap_err();
        assert!(err.to_string().contains("unknown variant"));
    }

    #[test]
    fn aggregation_proof_request_rejects_unknown_fields() {
        let raw = r#"{"proof_ids":["abc"],"extra":true}"#;
        let err = serde_json::from_str::<AggregationProofRequest>(raw).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn aggregation_proof_request_accepts_optional_prover_type() -> Result<(), serde_json::Error> {
        let raw = r#"{"proof_ids":["a","b"],"prover_type":"agent-risc0"}"#;
        let req = serde_json::from_str::<AggregationProofRequest>(raw)?;
        assert_eq!(req.proof_ids.len(), 2);
        assert_eq!(req.prover_type, Some(ProverType::AgentRisc0));
        Ok(())
    }

    fn encode_proposal_prove_task_id(
        pipeline: PipelineKey,
        proposal_id: u64,
        stage: ProposalStage,
    ) -> String {
        let task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: ProposalTaskRequest {
                proposal_id,
                l2_block_range: None,
            },
            stage,
        });
        encode_task_id(&task_id).expect("encode task id")
    }

    #[test]
    fn validate_aggregation_proof_ids_accepts_matching_proposal_proofs() {
        let proof_ids = vec![
            encode_proposal_prove_task_id(PipelineKey::ShastaAgentRisc0, 1, ProposalStage::Prove),
            encode_proposal_prove_task_id(PipelineKey::ShastaAgentRisc0, 2, ProposalStage::Prove),
        ];

        let (pipeline, decoded) =
            validate_aggregation_proof_ids(&proof_ids).expect("aggregation proof ids valid");
        assert_eq!(pipeline, PipelineKey::ShastaAgentRisc0);
        assert_eq!(decoded.len(), 2);
    }

    #[test]
    fn validate_aggregation_proof_ids_rejects_mixed_pipelines() {
        let proof_ids = vec![
            encode_proposal_prove_task_id(PipelineKey::ShastaAgentRisc0, 1, ProposalStage::Prove),
            encode_proposal_prove_task_id(PipelineKey::ShastaNative, 2, ProposalStage::Prove),
        ];

        let err = validate_aggregation_proof_ids(&proof_ids).unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("same pipeline"));
    }

    #[test]
    fn validate_aggregation_proof_ids_rejects_non_prove_stage() {
        let proof_ids = vec![
            encode_proposal_prove_task_id(
                PipelineKey::ShastaAgentRisc0,
                1,
                ProposalStage::Validation,
            ),
            encode_proposal_prove_task_id(PipelineKey::ShastaAgentRisc0, 2, ProposalStage::Prove),
        ];

        let err = validate_aggregation_proof_ids(&proof_ids).unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("proposal prove task"));
    }

    #[test]
    fn validate_aggregation_proof_ids_rejects_duplicates() {
        let proof_id =
            encode_proposal_prove_task_id(PipelineKey::ShastaAgentRisc0, 1, ProposalStage::Prove);
        let proof_ids = vec![proof_id.clone(), proof_id];

        let err = validate_aggregation_proof_ids(&proof_ids).unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("duplicate"));
    }

    fn state_with_default_prover(prover_type: ProverType) -> AppState {
        let mut config = Config::default();
        config.prover.prover_type = prover_type;
        AppState {
            config: Arc::new(config),
            pipelines: Arc::new(StaticPipelineFactory::default()),
        }
    }

    #[test]
    fn pipeline_key_from_request_respects_prover_type_override() {
        let state = state_with_default_prover(ProverType::AgentRisc0);

        let req = ProposalProofRequest {
            proposal_id: 1,
            prover_type: Some(ProverType::Risc0),
            l2_block_range: None,
        };
        assert_eq!(
            pipeline_key_from_request(&state, &req),
            PipelineKey::ShastaRisc0
        );

        let req = ProposalProofRequest {
            proposal_id: 1,
            prover_type: Some(ProverType::AgentRisc0),
            l2_block_range: None,
        };
        assert_eq!(
            pipeline_key_from_request(&state, &req),
            PipelineKey::ShastaAgentRisc0
        );
    }

    #[test]
    fn pipeline_key_from_request_falls_back_to_server_default() {
        let state = state_with_default_prover(ProverType::AgentRisc0);
        let req = ProposalProofRequest {
            proposal_id: 1,
            prover_type: None,
            l2_block_range: None,
        };
        assert_eq!(
            pipeline_key_from_request(&state, &req),
            PipelineKey::ShastaAgentRisc0
        );
    }

    #[test]
    fn proposal_task_request_rejects_invalid_l2_block_range() {
        let req = ProposalProofRequest {
            proposal_id: 1,
            prover_type: None,
            l2_block_range: Some(L2BlockRangeRequest { start: 12, end: 10 }),
        };
        let err = proposal_task_request(&req).unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("start must be <="));
    }

    #[test]
    fn proposal_task_request_maps_valid_l2_block_range() {
        let req = ProposalProofRequest {
            proposal_id: 7,
            prover_type: None,
            l2_block_range: Some(L2BlockRangeRequest { start: 10, end: 12 }),
        };
        let request = proposal_task_request(&req).expect("proposal task request");
        assert_eq!(request.proposal_id, 7);
        assert_eq!(
            request.l2_block_range,
            Some(raiko2_primitives::L2BlockRange { start: 10, end: 12 })
        );
    }
}

/// Proof status response.
#[derive(Serialize)]
pub struct ProofStatusResponse {
    pub id: String,
    pub status: ProofStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proof: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_data: Option<serde_json::Value>,
}

/// Get proof status.
pub async fn get_proof_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ProofStatusResponse>, ApiError> {
    info!("Getting proof status for: {}", id);

    let task_id = decode_task_id::<EngineTaskKey>(&id).map_err(|e| ApiError {
        status: StatusCode::BAD_REQUEST,
        message: format!("Invalid task id '{id}': {e}"),
    })?;

    let pipeline_key = task_id.0.pipeline_key();
    let engine = state.pipelines.get(pipeline_key).ok_or(ApiError {
        status: StatusCode::NOT_FOUND,
        message: format!("Pipeline not available: {}", pipeline_key.as_str()),
    })?;
    let view = engine
        .get_status(task_id)
        .await
        .map_err(|e| ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("Failed to read proof status: {e}"),
        })?
        .ok_or(ApiError {
            status: StatusCode::NOT_FOUND,
            message: format!("Proof job not found: {id}"),
        })?;

    Ok(Json(ProofStatusResponse {
        id,
        status: view.status,
        proof: view.proof,
        error: view.error,
        extra_data: view.extra_data,
    }))
}

/// Cancel proof request.
pub async fn cancel_proof(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ProofStatusResponse>, ApiError> {
    info!("Cancelling proof: {}", id);

    let task_id = decode_task_id::<EngineTaskKey>(&id).map_err(|e| ApiError {
        status: StatusCode::BAD_REQUEST,
        message: format!("Invalid task id '{id}': {e}"),
    })?;

    let pipeline_key = task_id.0.pipeline_key();
    let engine = state.pipelines.get(pipeline_key).ok_or(ApiError {
        status: StatusCode::NOT_FOUND,
        message: format!("Pipeline not available: {}", pipeline_key.as_str()),
    })?;
    engine.cancel(task_id.clone()).await.map_err(|e| ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: format!("Failed to cancel proof job: {e}"),
    })?;

    let view = engine
        .get_status(task_id)
        .await
        .map_err(|e| ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("Failed to read proof status: {e}"),
        })?
        .ok_or(ApiError {
            status: StatusCode::NOT_FOUND,
            message: format!("Proof job not found: {id}"),
        })?;

    Ok(Json(ProofStatusResponse {
        id,
        status: view.status,
        proof: view.proof,
        error: view.error,
        extra_data: view.extra_data,
    }))
}
