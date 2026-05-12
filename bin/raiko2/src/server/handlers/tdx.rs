//! TDX-specific HTTP handlers.

use axum::Json;
use serde_json::Value;

use super::errors::ApiError;

/// Return on-disk TDX bootstrap data (public key, attestation quote, nonce, metadata).
///
/// This data is generated once at server startup by `TdxProver::ensure_bootstrapped`
/// and persisted to `~/.config/raiko2/tdx/bootstrap.json`. It is intended to be
/// publicly inspectable so that operators can register the prover on-chain.
pub async fn bootstrap() -> Result<Json<Value>, ApiError> {
    let data = raiko2_prover::tdx::guest_data_from_bootstrap()
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(data))
}
