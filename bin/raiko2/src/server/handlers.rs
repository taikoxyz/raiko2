//! HTTP request handlers.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use super::state::{AppState, ProofJob, ProofJobStatus};

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
        supported_provers: vec!["risc0", "sp1"],
    })
}

/// Batch proof request.
#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Fields will be used when full proof generation is implemented
pub struct BatchProofRequest {
    pub batch_id: u64,
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

impl From<ProofJobStatus> for ProofStatus {
    fn from(status: ProofJobStatus) -> Self {
        match status {
            ProofJobStatus::Pending => ProofStatus::Pending,
            ProofJobStatus::Proving => ProofStatus::Proving,
            ProofJobStatus::Completed => ProofStatus::Completed,
            ProofJobStatus::Failed => ProofStatus::Failed,
            ProofJobStatus::Cancelled => ProofStatus::Cancelled,
        }
    }
}

/// Request a batch proof.
pub async fn request_batch_proof(
    State(state): State<AppState>,
    Json(req): Json<BatchProofRequest>,
) -> Result<Json<ProofResponse>, ApiError> {
    info!(
        "Received batch proof request: batch_id={}, l1_block={}",
        req.batch_id, req.l1_inclusion_block
    );

    let proof_id = format!(
        "proof-{}-{}-{}",
        req.batch_id,
        req.l1_inclusion_block,
        chrono_lite_timestamp()
    );

    // Create job entry
    let job = ProofJob {
        id: proof_id.clone(),
        batch_id: req.batch_id,
        status: ProofJobStatus::Pending,
        proof: None,
        error: None,
    };
    state.add_job(job).await;

    // Spawn background task to generate proof
    let state_clone = state.clone();
    let proof_id_clone = proof_id.clone();
    let batch_id = req.batch_id;

    tokio::spawn(async move {
        info!(
            "Starting proof generation for job: {}, batch_id: {}",
            proof_id_clone, batch_id
        );

        // Update status to proving
        state_clone
            .update_job(&proof_id_clone, ProofJobStatus::Proving, None, None)
            .await;

        // TODO: Fetch actual GuestInput from provider based on batch_id
        // For now, create a placeholder input
        let input = raiko2_primitives::GuestInput::default();
        let prover_config = raiko2_primitives::ProverConfig::default();

        // Generate proof using the prover
        match state_clone.prover.prove(input, &prover_config).await {
            Ok(proof) => {
                info!("Proof generated successfully for job: {}", proof_id_clone);
                state_clone
                    .update_job(
                        &proof_id_clone,
                        ProofJobStatus::Completed,
                        proof.proof,
                        None,
                    )
                    .await;
            }
            Err(e) => {
                error!(
                    "Proof generation failed for job {}: {:?}",
                    proof_id_clone, e
                );
                state_clone
                    .update_job(
                        &proof_id_clone,
                        ProofJobStatus::Failed,
                        None,
                        Some(e.to_string()),
                    )
                    .await;
            }
        }
    });

    Ok(Json(ProofResponse {
        id: proof_id,
        status: ProofStatus::Pending,
    }))
}

/// Get proof status.
pub async fn get_proof_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ProofStatusResponse>, ApiError> {
    info!("Getting proof status for: {}", id);

    match state.get_job(&id).await {
        Some(job) => Ok(Json(ProofStatusResponse {
            id: job.id,
            status: job.status.into(),
            proof: job.proof,
            error: job.error,
        })),
        None => Err(ApiError {
            status: StatusCode::NOT_FOUND,
            message: format!("Proof job not found: {}", id),
        }),
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

/// Cancel proof request.
pub async fn cancel_proof(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ProofStatusResponse>, ApiError> {
    info!("Cancelling proof: {}", id);

    match state.get_job(&id).await {
        Some(job) => {
            // Only cancel if still pending or proving
            if job.status == ProofJobStatus::Pending || job.status == ProofJobStatus::Proving {
                state
                    .update_job(&id, ProofJobStatus::Cancelled, None, None)
                    .await;
            }
            let updated_job = state.get_job(&id).await.unwrap();
            Ok(Json(ProofStatusResponse {
                id: updated_job.id,
                status: updated_job.status.into(),
                proof: updated_job.proof,
                error: updated_job.error,
            }))
        }
        None => Err(ApiError {
            status: StatusCode::NOT_FOUND,
            message: format!("Proof job not found: {}", id),
        }),
    }
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

/// Simple timestamp for proof IDs (no external dependency).
fn chrono_lite_timestamp() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
