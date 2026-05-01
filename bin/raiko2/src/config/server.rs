use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::fmt;

use super::validation;

/// Server configuration.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub admin_api_key: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8080,
            admin_api_key: None,
        }
    }
}

impl fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let admin_api_key = self.admin_api_key.as_ref().map(|_| "<redacted>");
        f.debug_struct("ServerConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("admin_api_key", &admin_api_key)
            .finish()
    }
}

impl ServerConfig {
    /// Validate server configuration.
    pub fn validate(&self) -> Result<()> {
        if self.host.is_empty() {
            bail!("{}", validation::INVALID_HOST);
        }
        if self.port == 0 {
            bail!("{}", validation::INVALID_PORT);
        }
        if self
            .admin_api_key
            .as_ref()
            .is_some_and(std::string::String::is_empty)
        {
            bail!("server.admin_api_key must not be empty when set");
        }
        Ok(())
    }
}
