use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use super::validation;

/// Validate that a string is a valid URL.
pub(crate) fn is_valid_url(url: &str) -> bool {
    url.starts_with("http://")
        || url.starts_with("https://")
        || url.starts_with("ws://")
        || url.starts_with("wss://")
}

/// RPC configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcConfig {
    pub l1_rpc: String,
    pub l2_rpc: String,
    pub l1_chain_id: u64,
    pub l2_chain_id: u64,
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            l1_rpc: "http://localhost:8545".to_string(),
            l2_rpc: "http://localhost:9545".to_string(),
            l1_chain_id: 1,
            l2_chain_id: 167_000,
        }
    }
}

impl RpcConfig {
    /// Validate RPC configuration.
    pub fn validate(&self) -> Result<()> {
        if !is_valid_url(&self.l1_rpc) {
            bail!(
                "{}: l1_rpc = '{}'",
                validation::INVALID_RPC_URL,
                self.l1_rpc
            );
        }
        if !is_valid_url(&self.l2_rpc) {
            bail!(
                "{}: l2_rpc = '{}'",
                validation::INVALID_RPC_URL,
                self.l2_rpc
            );
        }
        if self.l1_chain_id == 0 {
            bail!("{}: l1_chain_id", validation::INVALID_CHAIN_ID);
        }
        if self.l2_chain_id == 0 {
            bail!("{}: l2_chain_id", validation::INVALID_CHAIN_ID);
        }
        Ok(())
    }
}
