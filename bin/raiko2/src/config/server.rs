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
    pub acl: ServerAclConfig,
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ServerAclConfig {
    pub keys: Vec<ServerAclKey>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerAclKey {
    pub id: String,
    pub key: String,
    #[serde(default)]
    pub allow: Vec<ServerAclFeature>,
    #[serde(default)]
    pub rate_limit_per_minute: Option<u32>,
}

#[derive(Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
pub enum ServerAclFeature {
    #[serde(rename = "admin")]
    Admin,
    #[serde(rename = "admin.ballot.read")]
    AdminBallotRead,
    #[serde(rename = "admin.ballot.write")]
    AdminBallotWrite,
    #[serde(rename = "prover.submit")]
    ProverSubmit,
    #[serde(rename = "prover.clear")]
    ProverClear,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8080,
            acl: ServerAclConfig::default(),
        }
    }
}

impl fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let acl_keys = if self.acl.keys.is_empty() {
            "[]".to_string()
        } else {
            format!("{} <redacted> key(s)", self.acl.keys.len())
        };
        f.debug_struct("ServerConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("acl.keys", &acl_keys)
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
        self.acl.validate()?;
        Ok(())
    }
}

impl ServerAclConfig {
    fn validate(&self) -> Result<()> {
        for acl_key in &self.keys {
            if acl_key.id.is_empty() {
                bail!("server.acl.keys[].id must not be empty");
            }
            if acl_key.key.is_empty() {
                bail!("server.acl.keys[].key must not be empty");
            }
            if acl_key.allow.is_empty() {
                bail!("server.acl.keys[].allow must not be empty");
            }
            if acl_key.rate_limit_per_minute == Some(0) {
                bail!("server.acl.keys[].rate_limit_per_minute must be > 0 when set");
            }
        }
        Ok(())
    }
}
