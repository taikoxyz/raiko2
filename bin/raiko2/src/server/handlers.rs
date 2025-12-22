//! HTTP request handlers.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use raiko2_engine::EngineTaskKey;
use raiko2_engine::tasks::EngineOutput;
use raiko2_queue::{TaskState, decode_task_id, encode_task_id};
use serde::{Deserialize, Serialize};
use tracing::info;

use super::state::AppState;

/// Health check response.
#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
}

/// Health check endpoint.
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// Server info response.
#[derive(Serialize)]
pub struct InfoResponse {
    pub version: &'static str,
    pub prover: String,
    pub supported_provers: Vec<&'static str>,
}

/// Get server info.
pub async fn get_info(State(state): State<AppState>) -> Json<InfoResponse> {
    Json(InfoResponse {
        version: env!("CARGO_PKG_VERSION"),
        prover: format!("{:?}", state.config.prover.prover_type),
        supported_provers: vec!["risc0", "sp1", "native"],
    })
}

/// Proposal proof request.
#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Fields will be used when full proof generation is implemented
pub struct ProposalProofRequest {
    pub proposal_id: u64,
    pub l1_inclusion_block: u64,
    #[serde(default)]
    pub prover_type: Option<String>,
    #[serde(default)]
    pub blob_proof_type: Option<String>,
    #[serde(default)]
    pub prover: Option<String>,
    #[serde(default)]
    pub graffiti: Option<String>,
}

/// Proof response.
#[derive(Serialize)]
pub struct ProofResponse {
    pub id: String,
    pub status: ProofStatus,
}

/// Proof status.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProofStatus {
    Pending,
    Proving,
    Completed,
    Failed,
    Cancelled,
}

const fn status_from_task_state(state: &TaskState<EngineOutput, EngineTaskKey>) -> ProofStatus {
    match state {
        TaskState::Pending { .. } | TaskState::Ready => ProofStatus::Pending,
        TaskState::Retrying { .. } => ProofStatus::Pending,
        TaskState::Running { .. } => ProofStatus::Proving,
        TaskState::Succeeded { .. } => ProofStatus::Completed,
        TaskState::Failed { .. } => ProofStatus::Failed,
        TaskState::Cancelled => ProofStatus::Cancelled,
    }
}

/// Request a proposal proof.
pub async fn request_proposal_proof(
    State(state): State<AppState>,
    Json(req): Json<ProposalProofRequest>,
) -> Result<Json<ProofResponse>, ApiError> {
    info!(
        "Received proposal proof request: proposal_id={}, l1_block={}",
        req.proposal_id, req.l1_inclusion_block
    );

    let id = state
        .engine
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

    let view = state
        .engine
        .get(task_id)
        .await
        .map_err(|e| ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("Failed to read proof status: {e}"),
        })?
        .ok_or(ApiError {
            status: StatusCode::NOT_FOUND,
            message: format!("Proof job not found: {id}"),
        })?;

    let (proof, error) = match view.state.clone() {
        TaskState::Succeeded {
            output: EngineOutput::Proof(proof),
        } => (proof.output.proof, None),
        TaskState::Failed { error, .. } => (None, Some(error)),
        TaskState::Retrying { error, .. } => (None, Some(error)),
        _ => (None, None),
    };

    Ok(Json(ProofStatusResponse {
        id,
        status: status_from_task_state(&view.state),
        proof,
        error,
    }))
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

    state
        .engine
        .cancel(task_id.clone())
        .await
        .map_err(|e| ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("Failed to cancel proof job: {e}"),
        })?;

    let view = state
        .engine
        .get(task_id)
        .await
        .map_err(|e| ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("Failed to read proof status: {e}"),
        })?
        .ok_or(ApiError {
            status: StatusCode::NOT_FOUND,
            message: format!("Proof job not found: {id}"),
        })?;

    let (proof, error) = match view.state.clone() {
        TaskState::Succeeded {
            output: EngineOutput::Proof(proof),
        } => (proof.output.proof, None),
        TaskState::Failed { error, .. } => (None, Some(error)),
        _ => (None, None),
    };

    Ok(Json(ProofStatusResponse {
        id,
        status: status_from_task_state(&view.state),
        proof,
        error,
    }))
}

/// API error type.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let body = serde_json::json!({
            "error": self.message,
        });
        (self.status, Json(body)).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: err.to_string(),
        }
    }
}
