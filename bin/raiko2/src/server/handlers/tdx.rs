//! TDX-specific HTTP handlers.

use axum::Json;
use serde_json::Value;

use super::errors::ApiError;

/// Return on-disk TDX bootstrap data (public key, attestation quote, nonce, metadata).
///
/// The bootstrap file (`~/.config/raiko2/tdx/bootstrap.json`) is written by
/// `TdxProver::ensure_bootstrapped`, which runs only when the server is configured
/// for the `tdx/local` route. Returns 404 if the file has not been generated yet.
/// The data is publicly inspectable so operators can register the prover on-chain.
pub async fn bootstrap() -> Result<Json<Value>, ApiError> {
    let exists = raiko2_prover::tdx::config::bootstrap_exists()
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if !exists {
        return Err(ApiError::not_found(
            "TDX bootstrap not found — configure guest_system=tdx/runner=local and \
             ensure the tdxs attestation daemon is running so the prover can bootstrap",
        ));
    }
    let data = raiko2_prover::tdx::guest_data_from_bootstrap()
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(data))
}
