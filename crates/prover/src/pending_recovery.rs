use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::num::NonZeroU32;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkProverBackend {
    Boundless,
    Sp1,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PendingProofCheckpoint {
    pub backend: NetworkProverBackend,
    pub attempt: NonZeroU32,
    pub submitted_at_secs: u64,
    pub deadline_secs: u64,
    pub revision: u64,
    pub payload: Box<RawValue>,
}

#[derive(Debug, thiserror::Error)]
pub enum PendingProofRecoveryError {
    #[error("checkpoint backend {actual:?} does not match adapter {expected:?}")]
    BackendMismatch {
        expected: NetworkProverBackend,
        actual: NetworkProverBackend,
    },
    #[error("invalid {backend:?} checkpoint: {message}")]
    InvalidCheckpoint {
        backend: NetworkProverBackend,
        message: String,
    },
}

impl PendingProofCheckpoint {
    /// Builds a versioned backend checkpoint from a serializable provider payload.
    ///
    /// # Errors
    ///
    /// Returns [`PendingProofRecoveryError::InvalidCheckpoint`] when the payload cannot be
    /// serialized as JSON.
    pub fn from_payload<T: Serialize>(
        backend: NetworkProverBackend,
        attempt: NonZeroU32,
        submitted_at_secs: u64,
        deadline_secs: u64,
        revision: u64,
        payload: &T,
    ) -> Result<Self, PendingProofRecoveryError> {
        let payload = serde_json::value::to_raw_value(payload).map_err(|error| {
            PendingProofRecoveryError::InvalidCheckpoint {
                backend,
                message: error.to_string(),
            }
        })?;
        Ok(Self {
            backend,
            attempt,
            submitted_at_secs,
            deadline_secs,
            revision,
            payload,
        })
    }

    /// Decodes the provider-specific payload stored in this checkpoint.
    ///
    /// # Errors
    ///
    /// Returns [`PendingProofRecoveryError::InvalidCheckpoint`] when the stored JSON does not
    /// match the requested payload type.
    pub fn decode_payload<T: serde::de::DeserializeOwned>(
        &self,
    ) -> Result<T, PendingProofRecoveryError> {
        serde_json::from_str(self.payload.get()).map_err(|error| {
            PendingProofRecoveryError::InvalidCheckpoint {
                backend: self.backend,
                message: error.to_string(),
            }
        })
    }
}
