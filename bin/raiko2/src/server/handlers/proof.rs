use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::StatusCode,
};
use raiko2_engine::EngineTaskKey;
use raiko2_pipeline::PipelineKey;
use raiko2_queue::{decode_task_id, encode_task_id};
use serde::{Deserialize, Serialize};
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
}

/// Proof response.
#[derive(Serialize)]
pub struct ProofResponse {
    pub id: String,
    pub status: ProofStatus,
}

fn pipeline_key_from_request(state: &AppState, req: &ProposalProofRequest) -> PipelineKey {
    let prover_type = req.prover_type.unwrap_or(state.config.prover.prover_type);
    match prover_type {
        ProverType::Risc0 => PipelineKey::ShastaRisc0,
        ProverType::Sp1 => PipelineKey::ShastaSp1,
        ProverType::Native => PipelineKey::ShastaNative,
    }
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
    let id = engine
        .submit_proposal_proof(req.proposal_id)
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

#[cfg(test)]
mod tests {
    use super::ProposalProofRequest;

    #[test]
    fn proposal_proof_request_rejects_unknown_fields() {
        let raw = r#"{"proposal_id": 1, "l1_inclusion_block": 2}"#;
        let err = serde_json::from_str::<ProposalProofRequest>(raw).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn proposal_proof_request_accepts_optional_prover_type() -> Result<(), serde_json::Error> {
        let raw = r#"{"proposal_id": 1, "prover_type": "risc0"}"#;
        let req = serde_json::from_str::<ProposalProofRequest>(raw)?;
        assert_eq!(req.proposal_id, 1);
        assert!(req.prover_type.is_some());
        Ok(())
    }

    #[test]
    fn proposal_proof_request_rejects_invalid_prover_type() {
        let raw = r#"{"proposal_id": 1, "prover_type": "bogus"}"#;
        let err = serde_json::from_str::<ProposalProofRequest>(raw).unwrap_err();
        assert!(err.to_string().contains("unknown variant"));
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
    }))
}
