use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

/// Optional preflight behavior configured at server startup.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreflightConfig {
    /// When set, preflight cross-checks proposal boundary blocks against this L2 RPC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify_checkpoint_l2_rpc: Option<String>,
}

impl PreflightConfig {
    /// # Errors
    ///
    /// Returns an error when the configured verification RPC URL is invalid.
    pub fn validate(&self) -> Result<()> {
        let Some(url) = self.verify_checkpoint_l2_rpc.as_deref() else {
            return Ok(());
        };
        let url = url.trim();
        if url.is_empty() {
            bail!("preflight.verify_checkpoint_l2_rpc must not be empty when set");
        }
        if !super::rpc::is_valid_url(url) {
            bail!("preflight.verify_checkpoint_l2_rpc is not a valid URL: {url}");
        }
        Ok(())
    }
}
