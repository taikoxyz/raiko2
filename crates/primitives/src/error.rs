//! Error types for raiko2.

use crate::proof::ProverError;
use reth_stateless::validation::StatelessValidationError;
use std::io;
use utoipa::ToSchema;

/// Main error type for Raiko operations.
#[derive(Debug, thiserror::Error, ToSchema)]
pub enum RaikoError {
    /// For invalid proof type generation request.
    #[error("Unknown proof type: {0}")]
    InvalidProofType(String),

    /// For invalid blob option.
    #[error("Invalid blob option: {0}")]
    InvalidBlobOption(String),

    /// For invalid proof request configuration.
    #[error("Invalid proof request: {0}")]
    InvalidRequestConfig(String),

    /// For requesting a proof of a type that is not supported.
    #[error("Feature not supported: {0}")]
    #[schema(value_type = Value)]
    FeatureNotSupportedError(String),

    /// For invalid type conversion.
    #[error("Invalid conversion: {0}")]
    Conversion(String),

    /// For RPC errors with context.
    #[error("RPC error ({context}): {message}")]
    RpcWithContext {
        /// The RPC method or operation that failed.
        context: String,
        /// The error message.
        message: String,
    },

    /// For RPC errors.
    #[error("There was an error with the RPC provider: {0}")]
    RPC(String),

    /// For preflight errors.
    #[error("There was an error running the preflight: {0}")]
    Preflight(String),

    /// For errors produced by the guest provers.
    #[error("There was an error with a guest prover: {0}")]
    #[schema(value_type = Value)]
    Guest(#[from] ProverError),

    /// For I/O errors with path context.
    #[error("I/O error on '{path}': {message}")]
    IoWithPath {
        /// The file path involved.
        path: String,
        /// The error message.
        message: String,
    },

    /// For I/O errors.
    #[error("There was an I/O error: {0}")]
    Io(String),

    /// For serialization errors with format context.
    #[error("Serialization error ({format}): {message}")]
    SerializationWithFormat {
        /// The serialization format (json, bincode, etc).
        format: String,
        /// The error message.
        message: String,
    },

    /// For serialization errors.
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// For anyhow errors.
    #[error("Error: {0}")]
    Anyhow(String),

    /// For stateless validation errors with details.
    #[error("Stateless validation failed: {reason}")]
    StatelessValidationDetailed {
        /// The validation failure reason.
        reason: String,
        /// Optional block number context.
        block_number: Option<u64>,
    },

    /// For stateless validation errors.
    #[error("Stateless validation error: {0}")]
    StatelessValidation(String),

    /// For configuration errors.
    #[error("Configuration error: {0}")]
    Configuration(String),

    /// For provider/data fetching errors.
    #[error("Provider error: {0}")]
    Provider(String),
}

impl RaikoError {
    /// Create an RPC error with context.
    pub fn rpc_with_context(context: impl Into<String>, message: impl Into<String>) -> Self {
        RaikoError::RpcWithContext {
            context: context.into(),
            message: message.into(),
        }
    }

    /// Create an I/O error with path context.
    pub fn io_with_path(path: impl Into<String>, message: impl Into<String>) -> Self {
        RaikoError::IoWithPath {
            path: path.into(),
            message: message.into(),
        }
    }

    /// Create a serialization error with format context.
    pub fn serialization_with_format(
        format: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        RaikoError::SerializationWithFormat {
            format: format.into(),
            message: message.into(),
        }
    }

    /// Create a detailed stateless validation error.
    pub fn stateless_validation_detailed(
        reason: impl Into<String>,
        block_number: Option<u64>,
    ) -> Self {
        RaikoError::StatelessValidationDetailed {
            reason: reason.into(),
            block_number,
        }
    }
}

impl From<io::Error> for RaikoError {
    fn from(e: io::Error) -> Self {
        RaikoError::Io(e.to_string())
    }
}

impl From<serde_json::Error> for RaikoError {
    fn from(e: serde_json::Error) -> Self {
        RaikoError::SerializationWithFormat {
            format: "json".to_string(),
            message: e.to_string(),
        }
    }
}

impl From<anyhow::Error> for RaikoError {
    fn from(e: anyhow::Error) -> Self {
        RaikoError::Anyhow(e.to_string())
    }
}

impl From<StatelessValidationError> for RaikoError {
    fn from(e: StatelessValidationError) -> Self {
        // Extract more meaningful information from the error
        let reason = match &e {
            StatelessValidationError::SignerRecovery => {
                "Failed to recover transaction signer".to_string()
            }
            StatelessValidationError::MissingAncestorHeader => {
                "Missing ancestor header in witness".to_string()
            }
            StatelessValidationError::InvalidAncestorChain => {
                "Invalid ancestor chain: headers are not contiguous".to_string()
            }
            _ => format!("{:?}", e),
        };
        RaikoError::StatelessValidationDetailed {
            reason,
            block_number: None,
        }
    }
}

/// Result type for Raiko operations.
pub type RaikoResult<T> = Result<T, RaikoError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = RaikoError::InvalidProofType("unknown".to_string());
        assert!(err.to_string().contains("Unknown proof type"));
        assert!(err.to_string().contains("unknown"));
    }

    #[test]
    fn test_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let err: RaikoError = io_err.into();
        assert!(matches!(err, RaikoError::Io(_)));
    }

    #[test]
    fn test_error_from_serde() {
        let json_err = serde_json::from_str::<()>("invalid").unwrap_err();
        let err: RaikoError = json_err.into();
        assert!(matches!(err, RaikoError::SerializationWithFormat { .. }));
        assert!(err.to_string().contains("json"));
    }

    #[test]
    fn test_error_from_anyhow() {
        let anyhow_err = anyhow::anyhow!("test error");
        let err: RaikoError = anyhow_err.into();
        assert!(matches!(err, RaikoError::Anyhow(_)));
        assert!(err.to_string().contains("test error"));
    }

    #[test]
    fn test_rpc_with_context() {
        let err = RaikoError::rpc_with_context("eth_getBlockByNumber", "connection refused");
        assert!(err.to_string().contains("eth_getBlockByNumber"));
        assert!(err.to_string().contains("connection refused"));
    }

    #[test]
    fn test_io_with_path() {
        let err = RaikoError::io_with_path("/etc/config.toml", "file not found");
        assert!(err.to_string().contains("/etc/config.toml"));
        assert!(err.to_string().contains("file not found"));
    }

    #[test]
    fn test_serialization_with_format() {
        let err = RaikoError::serialization_with_format("bincode", "invalid data");
        assert!(err.to_string().contains("bincode"));
        assert!(err.to_string().contains("invalid data"));
    }

    #[test]
    fn test_stateless_validation_detailed() {
        let err = RaikoError::stateless_validation_detailed("state root mismatch", Some(12345));
        if let RaikoError::StatelessValidationDetailed {
            reason,
            block_number,
        } = err
        {
            assert_eq!(reason, "state root mismatch");
            assert_eq!(block_number, Some(12345));
        } else {
            panic!("Expected StatelessValidationDetailed");
        }
    }

    #[test]
    fn test_all_error_variants() {
        // Ensure all error variants have proper Display impl
        let errors: Vec<RaikoError> = vec![
            RaikoError::InvalidProofType("test".into()),
            RaikoError::InvalidBlobOption("test".into()),
            RaikoError::InvalidRequestConfig("test".into()),
            RaikoError::FeatureNotSupportedError("test".into()),
            RaikoError::Conversion("test".into()),
            RaikoError::RpcWithContext {
                context: "test".into(),
                message: "test".into(),
            },
            RaikoError::RPC("test".into()),
            RaikoError::Preflight("test".into()),
            RaikoError::IoWithPath {
                path: "test".into(),
                message: "test".into(),
            },
            RaikoError::Io("test".into()),
            RaikoError::SerializationWithFormat {
                format: "test".into(),
                message: "test".into(),
            },
            RaikoError::Serialization("test".into()),
            RaikoError::Anyhow("test".into()),
            RaikoError::StatelessValidationDetailed {
                reason: "test".into(),
                block_number: Some(1),
            },
            RaikoError::StatelessValidation("test".into()),
            RaikoError::Configuration("test".into()),
            RaikoError::Provider("test".into()),
        ];

        for err in errors {
            // Each error should have a non-empty display string
            assert!(!err.to_string().is_empty());
        }
    }
}
