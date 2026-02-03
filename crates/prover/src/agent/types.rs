use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AsyncProofRequestData {
    pub prover_type: String,
    pub input: Vec<u8>,
    pub output: Vec<u8>,
    pub proof_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elf: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
}

impl AsyncProofRequestData {
    #[must_use]
    pub fn new(prover_type: &str, proof_type: &str, input: Vec<u8>, output: Vec<u8>) -> Self {
        Self {
            prover_type: prover_type.to_string(),
            input,
            output,
            proof_type: proof_type.to_string(),
            elf: None,
            config: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AsyncProofResponse {
    pub request_id: String,
    pub prover_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_request_id: Option<String>,
    pub status: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StatusResponse {
    pub request_id: String,
    pub prover_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_request_id: Option<String>,
    pub status: String,
    pub status_message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proof_data: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
