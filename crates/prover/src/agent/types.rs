use alloy_primitives::hex::encode_prefixed;
use raiko2_primitives::{RaikoError, RaikoResult};
use raiko2_protocol_shasta::shasta::ProofCarryData;
use serde::{Deserialize, Serialize};

pub const RAIKO2_INPUT_ENCODING_KEY: &str = "raiko2_input_encoding";
pub const RAIKO2_RISC0_SHASTA_BATCH_V1: &str = "risc0_shasta_batch_v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Raiko2InputEncodingConfig {
    pub kind: String,
    pub proof_carry_data: String,
}

impl Raiko2InputEncodingConfig {
    pub fn risc0_shasta_batch(proof_carry_data: &ProofCarryData) -> RaikoResult<Self> {
        let proof_carry_data = bincode::serialize(proof_carry_data).map_err(|e| {
            RaikoError::InvalidRequestConfig(format!(
                "Failed to encode ProofCarryData for agent request: {e}"
            ))
        })?;
        Ok(Self {
            kind: RAIKO2_RISC0_SHASTA_BATCH_V1.to_string(),
            proof_carry_data: encode_prefixed(proof_carry_data),
        })
    }
}

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

    pub fn with_raiko2_input_encoding(
        mut self,
        encoding: Raiko2InputEncodingConfig,
    ) -> RaikoResult<Self> {
        let mut config = self.config.unwrap_or_default();
        let value = serde_json::to_value(encoding).map_err(|e| {
            RaikoError::InvalidRequestConfig(format!(
                "Failed to serialize agent input encoding config: {e}"
            ))
        })?;
        config[RAIKO2_INPUT_ENCODING_KEY] = value;
        self.config = Some(config);
        Ok(self)
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, B256};
    use raiko2_protocol_shasta::shasta::{
        ProofCarryData, ShastaTransitionInput, TransitionInputData,
    };

    #[test]
    fn raiko2_batch_input_encoding_serializes_proof_carry_data_hex() {
        let proof_carry_data = ProofCarryData {
            chain_id: 167_013,
            verifier: Address::repeat_byte(0x11),
            transition_input: TransitionInputData {
                proposal_id: 7,
                proposal_hash: B256::repeat_byte(0x22),
                parent_proposal_hash: B256::repeat_byte(0x33),
                parent_block_hash: B256::repeat_byte(0x44),
                actual_prover: Address::repeat_byte(0x55),
                transition: ShastaTransitionInput {
                    proposer: Address::repeat_byte(0x66),
                    timestamp: 123,
                },
                checkpoint: Default::default(),
            },
        };

        let request = AsyncProofRequestData::new("boundless", "batch", vec![1, 2], Vec::new())
            .with_raiko2_input_encoding(
                Raiko2InputEncodingConfig::risc0_shasta_batch(&proof_carry_data)
                    .expect("encoding config"),
            )
            .expect("request config");

        let config = request.config.expect("config should be present");
        let encoding = config
            .get(RAIKO2_INPUT_ENCODING_KEY)
            .expect("encoding key should exist");
        assert_eq!(encoding["kind"], RAIKO2_RISC0_SHASTA_BATCH_V1);
        assert!(
            encoding["proof_carry_data"]
                .as_str()
                .expect("hex string")
                .starts_with("0x")
        );
    }
}
