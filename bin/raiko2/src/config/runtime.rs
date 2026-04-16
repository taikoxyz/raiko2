use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Runtime workspace configuration owned by the host.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub root: PathBuf,
    #[serde(default = "default_inactive_ttl_secs")]
    pub inactive_ttl_secs: u64,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("./data/runtime"),
            inactive_ttl_secs: default_inactive_ttl_secs(),
        }
    }
}

impl RuntimeConfig {
    /// # Errors
    ///
    /// Returns an error if the configured runtime root is empty.
    pub fn validate(&self) -> Result<()> {
        if self.root.as_os_str().is_empty() {
            bail!("runtime.root must not be empty");
        }
        Ok(())
    }
}

const fn default_inactive_ttl_secs() -> u64 {
    7_200
}
