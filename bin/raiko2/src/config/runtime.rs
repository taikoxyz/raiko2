use anyhow::{Result, bail};
use raiko2_runtime::validate_environment_id;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactStoreBackend {
    #[default]
    Filesystem,
    Gcs,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactStoreConfig {
    #[serde(default)]
    pub backend: ArtifactStoreBackend,
    pub bucket: Option<String>,
    #[serde(default)]
    pub prefix: String,
}

/// Runtime workspace configuration owned by the host.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    pub root: PathBuf,
    pub environment_id: String,
    #[serde(default)]
    pub artifact_store: ArtifactStoreConfig,
    #[serde(default = "default_inactive_ttl_secs")]
    pub inactive_ttl_secs: u64,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("./data/runtime"),
            environment_id: "local".to_string(),
            artifact_store: ArtifactStoreConfig::default(),
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
        validate_environment_id(&self.environment_id)?;
        match self.artifact_store.backend {
            ArtifactStoreBackend::Filesystem => {
                if self.artifact_store.bucket.is_some() {
                    bail!("runtime.artifact_store.bucket is only valid for backend=gcs");
                }
                if !self.artifact_store.prefix.is_empty() {
                    bail!("runtime.artifact_store.prefix is only valid for backend=gcs");
                }
            }
            ArtifactStoreBackend::Gcs => {
                let bucket = self.artifact_store.bucket.as_deref().unwrap_or_default();
                if bucket.trim().is_empty() {
                    bail!("runtime.artifact_store.bucket must be set for backend=gcs");
                }
            }
        }
        Ok(())
    }
}

const fn default_inactive_ttl_secs() -> u64 {
    7_200
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_id_is_required_in_runtime_config() {
        let error = toml::from_str::<RuntimeConfig>("root = './runtime'")
            .expect_err("environment_id must be explicit");
        assert!(error.to_string().contains("environment_id"));
    }

    #[test]
    fn gcs_backend_requires_bucket() {
        let config = RuntimeConfig {
            artifact_store: ArtifactStoreConfig {
                backend: ArtifactStoreBackend::Gcs,
                ..ArtifactStoreConfig::default()
            },
            ..RuntimeConfig::default()
        };

        let error = config.validate().expect_err("GCS bucket must be required");
        assert!(error.to_string().contains("bucket"));
    }

    #[test]
    fn filesystem_backend_rejects_gcs_bucket() {
        let config = RuntimeConfig {
            artifact_store: ArtifactStoreConfig {
                bucket: Some("unexpected".to_string()),
                ..ArtifactStoreConfig::default()
            },
            ..RuntimeConfig::default()
        };

        let error = config
            .validate()
            .expect_err("filesystem backend must reject bucket");
        assert!(error.to_string().contains("only valid for backend=gcs"));
    }
}
