use alloy_primitives::B256;

pub use alloy_rpc_types_debug::ExecutionWitness;
use reth_consensus::ConsensusError;

/// Errors that can occur during stateless validation.
#[derive(Debug, thiserror::Error)]
pub enum StatelessValidationError {
    #[error("ancestor header count ({count}) exceeds limit ({limit})")]
    AncestorHeaderLimitExceeded { count: usize, limit: usize },

    #[error("invalid ancestor chain")]
    InvalidAncestorChain,

    #[error("failed to reveal witness data for pre-state root {pre_state_root}")]
    WitnessRevealFailed { pre_state_root: B256 },

    #[error("stateless block execution failed: {0}")]
    StatelessExecutionFailed(String),

    #[error("consensus validation failed: {0}")]
    ConsensusValidationFailed(#[from] ConsensusError),

    #[error("stateless state root calculation failed")]
    StatelessStateRootCalculationFailed,

    #[error("stateless pre-state root calculation failed")]
    StatelessPreStateRootCalculationFailed,

    #[error("missing required ancestor headers")]
    MissingAncestorHeader,

    #[error("could not deserialize ancestor headers")]
    HeaderDeserializationFailed,

    #[error("mismatched post-state root: {got}\n {expected}")]
    PostStateRootMismatch { got: B256, expected: B256 },

    #[error("mismatched pre-state root: {got}\n {expected}")]
    PreStateRootMismatch { got: B256, expected: B256 },

    #[error("signer recovery failed")]
    SignerRecovery,

    #[error("signature s value not normalized for homestead block")]
    HomesteadSignatureNotNormalized,

    #[error("{0}")]
    Custom(&'static str),
}
