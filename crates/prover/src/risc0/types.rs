use alloy_primitives::B256;
use raiko2_primitives::Proof;
use serde::{Deserialize, Serialize};

/// RISC0 prover configuration parameters.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Risc0Config {
    /// Whether to use Bonsai proving service.
    pub bonsai: bool,
    /// Whether to generate SNARK proof.
    pub snark: bool,
    /// Whether to enable profiling.
    pub profile: bool,
    /// Execution power of 2 (for cycle limit).
    pub execution_po2: u32,
    /// Whether to verify the proof after generation.
    #[serde(default = "default_true")]
    pub verify: bool,
}

const fn default_true() -> bool {
    true
}

impl Default for Risc0Config {
    fn default() -> Self {
        Self {
            bonsai: true,
            snark: true,
            profile: false,
            execution_po2: 20,
            verify: true,
        }
    }
}

/// RISC0 proof response.
#[derive(Clone, Serialize, Deserialize)]
pub struct Risc0Response {
    /// Hex-encoded proof (journal bytes)
    pub proof: String,
    /// JSON-serialized receipt
    pub receipt: String,
    /// Image ID of the guest program
    pub image_id: String,
    /// Public input hash
    pub input: B256,
}

impl From<Risc0Response> for Proof {
    fn from(value: Risc0Response) -> Self {
        Self {
            proof: Some(value.proof),
            quote: Some(value.receipt),
            input: Some(value.input),
            uuid: Some(value.image_id),
            kzg_proof: None,
            extra_data: None,
        }
    }
}
