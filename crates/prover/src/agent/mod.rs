#![allow(missing_docs)]

pub mod aggregation;
pub mod types;

use crate::agent::types::{AsyncProofRequestData, AsyncProofResponse, StatusResponse};
use raiko2_primitives::{RaikoError, RaikoResult};

#[derive(Clone, Debug)]
pub struct AgentConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub poll_interval_ms: u64,
    pub timeout_ms: u64,
}

#[derive(Clone)]
pub struct AgentClient {
    config: AgentConfig,
}

impl AgentClient {
    pub fn new(config: AgentConfig) -> Self {
        Self { config }
    }

    pub async fn submit_proof(
        &self,
        _request: &AsyncProofRequestData,
    ) -> RaikoResult<AsyncProofResponse> {
        Err(RaikoError::InvalidRequestConfig(
            "Agent client not implemented".to_string(),
        ))
    }

    pub async fn poll_status(&self, _request_id: &str) -> RaikoResult<StatusResponse> {
        Err(RaikoError::InvalidRequestConfig(
            "Agent client not implemented".to_string(),
        ))
    }
}
