use anyhow::{Result, bail};
use raiko2_runtime::validate_scope_component;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeStoreBackend {
    #[default]
    Memory,
    Gcs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeStoreConfig {
    pub backend: RuntimeStoreBackend,
    pub bucket: Option<String>,
    #[serde(default = "default_prefix")]
    pub prefix: String,
}

impl Default for RuntimeStoreConfig {
    fn default() -> Self {
        Self {
            backend: RuntimeStoreBackend::Memory,
            bucket: None,
            prefix: default_prefix(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    pub environment: String,
    pub namespace: String,
    pub store: RuntimeStoreConfig,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            environment: "development".to_string(),
            namespace: "raiko2-development".to_string(),
            store: RuntimeStoreConfig::default(),
        }
    }
}

impl RuntimeConfig {
    pub fn validate(&self) -> Result<()> {
        validate_scope_component("runtime.environment", &self.environment)?;
        validate_scope_component("runtime.namespace", &self.namespace)?;
        match self.store.backend {
            RuntimeStoreBackend::Memory => {
                if self.store.bucket.is_some() {
                    bail!("runtime.store.bucket is only valid for backend=gcs");
                }
                if !matches!(self.environment.as_str(), "development" | "local" | "test") {
                    bail!(
                        "runtime.store.backend=memory is only supported for development, local, or test environments"
                    );
                }
            }
            RuntimeStoreBackend::Gcs => {
                if self
                    .store
                    .bucket
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                {
                    bail!("runtime.store.bucket must be set for backend=gcs");
                }
                if self.store.prefix.trim().is_empty() {
                    bail!("runtime.store.prefix must not be empty for backend=gcs");
                }
            }
        }
        Ok(())
    }
}

fn default_prefix() -> String {
    "raiko2/runtime/v1".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gcs_requires_bucket() {
        let config = RuntimeConfig {
            store: RuntimeStoreConfig {
                backend: RuntimeStoreBackend::Gcs,
                ..RuntimeStoreConfig::default()
            },
            ..RuntimeConfig::default()
        };
        assert!(
            config
                .validate()
                .unwrap_err()
                .to_string()
                .contains("bucket")
        );
    }

    #[test]
    fn environment_and_namespace_are_distinct_required_scopes() {
        let config = RuntimeConfig {
            environment: "devnet".into(),
            namespace: "raiko2-devnet-a".into(),
            store: RuntimeStoreConfig {
                backend: RuntimeStoreBackend::Gcs,
                bucket: Some("runtime-state".into()),
                ..RuntimeStoreConfig::default()
            },
        };
        config.validate().expect("valid runtime scope");
    }

    #[test]
    fn memory_store_is_rejected_for_deployed_environments() {
        let config = RuntimeConfig {
            environment: "mainnet".into(),
            namespace: "raiko2-mainnet".into(),
            ..RuntimeConfig::default()
        };

        assert!(
            config
                .validate()
                .unwrap_err()
                .to_string()
                .contains("backend=memory")
        );
    }
}
