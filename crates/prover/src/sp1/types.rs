use alloy_primitives::B256;
use raiko2_primitives::Proof;
use serde::{Deserialize, Serialize};
use sp1_sdk::{SP1ProofMode, SP1ProofWithPublicValues, SP1VerifyingKey};
use tracing::error;

/// SP1 prover configuration parameters.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sp1Config {
    /// Proof mode (Core, Compressed, Plonk).
    #[serde(default)]
    pub recursion: RecursionMode,
    /// Prover mode (Mock, Local, Network).
    pub prover: Option<ProverMode>,
    /// Whether to verify the proof after generation.
    #[serde(default = "default_true")]
    pub verify: bool,
}

const fn default_true() -> bool {
    true
}

impl Default for Sp1Config {
    fn default() -> Self {
        Self {
            recursion: RecursionMode::Plonk,
            prover: None,
            verify: true,
        }
    }
}

/// SP1 proof recursion mode.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RecursionMode {
    /// Core proof (no recursion).
    Core,
    /// Compressed proof.
    Compressed,
    /// Plonk proof (on-chain verifiable).
    #[default]
    Plonk,
}

impl From<RecursionMode> for SP1ProofMode {
    fn from(value: RecursionMode) -> Self {
        match value {
            RecursionMode::Core => SP1ProofMode::Core,
            RecursionMode::Compressed => SP1ProofMode::Compressed,
            RecursionMode::Plonk => SP1ProofMode::Plonk,
        }
    }
}

/// SP1 prover mode.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ProverMode {
    /// Mock prover for testing.
    Mock,
    /// Local CPU prover.
    Local,
    /// Network prover (Succinct network).
    Network,
}

/// SP1 proof response.
#[derive(Clone, Serialize, Deserialize)]
pub struct Sp1Response {
    /// Hex-encoded serialized proof
    pub proof: Option<String>,
    /// Verifying key hash (bytes32)
    pub vkey_hash: Option<String>,
    /// Public input commitment
    pub input: B256,
    /// For aggregation
    pub sp1_proof: Option<SP1ProofWithPublicValues>,
    /// Verifying key for verification
    #[serde(skip)]
    pub vkey: Option<SP1VerifyingKey>,
    /// Additional fork/backend metadata.
    pub extra_data: Option<serde_json::Value>,
}

impl From<Sp1Response> for Proof {
    fn from(value: Sp1Response) -> Self {
        let quote = match value.sp1_proof.as_ref() {
            Some(proof) => match serde_json::to_string(&proof.proof) {
                Ok(serialized) => Some(serialized),
                Err(err) => {
                    error!(error = %err, "failed to serialize sp1 proof");
                    None
                }
            },
            None => None,
        };

        Self {
            proof: value.proof,
            quote,
            input: Some(value.input),
            uuid: value.vkey_hash,
            kzg_proof: None,
            extra_data: value.extra_data,
        }
    }
}
