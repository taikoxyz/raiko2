//! TDX prover configuration and response types.

use alloy_primitives::B256;
use raiko2_primitives::Proof;
use serde::{Deserialize, Serialize};

/// TDX prover configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TdxConfig {
    /// On-chain verifier instance ID.
    pub instance_id: u32,
    /// Path to the TDX attestation service Unix socket.
    #[serde(default = "default_socket_path")]
    pub socket_path: String,
}

fn default_socket_path() -> String {
    "/var/tdxs.sock".to_string()
}

impl Default for TdxConfig {
    fn default() -> Self {
        Self {
            instance_id: 0,
            socket_path: default_socket_path(),
        }
    }
}

/// TDX proof response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TdxResponse {
    /// Hex-encoded proof bytes (89 bytes: `instance_id` + address + signature).
    pub proof: String,
    /// Hex-encoded TDX attestation quote.
    pub quote: String,
    /// Public input hash (`instance_hash` or `aggregation_hash`).
    pub input: B256,
    /// Encoded proof carry data for aggregation.
    pub extra_data: Option<serde_json::Value>,
}

impl From<TdxResponse> for Proof {
    fn from(value: TdxResponse) -> Self {
        Self {
            proof: Some(value.proof),
            quote: Some(value.quote),
            input: Some(value.input),
            uuid: None,
            kzg_proof: None,
            extra_data: value.extra_data,
        }
    }
}
